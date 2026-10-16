//! Unified Write Filter (UWF) management for Windows 10 **Pro**.
//!
//! Background: UWF ships as an optional Windows feature
//! (`Client-UnifiedWriteFilter`). It is usable on Pro once that feature is
//! installed — no edition change to Enterprise/Education is required. This is
//! the approach of the referenced UWF-Manager project
//! (https://github.com/JIRBOY/UWF-Manager).
//!
//! State is **read** through the documented WMI namespace
//! `root\standardcimv2\embedded` (class `UWF_Filter`). State is **changed** via
//! the in-box `uwfmgr.exe` CLI, which is present once the feature is installed.
//!
//! Turning protection on from scratch takes two reboots: the first completes
//! the feature install (uwfmgr cannot be used before it), then the agent's
//! startup policy run enables the filter, which applies on the second reboot.
//! [`state`] reports where a machine is in that sequence.

use anyhow::Result;
use labctl_common::policy::UwfScope;

#[cfg(windows)]
mod imp {
    use super::*;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use wmi::{COMLibrary, WMIConnection};

    const NS: &str = "root\\standardcimv2\\embedded";
    const FEATURE: &str = "Client-UnifiedWriteFilter";

    /// A background apply (possibly a long DISM install) is running.
    static BUSY: AtomicBool = AtomicBool::new(false);
    /// Error from the most recent apply; cleared when one succeeds.
    static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

    #[derive(Deserialize, Debug)]
    #[serde(rename = "UWF_Filter")]
    #[allow(non_snake_case)]
    struct UwfFilter {
        /// Whether UWF is protecting the volume in the *current* session.
        CurrentEnabled: Option<bool>,
        /// Whether UWF will be enabled after the next reboot.
        NextEnabled: Option<bool>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename = "Win32_OptionalFeature")]
    #[allow(non_snake_case)]
    struct OptionalFeature {
        /// 1 = enabled (or enabled pending reboot), 2 = disabled, 3 = absent.
        InstallState: Option<u32>,
    }

    fn query_filter() -> Result<UwfFilter> {
        let com = COMLibrary::new()?;
        let wmi = WMIConnection::with_namespace_path(NS, com)?;
        let rows: Vec<UwfFilter> = wmi.query()?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("UWF_Filter not present; feature not installed?"))
    }

    /// The optional feature is marked enabled (it may still need a reboot).
    fn feature_marked_enabled() -> bool {
        let query = || -> Result<bool> {
            let com = COMLibrary::new()?;
            let wmi = WMIConnection::new(com)?;
            let rows: Vec<OptionalFeature> = wmi.raw_query(format!(
                "SELECT InstallState FROM Win32_OptionalFeature WHERE Name = '{FEATURE}'"
            ))?;
            Ok(rows.first().and_then(|f| f.InstallState) == Some(1))
        };
        query().unwrap_or(false)
    }

    pub fn is_enabled() -> Result<bool> {
        Ok(query_filter()?.CurrentEnabled.unwrap_or(false))
    }

    /// Where this machine is in the enable/disable sequence (see the
    /// `ClientStatus::uwf_state` doc for the values).
    pub fn state() -> String {
        if BUSY.load(Ordering::Relaxed) {
            return "installing".into();
        }
        if let Some(e) = LAST_ERROR.lock().unwrap().clone() {
            return format!("error:{e}");
        }
        match query_filter() {
            Ok(f) => match (f.CurrentEnabled.unwrap_or(false), f.NextEnabled.unwrap_or(false)) {
                (true, true) => "on",
                (true, false) => "off_next_boot",
                (false, true) => "on_next_boot",
                (false, false) => "off",
            }
            .into(),
            Err(_) if feature_marked_enabled() => "install_pending_reboot".into(),
            Err(_) => "not_installed".into(),
        }
    }

    /// Request the desired UWF state in the background and return at once:
    /// installing the feature with DISM can take minutes, and the console
    /// should not wait for it. Progress and errors show up in [`state`].
    pub fn apply(enabled: bool, scope: UwfScope) -> Result<()> {
        if BUSY.swap(true, Ordering::SeqCst) {
            tracing::info!("UWF change already in progress; skipping");
            return Ok(());
        }
        std::thread::spawn(move || {
            let result = apply_now(enabled, scope);
            if let Err(e) = &result {
                tracing::error!(error = %e, "uwf policy failed");
            }
            *LAST_ERROR.lock().unwrap() = result.err().map(|e| e.to_string());
            BUSY.store(false, Ordering::SeqCst);
        });
        Ok(())
    }

    fn apply_now(enabled: bool, scope: UwfScope) -> Result<()> {
        if query_filter().is_err() {
            if !enabled {
                return Ok(()); // not installed means not protecting
            }
            if !feature_marked_enabled() {
                tracing::info!("installing optional feature {FEATURE} via DISM");
                run(
                    "dism.exe",
                    &["/Online", "/Enable-Feature", &format!("/FeatureName:{FEATURE}"), "/All", "/NoRestart"],
                )?;
            }
            // uwfmgr only works after the reboot that finishes the install;
            // the startup policy run after that reboot enables the filter.
            tracing::info!("UWF feature installed; reboot required before it can be enabled");
            return Ok(());
        }

        run("uwfmgr.exe", &["filter", if enabled { "enable" } else { "disable" }])?;
        if enabled {
            for vol in target_volumes(scope) {
                // Protect each target volume; already-protected is not an error.
                if let Err(e) = run("uwfmgr.exe", &["volume", "protect", &vol]) {
                    tracing::warn!(%vol, error = %e, "protect volume failed");
                }
            }
        }
        tracing::info!(enabled, ?scope, "requested UWF state change (reboot required)");
        Ok(())
    }

    /// The drive specs (e.g. "C:") to protect for the given scope.
    fn target_volumes(scope: UwfScope) -> Vec<String> {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        match scope {
            UwfScope::SystemOnly => vec![system],
            UwfScope::AllVolumes => {
                let mut drives = fixed_drives();
                if drives.is_empty() {
                    drives.push(system);
                }
                drives
            }
        }
    }

    /// Enumerate fixed (non-removable, non-network) drives as "X:" specs.
    fn fixed_drives() -> Vec<String> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        const DRIVE_FIXED: u32 = 3;

        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root: Vec<u16> = format!("{letter}:\\")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_FIXED {
                out.push(format!("{letter}:"));
            }
        }
        out
    }

    pub fn reboot(restore: bool, delay_secs: u32) -> Result<()> {
        // When UWF is enabled, a plain reboot already discards the overlay,
        // returning the machine to its protected baseline — that *is* the
        // restore. We therefore never commit before this reboot.
        tracing::info!(restore, delay_secs, "initiating reboot");
        run(
            "shutdown.exe",
            &["/r", "/t", &delay_secs.to_string(), "/c", "LabCtl scheduled reboot"],
        )?;
        Ok(())
    }

    fn run(exe: &str, args: &[&str]) -> Result<()> {
        let out = crate::hidden_command(exe).args(args).output()?;
        if !out.status.success() {
            // uwfmgr and DISM report errors on stdout, not stderr.
            let msg = [out.stdout.as_slice(), out.stderr.as_slice()]
                .iter()
                .map(|b| decode_console(b))
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            anyhow::bail!("{exe} {} failed ({}): {}", args.join(" "), out.status, msg.trim());
        }
        Ok(())
    }

    /// Console tools print in the OEM code page (e.g. GBK on Chinese
    /// Windows) or UTF-16; decode either into readable text.
    fn decode_console(bytes: &[u8]) -> String {
        if bytes.len() >= 2 && bytes.len() % 2 == 0 && bytes[1] == 0 {
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return String::from_utf16_lossy(&wide);
        }
        if let Ok(s) = std::str::from_utf8(bytes) {
            return s.to_string();
        }
        use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
        let n = unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; n as usize];
        unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
        String::from_utf16_lossy(&wide)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn is_enabled() -> Result<bool> {
        anyhow::bail!("UWF is Windows-only")
    }
    pub fn state() -> String {
        "not_installed".into()
    }
    pub fn apply(enabled: bool, scope: UwfScope) -> Result<()> {
        tracing::info!(enabled, ?scope, "uwf apply (no-op on this platform)");
        Ok(())
    }
    pub fn reboot(restore: bool, delay_secs: u32) -> Result<()> {
        tracing::info!(restore, delay_secs, "reboot (no-op on this platform)");
        Ok(())
    }
}

pub use imp::{apply, is_enabled, reboot, state};

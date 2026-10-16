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
//! Enabling/disabling UWF only takes effect after the reboot UWF itself
//! mandates; callers surface that to the operator.

use anyhow::Result;

#[cfg(windows)]
mod imp {
    use super::*;
    use serde::Deserialize;
    use std::process::Command;
    use wmi::{COMLibrary, WMIConnection};

    const NS: &str = "root\\standardcimv2\\embedded";

    #[derive(Deserialize, Debug)]
    #[serde(rename = "UWF_Filter")]
    #[allow(non_snake_case)]
    struct UwfFilter {
        /// Whether UWF is protecting the volume in the *current* session.
        CurrentEnabled: Option<bool>,
        /// Whether UWF will be enabled after the next reboot.
        NextEnabled: Option<bool>,
    }

    fn query_filter() -> Result<UwfFilter> {
        let com = COMLibrary::new()?;
        let wmi = WMIConnection::with_namespace_path(NS, com)?;
        let rows: Vec<UwfFilter> = wmi.query()?;
        rows.into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("UWF_Filter not present; feature not installed?"))
    }

    pub fn is_enabled() -> Result<bool> {
        Ok(query_filter()?.CurrentEnabled.unwrap_or(false))
    }

    /// Ensure the optional feature is installed, then enable/disable via
    /// `uwfmgr`. Returns immediately; the change needs a reboot to apply.
    pub fn apply(enabled: bool) -> Result<()> {
        ensure_feature()?;
        let sub = if enabled { "enable" } else { "disable" };
        run("uwfmgr.exe", &["filter", sub])?;
        tracing::info!(enabled, "requested UWF state change (reboot required)");
        Ok(())
    }

    fn ensure_feature() -> Result<()> {
        // If the WMI class resolves, the feature is already installed.
        if query_filter().is_ok() {
            return Ok(());
        }
        tracing::info!("installing optional feature Client-UnifiedWriteFilter via DISM");
        run(
            "dism.exe",
            &[
                "/Online",
                "/Enable-Feature",
                "/FeatureName:Client-UnifiedWriteFilter",
                "/All",
                "/NoRestart",
            ],
        )?;
        Ok(())
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
        let out = Command::new(exe).args(args).output()?;
        if !out.status.success() {
            anyhow::bail!(
                "{exe} {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn is_enabled() -> Result<bool> {
        anyhow::bail!("UWF is Windows-only")
    }
    pub fn apply(enabled: bool) -> Result<()> {
        tracing::info!(enabled, "uwf apply (no-op on this platform)");
        Ok(())
    }
    pub fn reboot(restore: bool, delay_secs: u32) -> Result<()> {
        tracing::info!(restore, delay_secs, "reboot (no-op on this platform)");
        Ok(())
    }
}

pub use imp::{apply, is_enabled, reboot};

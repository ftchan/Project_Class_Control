//! Unified Write Filter (UWF) management for Windows 10 **Pro**.
//!
//! Background: UWF ships as an optional Windows feature
//! (`Client-UnifiedWriteFilter`). It is usable on Pro once that feature is
//! installed — no edition change to Enterprise/Education is required. This is
//! the approach of the referenced UWF-Manager project
//! (https://github.com/JIRBOY/UWF-Manager).
//!
//! State is both read and changed through the documented WMI namespace
//! `root\standardcimv2\embedded` (`UWF_Filter`, `UWF_Volume`). The in-box
//! `uwfmgr.exe` is not used to change it: it refuses on Pro editions ("filter
//! enable failed"), while the WMI provider it wraps does not check the edition.
//!
//! Turning protection on from scratch takes two reboots: the first completes
//! the feature install (the WMI classes do not exist before it), then the
//! agent's startup policy run enables the filter, which applies on the second
//! reboot. [`state`] reports where a machine is in that sequence.
//!
//! The overlay, which holds writes to protected volumes until the next
//! reboot, is disk-based: a RAM overlay grows in memory and can exhaust it.
//! UWF always creates the disk overlay file on the system volume; it cannot
//! be placed on another disk.

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
    /// UWF was turned off for one boot to replace a RAM overlay with a disk one.
    static SWITCHING_TO_DISK: AtomicBool = AtomicBool::new(false);

    /// `UWF_OverlayConfig.Type` of a disk-based overlay (0 = RAM).
    const DISK_OVERLAY: u32 = 1;

    #[derive(Deserialize, Debug)]
    #[serde(rename = "UWF_Filter")]
    #[allow(non_snake_case)]
    struct UwfFilter {
        /// Whether UWF is protecting the volume in the *current* session.
        CurrentEnabled: Option<bool>,
        /// Whether UWF will be enabled after the next reboot.
        NextEnabled: Option<bool>,
    }

    /// `UWF_Filter` with its object path, for method calls.
    #[derive(Deserialize)]
    #[serde(rename = "UWF_Filter")]
    #[allow(non_snake_case)]
    struct UwfFilterPath {
        __Path: String,
    }

    #[derive(Deserialize)]
    #[serde(rename = "UWF_Volume")]
    #[allow(non_snake_case)]
    struct UwfVolume {
        __Path: String,
        /// true = settings of the current session, false = next session.
        CurrentSession: Option<bool>,
        DriveLetter: Option<String>,
        Protected: Option<bool>,
    }

    #[derive(Deserialize)]
    #[serde(rename = "UWF_OverlayConfig")]
    #[allow(non_snake_case)]
    struct OverlayConfig {
        __Path: String,
        /// true = settings of the current session, false = next session.
        CurrentSession: Option<bool>,
        /// 0 = RAM-based, 1 = disk-based.
        Type: Option<u32>,
        /// Maximum overlay size in MB.
        MaximumSize: Option<i32>,
    }

    #[derive(Deserialize)]
    #[serde(rename = "UWF_Overlay")]
    #[allow(non_snake_case)]
    struct Overlay {
        /// Current overlay size in MB.
        OverlayConsumption: Option<u32>,
    }

    /// Output of UWF methods: 0 on success, an error code otherwise.
    #[derive(Deserialize)]
    #[allow(non_snake_case)]
    struct MethodResult {
        ReturnValue: u32,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename = "Win32_OptionalFeature")]
    #[allow(non_snake_case)]
    struct OptionalFeature {
        /// 1 = enabled (or enabled pending reboot), 2 = disabled, 3 = absent.
        InstallState: Option<u32>,
    }

    fn uwf_wmi() -> Result<WMIConnection> {
        Ok(WMIConnection::with_namespace_path(NS, COMLibrary::new()?)?)
    }

    fn query_filter() -> Result<UwfFilter> {
        let rows: Vec<UwfFilter> = uwf_wmi()?.query()?;
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

    /// Persist policy.json through an active UWF overlay. Without this, a
    /// local request to disable UWF would be forgotten at the next reboot.
    pub fn commit_policy_file() -> Result<()> {
        if !is_enabled().unwrap_or(false) {
            return Ok(());
        }
        #[derive(serde::Serialize)]
        #[allow(non_snake_case)]
        struct CommitArgs<'a> { FileName: &'a str }

        let path = labctl_common::paths::policy_file();
        let file = path.to_str().ok_or_else(|| anyhow::anyhow!("策略文件路径无法转换为 Unicode"))?;
        let drive = path.components().next().and_then(|c| c.as_os_str().to_str())
            .ok_or_else(|| anyhow::anyhow!("无法确定策略文件所在磁盘"))?;
        let wmi = uwf_wmi()?;
        let volumes: Vec<UwfVolume> = wmi.query()?;
        let volume = volumes.iter().find(|v| v.CurrentSession == Some(true)
            && v.Protected == Some(true)
            && v.DriveLetter.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(drive)));
        if let Some(volume) = volume {
            let out: MethodResult = wmi.exec_instance_method::<UwfVolume, _, _>(
                "CommitFile", &volume.__Path, CommitArgs { FileName: file })?;
            check("UWF_Volume.CommitFile()", out.ReturnValue)?;
        }
        Ok(())
    }

    pub fn disable_for_restore() -> Result<()> {
        if query_filter().is_ok() { set_filter(false)?; }
        Ok(())
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
                (true, false) if SWITCHING_TO_DISK.load(Ordering::Relaxed) => "switching_to_disk",
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
                // 3010 = installed, reboot required: the normal outcome here.
                run_ok(
                    "dism.exe",
                    &["/Online", "/Enable-Feature", &format!("/FeatureName:{FEATURE}"), "/All", "/NoRestart"],
                    &[0, 3010],
                )?;
            }
            // The WMI classes only appear after the reboot that finishes the
            // install; the startup policy run after that reboot enables it.
            tracing::info!("UWF feature installed; reboot required before it can be enabled");
            return Ok(());
        }

        if enabled {
            // Overlay settings can only change while UWF is off in this
            // session. A machine already protected with a RAM overlay is
            // turned off for one boot; the startup apply after that reboot
            // lands here again with UWF off and switches to a disk overlay.
            if is_enabled()? {
                if overlay_config(true)?.Type != Some(DISK_OVERLAY) {
                    set_filter(false)?;
                    SWITCHING_TO_DISK.store(true, Ordering::SeqCst);
                    tracing::info!("RAM overlay in use; UWF off for one boot to switch to a disk overlay");
                    return Ok(());
                }
            } else {
                configure_disk_overlay()?;
            }

            // Protect the volumes first so the filter never starts with
            // nothing protected.
            let mut failed = Vec::new();
            for vol in target_volumes(scope) {
                if let Err(e) = protect_volume(&vol) {
                    tracing::warn!(%vol, error = %e, "protect volume failed");
                    failed.push(format!("{vol} {e}"));
                }
            }
            set_filter(true)?;
            anyhow::ensure!(failed.is_empty(), "已开启筛选器，但以下磁盘保护失败：{}", failed.join("；"));
        } else {
            set_filter(false)?;
        }
        tracing::info!(enabled, ?scope, "requested UWF state change (reboot required)");
        Ok(())
    }

    fn overlay_config(current: bool) -> Result<OverlayConfig> {
        let rows: Vec<OverlayConfig> = uwf_wmi()?.query()?;
        rows.into_iter()
            .find(|c| c.CurrentSession == Some(current))
            .ok_or_else(|| anyhow::anyhow!("找不到 UWF_OverlayConfig 实例"))
    }

    /// Use a disk-based overlay from the next session on, sized from the free
    /// space on the system volume. UWF must be off in the current session.
    fn configure_disk_overlay() -> Result<()> {
        let next = overlay_config(false)?;
        if next.Type == Some(DISK_OVERLAY) && next.MaximumSize.unwrap_or(0) >= 1024 {
            return Ok(());
        }
        let free = system_volume_free_mb()?;
        let size = super::disk_overlay_size_mb(free).ok_or_else(|| {
            anyhow::anyhow!("系统盘剩余空间不足（{free} MB），无法建立 UWF 磁盘缓存（至少需要 3072 MB）")
        })?;

        #[derive(serde::Serialize)]
        struct SizeArg {
            size: u32,
        }
        #[derive(serde::Serialize)]
        struct TypeArg {
            #[serde(rename = "type")]
            kind: u32,
        }
        let wmi = uwf_wmi()?;
        let out: MethodResult = wmi
            .exec_instance_method::<OverlayConfig, _, _>("SetMaximumSize", &next.__Path, SizeArg { size })
            .map_err(|e| anyhow::anyhow!("UWF_OverlayConfig.SetMaximumSize() 调用失败：{e}"))?;
        check("UWF_OverlayConfig.SetMaximumSize()", out.ReturnValue)?;
        let out: MethodResult = wmi
            .exec_instance_method::<OverlayConfig, _, _>("SetType", &next.__Path, TypeArg { kind: DISK_OVERLAY })
            .map_err(|e| anyhow::anyhow!("UWF_OverlayConfig.SetType() 调用失败：{e}"))?;
        check("UWF_OverlayConfig.SetType()", out.ReturnValue)?;
        tracing::info!(size, free, "UWF overlay set to disk-based for the next session");
        Ok(())
    }

    /// Free space on the system volume in MB.
    fn system_volume_free_mb() -> Result<u64> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(Some(0)).collect();
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), None, None)? };
        Ok(free / (1024 * 1024))
    }

    /// Overlay type and size in the current session (and the next session
    /// when it differs), e.g. "磁盘 8192 MB，已用 120 MB".
    pub fn overlay_info() -> Option<String> {
        let describe = |c: &OverlayConfig| {
            let kind = if c.Type == Some(DISK_OVERLAY) { "磁盘" } else { "内存" };
            format!("{kind} {} MB", c.MaximumSize.unwrap_or(0))
        };
        let current = overlay_config(true).ok()?;
        let mut text = describe(&current);
        if is_enabled().unwrap_or(false) {
            let used: Option<u32> = uwf_wmi()
                .ok()
                .and_then(|w| w.query::<Overlay>().ok())
                .and_then(|rows| rows.into_iter().next())
                .and_then(|o| o.OverlayConsumption);
            if let Some(used) = used {
                text.push_str(&format!("，已用 {used} MB"));
            }
        }
        if let Ok(next) = overlay_config(false) {
            if next.Type != current.Type || next.MaximumSize != current.MaximumSize {
                text.push_str(&format!("（下次启动：{}）", describe(&next)));
            }
        }
        Some(text)
    }

    /// `UWF_Filter.Enable()` / `Disable()`; applies after the next reboot.
    fn set_filter(enabled: bool) -> Result<()> {
        let wmi = uwf_wmi()?;
        let rows: Vec<UwfFilterPath> = wmi.query()?;
        let path = &rows.first().ok_or_else(|| anyhow::anyhow!("找不到 UWF_Filter 实例"))?.__Path;
        let method = if enabled { "Enable" } else { "Disable" };
        let out: MethodResult = wmi
            .exec_instance_method::<UwfFilterPath, _, _>(method, path, ())
            .map_err(|e| anyhow::anyhow!("UWF_Filter.{method}() 调用失败：{e}"))?;
        check(&format!("UWF_Filter.{method}()"), out.ReturnValue)
    }

    /// Protect `drive` (e.g. "C:") from the next session on: call `Protect()`
    /// on its next-session `UWF_Volume`, or create that entry when the volume
    /// is not in the UWF configuration yet (documented approach; the wmi
    /// crate cannot create instances, so PowerShell's CIM cmdlets do it).
    fn protect_volume(drive: &str) -> Result<()> {
        let wmi = uwf_wmi()?;
        let volumes: Vec<UwfVolume> = wmi.query()?;
        let next = volumes.iter().find(|v| {
            v.CurrentSession == Some(false)
                && v.DriveLetter.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(drive))
        });
        match next {
            Some(v) if v.Protected == Some(true) => Ok(()),
            Some(v) => {
                let out: MethodResult = wmi
                    .exec_instance_method::<UwfVolume, _, _>("Protect", &v.__Path, ())
                    .map_err(|e| anyhow::anyhow!("UWF_Volume.Protect() 调用失败：{e}"))?;
                check("UWF_Volume.Protect()", out.ReturnValue)
            }
            None => run(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!(
                        "$ErrorActionPreference='Stop'; New-CimInstance -Namespace root/standardcimv2/embedded \
                         -ClassName UWF_Volume -Property @{{CurrentSession=$false; DriveLetter='{drive}'; Protected=$true}} | Out-Null"
                    ),
                ],
            ),
        }
    }

    /// Turn a UWF method's return value into a readable error.
    fn check(what: &str, code: u32) -> Result<()> {
        if code == 0 {
            return Ok(());
        }
        let hint = match code {
            0x8007_0005 | 5 => "拒绝访问（需要管理员/SYSTEM 权限）",
            0x8007_0032 | 50 => "此 Windows 版本不支持该操作",
            0x8007_0490 | 1168 => "找不到对象（磁盘不存在或未在 UWF 配置中）",
            0x8004_1001 => "WMI 一般性错误",
            _ => "未知错误",
        };
        anyhow::bail!("{what} 返回错误码 0x{code:08X}：{hint}")
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
        run_ok(exe, args, &[0])
    }

    /// Like [`run`], but `ok_codes` are exit codes that mean success.
    fn run_ok(exe: &str, args: &[&str], ok_codes: &[i32]) -> Result<()> {
        let out = crate::hidden_command(exe).args(args).output()?;
        if !out.status.code().is_some_and(|c| ok_codes.contains(&c)) {
            // DISM reports errors on stdout, PowerShell on stderr; keep both.
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
    pub fn commit_policy_file() -> Result<()> { Ok(()) }
    pub fn disable_for_restore() -> Result<()> { Ok(()) }
    pub fn overlay_info() -> Option<String> { None }
}

pub use imp::{apply, commit_policy_file, disable_for_restore, is_enabled, overlay_info, reboot, state};

/// Disk overlay size for the free space on the system volume: half of it,
/// between 2 GB and 20 GB, keeping at least 1 GB free beside it. `None` when
/// the volume is too full for a disk overlay.
fn disk_overlay_size_mb(free_mb: u64) -> Option<u32> {
    const MIN_MB: u64 = 2048;
    const MAX_MB: u64 = 20480;
    let size = (free_mb / 2).clamp(MIN_MB, MAX_MB);
    (free_mb >= size + 1024).then_some(size as u32)
}

#[cfg(test)]
mod tests {
    use super::disk_overlay_size_mb;

    #[test]
    fn disk_overlay_sizes_from_free_space() {
        assert_eq!(disk_overlay_size_mb(2_000), None);
        assert_eq!(disk_overlay_size_mb(3_072), Some(2_048));
        assert_eq!(disk_overlay_size_mb(30_000), Some(15_000));
        assert_eq!(disk_overlay_size_mb(200_000), Some(20_480));
    }
}

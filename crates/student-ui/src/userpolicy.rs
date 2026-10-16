//! Per-user policy enforcement, applied from the GUI because it runs in the
//! interactive user session (HKCU here is the signed-in student's hive, unlike
//! the system service which runs in session 0). Covers desktop-wallpaper lock
//! and the Task Manager lock. Machine-wide policies (USB, UWF) stay with the
//! service. All values are the documented, reversible Group-Policy registry
//! values; clearing them restores normal behavior.

use labctl_common::policy::Policy;

#[cfg(windows)]
pub fn apply(p: &Policy) {
    if let Err(e) = apply_wallpaper(p.lock_wallpaper, p.wallpaper_path.as_deref()) {
        tracing::warn!(error = %e, "wallpaper (user) failed");
    }
    if let Err(e) = apply_task_manager(p.disable_task_manager) {
        tracing::warn!(error = %e, "task manager policy failed");
    }
}

#[cfg(windows)]
fn apply_task_manager(disable: bool) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (sys, _) =
        hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Policies\System")?;
    if disable {
        sys.set_value("DisableTaskMgr", &1u32)?;
    } else {
        let _ = sys.delete_value("DisableTaskMgr");
    }
    Ok(())
}

#[cfg(windows)]
fn apply_wallpaper(lock: bool, path: Option<&str>) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (active_desktop, _) =
        hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop")?;
    let (system, _) =
        hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Policies\System")?;

    if lock {
        active_desktop.set_value("NoChangingWallPaper", &1u32)?;
        if let Some(p) = path {
            system.set_value("Wallpaper", &p)?;
            system.set_value("WallpaperStyle", &"10")?;
            set_wallpaper_now(p)?;
        }
    } else {
        let _ = active_desktop.delete_value("NoChangingWallPaper");
        let _ = system.delete_value("Wallpaper");
        let _ = system.delete_value("WallpaperStyle");
    }
    Ok(())
}

#[cfg(windows)]
fn set_wallpaper_now(path: &str) -> anyhow::Result<()> {
    // SystemParametersInfoW via a tiny FFI decl to avoid pulling the windows
    // crate into the GUI just for this call.
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(uiAction: u32, uiParam: u32, pvParam: *mut u16, fWinIni: u32) -> i32;
    }
    const SPI_SETDESKWALLPAPER: u32 = 0x0014;
    const SPIF_UPDATEINIFILE: u32 = 0x01;
    const SPIF_SENDCHANGE: u32 = 0x02;
    let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wide.as_mut_ptr(),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };
    anyhow::ensure!(ok != 0, "SystemParametersInfoW failed");
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(_p: &Policy) {}

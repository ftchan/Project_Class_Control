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

    // `lock` only decides whether the student may change it; a pushed
    // wallpaper is applied either way.
    if lock {
        active_desktop.set_value("NoChangingWallPaper", &1u32)?;
        if let Some(p) = path {
            system.set_value("Wallpaper", &p)?;
            system.set_value("WallpaperStyle", &"10")?;
        }
    } else {
        let _ = active_desktop.delete_value("NoChangingWallPaper");
        let _ = system.delete_value("Wallpaper");
        let _ = system.delete_value("WallpaperStyle");
    }
    if let Some(p) = path.filter(|p| std::path::Path::new(p).exists()) {
        set_wallpaper_now(p)?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_wallpaper_now(path: &str) -> anyhow::Result<()> {
    // SystemParametersInfoW via a tiny FFI decl to avoid pulling the windows
    // crate into the GUI just for this call.
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(uiAction: u32, uiParam: u32, pvParam: *mut core::ffi::c_void, fWinIni: u32) -> i32;
    }
    const SPI_SETDESKWALLPAPER: u32 = 0x0014;
    const SPIF_UPDATEINIFILE: u32 = 0x01;
    const SPIF_SENDCHANGE: u32 = 0x02;
    let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wide.as_mut_ptr().cast(),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };
    anyhow::ensure!(ok != 0, "SystemParametersInfoW failed");
    Ok(())
}

/// One-click "restore desktop defaults" for the signed-in user:
///   * default mouse-cursor scheme (applied immediately),
///   * default desktop icon size (48 px; registry only — takes effect on the
///     next Explorer restart / sign-in, per requirement: no Explorer restart),
///   * default wallpaper (only if `wallpaper` is given and the file exists).
#[cfg(windows)]
pub fn apply_defaults(wallpaper: Option<&str>) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    if let Err(e) = reset_cursors() {
        tracing::warn!(error = %e, "reset cursors failed");
        errors.push(format!("鼠标指针：{e}"));
    }
    if let Err(e) = reset_icon_size() {
        tracing::warn!(error = %e, "reset icon size failed");
        errors.push(format!("图标大小：{e}"));
    }
    if let Some(p) = wallpaper {
        if std::path::Path::new(p).exists() {
            if let Err(e) = set_wallpaper_now(p) {
                tracing::warn!(error = %e, "set default wallpaper failed");
                errors.push(format!("默认壁纸：{e}"));
            }
        } else {
            errors.push(format!("默认壁纸不存在：{p}"));
        }
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("；"));
    Ok(())
}

#[cfg(windows)]
fn reset_cursors() -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (cursors, _) = hkcu.create_subkey(r"Control Panel\Cursors")?;
    // Selecting the empty "(no scheme)" / Windows default: clear each pointer
    // so Windows falls back to its built-in default cursors.
    for name in [
        "AppStarting", "Arrow", "Crosshair", "Hand", "Help", "IBeam", "No", "NWPen",
        "SizeAll", "SizeNESW", "SizeNS", "SizeNWSE", "SizeWE", "UpArrow", "Wait",
    ] {
        let _ = cursors.set_value(name, &"");
    }
    cursors.set_value("", &"Windows Default")?; // scheme (Default) value
    apply_cursor_change()
}

#[cfg(windows)]
fn apply_cursor_change() -> anyhow::Result<()> {
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(a: u32, b: u32, c: *mut core::ffi::c_void, d: u32) -> i32;
    }
    const SPI_SETCURSORS: u32 = 0x0057;
    const SPIF_UPDATEINIFILE: u32 = 0x01;
    const SPIF_SENDCHANGE: u32 = 0x02;
    let ok = unsafe {
        SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
    };
    anyhow::ensure!(ok != 0, "SPI_SETCURSORS failed");
    Ok(())
}

#[cfg(windows)]
fn reset_icon_size() -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    // Desktop icon size lives in the desktop view's "bag". 48 = medium (the
    // Windows default). Explorer refresh (below) makes it take effect.
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (desktop, _) =
        hkcu.create_subkey(r"Software\Microsoft\Windows\Shell\Bags\1\Desktop")?;
    desktop.set_value("IconSize", &48u32)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(_p: &Policy) {}

#[cfg(not(windows))]
pub fn apply_defaults(_wallpaper: Option<&str>) -> anyhow::Result<()> { Ok(()) }

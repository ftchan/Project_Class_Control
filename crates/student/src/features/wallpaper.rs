//! Wallpaper locking.
//!
//! Enforcement uses the same registry values Group Policy's "Prevent changing
//! desktop background" and "Desktop Wallpaper" settings write. These are
//! documented, reversible policy values — clearing them restores normal
//! behavior. No undocumented or hidden mechanism is used.

use anyhow::Result;

#[cfg(windows)]
pub fn apply(lock: bool, path: Option<&str>) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // "Prevent changing desktop background" policy.
    let (active_desktop, _) = hkcu.create_subkey(
        r"Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop",
    )?;
    let (system, _) =
        hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Policies\System")?;

    if lock {
        active_desktop.set_value("NoChangingWallPaper", &1u32)?;
        if let Some(p) = path {
            // Force a specific image, centered/filled.
            system.set_value("Wallpaper", &p)?;
            system.set_value("WallpaperStyle", &"10")?; // 10 = Fill
            set_wallpaper_now(p)?;
        }
    } else {
        // Clear the lock; ignore "not found".
        let _ = active_desktop.delete_value("NoChangingWallPaper");
        let _ = system.delete_value("Wallpaper");
        let _ = system.delete_value("WallpaperStyle");
    }
    Ok(())
}

#[cfg(windows)]
fn set_wallpaper_now(path: &str) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
    };

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut _),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(lock: bool, path: Option<&str>) -> Result<()> {
    tracing::info!(lock, ?path, "wallpaper policy (no-op on this platform)");
    Ok(())
}

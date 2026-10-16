//! Machine-wide wallpaper lock, applied by the service.
//!
//! The per-user "Prevent changing desktop background" value (HKCU) that the
//! GUI writes no longer locks the Settings app on Windows 11, and the GUI
//! cannot write machine policy. The service therefore sets the machine-wide
//! equivalents, both documented and reversible:
//!   * HKLM `Policies\ActiveDesktop\NoChangingWallPaper` greys out the
//!     background page in Settings ("managed by your organization");
//!   * `PersonalizationCSP` (DesktopImagePath/Url/Status) enforces the image
//!     for every account at sign-in.
//! Unlocking removes only the values written here.

use anyhow::Result;

#[cfg(windows)]
pub fn apply(lock: bool, path: Option<&str>) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (active_desktop, _) =
        hklm.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop")?;
    if lock {
        active_desktop.set_value("NoChangingWallPaper", &1u32)?;
    } else {
        let _ = active_desktop.delete_value("NoChangingWallPaper");
    }

    const CSP: &str = r"Software\Microsoft\Windows\CurrentVersion\PersonalizationCSP";
    match path.filter(|p| lock && std::path::Path::new(p).exists()) {
        Some(p) => {
            let (csp, _) = hklm.create_subkey(CSP)?;
            csp.set_value("DesktopImagePath", &p)?;
            csp.set_value("DesktopImageUrl", &p)?;
            csp.set_value("DesktopImageStatus", &1u32)?;
        }
        None => {
            // Only clear an image that this client set (it lives in our data
            // directory); leave anything an administrator configured.
            if let Ok(csp) = hklm.open_subkey_with_flags(CSP, KEY_READ | KEY_SET_VALUE) {
                let ours = csp
                    .get_value::<String, _>("DesktopImagePath")
                    .map(|v| is_ours(&v))
                    .unwrap_or(false);
                if ours {
                    for v in ["DesktopImagePath", "DesktopImageUrl", "DesktopImageStatus"] {
                        let _ = csp.delete_value(v);
                    }
                    // The key did not exist before we locked; drop it if empty.
                    if csp.query_info().is_ok_and(|i| i.values == 0 && i.sub_keys == 0) {
                        drop(csp);
                        let _ = hklm.delete_subkey(CSP);
                    }
                }
            }
        }
    }
    tracing::info!(lock, ?path, "applied machine wallpaper lock");
    Ok(())
}

/// A path inside this client's data directory.
#[cfg(windows)]
fn is_ours(path: &str) -> bool {
    let data = labctl_common::paths::data_dir();
    path.to_lowercase().starts_with(&data.to_string_lossy().to_lowercase())
}

#[cfg(not(windows))]
pub fn apply(lock: bool, path: Option<&str>) -> Result<()> {
    tracing::info!(lock, ?path, "wallpaper lock (no-op on this platform)");
    Ok(())
}

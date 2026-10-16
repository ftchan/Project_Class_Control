//! USB *storage* policy.
//!
//! Uses the two documented machine policies admins already use for this:
//!   * USBSTOR service `Start` value — 3 allows the mass-storage driver to
//!     load, 4 prevents it (drives will not mount).
//!   * StorageDevicePolicies `WriteProtect` — 1 makes removable volumes
//!     read-only.
//! Only mass storage is affected; HID devices (keyboard/mouse) are untouched.

use anyhow::Result;
use labctl_common::policy::UsbStoragePolicy;

#[cfg(windows)]
pub fn apply(policy: UsbStoragePolicy) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    let usbstor = hklm.open_subkey_with_flags(
        r"SYSTEM\CurrentControlSet\Services\USBSTOR",
        KEY_SET_VALUE,
    )?;
    let (store_pol, _) =
        hklm.create_subkey(r"SYSTEM\CurrentControlSet\Control\StorageDevicePolicies")?;

    match policy {
        UsbStoragePolicy::Allow => {
            usbstor.set_value("Start", &3u32)?;
            store_pol.set_value("WriteProtect", &0u32)?;
        }
        UsbStoragePolicy::ReadOnly => {
            usbstor.set_value("Start", &3u32)?;
            store_pol.set_value("WriteProtect", &1u32)?;
        }
        UsbStoragePolicy::Deny => {
            usbstor.set_value("Start", &4u32)?;
        }
    }
    tracing::info!(?policy, "applied USB storage policy");
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(policy: UsbStoragePolicy) -> Result<()> {
    tracing::info!(?policy, "usb storage policy (no-op on this platform)");
    Ok(())
}

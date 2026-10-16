//! Policy enforcement. Each sub-module maps one policy field to Windows
//! settings. On non-Windows builds the implementations are no-ops that log, so
//! the crate still compiles and the networking/logic can be tested anywhere.

pub mod wallpaper;
pub mod usb;
pub mod uwf;

use anyhow::Result;
use labctl_common::policy::Policy;

/// Apply every field of `policy` to the machine. Individual failures are
/// logged but do not abort the rest, so a single unsupported setting cannot
/// leave the machine half-configured.
pub fn apply_policy(policy: &Policy) {
    if let Err(e) = wallpaper::apply(policy.lock_wallpaper, policy.wallpaper_path.as_deref()) {
        tracing::error!(error = %e, "wallpaper policy failed");
    }
    if let Err(e) = usb::apply(policy.usb_storage) {
        tracing::error!(error = %e, "usb storage policy failed");
    }
    if let Some(desired) = policy.uwf_enabled {
        if let Err(e) = uwf::apply(desired) {
            tracing::error!(error = %e, "uwf policy failed");
        }
    }
}

/// Collect the current UWF state for status reports.
pub fn current_uwf_enabled() -> Option<bool> {
    match uwf::is_enabled() {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!(error = %e, "could not read UWF state");
            None
        }
    }
}

/// Reboot the machine, optionally requesting a restore on next boot.
pub fn reboot_and_restore(restore: bool, delay_secs: u32) -> Result<()> {
    uwf::reboot(restore, delay_secs)
}

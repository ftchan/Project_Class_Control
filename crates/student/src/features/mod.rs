//! Policy enforcement. Each sub-module maps one policy field to Windows
//! settings. On non-Windows builds the implementations are no-ops that log, so
//! the crate still compiles and the networking/logic can be tested anywhere.

pub mod usb;
pub mod uwf;

use anyhow::Result;
use labctl_common::policy::Policy;

/// Apply every field of `policy` to the machine. Individual failures are
/// logged and returned but do not abort the rest, so a single unsupported
/// setting cannot leave the machine half-configured. UWF changes continue in
/// the background; their progress is reported by [`uwf_state`].
pub fn apply_policy(policy: &Policy) -> Vec<String> {
    let mut errors = Vec::new();
    // Wallpaper is a per-user setting applied by the GUI client in the
    // interactive session (the service runs in session 0, where HKCU is the
    // system account and has no effect on the student's desktop).
    if let Err(e) = usb::apply(policy.usb_storage) {
        tracing::error!(error = %e, "usb storage policy failed");
        errors.push(format!("U 盘策略失败：{e}"));
    }
    if let Some(desired) = policy.uwf_enabled {
        if let Err(e) = uwf::apply(desired, policy.uwf_scope) {
            tracing::error!(error = %e, "uwf policy failed");
            errors.push(format!("UWF 失败：{e}"));
        }
    }
    crate::network::apply(policy.block_internet);
    errors
}

/// Detailed UWF state for status reports (see `ClientStatus::uwf_state`).
pub fn uwf_state() -> String {
    uwf::state()
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

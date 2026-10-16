//! Load and persist the local policy file.

use anyhow::{Context, Result};
use labctl_common::policy::{Policy, UsbStoragePolicy};
use std::fs;
use std::path::PathBuf;

/// Safe lab baseline used when no policy has been deployed yet: USB mass
/// storage is denied and internet access is blocked (LAN stays up so the
/// teacher console can still manage the machine). An administrator can loosen
/// either of these from the teacher console at any time.
pub fn default_policy() -> Policy {
    Policy {
        usb_storage: UsbStoragePolicy::Deny,
        block_internet: true,
        ..Policy::default()
    }
}

pub fn load() -> Policy {
    let path = labctl_common::paths::policy_file();
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(labctl_common::paths::strip_bom(&bytes)).unwrap_or_else(|e| {
            tracing::warn!(?path, error = %e, "policy file unreadable; using baseline");
            default_policy()
        }),
        Err(_) => {
            tracing::info!("no policy file; using baseline (USB denied)");
            default_policy()
        }
    }
}

pub fn save(policy: &Policy) -> Result<()> {
    let path = labctl_common::paths::policy_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(policy)?).context("write temp policy")?;
    fs::rename(&tmp, &path).context("atomically replace policy")?;
    Ok(())
}

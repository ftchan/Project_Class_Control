//! Load and persist the local policy file.

use anyhow::{Context, Result};
use labctl_common::policy::Policy;
use std::fs;
use std::path::PathBuf;

pub fn load() -> Policy {
    let path = labctl_common::paths::policy_file();
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(?path, error = %e, "policy file unreadable; using defaults");
            Policy::default()
        }),
        Err(_) => {
            tracing::info!("no policy file; using defaults");
            Policy::default()
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

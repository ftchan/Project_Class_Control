//! Local state for the GUI: the access-password gate, autostart registration,
//! and single-instance signaling.

use anyhow::Result;
use labctl_common::policy::Policy;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Control port used to detect/wake an already-running instance.
pub use labctl_common::UI_CONTROL_PORT;

fn ui_pass_file() -> PathBuf {
    labctl_common::paths::data_dir().join("ui-pass.hash")
}

/// Where the required password comes from.
pub struct Gate {
    /// Argon2 hash to check against, if any.
    pub hash: Option<String>,
    /// True if the password is set by the teacher policy (not locally editable).
    pub teacher_managed: bool,
}

impl Gate {
    /// First run == no password configured anywhere.
    pub fn first_run(&self) -> bool {
        self.hash.is_none()
    }
}

/// Resolve the gate: teacher policy wins; else a locally-set password; else none.
pub fn load_gate(policy: &Policy) -> Gate {
    if let Some(h) = &policy.config_passphrase_hash {
        return Gate { hash: Some(h.clone()), teacher_managed: true };
    }
    match fs::read_to_string(ui_pass_file()) {
        Ok(s) if !s.trim().is_empty() => Gate { hash: Some(s.trim().to_string()), teacher_managed: false },
        _ => Gate { hash: None, teacher_managed: false },
    }
}

/// Persist a locally-set password (first-run), returning its hash.
pub fn set_local_password(pw: &str) -> Result<String> {
    let h = labctl_common::auth::hash_passphrase(pw)?;
    let path = ui_pass_file();
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(&path, &h)?;
    Ok(h)
}

/// Read the locally-persisted policy (for display). Mirrors the agent's
/// baseline (USB denied) when no policy file exists yet.
pub fn load_policy() -> Policy {
    use labctl_common::policy::UsbStoragePolicy;
    let path = labctl_common::paths::policy_file();
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(labctl_common::paths::strip_bom(&b)).ok())
        .unwrap_or(Policy {
            usb_storage: UsbStoragePolicy::Deny,
            ..Policy::default()
        })
}

/// If another instance is already running, poke it to show and return true.
pub fn signal_existing_instance() -> bool {
    match std::net::TcpStream::connect(("127.0.0.1", UI_CONTROL_PORT)) {
        Ok(mut s) => {
            let _ = s.write_all(b"show");
            true
        }
        Err(_) => false,
    }
}

/// Register this exe to start automatically at user logon (so the managed
/// client is always present in the tray). Standard HKCU Run key; the user/admin
/// can see and remove it in Task Manager → Startup.
#[cfg(windows)]
pub fn ensure_autostart() -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let exe = std::env::current_exe()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    run.set_value("LabCtlStudentUI", &format!("\"{}\"", exe.display()))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn ensure_autostart() -> Result<()> {
    Ok(())
}

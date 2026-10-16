//! The terminal policy an administrator configures for managed clients.
//!
//! A `Policy` is serialized to JSON and stored locally on each student
//! machine. The student service reads it on startup and whenever the teacher
//! console pushes an update. Every field is optional-by-default so that older
//! clients tolerate newer fields.

use serde::{Deserialize, Serialize};

/// How removable USB *storage* should be treated. This only governs mass
/// storage; keyboards, mice and other HID devices are never affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UsbStoragePolicy {
    /// No restriction (default).
    #[default]
    Allow,
    /// Mounted read-only — users can read files but not write.
    ReadOnly,
    /// Mass-storage driver disabled; drives will not mount at all.
    Deny,
}

/// Which volumes UWF protects (and therefore restores on reboot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UwfScope {
    /// Only the system drive (typically C:).
    #[default]
    SystemOnly,
    /// Every fixed drive on the machine.
    AllVolumes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// When true, the desktop wallpaper is pinned and the Settings/Personalize
    /// path to change it is disabled for standard users.
    #[serde(default)]
    pub lock_wallpaper: bool,

    /// Local path to the wallpaper to enforce when `lock_wallpaper` is set.
    /// The teacher console fills this in with a path that is valid **on the
    /// student** (the agent materializes the pushed image and rewrites this to
    /// a local file), so it never points at a teacher-only path.
    #[serde(default)]
    pub wallpaper_path: Option<String>,

    /// Base64-encoded wallpaper image, transferred so every student has the
    /// actual file (a teacher-side path would not exist on the student and
    /// would blank the desktop). The agent writes it to a local file and then
    /// clears this field before persisting, so `policy.json` stays small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper_data_base64: Option<String>,

    /// Original wallpaper file name (used to keep the correct extension).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper_name: Option<String>,

    /// Removable storage handling.
    #[serde(default)]
    pub usb_storage: UsbStoragePolicy,

    /// Desired UWF (Unified Write Filter) state. `None` means "do not manage".
    /// A change only takes effect after the reboot UWF itself requires.
    #[serde(default)]
    pub uwf_enabled: Option<bool>,

    /// Which volumes UWF should protect/restore when enabled.
    #[serde(default)]
    pub uwf_scope: UwfScope,

    /// Disable Task Manager for the signed-in user (the documented
    /// "Remove Task Manager" Group Policy, i.e. the `DisableTaskMgr` value).
    /// Reversible: clearing it restores Task Manager. Applied in the user
    /// session by the GUI client, not by the system service.
    #[serde(default)]
    pub disable_task_manager: bool,

    /// Block internet access while keeping the local network usable (so the
    /// teacher console can still manage the machine). Implemented via Windows
    /// Firewall: default-deny outbound with allow rules for private/LAN ranges
    /// and the management ports. Reversible. Enforced by the system service.
    #[serde(default)]
    pub block_internet: bool,

    /// Argon2 PHC-format hash of the client access passphrase. When present,
    /// the local config UI requires it before settings can be viewed/changed.
    #[serde(default)]
    pub config_passphrase_hash: Option<String>,

    /// Keep the client service running: if it exits unexpectedly, the Windows
    /// Service Control Manager restarts it. This is ordinary service
    /// resiliency, not an attempt to hide from the user or Task Manager.
    #[serde(default = "default_true")]
    pub keep_service_alive: bool,

    /// Monotonic revision, bumped by the teacher console on every push so the
    /// client can tell a newer policy from an older one.
    #[serde(default)]
    pub revision: u64,
}

fn default_true() -> bool {
    true
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            lock_wallpaper: false,
            wallpaper_path: None,
            wallpaper_data_base64: None,
            wallpaper_name: None,
            usb_storage: UsbStoragePolicy::Allow,
            uwf_enabled: None,
            uwf_scope: UwfScope::SystemOnly,
            disable_task_manager: false,
            block_internet: false,
            config_passphrase_hash: None,
            keep_service_alive: true,
            revision: 0,
        }
    }
}

impl Policy {
    /// Merge an incoming policy over this one, taking the new one's fields and
    /// adopting its revision only if it is strictly newer. A push without an
    /// access passphrase keeps the current one (the console leaves the field
    /// empty to mean "unchanged").
    pub fn apply(&mut self, mut incoming: Policy) -> bool {
        if incoming.revision < self.revision {
            return false;
        }
        if incoming.config_passphrase_hash.is_none() {
            incoming.config_passphrase_hash = self.config_passphrase_hash.take();
        }
        *self = incoming;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Policy;

    #[test]
    fn empty_passphrase_keeps_current() {
        let mut p = Policy { config_passphrase_hash: Some("old".into()), revision: 1, ..Policy::default() };
        assert!(p.apply(Policy { revision: 2, ..Policy::default() }));
        assert_eq!(p.config_passphrase_hash.as_deref(), Some("old"));
        assert!(p.apply(Policy { config_passphrase_hash: Some("new".into()), revision: 3, ..Policy::default() }));
        assert_eq!(p.config_passphrase_hash.as_deref(), Some("new"));
    }
}

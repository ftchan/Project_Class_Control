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

    /// Absolute path to the wallpaper to enforce when `lock_wallpaper` is set.
    /// If `None`, whatever is current at lock time is retained.
    #[serde(default)]
    pub wallpaper_path: Option<String>,

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
            usb_storage: UsbStoragePolicy::Allow,
            uwf_enabled: None,
            uwf_scope: UwfScope::SystemOnly,
            disable_task_manager: false,
            config_passphrase_hash: None,
            keep_service_alive: true,
            revision: 0,
        }
    }
}

impl Policy {
    /// Merge an incoming policy over this one, taking the new one's fields and
    /// adopting its revision only if it is strictly newer.
    pub fn apply(&mut self, incoming: Policy) -> bool {
        if incoming.revision < self.revision {
            return false;
        }
        *self = incoming;
        true
    }
}

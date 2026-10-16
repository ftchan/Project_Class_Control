//! Length-prefixed JSON message protocol between teacher console and clients.
//!
//! Framing: a 4-byte big-endian length followed by that many bytes of UTF-8
//! JSON. Both directions use the same `Envelope`.

use crate::policy::Policy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Protocol version; a mismatch is reported rather than silently misparsed.
pub const PROTOCOL_VERSION: u16 = 1;

/// Snapshot of a managed client, shown as a tile in the console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientStatus {
    pub machine_id: Uuid,
    pub hostname: String,
    pub ip: String,
    pub logged_in_user: Option<String>,
    /// Current policy revision the client has applied.
    pub policy_revision: u64,
    pub uwf_enabled: Option<bool>,
    /// True while the foreground session is locked by our passphrase gate.
    pub session_locked: bool,
    pub agent_version: String,
}

/// A downscaled screen thumbnail for the classroom overview grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenThumbnail {
    pub machine_id: Uuid,
    pub width: u32,
    pub height: u32,
    /// JPEG bytes, base64-encoded for JSON transport.
    pub jpeg_base64: String,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Commands the teacher console sends to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Ask the client for a fresh status report.
    QueryStatus,
    /// Push a new policy. The client applies and persists it.
    PushPolicy { policy: Policy },
    /// Ensure the client's own foreground/lock UI is running and visible.
    LaunchClientUi,
    /// Request a single screen thumbnail at the given max dimension.
    RequestThumbnail { max_dim: u32 },
    /// Reboot the machine. `restore` asks for a restore-point rollback on the
    /// next boot, to be honored by whatever restore mechanism is deployed
    /// (e.g. UWF discard, or a disk-restore agent).
    RebootAndRestore { restore: bool, delay_secs: u32 },

    /// One-click "restore desktop defaults" for the signed-in user: default
    /// mouse-cursor scheme, default desktop icon size, and (if an image is
    /// given) a default wallpaper. The image is transferred as bytes so the
    /// student has a real local file. Applied by the GUI in the user session.
    ApplyDefaults {
        wallpaper_data_base64: Option<String>,
        wallpaper_name: Option<String>,
    },

    /// Push a new client executable and seamlessly update it: the agent writes
    /// the bytes, stops the running copy, swaps the file, and (if `run_after`)
    /// relaunches it hidden in the tray. Used to roll out a newer GUI client.
    UpdateClient {
        /// File name to install, e.g. "labctl-student-ui.exe".
        filename: String,
        /// Absolute target path; if `None`, install next to the agent exe.
        target_path: Option<String>,
        /// Base64-encoded executable bytes.
        data_base64: String,
        /// Relaunch the client after replacing it.
        run_after: bool,
    },
}

/// Replies and unsolicited reports a client sends to the console.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Report {
    Status(ClientStatus),
    Thumbnail(ScreenThumbnail),
    /// Positive acknowledgement of a command, optionally with a note.
    Ack { command: String, note: Option<String> },
    /// A command could not be carried out.
    Error { command: String, message: String },
}

/// Top-level framed message in either direction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u16,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Body {
    Command(Command),
    Report(Report),
}

impl Envelope {
    pub fn command(cmd: Command) -> Self {
        Self { version: PROTOCOL_VERSION, body: Body::Command(cmd) }
    }
    pub fn report(rep: Report) -> Self {
        Self { version: PROTOCOL_VERSION, body: Body::Report(rep) }
    }
}

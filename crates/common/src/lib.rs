//! Shared types for the lab classroom-management tool.
//!
//! This crate is platform-neutral: it carries the wire protocol, the policy
//! model, and small helpers (password hashing, paths) used by both the
//! student client and the teacher console.

pub mod protocol;
pub mod policy;
pub mod auth;
pub mod paths;
#[cfg(feature = "net")]
pub mod net;

/// Default TCP port the student client listens on for the teacher console.
pub const DEFAULT_PORT: u16 = 47800;

/// Default UDP port used for LAN discovery beacons.
pub const DISCOVERY_PORT: u16 = 47801;

/// Loopback TCP port the GUI client listens on for local control messages
/// (single-instance "show", and "defaults" from the agent).
pub const UI_CONTROL_PORT: u16 = 47802;

/// Magic string prefixed to discovery beacons so we ignore unrelated traffic.
pub const DISCOVERY_MAGIC: &str = "LABCTL/1";

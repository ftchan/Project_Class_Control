//! Client access gate.
//!
//! When the policy carries a `config_passphrase_hash`, the client requires the
//! passphrase before its configuration can be used. This module implements a
//! simple console gate (`labctl-student ui`); a production deployment would
//! render the same check in a borderless fullscreen window shown at logon.
//!
//! This gate guards *the client's own configuration surface*. It is a
//! deterrent against casual tampering by the signed-in user, not a security
//! boundary against an administrator.

use crate::config;
use anyhow::Result;
use labctl_common::auth::verify_passphrase;
use std::io::Write;

/// Returns the stored hash if a passphrase is configured.
pub fn configured_hash() -> Option<String> {
    config::load().config_passphrase_hash
}

/// Verify a single attempt against the configured passphrase.
#[allow(dead_code)] // used by the foreground gate UI
pub fn verify(attempt: &str) -> bool {
    match configured_hash() {
        Some(hash) => verify_passphrase(attempt, &hash).unwrap_or(false),
        None => true, // no passphrase set => open
    }
}

/// Standalone console gate used by `labctl-student ui`.
pub fn run_standalone_gate() -> Result<()> {
    let Some(hash) = configured_hash() else {
        println!("No access passphrase is configured; nothing to unlock.");
        return Ok(());
    };

    for remaining in (1..=3).rev() {
        print!("Enter client access passphrase: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if verify_passphrase(line.trim_end_matches(['\r', '\n']), &hash).unwrap_or(false) {
            println!("Unlocked.");
            return Ok(());
        }
        eprintln!("Incorrect. {} attempt(s) left.", remaining - 1);
    }
    anyhow::bail!("too many failed attempts");
}

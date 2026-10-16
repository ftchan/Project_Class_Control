//! Client access gate state.
//!
//! When the policy carries a `config_passphrase_hash`, the GUI client requires
//! the passphrase before it can be used. The agent reports whether such a gate
//! is configured so the console can show it. The actual gate UI lives in the
//! `ui` module.

use crate::config;

/// Returns the stored hash if a passphrase is configured.
pub fn configured_hash() -> Option<String> {
    config::load().config_passphrase_hash
}

//! Inbound firewall rules so the teacher console on the LAN can reach this
//! machine's agent. Without these, Windows Firewall silently drops inbound
//! TCP 47800 (commands/status/thumbnails) and UDP 47801 (discovery), so the
//! console never sees the client even though the agent is running.
//!
//! The agent (LocalSystem) adds these on startup as a self-heal, and the
//! installer adds them too. Rules are scoped to our two ports only.

#[cfg(windows)]
pub fn ensure_rules() -> Vec<String> {
    use labctl_common::{DEFAULT_PORT, DISCOVERY_PORT};
    let mut errors = Vec::new();
    for (name, proto, port) in [
        ("LabCtl Agent (TCP 47800)", "TCP", DEFAULT_PORT),
        ("LabCtl Discovery (UDP 47801)", "UDP", DISCOVERY_PORT),
    ] {
        if let Err(e) = add_rule(name, proto, port) {
            tracing::warn!(%name, error = %e, "firewall rule failed");
            errors.push(format!("{name}: {e}"));
        }
    }
    errors
}

#[cfg(windows)]
fn add_rule(name: &str, proto: &str, port: u16) -> anyhow::Result<()> {
    // Delete any existing rule with this name first, so repeated starts stay
    // idempotent instead of stacking duplicate rules.
    let _ = crate::hidden_command("netsh")
        .args(["advfirewall", "firewall", "delete", "rule", &format!("name={name}")])
        .output();

    let out = crate::hidden_command("netsh")
        .args([
            "advfirewall", "firewall", "add", "rule",
            &format!("name={name}"),
            "dir=in",
            "action=allow",
            &format!("protocol={proto}"),
            &format!("localport={port}"),
            "profile=any",
        ])
        .output();

    let out = out?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    tracing::info!(%name, "firewall rule ensured");
    Ok(())
}

#[cfg(not(windows))]
pub fn ensure_rules() -> Vec<String> { Vec::new() }

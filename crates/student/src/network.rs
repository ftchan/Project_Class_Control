//! "Block internet" policy via Windows Firewall.
//!
//! When enabled, the machine can still reach the **local network** (so the
//! teacher console keeps managing it and intranet resources work), but public
//! internet access is denied. This is done by switching the firewall's default
//! outbound action to block and adding allow rules for private/LAN ranges plus
//! broadcast/multicast used by discovery. Fully reversible.

#[cfg(windows)]
pub fn apply(block: bool) -> anyhow::Result<()> {
    // Private + link-local + multicast + limited broadcast. Covers typical
    // school LANs and our UDP discovery.
    const LAN: &str =
        "127.0.0.0/8,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,169.254.0.0/16,224.0.0.0/4,255.255.255.255";
    const OUT_RULE: &str = "LabCtl Allow LAN (out)";
    const IN_RULE: &str = "LabCtl Allow LAN (in)";

    let netsh = |args: &[&str]| -> anyhow::Result<()> {
        let out = crate::hidden_command("netsh").args(args).output()?;
        anyhow::ensure!(out.status.success(), "netsh {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
        Ok(())
    };

    // Always refresh the LAN allow rules first so enabling the block can never
    // lock out LAN management.
    // netsh reports "no rules match" as non-zero on some Windows versions.
    let _ = netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={OUT_RULE}")]);
    let _ = netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={IN_RULE}")]);

    if block {
        netsh(&[
            "advfirewall", "firewall", "add", "rule", &format!("name={OUT_RULE}"),
            "dir=out", "action=allow", &format!("remoteip={LAN}"), "profile=any",
        ])?;
        netsh(&[
            "advfirewall", "firewall", "add", "rule", &format!("name={IN_RULE}"),
            "dir=in", "action=allow", &format!("remoteip={LAN}"), "profile=any",
        ])?;
        // Default-deny outbound (inbound stays default-deny as usual).
        netsh(&["advfirewall", "set", "allprofiles", "firewallpolicy", "blockinbound,blockoutbound"])?;
        tracing::info!("internet access blocked (LAN still allowed)");
    } else {
        // Restore the Windows default policy.
        netsh(&["advfirewall", "set", "allprofiles", "firewallpolicy", "blockinbound,allowoutbound"])?;
        tracing::info!("internet access restored");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(block: bool) -> anyhow::Result<()> {
    tracing::info!(block, "block_internet (no-op on this platform)");
    Ok(())
}

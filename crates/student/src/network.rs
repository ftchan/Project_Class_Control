//! "Block internet" policy via Windows Firewall.
//!
//! When enabled, the machine can still reach the **local network** (so the
//! teacher console keeps managing it and intranet resources work), but public
//! internet access is denied. This is done by switching the firewall's default
//! outbound action to block and adding allow rules for private/LAN ranges plus
//! broadcast/multicast used by discovery. Fully reversible.

#[cfg(windows)]
pub fn apply(block: bool) {
    use std::process::Command;

    // Private + link-local + multicast + limited broadcast. Covers typical
    // school LANs and our UDP discovery.
    const LAN: &str =
        "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,169.254.0.0/16,224.0.0.0/4,255.255.255.255";
    const OUT_RULE: &str = "LabCtl Allow LAN (out)";
    const IN_RULE: &str = "LabCtl Allow LAN (in)";

    let netsh = |args: &[&str]| {
        let out = Command::new("netsh").args(args).output();
        match out {
            Ok(o) if o.status.success() => true,
            Ok(o) => {
                tracing::warn!(?args, stderr = %String::from_utf8_lossy(&o.stderr), "netsh failed");
                false
            }
            Err(e) => {
                tracing::warn!(?args, error = %e, "netsh not runnable");
                false
            }
        }
    };

    // Always refresh the LAN allow rules first so enabling the block can never
    // lock out LAN management.
    let _ = netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={OUT_RULE}")]);
    let _ = netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={IN_RULE}")]);

    if block {
        netsh(&[
            "advfirewall", "firewall", "add", "rule", &format!("name={OUT_RULE}"),
            "dir=out", "action=allow", &format!("remoteip={LAN}"), "profile=any",
        ]);
        netsh(&[
            "advfirewall", "firewall", "add", "rule", &format!("name={IN_RULE}"),
            "dir=in", "action=allow", &format!("remoteip={LAN}"), "profile=any",
        ]);
        // Default-deny outbound (inbound stays default-deny as usual).
        netsh(&["advfirewall", "set", "allprofiles", "firewallpolicy", "blockinbound,blockoutbound"]);
        tracing::info!("internet access blocked (LAN still allowed)");
    } else {
        // Restore the Windows default policy.
        netsh(&["advfirewall", "set", "allprofiles", "firewallpolicy", "blockinbound,allowoutbound"]);
        tracing::info!("internet access restored");
    }
}

#[cfg(not(windows))]
pub fn apply(block: bool) {
    tracing::info!(block, "block_internet (no-op on this platform)");
}

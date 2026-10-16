//! Student (managed client) entry point.
//!
//! Usage:
//!   labctl-student run           Run the agent in the foreground (dev/testing).
//!   labctl-student service       Run as a Windows service (invoked by the SCM).
//!   labctl-student install       Register the Windows service (needs admin).
//!   labctl-student uninstall     Remove the Windows service (needs admin).
//!
//! The agent listens on the LAN for the teacher console, enforces the local
//! policy, and answers status/thumbnail requests. All of this operates only on
//! this machine and only on an administrator-deployed policy.

mod agent;
mod config;
mod features;
mod capture;
mod lock;
#[cfg(windows)]
mod service;

use anyhow::Result;

fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "run".into());

    match mode.as_str() {
        #[cfg(windows)]
        "service" => return service::run_as_service(),
        #[cfg(windows)]
        "install" => return service::install(),
        #[cfg(windows)]
        "uninstall" => return service::uninstall(),
        "ui" => return lock::run_standalone_gate(),
        "hash" => {
            // `labctl-student hash <passphrase>` prints an Argon2id PHC hash,
            // so deployment scripts can bake an access passphrase into the
            // initial policy without needing the teacher console.
            let pass = std::env::args().nth(2).unwrap_or_default();
            if pass.is_empty() {
                eprintln!("usage: labctl-student hash <passphrase>");
                std::process::exit(2);
            }
            println!("{}", labctl_common::auth::hash_passphrase(&pass)?);
            return Ok(());
        }
        "run" => {}
        other => {
            eprintln!("unknown mode: {other}");
            eprintln!("modes: run | service | install | uninstall | ui");
            std::process::exit(2);
        }
    }

    // Foreground run: set up logging and drive the agent on a Tokio runtime.
    init_tracing();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(agent::run(agent::ShutdownSignal::ctrl_c()))
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LABCTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

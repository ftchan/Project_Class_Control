//! The student agent: a small LAN server that enforces local policy and
//! answers the teacher console. It only ever acts on this machine.

use crate::{capture, config, features, lock};
use anyhow::Result;
use labctl_common::net::{read_frame, write_frame};
use labctl_common::policy::Policy;
use labctl_common::protocol::{Body, ClientStatus, Command, Envelope, Report};
use labctl_common::{DEFAULT_PORT, DISCOVERY_MAGIC, DISCOVERY_PORT};
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Shared agent state.
struct State {
    machine_id: Uuid,
    policy: Mutex<Policy>,
}

/// A future that resolves when the agent should stop.
pub enum ShutdownSignal {
    CtrlC,
    /// Used by the Windows service path to stop on an SCM Stop control.
    #[allow(dead_code)]
    Channel(tokio::sync::oneshot::Receiver<()>),
}

impl ShutdownSignal {
    pub fn ctrl_c() -> Self {
        ShutdownSignal::CtrlC
    }
    async fn wait(self) {
        match self {
            ShutdownSignal::CtrlC => {
                let _ = tokio::signal::ctrl_c().await;
            }
            ShutdownSignal::Channel(rx) => {
                let _ = rx.await;
            }
        }
    }
}

pub async fn run(shutdown: ShutdownSignal) -> Result<()> {
    // Allow the console on the LAN to reach us (best-effort; needs admin).
    crate::firewall::ensure_rules();

    let policy = config::load();
    tracing::info!(revision = policy.revision, "applying startup policy");
    features::apply_policy(&policy);

    let state = Arc::new(State {
        machine_id: load_or_create_machine_id(),
        policy: Mutex::new(policy),
    });

    let tcp = TcpListener::bind(("0.0.0.0", DEFAULT_PORT)).await?;
    tracing::info!(port = DEFAULT_PORT, "agent listening");

    let discovery = spawn_discovery(state.clone());
    let accept = {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                match tcp.accept().await {
                    Ok((sock, peer)) => {
                        tracing::info!(%peer, "console connected");
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(sock, state).await {
                                tracing::warn!(error = %e, "connection ended");
                            }
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "accept failed"),
                }
            }
        })
    };

    shutdown.wait().await;
    tracing::info!("shutting down agent");
    accept.abort();
    discovery.abort();
    Ok(())
}

async fn handle_conn(sock: tokio::net::TcpStream, state: Arc<State>) -> Result<()> {
    let peer = sock.peer_addr().ok();
    let (mut rd, mut wr) = sock.into_split();
    while let Some(env) = read_frame(&mut rd).await? {
        let Body::Command(cmd) = env.body else {
            continue; // clients ignore stray reports
        };
        let reply = dispatch(&state, cmd, peer).await;
        write_frame(&mut wr, &Envelope::report(reply)).await?;
    }
    Ok(())
}

async fn dispatch(state: &State, cmd: Command, peer: Option<std::net::SocketAddr>) -> Report {
    match cmd {
        Command::QueryStatus => Report::Status(status(state).await),
        Command::PushPolicy { policy } => apply_push(state, policy).await,
        Command::LaunchClientUi => match launch_ui() {
            Ok(()) => Report::Ack { command: "launch_client_ui".into(), note: None },
            Err(e) => Report::Error { command: "launch_client_ui".into(), message: e.to_string() },
        },
        Command::RequestThumbnail { max_dim } => {
            match capture::capture_thumbnail(state.machine_id, max_dim.clamp(64, 1920)) {
                Ok(t) => Report::Thumbnail(t),
                Err(e) => Report::Error { command: "request_thumbnail".into(), message: e.to_string() },
            }
        }
        Command::RebootAndRestore { restore, delay_secs } => {
            tracing::warn!(?peer, restore, "reboot requested by console");
            match features::reboot_and_restore(restore, delay_secs.clamp(0, 600)) {
                Ok(()) => Report::Ack { command: "reboot_and_restore".into(), note: Some("scheduled".into()) },
                Err(e) => Report::Error { command: "reboot_and_restore".into(), message: e.to_string() },
            }
        }
    }
}

async fn apply_push(state: &State, incoming: Policy) -> Report {
    let mut guard = state.policy.lock().await;
    if !guard.apply(incoming) {
        return Report::Ack { command: "push_policy".into(), note: Some("ignored older revision".into()) };
    }
    let snapshot = guard.clone();
    drop(guard);
    features::apply_policy(&snapshot);
    if let Err(e) = config::save(&snapshot) {
        return Report::Error { command: "push_policy".into(), message: e.to_string() };
    }
    Report::Ack { command: "push_policy".into(), note: Some(format!("applied rev {}", snapshot.revision)) }
}

async fn status(state: &State) -> ClientStatus {
    let policy = state.policy.lock().await.clone();
    ClientStatus {
        machine_id: state.machine_id,
        hostname: hostname(),
        ip: String::new(), // filled in by the console from the socket
        logged_in_user: std::env::var("USERNAME").ok(),
        policy_revision: policy.revision,
        uwf_enabled: features::current_uwf_enabled(),
        session_locked: lock::configured_hash().is_some(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn spawn_discovery(state: Arc<State>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let sock = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "discovery disabled");
                return;
            }
        };
        let mut buf = [0u8; 256];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
            if &buf[..n.min(DISCOVERY_MAGIC.len())] != DISCOVERY_MAGIC.as_bytes() {
                continue;
            }
            // Answer with our id/hostname/port so the console can connect.
            let reply = format!(
                "{DISCOVERY_MAGIC} {} {} {}",
                state.machine_id,
                hostname(),
                DEFAULT_PORT
            );
            let _ = sock.send_to(reply.as_bytes(), from).await;
        }
    })
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn launch_ui() -> Result<()> {
    // Start the GUI client (labctl-student-ui) if it isn't already running.
    // It lives next to this agent executable. A second launch simply signals
    // the existing tray instance to show itself, so this is idempotent.
    //
    // Note: when the agent runs as a session-0 service, a plain spawn won't
    // surface in the interactive desktop; the GUI's logon autostart is the
    // primary path, and this is a best-effort nudge.
    let dir = std::env::current_exe()?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let ui_exe = dir.join(if cfg!(windows) {
        "labctl-student-ui.exe"
    } else {
        "labctl-student-ui"
    });
    std::process::Command::new(ui_exe).arg("--show").spawn()?;
    Ok(())
}

fn load_or_create_machine_id() -> Uuid {
    let path = labctl_common::paths::data_dir().join("machine-id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(id) = s.trim().parse() {
            return id;
        }
    }
    let id = Uuid::new_v4();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, id.to_string());
    id
}

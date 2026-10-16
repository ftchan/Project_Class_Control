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

    let state = Arc::new(State {
        machine_id: load_or_create_machine_id(),
        policy: Mutex::new(policy.clone()),
    });

    // Bind and start serving FIRST, so the console/GUI can always reach us
    // quickly. Policy enforcement (which may run slow tools like netsh/DISM)
    // happens afterwards on a blocking thread and never delays connectivity.
    let tcp = TcpListener::bind(("0.0.0.0", DEFAULT_PORT)).await?;
    tracing::info!(port = DEFAULT_PORT, "agent listening");

    tokio::task::spawn_blocking(move || {
        tracing::info!(revision = policy.revision, "applying startup policy");
        features::apply_policy(&policy);
    });

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
        Command::UpdateClient { filename, target_path, data_base64, run_after } => {
            match update_client(&filename, target_path.as_deref(), &data_base64, run_after) {
                Ok(note) => Report::Ack { command: "update_client".into(), note: Some(note) },
                Err(e) => Report::Error { command: "update_client".into(), message: e.to_string() },
            }
        }
        Command::ApplyDefaults { wallpaper_data_base64, wallpaper_name } => {
            // Save any wallpaper image locally so the GUI gets a valid local
            // path; per-user settings are applied by the GUI in the session.
            let local = wallpaper_data_base64
                .as_deref()
                .and_then(|b| save_defaults_wallpaper(b, wallpaper_name.as_deref()));
            match forward_defaults_to_ui(local.as_deref()) {
                Ok(()) => Report::Ack { command: "apply_defaults".into(), note: Some("已下发到客户端".into()) },
                Err(e) => Report::Error { command: "apply_defaults".into(), message: e.to_string() },
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

async fn apply_push(state: &State, mut incoming: Policy) -> Report {
    // Materialize any transferred wallpaper image to a LOCAL file and rewrite
    // the path, so students never point at a teacher-only path (which blanks
    // the desktop). Clears the bytes so policy.json stays small.
    materialize_wallpaper(&mut incoming);

    let mut guard = state.policy.lock().await;
    if !guard.apply(incoming) {
        return Report::Ack { command: "push_policy".into(), note: Some("ignored older revision".into()) };
    }
    let snapshot = guard.clone();
    drop(guard);
    let snap2 = snapshot.clone();
    let _ = tokio::task::spawn_blocking(move || features::apply_policy(&snap2)).await;
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
    // Start the GUI (same binary, ui mode) if not already running.
    // It lives next to this agent executable. A second launch simply signals
    // the existing tray instance to show itself, so this is idempotent.
    //
    // Note: when the agent runs as a session-0 service, a plain spawn won't
    // surface in the interactive desktop; the GUI's logon autostart is the
    // primary path, and this is a best-effort nudge.
    // The GUI is the same binary in `ui` mode.
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).args(["ui", "--show"]).spawn()?;
    Ok(())
}

/// Seamlessly replace a client executable and relaunch it.
///
/// Windows can't overwrite a running .exe, so we write the new bytes to a
/// sibling `.new` file, stop the running copy, swap files, then relaunch.
fn update_client(
    filename: &str,
    target_path: Option<&str>,
    data_base64: &str,
    run_after: bool,
) -> anyhow::Result<String> {
    use base64::Engine as _;
    use std::path::PathBuf;

    // Reject path separators in the bare filename to avoid surprises.
    anyhow::ensure!(
        !filename.contains(['/', '\\']) && filename.ends_with(".exe"),
        "invalid client filename"
    );

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|e| anyhow::anyhow!("bad base64: {e}"))?;
    anyhow::ensure!(!bytes.is_empty(), "empty payload");

    let target: PathBuf = match target_path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_exe()?
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(filename),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let basename = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename)
        .to_string();
    let new_path = target.with_file_name(format!("{basename}.new"));
    std::fs::write(&new_path, &bytes).map_err(|e| anyhow::anyhow!("write new exe: {e}"))?;

    let size = bytes.len();

    #[cfg(windows)]
    {
        // The client is the same binary as the running service, so we can't
        // replace it from within the process. Spawn a detached helper that
        // waits for us to send our reply, stops the service, swaps the file,
        // restarts the service, and relaunches the GUI.
        let start_ui = if run_after {
            format!("start \"\" \"{t}\" ui --show\r\n", t = target.display())
        } else {
            String::new()
        };
        let bat = format!(
            "@echo off\r\n\
             ping 127.0.0.1 -n 3 >nul\r\n\
             sc stop LabCtlStudent >nul 2>&1\r\n\
             taskkill /f /im \"{base}\" >nul 2>&1\r\n\
             ping 127.0.0.1 -n 2 >nul\r\n\
             move /y \"{new}\" \"{target}\" >nul 2>&1\r\n\
             sc start LabCtlStudent >nul 2>&1\r\n\
             {start_ui}\
             del \"%~f0\"\r\n",
            base = basename,
            new = new_path.display(),
            target = target.display(),
        );
        let bat_path = std::env::temp_dir().join("labctl-update.bat");
        std::fs::write(&bat_path, bat).map_err(|e| anyhow::anyhow!("write updater: {e}"))?;
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "/min", &bat_path.to_string_lossy()])
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawn updater: {e}"))?;
        return Ok(format!("更新已计划（{size} 字节），服务将重启"));
    }

    #[cfg(not(windows))]
    {
        // Dev/test path: just swap the file in place.
        let old_path = target.with_file_name(format!("{basename}.old"));
        let _ = std::fs::remove_file(&old_path);
        if target.exists() {
            std::fs::rename(&target, &old_path).ok();
        }
        std::fs::rename(&new_path, &target).map_err(|e| anyhow::anyhow!("install new exe: {e}"))?;
        let _ = std::fs::remove_file(&old_path);
        if run_after {
            std::process::Command::new(&target).spawn().ok();
        }
        Ok(format!("已更新 {basename}（{size} 字节）"))
    }
}

/// If the policy carries wallpaper bytes, write them to a local file and set
/// `wallpaper_path` to that file; then drop the bytes. No-op if absent.
fn materialize_wallpaper(policy: &mut Policy) {
    use base64::Engine as _;
    let Some(b64) = policy.wallpaper_data_base64.take() else { return };
    let name = policy.wallpaper_name.take().unwrap_or_else(|| "wallpaper.jpg".into());
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg");
    match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
        Ok(bytes) if !bytes.is_empty() => {
            let path = labctl_common::paths::data_dir().join(format!("wallpaper.{ext}"));
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(&path, &bytes).is_ok() {
                policy.wallpaper_path = Some(path.to_string_lossy().into_owned());
                tracing::info!(?path, "materialized wallpaper locally");
            }
        }
        _ => tracing::warn!("wallpaper payload empty/invalid"),
    }
}

/// Write a defaults wallpaper payload to a local file; return its path.
fn save_defaults_wallpaper(b64: &str, name: Option<&str>) -> Option<String> {
    use base64::Engine as _;
    let ext = name
        .and_then(|n| std::path::Path::new(n).extension().and_then(|e| e.to_str()))
        .unwrap_or("jpg");
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let path = labctl_common::paths::data_dir().join(format!("default-wallpaper.{ext}"));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, &bytes).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Send the "restore defaults" request to the GUI client's control port.
/// If the GUI isn't running, try to launch it first, then deliver.
fn forward_defaults_to_ui(wallpaper: Option<&str>) -> anyhow::Result<()> {
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::Duration;

    let msg = match wallpaper {
        Some(p) if !p.is_empty() => format!("defaults\t{p}"),
        _ => "defaults".to_string(),
    };

    let deliver = || -> anyhow::Result<()> {
        let mut s = TcpStream::connect_timeout(
            &(std::net::Ipv4Addr::LOCALHOST, labctl_common::UI_CONTROL_PORT).into(),
            Duration::from_millis(500),
        )?;
        s.write_all(msg.as_bytes())?;
        Ok(())
    };

    if deliver().is_ok() {
        return Ok(());
    }
    // GUI not up — try to start it, give it a moment, then retry once.
    let _ = launch_ui();
    std::thread::sleep(Duration::from_millis(1500));
    deliver()
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

//! Student (managed client) entry point.
//!
//! Usage:
//!   labctl-student run           Run the agent in the foreground (dev/testing).
//!   labctl-student service       Run as a Windows service (invoked by the SCM).
//!   labctl-student install       Register the Windows service (needs admin).
//!   labctl-student uninstall     Remove the Windows service (needs admin).
//!
//! The agent listens on the LAN for the teacher console, enforces the local
//! policy, and answers status/thumbnail requests. The same binary also hosts
//! the student GUI client (`ui` mode). All of this operates only on this
//! machine and only on an administrator-deployed policy.
//!
//! Single binary, dispatched by the first argument:
//!   run | service | install | uninstall | doctor | hash | ui [--show]
//!
//! The executable is built as a GUI-subsystem app (no console window for the
//! tray UI); console sub-commands attach to the parent terminal so their
//! output still appears when run from a shell.

// Avoid a console window for the tray GUI; console modes re-attach at runtime.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent;
mod config;
mod features;
mod firewall;
mod network;
mod capture;
mod lock;
mod ui;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod setup;

use anyhow::Result;

/// Re-attach to the launching terminal's console so stdout/stderr are visible
/// for console sub-commands (the binary itself is GUI-subsystem on Windows).
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| {
        #[cfg(windows)]
        { return if service::is_running() { "open" } else { "setup" }.into(); }
        #[cfg(not(windows))]
        { "run".into() }
    });

    // GUI and service modes have no console; everything else attaches to the
    // parent terminal so its output is visible.
    if !matches!(mode.as_str(), "ui" | "service" | "open") {
        attach_parent_console();
    }

    match mode.as_str() {
        #[cfg(windows)]
        "service" => return service::run_as_service(),
        #[cfg(windows)]
        "install" => return service::install(),
        #[cfg(windows)]
        "uninstall" => return service::uninstall(),
        #[cfg(windows)]
        "setup" => return setup::run(),
        #[cfg(windows)]
        "open" => return setup::open_ui(),
        "doctor" => return doctor(),
        "ui" => {
            init_tracing(ui_log_file());
            let show = std::env::args().any(|a| a == "--show") || std::env::args().len() == 1;
            return ui::run_gui(show);
        }
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
            eprintln!("modes: run | service | install | uninstall | ui | doctor | hash");
            std::process::exit(2);
        }
    }

    // Foreground run: set up logging and drive the agent on a Tokio runtime.
    init_tracing(Some(labctl_common::paths::log_dir().join("agent.log")));
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(agent::run(agent::ShutdownSignal::ctrl_c()))
}

/// Quick self-check: is the agent listening locally, and is a policy present?
/// Run `labctl-student doctor` on a machine where the GUI says "refused".
fn doctor() -> Result<()> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let port = labctl_common::DEFAULT_PORT;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    println!("LabCtl 学生端自检");
    println!("-------------------");

    match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(_) => println!("[OK] 本机 agent 正在监听 127.0.0.1:{port}（GUI 可连接）"),
        Err(e) => {
            println!("[X ] 无法连接 127.0.0.1:{port}：{e}");
            println!("     => agent 服务没有运行。请以管理员执行：labctl-student install");
            println!("        或检查服务状态：sc query LabCtlStudent");
        }
    }

    let pf = labctl_common::paths::policy_file();
    if pf.exists() {
        println!("[OK] 策略文件存在：{}", pf.display());
    } else {
        println!("[! ] 未找到策略文件：{}（将使用默认策略）", pf.display());
    }

    // Ask the agent: it runs as SYSTEM and can read the UWF WMI namespace.
    match ui::ipc::send(labctl_common::protocol::Command::QueryStatus) {
        Ok(labctl_common::protocol::Report::Status(st)) => {
            println!("[i ] 当前登录用户：{}", st.logged_in_user.as_deref().unwrap_or("（无）"));
            println!("[i ] UWF 状态：{}", st.uwf_state.as_deref().unwrap_or("未知"));
        }
        _ => println!("[! ] 无法从 agent 读取状态（UWF 状态未知）"),
    }
    println!("[i ] 服务日志：{}", labctl_common::paths::log_dir().join("agent.log").display());

    println!();
    println!("提示：教师机看不到本机时，多为防火墙拦截 TCP {port} / UDP {}。",
        labctl_common::DISCOVERY_PORT);
    println!("      agent 启动时会自动添加放行规则（需管理员/服务权限）。");
    Ok(())
}

/// A `Command` that never opens a console window. This exe is GUI-subsystem,
/// so console children (netsh, uwfmgr, ...) would otherwise flash a window
/// whenever the agent runs in an interactive session.
pub(crate) fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Log to `file` when given (the service has no console, so without a file
/// its errors would be lost), falling back to stderr if it cannot be opened.
fn init_tracing(file: Option<std::path::PathBuf>) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LABCTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    if let Some(log) = file.and_then(|p| open_log(&p)) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(log))
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }
}

/// Open `path` for appending, keeping one previous file once it passes 2 MB.
fn open_log(path: &std::path::Path) -> Option<std::fs::File> {
    const MAX_LOG: u64 = 2 * 1024 * 1024;
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(path).map(|m| m.len() > MAX_LOG).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// The GUI runs as the signed-in user, who cannot write the service's log
/// folder under ProgramData; it logs to the user's local app data instead.
fn ui_log_file() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::PathBuf::from(d).join("LabCtl").join("ui.log"))
}

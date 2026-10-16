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
mod firewall;
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
        "doctor" => return doctor(),
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
            eprintln!("modes: run | service | install | uninstall | ui | doctor | hash");
            std::process::exit(2);
        }
    }

    // Foreground run: set up logging and drive the agent on a Tokio runtime.
    init_tracing();
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

    println!();
    println!("提示：教师机看不到本机时，多为防火墙拦截 TCP {port} / UDP {}。",
        labctl_common::DISCOVERY_PORT);
    println!("      agent 启动时会自动添加放行规则（需管理员/服务权限）。");
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LABCTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

//! One-file deployment for a classroom machine. Running the EXE directly
//! installs it; subsequent direct launches open the already installed UI.

use anyhow::{Context, Result};
use labctl_common::policy::{Policy, UsbStoragePolicy};
use std::{ffi::OsStr, fs, os::windows::ffi::OsStrExt, path::PathBuf, time::Duration};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK, SW_SHOWNORMAL},
        },
    },
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn default_exe() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into()))
        .join("LabCtl")
        .join("labctl-student.exe")
}

fn installed_exe() -> PathBuf {
    let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
    let registered = hklm
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .ok()
        .and_then(|run| run.get_value::<String, _>("LabCtlStudentUI").ok());
    if let Some(value) = registered {
        if let Some(exe) = value.strip_prefix('"').and_then(|s| s.split('"').next()) {
            if !exe.is_empty() {
                return PathBuf::from(exe);
            }
        }
    }
    default_exe()
}

#[derive(Default)]
struct SetupOptions {
    quiet: bool,
    install_dir: Option<PathBuf>,
    usb: Option<UsbStoragePolicy>,
    uwf: Option<Option<bool>>,
    lock_wallpaper: bool,
    wallpaper_path: Option<String>,
    passphrase: Option<String>,
}

impl SetupOptions {
    fn parse() -> Result<Self> {
        let mut args = std::env::args().skip(2);
        let mut out = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--quiet" => out.quiet = true,
                "--install-dir" => {
                    out.install_dir = Some(PathBuf::from(
                        args.next().context("--install-dir 需要目录")?,
                    ))
                }
                "--lock-wallpaper" => out.lock_wallpaper = true,
                "--wallpaper-path" => {
                    out.wallpaper_path = Some(args.next().context("--wallpaper-path 需要路径")?)
                }
                "--passphrase" => {
                    out.passphrase = Some(args.next().context("--passphrase 需要密码")?)
                }
                "--usb" => {
                    out.usb = Some(
                        match args
                            .next()
                            .context("--usb 需要 allow/read_only/deny")?
                            .as_str()
                        {
                            "allow" => UsbStoragePolicy::Allow,
                            "read_only" | "readonly" => UsbStoragePolicy::ReadOnly,
                            "deny" => UsbStoragePolicy::Deny,
                            other => anyhow::bail!("未知的 U 盘策略：{other}"),
                        },
                    )
                }
                "--uwf" => {
                    out.uwf = Some(
                        match args.next().context("--uwf 需要 on/off/unmanaged")?.as_str() {
                            "on" => Some(true),
                            "off" => Some(false),
                            "unmanaged" => None,
                            other => anyhow::bail!("未知的 UWF 设置：{other}"),
                        },
                    )
                }
                other => anyhow::bail!("未知的安装参数：{other}"),
            }
        }
        Ok(out)
    }
}

pub fn open_ui() -> Result<()> {
    use std::io::Write;
    if let Ok(mut stream) =
        std::net::TcpStream::connect(("127.0.0.1", labctl_common::UI_CONTROL_PORT))
    {
        stream.write_all(b"show")?;
        return Ok(());
    }
    std::process::Command::new(installed_exe())
        .args(["ui", "--show"])
        .spawn()
        .context("打开已安装的学生端界面")?;
    Ok(())
}

fn is_elevated() -> Result<bool> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0;
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    result?;
    Ok(elevation.TokenIsElevated != 0)
}

fn quote_argument(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            out.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

fn elevate() -> Result<()> {
    let exe = std::env::current_exe()?;
    let path = wide(exe.as_os_str());
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        args.push("setup".into());
    }
    let parameters = args
        .iter()
        .map(|a| quote_argument(a))
        .collect::<Vec<_>>()
        .join(" ");
    let params = wide(OsStr::new(&parameters));
    let status = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("runas"),
            PCWSTR(path.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    anyhow::ensure!(
        status.0 as isize > 32,
        "管理员授权未完成（ShellExecute 错误 {}）",
        status.0
    );
    Ok(())
}

fn inform(message: &str) {
    let body = wide(OsStr::new(message));
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(body.as_ptr()),
            w!("LabCtl 学生端"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub fn run() -> Result<()> {
    let options = SetupOptions::parse()?;
    if !is_elevated()? {
        return elevate();
    }
    let result = install_all(&options);
    if !options.quiet {
        match &result {
            Ok(()) => inform("学生端已安装并启动。首次启用 UWF 可能需要重启两次才能生效。"),
            Err(e) => inform(&format!("安装失败：{e:#}")),
        }
    }
    result
}

fn install_all(options: &SetupOptions) -> Result<()> {
    use sysinfo::System;

    let source = std::env::current_exe()?;
    let previous_target = installed_exe();
    let target = options
        .install_dir
        .as_ref()
        .map(|dir| dir.join("labctl-student.exe"))
        .unwrap_or_else(|| previous_target.clone());
    anyhow::ensure!(
        target.is_absolute(),
        "安装目录必须是绝对路径：{}",
        target.display()
    );
    let same_file = source
        .to_string_lossy()
        .eq_ignore_ascii_case(&target.to_string_lossy());
    fs::create_dir_all(target.parent().unwrap()).context("创建安装目录")?;

    // Match the old install.bat defaults, but retain a teacher/admin policy
    // on reinstall so an upgrade never silently resets classroom settings.
    let policy_exists = labctl_common::paths::policy_file().exists();
    let mut policy = if policy_exists {
        crate::config::load()
    } else {
        Policy {
            usb_storage: UsbStoragePolicy::Deny,
            uwf_enabled: Some(true),
            revision: chrono::Utc::now().timestamp() as u64,
            ..Policy::default()
        }
    };
    let override_policy = options.usb.is_some()
        || options.uwf.is_some()
        || options.lock_wallpaper
        || options.wallpaper_path.is_some()
        || options.passphrase.is_some();
    if let Some(usb) = options.usb {
        policy.usb_storage = usb;
    }
    if let Some(uwf) = options.uwf {
        policy.uwf_enabled = uwf;
    }
    if options.lock_wallpaper {
        policy.lock_wallpaper = true;
    }
    if let Some(path) = &options.wallpaper_path {
        policy.wallpaper_path = Some(path.clone());
    }
    if let Some(passphrase) = &options.passphrase {
        policy.config_passphrase_hash = Some(labctl_common::auth::hash_passphrase(passphrase)?);
    }
    if !policy_exists || override_policy {
        if policy_exists {
            policy.revision = policy.revision.saturating_add(1);
        }
        crate::config::save(&policy).context("保存初始策略")?;
        crate::features::uwf::commit_policy_file().context("持久化策略文件")?;
    }

    if policy.uwf_enabled == Some(true) && crate::features::uwf::state() == "not_installed" {
        let output = crate::hidden_command("dism.exe")
            .args([
                "/Online",
                "/Enable-Feature",
                "/FeatureName:Client-UnifiedWriteFilter",
                "/All",
                "/NoRestart",
            ])
            .output()
            .context("启用 UWF 功能")?;
        anyhow::ensure!(
            matches!(output.status.code(), Some(0 | 3010)),
            "启用 UWF 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
    let (run, _) = hklm.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    let firewall_errors = crate::firewall::ensure_rules();
    anyhow::ensure!(
        firewall_errors.is_empty(),
        "配置防火墙失败：{}",
        firewall_errors.join("；")
    );

    // Stage the new binary before touching the running installation. Never
    // copy over a live EXE: Windows can reject that or leave a partial file.
    let staged = target.with_extension("exe.new");
    if !same_file {
        fs::copy(&source, &staged).context("准备新版本程序文件")?;
    }

    // Stop the old service only after preparing policy and optional features.
    // Limit process cleanup to this installation path, never to another copy
    // that happens to have the same file name.
    crate::service::uninstall().context("停止旧版学生端服务")?;
    if !same_file {
        let mut system = System::new();
        system.refresh_processes();
        for process in system.processes().values() {
            if process.pid().as_u32() != std::process::id()
                && process.exe().is_some_and(|p| {
                    let path = p.to_string_lossy();
                    path.eq_ignore_ascii_case(&target.to_string_lossy())
                        || path.eq_ignore_ascii_case(&previous_target.to_string_lossy())
                })
            {
                let _ = process.kill();
            }
        }
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            system.refresh_processes();
            if !system.processes().values().any(|p| {
                p.pid().as_u32() != std::process::id()
                    && p.exe().is_some_and(|exe| {
                        let path = exe.to_string_lossy();
                        path.eq_ignore_ascii_case(&target.to_string_lossy())
                            || path.eq_ignore_ascii_case(&previous_target.to_string_lossy())
                    })
            }) {
                break;
            }
        }
    }

    let backup = target.with_extension(format!("exe.previous.{}", std::process::id()));
    if !same_file {
        if target.exists() {
            if let Err(e) = fs::rename(&target, &backup) {
                let _ = crate::hidden_command(&previous_target)
                    .arg("install")
                    .status();
                return Err(e).context("保存旧版程序文件");
            }
        }
        if let Err(e) = fs::rename(&staged, &target) {
            if backup.exists() {
                let _ = fs::rename(&backup, &target);
            }
            let _ = crate::hidden_command(&previous_target)
                .arg("install")
                .status();
            return Err(e).context("启用新版本程序文件");
        }
    }

    // SCM must point at the installed copy, not at a removable source file.
    let status = crate::hidden_command(&target).arg("install").status();
    if !status.as_ref().is_ok_and(|s| s.success()) {
        let reason = status
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string());
        let _ = crate::service::uninstall();
        if backup.exists() {
            let _ = fs::remove_file(&target);
            let _ = fs::rename(&backup, &target);
        }
        if previous_target.exists() {
            let _ = crate::hidden_command(&previous_target)
                .arg("install")
                .status();
        }
        anyhow::bail!("注册学生端服务失败：{reason}");
    }
    if let Err(e) = run.set_value("LabCtlStudentUI", &format!("\"{}\" ui", target.display())) {
        tracing::warn!(error = %e, "could not set login autostart; service recovery remains active");
    }
    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }
    // The service can start the UI with the signed-in user's token. Retry
    // briefly while its listener comes up; its recovery loop remains a
    // fallback if no one is logged in yet.
    for _ in 0..10 {
        if crate::ui::ipc::send(labctl_common::protocol::Command::LaunchClientUi).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::quote_argument;

    #[test]
    fn elevation_preserves_paths_and_quotes() {
        assert_eq!(
            quote_argument(r"C:\Program Files\LabCtl\"),
            r#""C:\Program Files\LabCtl\\""#
        );
        assert_eq!(quote_argument("a\"b"), "\"a\\\"b\"");
    }
}

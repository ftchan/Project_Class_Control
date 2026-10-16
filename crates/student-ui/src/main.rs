//! Student GUI client (slint).
//!
//! A tray-resident, teacher-managed client for the lab machine. It:
//!   * requires an access password to view/operate (first run sets it);
//!   * lets the user turn UWF on/off (routed through the local agent, which
//!     has the privileges);
//!   * reflects the teacher-pushed policy (polled from the local agent);
//!   * stays in the tray — closing the window hides it, exiting is gated by
//!     the password; it auto-starts at logon.
//!
//! It is a thin, user-facing front end over the local agent service. All real
//! enforcement lives in the agent; this UI only requests actions on the same
//! machine.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ipc;
mod store;

slint::include_modules!();

use labctl_common::policy::{Policy, UsbStoragePolicy};
use labctl_common::protocol::{Command, Report};
use slint::{CloseRequestResponse, Timer, TimerMode, Weak};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LABCTL_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    // Single instance: if one is already running, wake it and exit.
    if store::signal_existing_instance() {
        return Ok(());
    }
    if let Err(e) = store::ensure_autostart() {
        tracing::warn!(error = %e, "could not register autostart");
    }

    let ui = MainWindow::new()?;
    let policy = store::load_policy();
    let gate = Rc::new(RefCell::new(store::load_gate(&policy)));

    ui.set_locked(true);
    ui.set_first_run(gate.borrow().first_run());
    ui.set_hostname(hostname().into());
    if gate.borrow().teacher_managed {
        ui.set_message("访问密码由教师端统一设置".into());
    }
    apply_policy_display(&ui, &policy);

    wire_unlock(&ui, gate.clone());
    wire_toggle_uwf(&ui);
    wire_window_buttons(&ui);

    // Tray is Windows-only (the deployment target). On dev platforms it is a
    // no-op so the window still runs for UI work.
    let _tray_guard = start_tray(ui.as_weak());
    let refresh_timer = refresh_timer(ui.as_weak());
    spawn_control_listener(ui.as_weak());

    ui.show()?;
    slint::run_event_loop()?;

    drop(refresh_timer);
    drop(_tray_guard);
    Ok(())
}

// ---------------- callbacks ----------------

fn wire_unlock(ui: &MainWindow, gate: Rc<RefCell<store::Gate>>) {
    let ui_w = ui.as_weak();
    ui.on_unlock(move |pw| {
        let Some(ui) = ui_w.upgrade() else { return };
        let pw = pw.to_string();
        let mut g = gate.borrow_mut();
        if g.first_run() {
            match store::set_local_password(&pw) {
                Ok(h) => {
                    g.hash = Some(h);
                    ui.set_first_run(false);
                    unlock_ok(&ui);
                }
                Err(e) => ui.set_message(format!("保存密码失败：{e}").into()),
            }
        } else {
            let ok = g
                .hash
                .as_deref()
                .map(|h| labctl_common::auth::verify_passphrase(&pw, h).unwrap_or(false))
                .unwrap_or(false);
            if ok {
                unlock_ok(&ui);
            } else {
                ui.set_message("密码错误".into());
            }
        }
    });
}

fn unlock_ok(ui: &MainWindow) {
    ui.set_locked(false);
    ui.set_message("".into());
    ui.set_pw("".into());
    ui.set_pw2("".into());
    refresh(ui);
}

fn wire_toggle_uwf(ui: &MainWindow) {
    let ui_w = ui.as_weak();
    ui.on_toggle_uwf(move |target| {
        let Some(ui) = ui_w.upgrade() else { return };
        ui.set_busy(true);
        ui.set_message(if target { "正在请求开启 UWF…" } else { "正在请求关闭 UWF…" }.into());
        let back = ui.as_weak();
        std::thread::spawn(move || {
            let mut p = store::load_policy();
            p.uwf_enabled = Some(target);
            p.revision = now_ms();
            let msg = match ipc::send(Command::PushPolicy { policy: p }) {
                Ok(Report::Ack { note, .. }) => format!(
                    "已提交，重启后生效{}",
                    note.map(|n| format!("（{n}）")).unwrap_or_default()
                ),
                Ok(Report::Error { message, .. }) => format!("失败：{message}"),
                Ok(_) => "已提交".to_string(),
                Err(e) => format!("无法连接本机服务：{e}"),
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = back.upgrade() {
                    ui.set_busy(false);
                    ui.set_message(msg.into());
                    refresh(&ui);
                }
            });
        });
    });
}

fn wire_window_buttons(ui: &MainWindow) {
    let hide_w = ui.as_weak();
    ui.on_hide_window(move || {
        if let Some(u) = hide_w.upgrade() {
            let _ = u.hide();
        }
    });
    ui.on_request_exit(move || {
        let _ = slint::quit_event_loop();
    });

    // Closing the window hides to tray instead of quitting.
    let close_w = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(u) = close_w.upgrade() {
            let _ = u.hide();
        }
        CloseRequestResponse::HideWindow
    });
}

// ---------------- status refresh ----------------

fn apply_policy_display(ui: &MainWindow, p: &Policy) {
    ui.set_wallpaper_locked(p.lock_wallpaper);
    ui.set_usb_text(
        match p.usb_storage {
            UsbStoragePolicy::Allow => "允许",
            UsbStoragePolicy::ReadOnly => "只读",
            UsbStoragePolicy::Deny => "禁用",
        }
        .into(),
    );
    ui.set_rev_text(format!("策略 rev {}", p.revision).into());
    match p.uwf_enabled {
        Some(true) => {
            ui.set_uwf_on(true);
            ui.set_uwf_text("已开启".into());
        }
        Some(false) => {
            ui.set_uwf_on(false);
            ui.set_uwf_text("已关闭".into());
        }
        None => {
            ui.set_uwf_on(false);
            ui.set_uwf_text("未管理".into());
        }
    }
}

/// Refresh display: local policy immediately, live UWF/host from the agent.
fn refresh(ui: &MainWindow) {
    let p = store::load_policy();
    apply_policy_display(ui, &p);

    let back = ui.as_weak();
    std::thread::spawn(move || {
        if let Ok(Report::Status(st)) = ipc::send(Command::QueryStatus) {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = back.upgrade() {
                    if let Some(on) = st.uwf_enabled {
                        ui.set_uwf_on(on);
                        ui.set_uwf_text(if on { "已开启" } else { "已关闭" }.into());
                    }
                    if !st.hostname.is_empty() {
                        ui.set_hostname(st.hostname.into());
                    }
                    ui.set_rev_text(format!("策略 rev {}", st.policy_revision).into());
                }
            });
        }
    });
}

fn refresh_timer(ui_w: Weak<MainWindow>) -> Timer {
    let t = Timer::default();
    t.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        if let Some(u) = ui_w.upgrade() {
            if !u.get_locked() {
                refresh(&u);
            }
        }
    });
    t
}

// ---------------- tray (Windows) ----------------

/// On Windows, create the tray icon and a timer that polls its menu events.
/// Returns a guard that must be kept alive for the tray to persist.
#[cfg(windows)]
fn start_tray(ui_w: Weak<MainWindow>) -> Box<dyn std::any::Any> {
    use tray_icon::menu::{Menu, MenuItem};

    let show = MenuItem::new("显示主界面", true, None);
    let exit = MenuItem::new("退出程序", true, None);
    let menu = Menu::new();
    let _ = menu.append(&show);
    let _ = menu.append(&exit);

    let tray = tray_icon::TrayIconBuilder::new()
        .with_tooltip("机房学生端")
        .with_menu(Box::new(menu))
        .with_icon(make_icon())
        .build();

    let tray = match tray {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "tray creation failed");
            return Box::new(());
        }
    };

    let (show_id, exit_id) = (show.id().clone(), exit.id().clone());
    let menu_rx = tray_icon::menu::MenuEvent::receiver();
    let tray_rx = tray_icon::TrayIconEvent::receiver();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(150), move || {
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == show_id {
                show_and_lock(&ui_w);
            } else if ev.id == exit_id {
                if let Some(u) = ui_w.upgrade() {
                    if u.get_locked() {
                        let _ = u.show();
                        u.set_message("退出前请先输入密码".into());
                    } else {
                        let _ = slint::quit_event_loop();
                    }
                }
            }
        }
        while let Ok(_ev) = tray_rx.try_recv() {}
    });

    Box::new((tray, timer))
}

#[cfg(windows)]
fn make_icon() -> tray_icon::Icon {
    let (w, h) = (32usize, 32usize);
    let mut rgba = vec![0u8; w * h * 4];
    for px in rgba.chunks_mut(4) {
        px[0] = 0x18;
        px[1] = 0x18;
        px[2] = 0x1b;
        px[3] = 0xff;
    }
    tray_icon::Icon::from_rgba(rgba, w as u32, h as u32).expect("tray icon")
}

#[cfg(not(windows))]
fn start_tray(_ui_w: Weak<MainWindow>) -> Box<dyn std::any::Any> {
    tracing::info!("tray disabled on this platform (dev build)");
    Box::new(())
}

// ---------------- helpers ----------------

/// Show the window and force the lock gate (clearing any cached password).
fn show_and_lock(ui_w: &Weak<MainWindow>) {
    if let Some(u) = ui_w.upgrade() {
        u.set_locked(true);
        u.set_pw("".into());
        u.set_pw2("".into());
        u.set_message("".into());
        let _ = u.show();
    }
}

fn spawn_control_listener(ui_w: Weak<MainWindow>) {
    match std::net::TcpListener::bind(("127.0.0.1", store::UI_CONTROL_PORT)) {
        Ok(listener) => {
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stream.is_ok() {
                        let w = ui_w.clone();
                        let _ = slint::invoke_from_event_loop(move || show_and_lock(&w));
                    }
                }
            });
        }
        Err(e) => tracing::warn!(error = %e, "control listener bind failed"),
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

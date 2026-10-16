//! Student GUI client, built into the same binary as the agent.
//!
//! Launched via `labctl-student ui [--show]`. Tray-resident, teacher-managed:
//! password gate, read-only status, and per-user policy enforcement (wallpaper,
//! Task Manager) in the interactive session. Talks to the local agent over
//! loopback.

pub(crate) mod ipc;
mod store;
mod userpolicy;

slint::include_modules!();

use labctl_common::policy::{Policy, UsbStoragePolicy};
use labctl_common::protocol::{Command, Report};
use slint::{CloseRequestResponse, Timer, TimerMode, Weak};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Revision of the policy whose per-user parts we last enforced.
static LAST_APPLIED_REV: AtomicU64 = AtomicU64::new(u64::MAX);

pub fn run_gui(show_on_start: bool) -> anyhow::Result<()> {
    // Single instance: if one is already running, wake it and exit.
    if store::signal_existing_instance() {
        return Ok(());
    }
    if let Err(e) = store::clean_legacy_autostart() {
        tracing::warn!(error = %e, "could not clean legacy autostart entry");
    }

    let ui = MainWindow::new()?;
    ui.set_app_icon(app_icon());
    let policy = store::load_policy();
    let gate = Rc::new(RefCell::new(store::load_gate(&policy)));

    userpolicy::apply(&policy);
    LAST_APPLIED_REV.store(policy.revision, Ordering::Relaxed);

    ui.set_locked(true);
    ui.set_first_run(gate.borrow().first_run());
    ui.set_hostname(hostname().into());
    if gate.borrow().teacher_managed {
        ui.set_message("访问密码由教师端统一设置".into());
    }
    apply_policy_display(&ui, &policy);

    wire_unlock(&ui, gate.clone());
    wire_window_buttons(&ui);

    let _tray_guard = start_tray(ui.as_weak());
    let refresh_timer = refresh_timer(ui.as_weak());
    let policy_timer = policy_watch_timer();
    spawn_control_listener(ui.as_weak());

    if show_on_start {
        ui.show()?;
    }
    // Keep running with every window hidden: the client lives in the tray and
    // must keep enforcing per-user policy. Only the tray "exit" item quits.
    slint::run_event_loop_until_quit()?;

    drop(refresh_timer);
    drop(policy_timer);
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

fn wire_window_buttons(ui: &MainWindow) {
    let hide_w = ui.as_weak();
    ui.on_hide_window(move || hide_and_lock(&hide_w));

    ui.on_request_exit(move || {
        let _ = slint::quit_event_loop();
    });

    let close_w = ui.as_weak();
    ui.window().on_close_requested(move || {
        hide_and_lock(&close_w);
        CloseRequestResponse::HideWindow
    });
}

/// Hide to tray and re-lock, clearing any entered password.
fn hide_and_lock(ui_w: &Weak<MainWindow>) {
    if let Some(u) = ui_w.upgrade() {
        u.set_locked(true);
        u.set_pw("".into());
        u.set_pw2("".into());
        u.set_message("".into());
        let _ = u.hide();
    }
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
    // What the teacher asked for; the live state from the agent replaces it.
    ui.set_uwf_on(false);
    ui.set_uwf_text(
        match p.uwf_enabled {
            Some(true) => "已下发开启",
            Some(false) => "已下发关闭",
            None => "未管理",
        }
        .into(),
    );
}

/// Display text for `ClientStatus::uwf_state`, and whether it is protecting.
fn uwf_label(state: &str) -> (String, bool) {
    let text = match state {
        "on" => "已开启（重启即还原）",
        "on_next_boot" => "已设置，重启后生效",
        "off_next_boot" => "重启后关闭",
        "off" => "已关闭",
        "installing" => "正在安装 UWF 功能…",
        "install_pending_reboot" => "功能已安装，重启后启用",
        "not_installed" => "未安装",
        other => return (other.strip_prefix("error:").map_or(other.to_string(), |e| format!("出错：{e}")), false),
    };
    (text.to_string(), state == "on")
}

/// Refresh display: local policy immediately, live UWF/host from the agent.
fn refresh(ui: &MainWindow) {
    let p = store::load_policy();
    apply_policy_display(ui, &p);

    let back = ui.as_weak();
    std::thread::spawn(move || match ipc::send(Command::QueryStatus) {
        Ok(Report::Status(st)) => {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = back.upgrade() {
                    if let Some(state) = st.uwf_state.as_deref() {
                        let (text, on) = uwf_label(state);
                        ui.set_uwf_on(on);
                        ui.set_uwf_text(text.into());
                    }
                    if !st.hostname.is_empty() {
                        ui.set_hostname(st.hostname.into());
                    }
                    ui.set_rev_text(format!("策略 rev {}", st.policy_revision).into());
                }
            });
        }
        _ => {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = back.upgrade() {
                    ui.set_uwf_text("服务未运行".into());
                    ui.set_uwf_on(false);
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

/// Enforce per-user policy on revision change regardless of lock state, so it
/// takes effect while the client sits hidden in the tray.
fn policy_watch_timer() -> Timer {
    let t = Timer::default();
    t.start(TimerMode::Repeated, Duration::from_secs(3), || {
        let p = store::load_policy();
        if LAST_APPLIED_REV.load(Ordering::Relaxed) != p.revision {
            userpolicy::apply(&p);
            LAST_APPLIED_REV.store(p.revision, Ordering::Relaxed);
        }
    });
    t
}

// ---------------- tray (Windows) ----------------

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
        while let Ok(ev) = tray_rx.try_recv() {
            if let tray_icon::TrayIconEvent::Click { .. } = ev {
                show_and_lock(&ui_w);
            }
        }
    });

    Box::new((tray, timer))
}

#[cfg(windows)]
fn make_icon() -> tray_icon::Icon {
    let rgba = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icon32.rgba")).to_vec();
    tray_icon::Icon::from_rgba(rgba, 32, 32).expect("tray icon")
}

/// The window icon, decoded from the embedded raw RGBA (no PNG decoder needed).
fn app_icon() -> slint::Image {
    use slint::{Rgba8Pixel, SharedPixelBuffer};
    let raw = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icon256.rgba"));
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(256, 256);
    buf.make_mut_bytes().copy_from_slice(raw);
    slint::Image::from_rgba8(buf)
}

#[cfg(not(windows))]
fn start_tray(_ui_w: Weak<MainWindow>) -> Box<dyn std::any::Any> {
    tracing::info!("tray disabled on this platform (dev build)");
    Box::new(())
}

// ---------------- helpers ----------------

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
    use std::io::{Read, Write};
    match std::net::TcpListener::bind(("127.0.0.1", store::UI_CONTROL_PORT)) {
        Ok(listener) => {
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut s) = stream else { continue };
                    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf);
                    let msg = buf.trim().to_string();

                    if let Some(rest) = msg.strip_prefix("thumb") {
                        // The agent (a session-0 service) cannot see this
                        // desktop, so it asks us for the screen thumbnail.
                        let max_dim = rest.trim().parse().unwrap_or(320);
                        let reply = crate::capture::capture_thumbnail(uuid::Uuid::nil(), max_dim)
                            .map_err(anyhow::Error::from)
                            .and_then(|t| Ok(serde_json::to_vec(&t)?));
                        match reply {
                            Ok(json) => {
                                let _ = s.write_all(&json);
                            }
                            Err(e) => tracing::warn!(error = %e, "thumbnail capture failed"),
                        }
                    } else if let Some(rest) = msg.strip_prefix("defaults") {
                        let path = rest.trim().trim_start_matches('\t').trim();
                        let path = (!path.is_empty()).then(|| path.to_string());
                        std::thread::spawn(move || userpolicy::apply_defaults(path.as_deref()));
                    } else if msg == "show" {
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

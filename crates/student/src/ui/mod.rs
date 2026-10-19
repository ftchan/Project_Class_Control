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

use labctl_common::policy::{Policy, UsbStoragePolicy, UwfScope};
use labctl_common::protocol::{Command, Report};
use slint::{CloseRequestResponse, Timer, TimerMode, Weak};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
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
    wire_admin(&ui);

    let _tray_guard = start_tray(ui.as_weak());
    let refresh_timer = refresh_timer(ui.as_weak());
    let policy_timer = policy_watch_timer(ui.as_weak(), gate.clone());
    spawn_control_listener(ui.as_weak());

    if show_on_start {
        ui.show()?;
    }
    // Keep running with every window hidden: the client lives in the tray and
    // must keep enforcing per-user policy.
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

    let close_w = ui.as_weak();
    ui.window().on_close_requested(move || {
        hide_and_lock(&close_w);
        CloseRequestResponse::HideWindow
    });
}

fn show_admin_policy(ui: &MainWindow, p: &Policy) {
    ui.set_admin_lock_wallpaper(p.lock_wallpaper);
    ui.set_admin_wallpaper_path(p.wallpaper_path.as_deref().unwrap_or("").into());
    ui.set_admin_usb(match p.usb_storage {
        UsbStoragePolicy::Allow => "允许", UsbStoragePolicy::ReadOnly => "只读", UsbStoragePolicy::Deny => "禁用",
    }.into());
    ui.set_admin_uwf(match p.uwf_enabled { Some(true) => "开启", Some(false) => "关闭", None => "不管理" }.into());
    ui.set_admin_uwf_scope(match p.uwf_scope {
        UwfScope::SystemOnly => "仅系统盘", UwfScope::AllVolumes => "所有固定磁盘",
    }.into());
    ui.set_admin_task_manager(p.disable_task_manager);
    ui.set_admin_block_internet(p.block_internet);
    ui.set_admin_new_passphrase("".into());
    ui.set_admin_default_wallpaper_path("".into());
    apply_policy_display(ui, p);
}

fn wire_admin(ui: &MainWindow) {
    let clicks = Rc::new(Cell::new(0u8));
    let secret = Rc::new(RefCell::new(None::<String>));
    let current = Arc::new(Mutex::new(None::<Policy>));

    let w = ui.as_weak();
    let count = clicks.clone();
    ui.on_about_clicked(move || {
        let next = count.get() + 1;
        count.set(if next >= 5 { 0 } else { next });
        if next >= 5 {
            if let Some(ui) = w.upgrade() {
                ui.set_admin_error("".into());
                ui.set_admin_password("".into());
                ui.set_admin_dialog_open(true);
            }
        }
    });

    let w = ui.as_weak();
    let secret_for_unlock = secret.clone();
    let current_for_unlock = current.clone();
    ui.on_admin_unlock(move |password| {
        let Some(ui) = w.upgrade() else { return };
        let pw = password.to_string();
        match ipc::send(Command::AdminGetPolicy { password: pw.clone() }) {
            Ok(Report::Policy(policy)) => {
                show_admin_policy(&ui, &policy);
                *current_for_unlock.lock().unwrap() = Some(policy);
                *secret_for_unlock.borrow_mut() = Some(pw);
                ui.set_admin_password("".into());
                ui.set_admin_error("".into());
                ui.set_admin_dialog_open(false);
                ui.set_locked(false);
                ui.set_admin_mode(true);
            }
            Ok(Report::Error { message, .. }) => ui.set_admin_error(message.into()),
            Ok(_) => ui.set_admin_error("服务返回了意外响应".into()),
            Err(e) => ui.set_admin_error(format!("无法连接学生端服务：{e}").into()),
        }
    });

    let w = ui.as_weak();
    let secret_for_save = secret.clone();
    let current_for_save = current.clone();
    ui.on_admin_save(move || {
        let Some(ui) = w.upgrade() else { return };
        if !ui.get_admin_mode() { return; }
        let Some(password) = secret_for_save.borrow().clone() else { return };
        let Some(mut policy) = current_for_save.lock().unwrap().clone() else { return };
        policy.lock_wallpaper = ui.get_admin_lock_wallpaper();
        policy.wallpaper_path = Some(ui.get_admin_wallpaper_path().to_string()).filter(|s| !s.trim().is_empty());
        policy.usb_storage = match ui.get_admin_usb().as_str() {
            "允许" => UsbStoragePolicy::Allow, "只读" => UsbStoragePolicy::ReadOnly, _ => UsbStoragePolicy::Deny,
        };
        policy.uwf_enabled = match ui.get_admin_uwf().as_str() {
            "开启" => Some(true), "关闭" => Some(false), _ => None,
        };
        policy.uwf_scope = if ui.get_admin_uwf_scope().as_str() == "所有固定磁盘" {
            UwfScope::AllVolumes
        } else { UwfScope::SystemOnly };
        policy.disable_task_manager = ui.get_admin_task_manager();
        policy.block_internet = ui.get_admin_block_internet();
        let new_passphrase = ui.get_admin_new_passphrase().to_string();
        ui.set_busy(true);
        ui.set_message("正在保存管理员设置…".into());
        let w = w.clone();
        let current = current_for_save.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<(Report, Option<Policy>)> {
                if !new_passphrase.is_empty() {
                    policy.config_passphrase_hash = Some(labctl_common::auth::hash_passphrase(&new_passphrase)?);
                }
                let report = ipc::send(Command::AdminSetPolicy { password: password.clone(), policy })?;
                let updated = if matches!(report, Report::Ack { .. }) {
                    match ipc::send(Command::AdminGetPolicy { password })? { Report::Policy(p) => Some(p), _ => None }
                } else { None };
                Ok((report, updated))
            })();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    ui.set_busy(false);
                    match result {
                        Ok((Report::Ack { note, .. }, Some(p))) => {
                            show_admin_policy(&ui, &p);
                            *current.lock().unwrap() = Some(p);
                            ui.set_message(note.unwrap_or_else(|| "设置已保存".into()).into());
                        }
                        Ok((Report::Error { message, .. }, _)) => ui.set_message(message.into()),
                        Ok(_) => ui.set_message("设置已提交，但刷新策略失败".into()),
                        Err(e) => ui.set_message(format!("保存失败：{e}").into()),
                    }
                }
            });
        });
    });

    let w = ui.as_weak();
    let secret_for_restore = secret.clone();
    let current_for_restore = current.clone();
    ui.on_admin_restore(move || {
        let Some(ui) = w.upgrade() else { return };
        if !ui.get_admin_mode() { return; }
        let Some(password) = secret_for_restore.borrow().clone() else { return };
        ui.set_busy(true);
        ui.set_message("正在恢复初始状态…".into());
        let w = w.clone();
        let current = current_for_restore.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<(Report, Option<Policy>)> {
                let report = ipc::send(Command::AdminRestore { password: password.clone() })?;
                let updated = if matches!(report, Report::Ack { .. }) {
                    match ipc::send(Command::AdminGetPolicy { password })? { Report::Policy(p) => Some(p), _ => None }
                } else { None };
                Ok((report, updated))
            })();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    ui.set_busy(false);
                    match result {
                        Ok((Report::Ack { note, .. }, Some(p))) => {
                            show_admin_policy(&ui, &p);
                            *current.lock().unwrap() = Some(p);
                            ui.set_message(note.unwrap_or_else(|| "已恢复初始状态".into()).into());
                        }
                        Ok((Report::Error { message, .. }, _)) => ui.set_message(message.into()),
                        Ok(_) => ui.set_message("恢复已提交，但刷新策略失败".into()),
                        Err(e) => ui.set_message(format!("恢复失败：{e}").into()),
                    }
                }
            });
        });
    });

    let w = ui.as_weak();
    ui.on_admin_defaults(move || {
        let Some(ui) = w.upgrade() else { return };
        if !ui.get_admin_mode() { return; }
        let path = ui.get_admin_default_wallpaper_path().to_string();
        match userpolicy::apply_defaults(if path.trim().is_empty() { None } else { Some(&path) }) {
            Ok(()) => ui.set_message("已恢复桌面默认设置；图标大小可能要重新登录后生效".into()),
            Err(e) => ui.set_message(format!("恢复桌面默认设置失败：{e}").into()),
        }
    });
}

/// Hide to tray and re-lock, clearing any entered password.
fn hide_and_lock(ui_w: &Weak<MainWindow>) {
    if let Some(u) = ui_w.upgrade() {
        u.set_locked(true);
        u.set_pw("".into());
        u.set_pw2("".into());
        u.set_message("".into());
        u.set_admin_mode(false);
        u.set_admin_dialog_open(false);
        u.set_admin_password("".into());
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
    ui.set_uwf_detail("".into());
    ui.set_uwf_text(
        match p.uwf_enabled {
            Some(true) => "已下发开启",
            Some(false) => "已下发关闭",
            None => "未管理",
        }
        .into(),
    );
}

/// Display text for `ClientStatus::uwf_state`, whether it is protecting, and
/// the full error text when it failed.
fn uwf_label(state: &str) -> (String, bool, String) {
    if let Some(e) = state.strip_prefix("error:") {
        return ("出错".into(), false, e.to_string());
    }
    let text = match state {
        "on" => "已开启（重启即还原）",
        "on_next_boot" => "已设置，重启后生效",
        "off_next_boot" => "重启后关闭",
        "off" => "已关闭",
        "installing" => "正在安装 UWF 功能…",
        "install_pending_reboot" => "功能已安装，重启后启用",
        "switching_to_disk" => "切换为磁盘缓存，需重启",
        "not_installed" => "未安装",
        other => other,
    };
    (text.to_string(), state == "on", String::new())
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
                        let (text, on, detail) = uwf_label(state);
                        ui.set_uwf_on(on);
                        ui.set_uwf_text(text.into());
                        ui.set_uwf_detail(detail.into());
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
fn policy_watch_timer(ui_w: Weak<MainWindow>, gate: Rc<RefCell<store::Gate>>) -> Timer {
    let t = Timer::default();
    t.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        let p = store::load_policy();
        if LAST_APPLIED_REV.load(Ordering::Relaxed) != p.revision {
            userpolicy::apply(&p);
            LAST_APPLIED_REV.store(p.revision, Ordering::Relaxed);
            refresh_gate(&ui_w, &gate, &p);
        }
    });
    t
}

/// Pick up an access passphrase pushed by the teacher. When it changed, lock
/// again so the new one is required right away, not after a restart.
fn refresh_gate(ui_w: &Weak<MainWindow>, gate: &Rc<RefCell<store::Gate>>, p: &Policy) {
    let fresh = store::load_gate(p);
    let changed = fresh.hash != gate.borrow().hash;
    let teacher_managed = fresh.teacher_managed;
    *gate.borrow_mut() = fresh;
    let Some(ui) = ui_w.upgrade() else { return };
    ui.set_first_run(gate.borrow().first_run());
    if changed {
        ui.set_locked(true);
        ui.set_admin_mode(false);
        ui.set_pw("".into());
        ui.set_pw2("".into());
        ui.set_message(if teacher_managed { "访问密码已由教师端更新" } else { "" }.into());
    }
}

// ---------------- tray (Windows) ----------------

#[cfg(windows)]
fn start_tray(ui_w: Weak<MainWindow>) -> Box<dyn std::any::Any> {
    use tray_icon::menu::{Menu, MenuItem};

    let show = MenuItem::new("显示主界面", true, None);
    let menu = Menu::new();
    let _ = menu.append(&show);

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

    let show_id = show.id().clone();
    let menu_rx = tray_icon::menu::MenuEvent::receiver();
    let tray_rx = tray_icon::TrayIconEvent::receiver();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(150), move || {
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == show_id {
                show_and_lock(&ui_w);
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
        u.set_admin_mode(false);
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
                        std::thread::spawn(move || {
                            if let Err(e) = userpolicy::apply_defaults(path.as_deref()) {
                                tracing::warn!(error = %e, "apply defaults failed");
                            }
                        });
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

//! HTTP + WebSocket API for the local web UI.

use crate::assets;
use crate::engine::Engine;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use labctl_common::auth::hash_passphrase;
use labctl_common::policy::Policy;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/roster", get(roster))
        .route("/api/hash", post(hash))
        .route("/api/pick-wallpaper", post(pick_wallpaper))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/apply-defaults", post(apply_defaults))
        .route("/api/pick-exe", post(pick_exe))
        .route("/api/update-client", post(update_client))
        .route("/api/policy", post(policy_all))
        .route("/api/clients/:id/policy", post(policy_one))
        .route("/api/clients/:id/launch", post(launch))
        .route("/api/clients/:id/reboot", post(reboot))
        .route("/ws", get(ws_upgrade))
        .fallback(assets::serve)
        .with_state(engine)
}

async fn roster(State(engine): State<Arc<Engine>>) -> impl IntoResponse {
    Json(engine.roster().await)
}

#[derive(Deserialize)]
struct HashReq {
    passphrase: String,
}

async fn hash(Json(req): Json<HashReq>) -> impl IntoResponse {
    match hash_passphrase(&req.passphrase) {
        Ok(h) => Json(json!({ "hash": h })).into_response(),
        Err(e) => error(&e.to_string()),
    }
}

/// Persisted console drawer settings so the operator's last choices survive a
/// restart. Stored as an opaque JSON blob next to the console's data.
fn settings_path() -> std::path::PathBuf {
    labctl_common::paths::data_dir().join("console-settings.json")
}

async fn get_settings() -> impl IntoResponse {
    let v: serde_json::Value = std::fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    Json(v).into_response()
}

async fn save_settings(Json(v): Json<serde_json::Value>) -> impl IntoResponse {
    let path = settings_path();
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default());
    Json(json!({ "ok": true })).into_response()
}

/// Open a native file-picker on the console machine and return the chosen
/// wallpaper path. The browser cannot read local absolute paths, so the
/// dialog is driven server-side (the console runs on the teacher's own PC).
async fn pick_wallpaper() -> impl IntoResponse {
    let path = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("图片", &["jpg", "jpeg", "png", "bmp"])
            .set_title("选择壁纸")
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten();
    Json(json!({ "path": path })).into_response()
}

/// If the policy locks the wallpaper and names a path, read that image on the
/// console machine and embed it as bytes, so every student gets a real local
/// file instead of a teacher-only path (which would blank their desktop).
fn embed_wallpaper(policy: &mut Policy) {
    use base64::Engine as _;
    if !policy.lock_wallpaper {
        return;
    }
    let Some(path) = policy.wallpaper_path.clone() else { return };
    if path.trim().is_empty() {
        return;
    }
    match std::fs::read(&path) {
        Ok(bytes) if !bytes.is_empty() => {
            policy.wallpaper_data_base64 =
                Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
            policy.wallpaper_name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
        }
        _ => tracing::warn!(%path, "could not read wallpaper file to embed"),
    }
}

async fn policy_all(
    State(engine): State<Arc<Engine>>,
    Json(mut policy): Json<Policy>,
) -> impl IntoResponse {
    embed_wallpaper(&mut policy);
    let results = engine.push_policy_all(policy).await;
    let body: Vec<_> = results
        .into_iter()
        .map(|(id, r)| json!({ "id": id, "ok": r.is_ok(), "detail": describe(r) }))
        .collect();
    Json(json!({ "results": body })).into_response()
}

async fn policy_one(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<Uuid>,
    Json(mut policy): Json<Policy>,
) -> impl IntoResponse {
    embed_wallpaper(&mut policy);
    reply(engine.push_policy(id, policy).await)
}

/// Native picker for the client .exe to push.
async fn pick_exe() -> impl IntoResponse {
    let path = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("可执行文件", &["exe"])
            .set_title("选择要下发的客户端程序")
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten();
    Json(json!({ "path": path })).into_response()
}

#[derive(Deserialize)]
struct UpdateReq {
    /// Path to the .exe on the console machine.
    source_path: String,
    /// Optional install path on the client; None = next to the agent.
    #[serde(default)]
    target_path: Option<String>,
    /// Override the installed file name; defaults to the source basename.
    #[serde(default)]
    filename: Option<String>,
    #[serde(default = "default_true")]
    run_after: bool,
}

async fn update_client(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<UpdateReq>,
) -> impl IntoResponse {
    use base64::Engine as _;
    let bytes = match tokio::fs::read(&req.source_path).await {
        Ok(b) => b,
        Err(e) => return error(&format!("读取文件失败：{e}")),
    };
    if bytes.is_empty() {
        return error("所选文件为空");
    }
    let filename = req.filename.unwrap_or_else(|| {
        std::path::Path::new(&req.source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("labctl-student.exe")
            .to_string()
    });
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let results = engine
        .update_client_all(filename, req.target_path, data, req.run_after)
        .await;
    let body: Vec<_> = results
        .into_iter()
        .map(|(id, r)| json!({ "id": id, "ok": r.is_ok(), "detail": describe(r) }))
        .collect();
    Json(json!({ "size": bytes.len(), "results": body })).into_response()
}

#[derive(Deserialize)]
struct DefaultsReq {
    #[serde(default)]
    wallpaper_path: Option<String>,
}

async fn apply_defaults(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<DefaultsReq>,
) -> impl IntoResponse {
    use base64::Engine as _;
    // Read the chosen default wallpaper on the console and send it as bytes so
    // students get a real local file (not a teacher-only path).
    let (data, name) = match req.wallpaper_path.filter(|s| !s.trim().is_empty()) {
        Some(path) => match tokio::fs::read(&path).await {
            Ok(b) if !b.is_empty() => (
                Some(base64::engine::general_purpose::STANDARD.encode(&b)),
                std::path::Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string()),
            ),
            _ => (None, None),
        },
        None => (None, None),
    };
    let results = engine.apply_defaults_all(data, name).await;
    let body: Vec<_> = results
        .into_iter()
        .map(|(id, r)| json!({ "id": id, "ok": r.is_ok(), "detail": describe(r) }))
        .collect();
    Json(json!({ "results": body })).into_response()
}

async fn launch(State(engine): State<Arc<Engine>>, Path(id): Path<Uuid>) -> impl IntoResponse {
    reply(engine.launch_ui(id).await)
}

#[derive(Deserialize)]
struct RebootReq {
    #[serde(default = "default_true")]
    restore: bool,
    #[serde(default)]
    delay_secs: u32,
}
fn default_true() -> bool {
    true
}

async fn reboot(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RebootReq>,
) -> impl IntoResponse {
    reply(engine.reboot(id, req.restore, req.delay_secs).await)
}

async fn ws_upgrade(
    State(engine): State<Arc<Engine>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_loop(socket, engine))
}

async fn ws_loop(mut socket: WebSocket, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    // Send an immediate roster snapshot.
    let snapshot = crate::engine::Event::Roster { clients: engine.roster().await };
    if socket
        .send(Message::Text(serde_json::to_string(&snapshot).unwrap_or_default()))
        .await
        .is_err()
    {
        return;
    }
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let txt = serde_json::to_string(&ev).unwrap_or_default();
                if socket.send(Message::Text(txt)).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}

// --- helpers ---

fn reply(r: anyhow::Result<labctl_common::protocol::Report>) -> axum::response::Response {
    match r {
        Ok(report) => Json(json!({ "ok": true, "report": report_json(&report) })).into_response(),
        Err(e) => error(&e.to_string()),
    }
}

fn describe(r: anyhow::Result<labctl_common::protocol::Report>) -> serde_json::Value {
    match r {
        Ok(rep) => report_json(&rep),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn report_json(r: &labctl_common::protocol::Report) -> serde_json::Value {
    serde_json::to_value(r).unwrap_or(serde_json::Value::Null)
}

fn error(msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::BAD_GATEWAY,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

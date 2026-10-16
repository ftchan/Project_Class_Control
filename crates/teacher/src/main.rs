//! Teacher console entry point.
//!
//! Starts the discovery/poll engine and a local web server that serves the
//! shadcn UI and the control API. Binds to loopback by default so the console
//! UI is reachable only from the operator's own machine.

mod api;
mod assets;
mod engine;

use anyhow::Result;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let listener = bind_console().await?;
    let bind = listener.local_addr()?;

    let engine = engine::Engine::new();
    tokio::spawn(engine.clone().run_poller());

    let app = api::router(engine).layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::info!(%bind, "console UI at http://{bind}");
    println!("Lab console running. Open http://{bind} in your browser.");

    // Auto-open the console in the default browser (unless disabled).
    if std::env::var("LABCTL_NO_BROWSER").is_err() {
        open_in_browser(&format!("http://{bind}"));
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Bind the console listener. An explicit `LABCTL_CONSOLE_BIND` is used as-is.
/// Otherwise try 8080, then fall back: Windows often reserves ranges around
/// 8080 for Hyper-V/WSL (bind fails with os error 10013), and those ranges
/// change on every boot.
async fn bind_console() -> Result<tokio::net::TcpListener> {
    if let Some(addr) = std::env::var("LABCTL_CONSOLE_BIND")
        .ok()
        .and_then(|s| s.parse::<SocketAddr>().ok())
    {
        return tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!("cannot listen on {addr}: {e}"));
    }

    // Port 0 last: the OS picks a free port outside the reserved ranges.
    for port in [8080, 18080, 28080, 0] {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(e) => tracing::warn!(%addr, error = %e, "console port unavailable, trying next"),
        }
    }
    anyhow::bail!("no local port available for the console")
}

/// Best-effort: open `url` in the operator's default browser.
fn open_in_browser(url: &str) {
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // No extra console window flashing up for the `start` helper.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };
    #[cfg(not(windows))]
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        tracing::debug!(error = %e, "could not auto-open browser");
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LABCTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

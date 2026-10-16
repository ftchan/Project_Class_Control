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

    let bind: SocketAddr = std::env::var("LABCTL_CONSOLE_BIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8080)));

    let engine = engine::Engine::new();
    tokio::spawn(engine.clone().run_poller());

    let app = api::router(engine).layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "console UI at http://{bind}");
    println!("Lab console running. Open http://{bind} in your browser.");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("LABCTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

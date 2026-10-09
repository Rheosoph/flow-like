#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::routing::get;
use flow_like_compiler::{compiler_router, CompilerState};

mod metrics;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry();

    tracing::info!("Starting Flow-Like Kubernetes Compiler");

    let config = flow_like_compiler::CompilerConfig::from_env();
    let state = CompilerState::new(config);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9090".to_string());

    let app = compiler_router(state).route("/metrics", get(metrics::handler));
    let metrics_app = axum::Router::new().route("/metrics", get(metrics::handler));

    let addr = format!("0.0.0.0:{port}");
    let metrics_addr = format!("0.0.0.0:{metrics_port}");

    tracing::info!(%addr, "Compiler server listening");
    tracing::info!(%metrics_addr, "Metrics server listening");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let metrics_listener = tokio::net::TcpListener::bind(&metrics_addr).await?;

    tokio::select! {
        res = axum::serve(listener, app) => res?,
        res = axum::serve(metrics_listener, metrics_app) => res?,
    }

    Ok(())
}

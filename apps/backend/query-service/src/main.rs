//! Standalone query service for container deployments. Listens on `0.0.0.0:${PORT:-8090}`.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_query::server::{ServiceConfig, auth::TokenVerifier, query_router};
use std::sync::Arc;
use tokio::{net::TcpListener, signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let verifier = TokenVerifier::from_env().map_err(|error| {
        tracing::error!(%error, "BACKEND_PUB is missing or invalid; the query service cannot verify API tokens");
        std::io::Error::other(error)
    })?;
    let app = query_router(Arc::new(ServiceConfig::from_env()), verifier);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8090".to_string());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!(%port, "Flow-Like query service listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = terminate.recv() => {}
            _ = signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{Router, middleware, routing::get};
use flow_like_executor::{ExecutorState, executor_router};
use std::future::IntoFuture;
use tokio::sync::watch;

mod config;
mod metrics;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry()?;

    let config = config::Config::from_env()?;
    // Keep startup aligned with the queue worker. The remote-only `server`
    // bundle reports that no local inference runtime is configured.
    let _ = flow_like_catalog::initialize();

    tracing::info!(
        port = config.port,
        metrics_port = config.metrics_port,
        "starting Flow-Like Azure executor server"
    );

    // Everything a run may touch arrives inside the signed ExecutionRequest
    // (presigned URLs, RuntimeCredentials), which is why this process opens no
    // Azure client and holds no identity: only the executor tuning knobs and
    // the JWT verifier key are read from the environment.
    let state = ExecutorState::from_env();
    let app = executor_router(state).layer(middleware::from_fn(metrics::http_middleware));
    // Metrics stay off the ingress port: the executor's ingress is reachable
    // from the whole Container Apps environment, the metrics port only from
    // an in-environment scraper.
    let metrics_app = Router::new().route("/metrics", get(metrics::handler));

    let address = format!("0.0.0.0:{}", config.port);
    let metrics_address = format!("0.0.0.0:{}", config.metrics_port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    let metrics_listener = tokio::net::TcpListener::bind(&metrics_address).await?;

    tracing::info!(address = %address, "Azure executor is listening");
    tracing::info!(address = %metrics_address, "Prometheus metrics are listening");

    // SIGTERM (Container Apps revision rotation, scale-in) stops accepting new
    // connections and lets in-flight runs deliver their callbacks and stream
    // their final events instead of dying mid-run; the platform's termination
    // grace period is the hard ceiling.
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        tracing::info!("shutdown signal received; draining in-flight executions");
        let _ = shutdown_sender.send(true);
    });

    let server = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_requested(shutdown_receiver.clone()))
        .into_future();
    let metrics_server = axum::serve(metrics_listener, metrics_app)
        .with_graceful_shutdown(shutdown_requested(shutdown_receiver))
        .into_future();

    // Both must run to completion: a `select!` would return as soon as the
    // metrics listener finished draining and drop the executor mid-run.
    tokio::try_join!(server, metrics_server)?;
    Ok(())
}

async fn shutdown_requested(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let terminate = signal(SignalKind::terminate());
    match terminate {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

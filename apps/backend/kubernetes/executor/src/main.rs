//! Single-use Kubernetes execution runtime. Shared execution requires an
//! explicit trusted_shared deployment and must not host untrusted tenants.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{http::StatusCode, routing::get};
use flow_like_executor::{ExecutorConfig, ExecutorState, executor_router};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, watch};

mod metrics;
// Keep the dispatch protocol and validation identical across deployment targets.
#[path = "../../../docker-compose/runtime/src/once.rs"]
mod once;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("--once") {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
        return once::run(arguments.get(2).map(String::as_str).unwrap_or("callback")).await;
    }
    let server_mode = std::env::var("EXECUTOR_SERVER_MODE")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    if !server_mode {
        return Err(
            "Use --once callback, callback-queued, stream, or warm with a signed dispatch on stdin"
                .into(),
        );
    }
    if std::env::var("EXECUTION_ISOLATION_MODE").as_deref() != Ok("trusted_shared") {
        return Err("Shared server mode requires EXECUTION_ISOLATION_MODE=trusted_shared".into());
    }
    metrics::init_telemetry();
    flow_like_catalog::initialize();
    flow_like_executor::prepare_runtime();
    flow_like_executor::jwt::prepare_verification_key().await?;
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9090".to_string());
    let maximum = std::env::var("MAX_CONCURRENT_EXECUTIONS")
        .unwrap_or_else(|_| "10".into())
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=1024).contains(value))
        .ok_or("MAX_CONCURRENT_EXECUTIONS must be between 1 and 1024")?;
    let capacity = Arc::new(Semaphore::new(maximum));
    let executor_config = ExecutorConfig::from_env();
    let drain_timeout = executor_config.execution_timeout() + Duration::from_secs(60);
    let state = ExecutorState::with_admission(executor_config, capacity.clone());
    let readiness = capacity.clone();
    let app = executor_router(state)
        .route("/metrics", get(metrics::handler))
        .route(
            "/ready",
            get(move || {
                let capacity = readiness.clone();
                async move {
                    if capacity.is_closed() {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );
    let metrics_app = axum::Router::new().route("/metrics", get(metrics::handler));
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    let metrics_listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{metrics_port}")).await?;
    let (shutdown, receiver) = watch::channel(None::<tokio::time::Instant>);
    let deadline_receiver = receiver.clone();
    let metrics_receiver = receiver.clone();
    let signal_sender = shutdown.clone();
    let signal_capacity = capacity.clone();
    let signal = tokio::spawn(async move {
        shutdown_signal().await;
        signal_capacity.close();
        let _ = signal_sender.send(Some(tokio::time::Instant::now() + drain_timeout));
    });
    let lifetime = async {
        let result = tokio::try_join!(
            async {
                axum::serve(listener, app)
                    .with_graceful_shutdown(wait_for_shutdown(receiver))
                    .await
            },
            async {
                axum::serve(metrics_listener, metrics_app)
                    .with_graceful_shutdown(wait_for_shutdown(metrics_receiver))
                    .await
            },
        );
        capacity.close();
        shutdown.send_if_modified(|deadline| {
            if deadline.is_none() {
                *deadline = Some(tokio::time::Instant::now() + drain_timeout);
                true
            } else {
                false
            }
        });
        // Streaming tasks retain permits after their HTTP client disconnects.
        while capacity.available_permits() < maximum {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        result
    };
    tokio::pin!(lifetime);
    let result = tokio::select! {
        result = &mut lifetime => Some(result),
        _ = wait_for_shutdown(deadline_receiver.clone()) => {
            let deadline = (*deadline_receiver.borrow()).expect("shutdown deadline set");
            tokio::time::timeout_at(deadline, &mut lifetime).await.ok()
        }
    };
    capacity.close();
    signal.abort();
    result.ok_or("Executor shutdown exceeded its execution and drain budget")??;
    Ok(())
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<Option<tokio::time::Instant>>) {
    let _ = receiver.wait_for(Option::is_some).await;
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

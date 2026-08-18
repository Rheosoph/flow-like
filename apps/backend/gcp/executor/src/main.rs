//! Flow-Like GCP executor: the synchronous HTTP executor the API reaches at
//! `EXECUTOR_URL` when `EXECUTION_BACKEND=http`.
//!
//! This is the Kubernetes executor's server mode and nothing else — there is no
//! job-once mode, no queue, no database and no cloud SDK. Work arrives as an
//! `ExecutionRequest` whose body already carries the presigned URLs and scoped
//! runtime credentials the run needs, and progress leaves through the callback
//! URL inside the signed `executor_jwt`. The routes are `flow_like_executor`'s
//! own (`POST /execute`, `POST /execute/stream`, `POST /execute/sse`,
//! `GET /health`) plus the Cloud Run probes and `/metrics`, all on one port.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use flow_like_executor::{ExecutorState, executor_router};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

mod config;
mod telemetry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ordered before telemetry on purpose: the OTLP exporter opens a socket and
    // honours proxy environment variables. See
    // `config::reject_forbidden_environment`.
    config::reject_forbidden_environment()?;
    telemetry::init_telemetry();

    let config = config::Config::from_env()?;
    // Runtime hooks the catalog registers before the first execution. Under the
    // server feature set this is a no-op; it is called for parity with the
    // queue-worker's execution workload so the two GCP execution images
    // initialise identically whatever the feature set becomes.
    flow_like_catalog::initialize();

    tracing::info!(
        port = config.port,
        execution_timeout_seconds = config.executor.execution_timeout_secs,
        callback_timeout_ms = config.executor.callback_timeout_ms,
        callback_retries = config.executor.callback_retries,
        batch_interval_ms = config.executor.batch_interval_ms,
        max_batch_size = config.executor.max_batch_size,
        "starting Flow-Like GCP executor in server mode"
    );

    let health = HealthState::new();
    // Cloud Run publishes exactly one container port per service, so the second
    // listener the Azure and Kubernetes executors bind on METRICS_PORT would be
    // unreachable here. Prometheus is mounted on the serving router instead,
    // exactly as the GCP API image does. The executor's ingress is internal and
    // its only invoker is the API's service account, so `/metrics` is reachable
    // from inside the project and from nowhere else — the same posture as the
    // API's `/metrics`, which is kept out of the load balancer's URL map.
    let app = executor_router(ExecutorState::new(config.executor))
        .merge(health_routes(health.clone()))
        .route("/metrics", get(telemetry::metrics_handler))
        .layer(middleware::from_fn(record_request));

    let address = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, "GCP executor is listening");

    let (shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(false);
    let signal_health = health.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        signal_health.begin_drain();
        // No drain sleep. Cloud Run sends SIGTERM roughly ten seconds before
        // SIGKILL and has already taken the instance out of rotation by then;
        // the time is better spent letting in-flight runs finish than waiting
        // for a probe that will not be asked again.
        tracing::warn!("received a termination signal; draining");
        let _ = shutdown_sender.send(true);
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown_receiver.borrow() {
                if shutdown_receiver.changed().await.is_err() {
                    break;
                }
            }
        })
        .await?;

    Ok(())
}

#[derive(Clone)]
struct HealthState {
    accepting_traffic: Arc<AtomicBool>,
}

impl HealthState {
    fn new() -> Self {
        Self {
            accepting_traffic: Arc::new(AtomicBool::new(true)),
        }
    }

    fn begin_drain(&self) {
        self.accepting_traffic.store(false, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// The Cloud Run probes the runtime module points at this service: liveness on
/// `/health/live`, startup on `/health/ready`. `/health` itself is owned by
/// `executor_router` (it is what the API and the Kubernetes probes use) and
/// answers `200` for the life of the process, so it is not re-declared here —
/// `Router::merge` would panic on the duplicate.
///
/// Readiness has no dependency to report on: this process holds no database
/// connection and no rotating token. It closes only when a termination signal
/// has arrived, so a startup probe that reaches a draining instance is told so
/// instead of being handed a listener about to go away.
fn health_routes(state: HealthState) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .with_state(state)
}

async fn liveness() -> (StatusCode, Json<HealthResponse>) {
    respond(StatusCode::OK, "healthy")
}

async fn readiness(State(state): State<HealthState>) -> (StatusCode, Json<HealthResponse>) {
    if state.accepting_traffic.load(Ordering::SeqCst) {
        respond(StatusCode::OK, "ready")
    } else {
        respond(StatusCode::SERVICE_UNAVAILABLE, "draining")
    }
}

fn respond(status_code: StatusCode, status: &'static str) -> (StatusCode, Json<HealthResponse>) {
    (
        status_code,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
}

/// Applied with `Router::layer` after every route is registered, which is what
/// puts `MatchedPath` in the request extensions; a request that matched nothing
/// (the default 404 fallback) is counted under one fixed label.
async fn record_request(request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let method = request.method().as_str().to_owned();

    let response = next.run(request).await;
    telemetry::record_request(&route, &method, response.status().as_u16());
    response
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

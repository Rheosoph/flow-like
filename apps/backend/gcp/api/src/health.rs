use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone)]
pub struct HealthState {
    database: DatabaseConnection,
    accepting_traffic: Arc<AtomicBool>,
}

impl HealthState {
    pub fn new(database: DatabaseConnection) -> Self {
        Self {
            database,
            accepting_traffic: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Close readiness before the process stops serving.
    ///
    /// Two independent triggers call this: the Cloud SQL IAM token entering its
    /// safety window, and Cloud Run's SIGTERM. Both need the load balancer to
    /// stop sending new requests here *before* the listener goes away, otherwise
    /// in-flight traffic is answered with a connection reset instead of being
    /// routed to a healthy revision.
    pub fn begin_drain(&self) {
        self.accepting_traffic.store(false, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub fn routes(state: HealthState) -> Router {
    Router::new()
        // Compatibility endpoint used by the current Cloud Run and load-balancer
        // health checks. It deliberately has readiness (including DB) semantics.
        .route("/health", get(readiness))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/health/startup", get(readiness))
        .with_state(state)
}

async fn liveness() -> (StatusCode, Json<HealthResponse>) {
    response(StatusCode::OK, "healthy")
}

async fn readiness(State(state): State<HealthState>) -> (StatusCode, Json<HealthResponse>) {
    if !state.accepting_traffic.load(Ordering::SeqCst) {
        return response(StatusCode::SERVICE_UNAVAILABLE, "draining-database-token");
    }

    match tokio::time::timeout(Duration::from_secs(3), state.database.ping()).await {
        Ok(Ok(())) => response(StatusCode::OK, "ready"),
        Ok(Err(error)) => {
            tracing::error!(error = %error, "Cloud SQL readiness check failed");
            response(StatusCode::SERVICE_UNAVAILABLE, "database-unavailable")
        }
        Err(_) => {
            tracing::error!("Cloud SQL readiness check timed out");
            response(StatusCode::SERVICE_UNAVAILABLE, "database-timeout")
        }
    }
}

fn response(status_code: StatusCode, status: &'static str) -> (StatusCode, Json<HealthResponse>) {
    (
        status_code,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
}

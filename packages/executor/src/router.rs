//! Axum routes for the executor
//!
//! Provides both callback-based and streaming execution endpoints.

use crate::config::ExecutorConfig;
use crate::execute::execute;
use crate::streaming::{event_to_ndjson, execute_streaming_with_permit};
use crate::types::{DispatchPayload, ExecutionRequest};
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Response, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::StreamExt;
use serde::Serialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Shared executor state
#[derive(Clone)]
pub struct ExecutorState {
    pub config: ExecutorConfig,
    pub admission: Arc<Semaphore>,
}

impl ExecutorState {
    pub fn new(config: ExecutorConfig) -> Self {
        let capacity = std::env::var("MAX_CONCURRENT_EXECUTIONS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(10);
        Self::with_admission(config, Arc::new(Semaphore::new(capacity)))
    }

    pub fn with_admission(config: ExecutorConfig, admission: Arc<Semaphore>) -> Self {
        Self { config, admission }
    }

    fn try_admit(&self) -> Result<OwnedSemaphorePermit, (StatusCode, String)> {
        self.admission.clone().try_acquire_owned().map_err(|_| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "Execution capacity is occupied or draining".to_string(),
            )
        })
    }

    pub fn from_env() -> Self {
        Self::new(ExecutorConfig::from_env())
    }
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

/// Construct the executor router with all endpoints
pub fn executor_router(state: ExecutorState) -> Router {
    Router::new()
        .route("/execute", post(execute_callback))
        .route("/execute/stream", post(execute_stream))
        .route("/execute/sse", post(execute_sse))
        .route("/health", get(health_check))
        .with_state(Arc::new(state))
}

/// Health check endpoint
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        service: "flow-like-executor".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Execute with callback-based progress reporting
///
/// POST /execute
///
/// Events are sent to the callback URL specified in the JWT.
/// Returns immediately with status and waits for completion.
#[derive(Debug, Serialize)]
pub struct ExecuteResponse {
    pub run_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Every direct transport POSTs the API's `DispatchPayload` wire format.
/// Decoding through `TryFrom` (never `Json<ExecutionRequest>`) resolves the
/// ETag-bound Latest version sentinel before validation sees the request.
fn decode_dispatch(payload: DispatchPayload) -> Result<ExecutionRequest, (StatusCode, String)> {
    ExecutionRequest::try_from(payload).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn execute_callback(
    State(state): State<Arc<ExecutorState>>,
    Json(payload): Json<DispatchPayload>,
) -> Result<Json<ExecuteResponse>, (StatusCode, String)> {
    let request = decode_dispatch(payload)?;
    let _permit = state.try_admit()?;
    let started = std::time::Instant::now();
    let result = execute(request, state.config.clone()).await;
    state.config.record_completion(
        match &result {
            Ok(result) => match result.status {
                crate::types::ExecutionStatus::Completed => "completed",
                crate::types::ExecutionStatus::Cancelled => "cancelled",
                _ => "failed",
            },
            Err(_) => "error",
        },
        started.elapsed().as_secs_f64(),
    );
    match result {
        Ok(result) => Ok(Json(ExecuteResponse {
            run_id: result.run_id,
            status: format!("{:?}", result.status).to_lowercase(),
            error: result.error,
            duration_ms: result.duration_ms,
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Execute with streaming response (newline-delimited JSON)
///
/// POST /execute/stream
///
/// Returns a streaming response with events as NDJSON.
/// Each line is a complete JSON object.
async fn execute_stream(
    State(state): State<Arc<ExecutorState>>,
    Json(payload): Json<DispatchPayload>,
) -> Result<Response, (StatusCode, String)> {
    let request = decode_dispatch(payload)?;
    let permit = state.try_admit()?;
    let stream = execute_streaming_with_permit(request, state.config.clone(), Some(permit))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let body_stream = stream.map(|event| Ok::<_, Infallible>(event_to_ndjson(&event)));

    let body = Body::from_stream(body_stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/x-ndjson")
        .header("Transfer-Encoding", "chunked")
        .header("Cache-Control", "no-cache")
        .body(body)
        .unwrap())
}

/// Execute with Server-Sent Events
///
/// POST /execute/sse
///
/// Returns a streaming response using SSE format.
async fn execute_sse(
    State(state): State<Arc<ExecutorState>>,
    Json(payload): Json<DispatchPayload>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    (StatusCode, String),
> {
    let request = decode_dispatch(payload)?;
    let permit = state.try_admit()?;
    let stream = execute_streaming_with_permit(request, state.config.clone(), Some(permit))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let sse_stream = stream.map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        // All events are now InterComEvent, use event_type for SSE event field
        let event_type = &event.event_type;
        Ok::<_, Infallible>(
            axum::response::sse::Event::default()
                .event(event_type)
                .data(data),
        )
    });

    Ok(Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

    #[test]
    fn shared_admission_is_enforced_and_closes_for_drain() {
        let capacity = Arc::new(Semaphore::new(1));
        let state = ExecutorState::with_admission(ExecutorConfig::default(), capacity.clone());
        let queue_permit = capacity.clone().try_acquire_owned().unwrap();
        assert_eq!(
            state.try_admit().unwrap_err().0,
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(queue_permit);
        let http_permit = state.try_admit().unwrap();
        assert!(capacity.clone().try_acquire_owned().is_err());
        drop(http_permit);
        capacity.close();
        assert_eq!(
            state.try_admit().unwrap_err().0,
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    /// The exact body `build_executor_payload` POSTs to the direct transports.
    fn wire_payload(
        board_version: Option<(u32, u32, u32)>,
        board_etag: Option<&str>,
    ) -> DispatchPayload {
        serde_json::from_value(serde_json::json!({
            "job_id": "job-1",
            "run_id": "run-1",
            "app_id": "app-1",
            "board_id": "board-1",
            "board_version": board_version,
            "board_etag": board_etag,
            "node_id": "node-1",
            "user_id": "user-1",
            "credentials": {
                "Aws": {
                    "access_key_id": "AKIAIOSFODNN7EXAMPLE",
                    "secret_access_key": "secret",
                    "session_token": null,
                    "meta_bucket": "meta",
                    "content_bucket": "content",
                    "logs_bucket": "logs",
                    "region": "us-east-1",
                    "expiration": null
                }
            },
            "executor_jwt": "jwt",
            "callback_url": "https://api.example",
            "stream_state": true,
            "artifact": {
                "url": "https://meta.example/tmp/apps/app-1/compiled/drafts/board-1/etag-a_fp.flcb?sig",
                "path": "tmp/apps/app-1/compiled/drafts/board-1/etag-a_fp.flcb",
                "source_etag": "etag-a",
                "registry_fingerprint": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }
        }))
        .expect("wire payload deserializes as DispatchPayload")
    }

    #[test]
    fn router_decode_resolves_the_etag_latest_sentinel_before_validation() {
        let request = decode_dispatch(wire_payload(
            Some(ETAG_BOUND_LATEST_VERSION_SENTINEL),
            Some("etag-a"),
        ))
        .expect("sentinel + etag decodes");

        assert_eq!(request.board_version, None);
        assert_eq!(request.board_etag.as_deref(), Some("etag-a"));
    }

    #[test]
    fn router_decode_rejects_a_malformed_selector_with_bad_request() {
        let (status, message) =
            decode_dispatch(wire_payload(Some(ETAG_BOUND_LATEST_VERSION_SENTINEL), None))
                .expect_err("sentinel without etag is refused");

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(message.contains("board_etag"));
    }

    #[test]
    fn router_decode_passes_pinned_versions_through_unchanged() {
        let request =
            decode_dispatch(wire_payload(Some((1, 2, 3)), None)).expect("pinned version decodes");

        assert_eq!(request.board_version, Some((1, 2, 3)));
        assert_eq!(request.board_etag, None);
    }
}

//! Axum routes for the executor
//!
//! Provides both callback-based and streaming execution endpoints.

use crate::config::ExecutorConfig;
use crate::execute::execute;
use crate::streaming::{event_to_ndjson, execute_streaming};
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

/// Shared executor state
#[derive(Clone)]
pub struct ExecutorState {
    pub config: ExecutorConfig,
}

impl ExecutorState {
    pub fn new(config: ExecutorConfig) -> Self {
        Self { config }
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
    match execute(request, state.config.clone()).await {
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
    let stream = execute_streaming(request, state.config.clone())
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
    let stream = execute_streaming(request, state.config.clone())
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
            "stream_state": true
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

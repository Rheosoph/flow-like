//! Execution progress endpoints for executors and users
//!
//! Two main flows:
//! 1. **Executor → API**: Report progress/events (executor JWT)
//! 2. **User → API**: Long poll for status/events (user JWT)
//!
//! Events are stored with TTL and deleted after delivery.

use crate::{
    entity::{execution_usage_tracking, sea_orm_active_enums::ExecutionStatus},
    error::ApiError,
    execution::{
        state::{
            CreateEventInput, EventQuery, ExecutionStateStore, RunStatus as StateRunStatus,
            UpdateRunInput,
        },
        verify_execution_jwt, verify_user_jwt,
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use flow_like_types::{anyhow, create_id, tokio};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

// ============================================================================
// Executor endpoints (require executor JWT)
// ============================================================================

/// Request body for progress updates from executors
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ProgressUpdateRequest {
    /// Progress percentage (0-100)
    pub progress: Option<i32>,
    /// Current step description
    pub current_step: Option<String>,
    /// Final status (only set when execution completes)
    pub status: Option<ProgressStatus>,
    /// Output payload length (bytes) - we don't store the actual output
    pub output_len: Option<i64>,
    /// Error message (only set on failure)
    pub error: Option<String>,
}

/// Request body for pushing streaming events from executors
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct PushEventsRequest {
    /// Batch of events to push
    pub events: Vec<ExecutionEventInput>,
}

/// Single event input from executor
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ExecutionEventInput {
    /// Event type (log, progress, output, error, chunk, etc.)
    pub event_type: String,
    /// Event payload
    pub payload: serde_json::Value,
}

/// Status values that can be reported
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProgressStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Response from progress update
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ProgressUpdateResponse {
    pub accepted: bool,
    pub status: String,
}

/// Response from pushing events
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PushEventsResponse {
    pub accepted: i32,
    pub next_sequence: i32,
}

/// POST /execution/progress
///
/// Report execution progress. Requires executor JWT in Authorization header.
#[utoipa::path(
    post,
    path = "/execution/progress",
    tag = "execution",
    request_body = ProgressUpdateRequest,
    responses(
        (status = 200, description = "Progress update accepted", body = ProgressUpdateResponse),
        (status = 400, description = "Invalid request or JWT"),
        (status = 404, description = "Run not found")
    ),
    security(
        ("executor_jwt" = [])
    )
)]
#[tracing::instrument(name = "POST /execution/progress", skip(state, headers, body))]
pub async fn report_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ProgressUpdateRequest>,
) -> Result<Json<ProgressUpdateResponse>, ApiError> {
    let token = extract_bearer_token(&headers)?;

    let claims = verify_execution_jwt(token).map_err(|e| {
        tracing::warn!(error = %e, "Invalid execution JWT");
        ApiError::bad_request(format!("Invalid execution JWT: {}", e))
    })?;

    let store = get_state_store(&state).await?;

    let run = store
        .get_run_for_app(&claims.run_id, &claims.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
        .ok_or(ApiError::NOT_FOUND)?;

    // Don't accept updates for terminal states
    if run.status.is_terminal() {
        return Ok(Json(ProgressUpdateResponse {
            accepted: false,
            status: format!("{:?}", run.status),
        }));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let mut update = UpdateRunInput::default();

    if let Some(progress) = body.progress {
        update.progress = Some(progress.clamp(0, 100));
    }

    if let Some(step) = body.current_step {
        update.current_step = Some(step);
    }

    if let Some(status) = body.status {
        let new_status = match status {
            ProgressStatus::Running => {
                if run.started_at.is_none() {
                    update.started_at = Some(now);
                }
                StateRunStatus::Running
            }
            ProgressStatus::Completed => {
                update.completed_at = Some(now);
                update.progress = Some(100);
                if let Some(len) = body.output_len {
                    update.output_payload_len = Some(len);
                }
                StateRunStatus::Completed
            }
            ProgressStatus::Failed => {
                update.completed_at = Some(now);
                if let Some(error) = body.error.clone() {
                    update.error_message = Some(error);
                }
                StateRunStatus::Failed
            }
            ProgressStatus::Cancelled => {
                update.completed_at = Some(now);
                StateRunStatus::Cancelled
            }
        };
        update.status = Some(new_status.clone());
        tracing::info!(run_id = %claims.run_id, status = ?new_status, "Run status updated");
    }

    let updated = store
        .update_run(&claims.run_id, update)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to update run: {}", e)))?;

    if updated.status.is_terminal() {
        let duration_us = match (updated.started_at, updated.completed_at) {
            (Some(start), Some(end)) => (end - start) * 1000,
            _ => 0,
        };
        let exec_status = match updated.status {
            StateRunStatus::Completed => ExecutionStatus::Info,
            StateRunStatus::Failed => ExecutionStatus::Error,
            StateRunStatus::Cancelled => ExecutionStatus::Warn,
            _ => ExecutionStatus::Info,
        };
        let db = state.db.clone();
        let board_id = claims.board_id.clone();
        let event_id = claims.event_id.clone().unwrap_or_default();
        let app_id = claims.app_id.clone();
        let user_id = claims.sub.clone();
        let run_id = claims.run_id.clone();
        tokio::spawn(async move {
            if let Err(e) = track_execution_usage(
                &db,
                &board_id,
                &event_id,
                &run_id,
                duration_us,
                exec_status,
                &user_id,
                &app_id,
            )
            .await
            {
                tracing::warn!(error=%e, "Failed to track execution usage");
            }
        });
    }

    Ok(Json(ProgressUpdateResponse {
        accepted: true,
        status: format!("{:?}", updated.status),
    }))
}

/// POST /execution/events
///
/// Push streaming events from executor. Requires executor JWT.
#[utoipa::path(
    post,
    path = "/execution/events",
    tag = "execution",
    request_body = PushEventsRequest,
    responses(
        (status = 200, description = "Events pushed successfully", body = PushEventsResponse),
        (status = 400, description = "Invalid request or JWT")
    ),
    security(
        ("executor_jwt" = [])
    )
)]
#[tracing::instrument(name = "POST /execution/events", skip(state, headers, body))]
pub async fn push_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PushEventsRequest>,
) -> Result<Json<PushEventsResponse>, ApiError> {
    let token = extract_bearer_token(&headers)?;

    let claims = verify_execution_jwt(token)
        .map_err(|e| ApiError::bad_request(format!("Invalid execution JWT: {}", e)))?;

    let store = get_state_store(&state).await?;

    // Get current max sequence for this run
    let max_seq = store
        .get_max_sequence(&claims.run_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get max sequence: {}", e)))?;

    let expires_at = chrono::Utc::now().timestamp_millis() + 24 * 60 * 60 * 1000; // 24 hours
    let mut next_seq = max_seq.saturating_add(1);

    let events: Vec<CreateEventInput> = body
        .events
        .iter()
        .map(|e| {
            let input = CreateEventInput {
                id: create_id(),
                run_id: claims.run_id.clone(),
                sequence: next_seq,
                event_type: e.event_type.clone(),
                payload: e.payload.clone(),
                expires_at,
            };
            next_seq += 1;
            input
        })
        .collect();

    let accepted = store
        .push_events(events)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to push events: {}", e)))?;

    Ok(Json(PushEventsResponse {
        accepted,
        next_sequence: next_seq,
    }))
}

// ============================================================================
// User endpoints (require user JWT or app access)
// ============================================================================

/// Query params for long polling
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct PollParams {
    /// Last event sequence received (for pagination)
    pub after_sequence: Option<i32>,
    /// Timeout in seconds for long polling (max 30)
    pub timeout: Option<u64>,
}

/// Response with run status and events
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PollResponse {
    pub run_id: String,
    pub status: String,
    pub progress: i32,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub events: Vec<ExecutionEventOutput>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// Event output for users
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ExecutionEventOutput {
    pub sequence: i32,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

/// GET /execution/poll
///
/// Long poll for run status and events. Requires user JWT in Authorization header.
/// The JWT is returned from invoke endpoints (poll_token).
#[utoipa::path(
    get,
    path = "/execution/poll",
    tag = "execution",
    params(PollParams),
    responses(
        (status = 200, description = "Run status and events", body = PollResponse),
        (status = 400, description = "Invalid request or JWT"),
        (status = 404, description = "Run not found")
    ),
    security(
        ("user_jwt" = [])
    )
)]
#[tracing::instrument(name = "GET /execution/poll", skip(state, headers))]
pub async fn poll_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<PollParams>,
) -> Result<Json<PollResponse>, ApiError> {
    let token = extract_bearer_token(&headers)?;

    let claims = verify_user_jwt(token)
        .map_err(|e| ApiError::bad_request(format!("Invalid user JWT: {}", e)))?;

    let store = get_state_store(&state).await?;

    let timeout = params.timeout.unwrap_or(10).min(30);
    let after_seq = params.after_sequence.unwrap_or(0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);

    loop {
        // Get run status
        let run = store
            .get_run_for_app(&claims.run_id, &claims.app_id)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
            .ok_or(ApiError::NOT_FOUND)?;

        // Get undelivered events
        let events = store
            .get_events(EventQuery {
                run_id: claims.run_id.clone(),
                after_sequence: Some(after_seq),
                only_undelivered: true,
                limit: Some(100),
            })
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get events: {}", e)))?;

        // Return immediately if terminal state or we have events
        let is_terminal = run.status.is_terminal();
        if is_terminal || !events.is_empty() || std::time::Instant::now() >= deadline {
            // Mark events as delivered
            if !events.is_empty() {
                let event_ids: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
                let _ = store.mark_events_delivered(&event_ids).await;
            }

            return Ok(Json(PollResponse {
                run_id: run.id,
                status: format!("{:?}", run.status),
                progress: run.progress,
                current_step: run.current_step,
                error: run.error_message,
                events: events
                    .into_iter()
                    .map(|e| ExecutionEventOutput {
                        sequence: e.sequence,
                        event_type: e.event_type,
                        payload: e.payload,
                        created_at: chrono::DateTime::from_timestamp_millis(e.created_at)
                            .map(|dt| dt.to_rfc3339())
                            .unwrap_or_default(),
                    })
                    .collect(),
                started_at: run.started_at.and_then(|t| {
                    chrono::DateTime::from_timestamp_millis(t).map(|dt| dt.to_rfc3339())
                }),
                completed_at: run.completed_at.and_then(|t| {
                    chrono::DateTime::from_timestamp_millis(t).map(|dt| dt.to_rfc3339())
                }),
            }));
        }

        // Wait a bit before polling again
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// GET /execution/run/{run_id}
///
/// Get run status (requires app access via normal auth).
#[utoipa::path(
    get,
    path = "/execution/run/{run_id}",
    tag = "execution",
    params(
        ("run_id" = String, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Run status details", body = RunStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Run not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /execution/run/{run_id}", skip(state))]
pub async fn get_run_status(
    State(state): State<AppState>,
    axum::extract::Path(run_id): axum::extract::Path<String>,
    axum::Extension(user): axum::Extension<crate::middleware::jwt::AppUser>,
) -> Result<Json<RunStatusResponse>, ApiError> {
    let store = get_state_store(&state).await?;

    let run = store
        .get_run(&run_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
        .ok_or(ApiError::NOT_FOUND)?;

    crate::ensure_permission!(
        user,
        &run.app_id,
        &state,
        crate::permission::role_permission::RolePermissions::ReadBoards
    );

    Ok(Json(RunStatusResponse {
        run_id: run.id,
        board_id: run.board_id,
        event_id: run.event_id,
        status: format!("{:?}", run.status),
        mode: format!("{:?}", run.mode),
        progress: run.progress,
        current_step: run.current_step,
        error: run.error_message,
        input_payload_len: run.input_payload_len,
        output_payload_len: run.output_payload_len,
        started_at: run
            .started_at
            .and_then(|t| chrono::DateTime::from_timestamp_millis(t).map(|dt| dt.to_rfc3339())),
        completed_at: run
            .completed_at
            .and_then(|t| chrono::DateTime::from_timestamp_millis(t).map(|dt| dt.to_rfc3339())),
        created_at: chrono::DateTime::from_timestamp_millis(run.created_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
    }))
}

/// Response with run status details
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RunStatusResponse {
    pub run_id: String,
    pub board_id: String,
    pub event_id: Option<String>,
    pub status: String,
    pub mode: String,
    pub progress: i32,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub input_payload_len: i64,
    pub output_payload_len: i64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

// ============================================================================
// Tracking
// ============================================================================

async fn track_execution_usage(
    db: &sea_orm::DatabaseConnection,
    board_id: &str,
    node_id: &str,
    version: &str,
    microseconds: i64,
    status: ExecutionStatus,
    user_id: &str,
    app_id: &str,
) -> Result<(), flow_like_types::Error> {
    let now = chrono::Utc::now().naive_utc();
    let instance = std::env::var("INSTANCE_ID").ok();
    let record = execution_usage_tracking::ActiveModel {
        id: Set(create_id()),
        instance: Set(instance),
        board_id: Set(board_id.to_string()),
        node_id: Set(node_id.to_string()),
        version: Set(version.to_string()),
        microseconds: Set(microseconds),
        status: Set(status),
        user_id: Set(Some(user_id.to_string())),
        app_id: Set(Some(app_id.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    };
    record.insert(db).await?;
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("Missing Authorization header".to_string()))?
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::bad_request("Invalid Authorization header format".to_string()))
}

/// Get or create the execution state store from app state
async fn get_state_store(state: &AppState) -> Result<Arc<dyn ExecutionStateStore>, ApiError> {
    // Build config with available AppState components
    let mut config =
        crate::execution::state::StateStoreConfig::default().with_db(Arc::new(state.db.clone()));

    // Pass AWS config and content store for DynamoDB backend
    #[cfg(feature = "aws")]
    {
        config = config.with_aws_config(state.aws_client.clone());
    }

    #[cfg(feature = "dynamodb")]
    {
        config = config.with_content_store(state.content_bucket.clone());
    }

    crate::execution::state::create_state_store(config)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to create state store: {}", e)))
}

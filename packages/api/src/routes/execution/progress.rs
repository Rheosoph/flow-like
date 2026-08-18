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
            CreateEventInput, EventQuery, ExecutionStateStore, RunLeaseClaim,
            RunMode as StateRunMode, RunStatus as StateRunStatus, StateStoreError, UpdateRunInput,
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
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
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
    /// Broker job bound to this run (strict queue mode only).
    pub job_id: Option<String>,
    /// Unique token for one delivery attempt (strict queue mode only).
    pub lease_token: Option<String>,
    /// Requested ownership duration for claim/renewal in milliseconds.
    pub lease_duration_ms: Option<i64>,
}

/// Request body for pushing streaming events from executors
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct PushEventsRequest {
    /// Batch of events to push
    pub events: Vec<ExecutionEventInput>,
    /// Broker job and current delivery token authenticate strict queue event
    /// writes against the active Cosmos lease.
    pub job_id: Option<String>,
    pub lease_token: Option<String>,
}

/// Single event input from executor
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ExecutionEventInput {
    /// Stable executor-generated identity and sequence. Both are mandatory in
    /// strict queue mode so HTTP retries are idempotent.
    pub id: Option<String>,
    pub sequence: Option<i32>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_acquired: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<i64>,
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

    let lease_identity = queue_lease_identity(body.job_id.as_deref(), body.lease_token.as_deref())?;

    // Running is the atomic claim/renew operation for strict queue workers.
    // The Cosmos ETag write is the serialization point; a competing delivery
    // receives Busy until expiry, then may take over with a new token.
    if matches!(&body.status, Some(ProgressStatus::Running)) && lease_identity.is_some() {
        let (job_id, lease_token) = lease_identity.expect("lease identity was checked");
        let lease_duration_ms = body.lease_duration_ms.ok_or_else(|| {
            ApiError::bad_request("lease_duration_ms is required for a queue lease claim")
        })?;
        if !(30_000..=300_000).contains(&lease_duration_ms) {
            return Err(ApiError::bad_request(
                "lease_duration_ms must be between 30000 and 300000",
            ));
        }
        let claim = store
            .claim_run_lease(
                &claims.run_id,
                &claims.app_id,
                job_id,
                lease_token,
                lease_duration_ms,
            )
            .await
            .map_err(map_lease_error)?;
        return Ok(Json(match claim {
            RunLeaseClaim::Acquired { run, expires_at } => ProgressUpdateResponse {
                accepted: true,
                status: format!("{:?}", run.status),
                lease_acquired: Some(true),
                lease_expires_at: Some(expires_at),
            },
            RunLeaseClaim::Busy { run, expires_at } => ProgressUpdateResponse {
                accepted: false,
                status: format!("{:?}", run.status),
                lease_acquired: Some(false),
                lease_expires_at: Some(expires_at),
            },
            RunLeaseClaim::Terminal { run } => ProgressUpdateResponse {
                accepted: false,
                status: format!("{:?}", run.status),
                lease_acquired: Some(false),
                lease_expires_at: None,
            },
        }));
    }
    if lease_identity.is_some() && body.lease_duration_ms.is_some() {
        return Err(ApiError::bad_request(
            "lease_duration_ms is only valid for a running lease claim",
        ));
    }

    let run = store
        .get_run_for_app(&claims.run_id, &claims.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
        .ok_or(ApiError::NOT_FOUND)?;

    // A Cosmos-backed queued run is the Azure Service Bus contract. Once it
    // uses that backend, omitting ownership proof must never downgrade the
    // request to the legacy best-effort path.
    if lease_identity.is_none()
        && store.backend_name() == "cosmos"
        && run.mode == StateRunMode::Queue
    {
        return Err(ApiError::bad_request(
            "Cosmos queue progress requires job_id and lease_token",
        ));
    }

    // Don't accept updates for terminal states
    if run.status.is_terminal() {
        return Ok(Json(ProgressUpdateResponse {
            accepted: false,
            status: format!("{:?}", run.status),
            lease_acquired: lease_identity.map(|_| false),
            lease_expires_at: None,
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

    let updated = if let Some((job_id, lease_token)) = lease_identity {
        if !update
            .status
            .as_ref()
            .is_some_and(StateRunStatus::is_terminal)
        {
            return Err(ApiError::bad_request(
                "lease-protected progress must claim running or persist a terminal status",
            ));
        }
        store
            .update_run_with_lease(&claims.run_id, &claims.app_id, job_id, lease_token, update)
            .await
            .map_err(map_lease_error)?
    } else {
        store
            .update_run(&claims.run_id, update)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to update run: {}", e)))?
    };

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
        let user_id = execution_claim_user_id(&claims.sub).map(ToOwned::to_owned);
        if let Err(e) = track_execution_usage(
            &state.db,
            &claims.board_id,
            claims.event_id.as_deref().unwrap_or_default(),
            &claims.run_id,
            duration_us,
            exec_status,
            user_id.as_deref(),
            claims.technical_user_id.as_deref(),
            &claims.app_id,
        )
        .await
        {
            tracing::warn!(error=%e, "Failed to track execution usage");
        }
    }

    Ok(Json(ProgressUpdateResponse {
        accepted: true,
        status: format!("{:?}", updated.status),
        lease_acquired: lease_identity.map(|_| true),
        lease_expires_at: None,
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

    if body.events.len() > 1_000 {
        return Err(ApiError::bad_request(
            "an execution event callback may contain at most 1000 events",
        ));
    }
    let lease_identity = queue_lease_identity(body.job_id.as_deref(), body.lease_token.as_deref())?;
    if let Some((job_id, lease_token)) = lease_identity {
        store
            .validate_run_lease(&claims.run_id, &claims.app_id, job_id, lease_token)
            .await
            .map_err(map_lease_error)?;
    } else if store.backend_name() == "cosmos" {
        let run = store
            .get_run_for_app(&claims.run_id, &claims.app_id)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
            .ok_or(ApiError::NOT_FOUND)?;
        if run.mode == StateRunMode::Queue {
            return Err(ApiError::bad_request(
                "Cosmos queue events require job_id and lease_token",
            ));
        }
    }

    let expires_at = chrono::Utc::now().timestamp_millis() + 24 * 60 * 60 * 1000; // 24 hours
    let (events, next_seq) =
        if lease_identity.is_some() {
            let mut previous_sequence = None;
            let mut events = Vec::with_capacity(body.events.len());
            for event in &body.events {
                let id = event.id.as_deref().ok_or_else(|| {
                    ApiError::bad_request("strict queue events require a stable id")
                })?;
                validate_lease_component("event id", id)?;
                let sequence = event.sequence.ok_or_else(|| {
                    ApiError::bad_request("strict queue events require a stable sequence")
                })?;
                if sequence < 0 || previous_sequence.is_some_and(|previous| sequence <= previous) {
                    return Err(ApiError::bad_request(
                        "strict queue event sequences must be non-negative and strictly increasing",
                    ));
                }
                let canonical_id = execution_event_id(&claims.run_id, sequence);
                if id != canonical_id {
                    return Err(ApiError::bad_request(
                        "strict queue event id does not match its run and sequence",
                    ));
                }
                previous_sequence = Some(sequence);
                events.push(CreateEventInput {
                    id: id.to_string(),
                    run_id: claims.run_id.clone(),
                    sequence,
                    event_type: event.event_type.clone(),
                    payload: event.payload.clone(),
                    expires_at,
                });
            }
            let next = previous_sequence.map_or(0, |sequence| sequence.saturating_add(1));
            (events, next)
        } else {
            // Preserve the legacy allocation contract for non-queue runtimes.
            let max_seq = store.get_max_sequence(&claims.run_id).await.map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to get max sequence: {}", e))
            })?;
            let mut next = max_seq.saturating_add(1);
            let events = body
                .events
                .iter()
                .map(|event| {
                    let input = CreateEventInput {
                        id: create_id(),
                        run_id: claims.run_id.clone(),
                        sequence: next,
                        event_type: event.event_type.clone(),
                        payload: event.payload.clone(),
                        expires_at,
                    };
                    next += 1;
                    input
                })
                .collect();
            (events, next)
        };

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
#[tracing::instrument(name = "GET /execution/poll", skip_all)]
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
#[tracing::instrument(name = "GET /execution/run/{run_id}", skip(state, user))]
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
    user_id: Option<&str>,
    technical_user_id: Option<&str>,
    app_id: &str,
) -> Result<(), flow_like_types::Error> {
    let existing = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::Version.eq(version))
        .one(db)
        .await?;
    if existing.is_some() {
        return Ok(());
    }

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
        user_id: Set(user_id.map(ToOwned::to_owned)),
        technical_user_id: Set(technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(Some(app_id.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    };
    record.insert(db).await?;
    Ok(())
}

fn execution_claim_user_id(subject: &str) -> Option<&str> {
    let subject = subject.trim();
    if subject.is_empty() || subject == "local" || subject.starts_with("sink:") {
        None
    } else {
        Some(subject)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    crate::middleware::jwt::viewer_authorization(headers)
        .ok_or_else(|| ApiError::bad_request("Missing Authorization header".to_string()))?
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::bad_request("Invalid Authorization header format".to_string()))
}

fn queue_lease_identity<'a>(
    job_id: Option<&'a str>,
    lease_token: Option<&'a str>,
) -> Result<Option<(&'a str, &'a str)>, ApiError> {
    match (job_id, lease_token) {
        (None, None) => Ok(None),
        (Some(job_id), Some(lease_token)) => {
            validate_lease_component("job_id", job_id)?;
            validate_lease_component("lease_token", lease_token)?;
            Ok(Some((job_id, lease_token)))
        }
        _ => Err(ApiError::bad_request(
            "job_id and lease_token must be supplied together",
        )),
    }
}

fn validate_lease_component(name: &str, value: &str) -> Result<(), ApiError> {
    let valid = (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "{name} is not a valid queue ownership identifier"
        )))
    }
}

fn execution_event_id(run_id: &str, sequence: i32) -> String {
    let digest = blake3::hash(format!("{run_id}:{sequence}").as_bytes());
    format!("evt-{}", digest.to_hex())
}

fn map_lease_error(error: StateStoreError) -> ApiError {
    match error {
        StateStoreError::NotFound => ApiError::NOT_FOUND,
        StateStoreError::LeaseConflict(message) => ApiError::conflict(message),
        other => ApiError::internal_error(anyhow!("Execution lease operation failed: {other}")),
    }
}

/// Get or create the execution state store from app state
pub(crate) async fn get_state_store(
    state: &AppState,
) -> Result<Arc<dyn ExecutionStateStore>, ApiError> {
    // Build config with available AppState components
    let mut config =
        crate::execution::state::StateStoreConfig::default().with_db(Arc::new(state.db.clone()));

    // Pass AWS config and content store for DynamoDB backend
    #[cfg(feature = "aws")]
    {
        config = config.with_aws_config(state.aws_client.clone());
    }

    #[cfg(any(feature = "dynamodb", feature = "cosmos"))]
    {
        config = config.with_content_store(state.content_bucket.clone());
    }

    crate::execution::state::create_state_store(config)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to create state store: {}", e)))
}

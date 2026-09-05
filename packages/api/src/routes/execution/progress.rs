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
        ExecutionClaims, PageActionSealingContext,
        state::{
            CreateEventInput, EventQuery, ExecutionRunRecord, ExecutionStateStore,
            PostgresStateStore, RunLeaseClaim, RunMode as StateRunMode,
            RunStatus as StateRunStatus, StateStoreError, UpdateRunInput,
        },
        verify_execution_jwt, verify_user_jwt,
    },
    routes::app::prerun_shared::load_exact_prerun_manifest,
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
use std::{collections::HashSet, sync::Arc};
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
    /// Stable executor-generated identity and sequence. New executors send
    /// both in every mode; older non-queue executors may omit both.
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
    // The selected store's conditional write is the serialization point; a
    // competing delivery receives Busy until expiry.
    if matches!(&body.status, Some(ProgressStatus::Running))
        && let Some((job_id, lease_token)) = lease_identity
    {
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
            RunLeaseClaim::Acquired { run, expires_at } => {
                mirror_run_update_to_sql(&state, store.as_ref(), &run).await?;
                ProgressUpdateResponse {
                    accepted: true,
                    status: format!("{:?}", run.status),
                    lease_acquired: Some(true),
                    lease_expires_at: Some(expires_at),
                }
            }
            RunLeaseClaim::Busy { run, expires_at } => ProgressUpdateResponse {
                accepted: false,
                status: format!("{:?}", run.status),
                lease_acquired: Some(false),
                lease_expires_at: Some(expires_at),
            },
            RunLeaseClaim::Terminal { run } => {
                mirror_run_update_to_sql(&state, store.as_ref(), &run).await?;
                ProgressUpdateResponse {
                    accepted: false,
                    status: format!("{:?}", run.status),
                    lease_acquired: Some(false),
                    lease_expires_at: None,
                }
            }
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

    // Lease-backed queue workers must never downgrade an ownership-protected
    // callback to the legacy best-effort path.
    if lease_identity.is_none()
        && backend_requires_queue_lease(store.backend_name())
        && run.mode == StateRunMode::Queue
    {
        return Err(ApiError::bad_request(
            "queue progress requires job_id and lease_token for this state backend",
        ));
    }

    // Don't accept updates for terminal states
    if run.status.is_terminal() {
        mirror_run_update_to_sql(&state, store.as_ref(), &run).await?;
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

    mirror_run_update_to_sql(&state, store.as_ref(), &updated).await?;

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

async fn mirror_run_update_to_sql(
    state: &AppState,
    store: &dyn ExecutionStateStore,
    run: &ExecutionRunRecord,
) -> Result<(), ApiError> {
    if store.backend_name() != "postgres" {
        PostgresStateStore::new(Arc::new(state.db.clone()))
            .mirror_run_update(run)
            .await
            .map_err(|error| {
                ApiError::internal_error(anyhow!(
                    "Failed to mirror run '{}' into canonical SQL: {error}",
                    run.id
                ))
            })?;
    }
    if run.status.is_terminal() {
        let persisted = crate::entity::execution_run::Entity::find_by_id(&run.id)
            .filter(crate::entity::execution_run::Column::AppId.eq(&run.app_id))
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        crate::audit::record_execution_result(
            &crate::audit::ExecutionAuditContext::from(state),
            &persisted,
            &run.id,
            crate::entity::sea_orm_active_enums::AuditActorType::Executor,
        )
        .await?;
    }
    Ok(())
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
    Json(mut body): Json<PushEventsRequest>,
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
    } else if backend_requires_queue_lease(store.backend_name()) {
        let run = store
            .get_run_for_app(&claims.run_id, &claims.app_id)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
            .ok_or(ApiError::NOT_FOUND)?;
        if run.mode == StateRunMode::Queue {
            return Err(ApiError::bad_request(
                "queue events require job_id and lease_token for this state backend",
            ));
        }
    }

    // Executors are allowed to produce A2UI, but they do not hold the API's
    // Page-action signing key. Resolve the board identity from the signed
    // executor claims and replace every executable route before any payload
    // crosses the persistence boundary.
    if let Some(context) = page_action_sealing_context(&state, &claims).await? {
        seal_page_event_payloads(&context, &mut body.events);
    }

    let expires_at = chrono::Utc::now().timestamp_millis() + 24 * 60 * 60 * 1000; // 24 hours
    let identity_mode = event_identity_mode(&body.events, lease_identity.is_some())?;
    let (events, next_seq) = match identity_mode {
        EventIdentityMode::Stable => stable_event_inputs(&claims.run_id, &body.events, expires_at)?,
        EventIdentityMode::Legacy => {
            // Old executors omitted identity. Keep their allocation contract,
            // while new executors avoid this cross-instance read/max/write race.
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
                    next = next.saturating_add(1);
                    input
                })
                .collect();
            (events, next)
        }
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

/// Read persisted state with the run's executor capability. Trusted queue
/// consumers use this before acknowledgement, independently of sandbox stdout.
pub async fn executor_result(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = extract_bearer_token(&headers)?;
    let claims =
        verify_execution_jwt(token).map_err(|_| ApiError::bad_request("Invalid executor JWT"))?;
    let store = get_state_store(&state).await?;
    let run = store
        .get_run_for_app(&claims.run_id, &claims.app_id)
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("Failed to read execution result: {error}"))
        })?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(Json(serde_json::json!({
        "run_id": run.id,
        "status": run.status,
        "terminal": run.status.is_terminal(),
    })))
}

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

fn poll_after_sequence(after_sequence: Option<i32>) -> i32 {
    // Redis and DynamoDB turn this exclusive cursor into an inclusive lower
    // bound by adding one. Keep the public cursor inside that safe range and
    // treat older negative sentinels like the documented initial value.
    after_sequence.unwrap_or(-1).clamp(-1, i32::MAX - 1)
}

fn cursor_manages_delivery(after_sequence: Option<i32>) -> bool {
    after_sequence.is_some()
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
    let after_seq = poll_after_sequence(params.after_sequence);
    let cursor_manages_delivery = cursor_manages_delivery(params.after_sequence);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);

    loop {
        // Get run status
        let run = store
            .get_run_for_app(&claims.run_id, &claims.app_id)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get run: {}", e)))?
            .ok_or(ApiError::NOT_FOUND)?;

        // Get events after the client's acknowledged sequence.
        let events = store
            .get_events(EventQuery {
                run_id: claims.run_id.clone(),
                after_sequence: Some(after_seq),
                // Cursor-aware clients advance only after receiving the
                // response. Returning previously delivered events after the
                // cursor makes a dropped Lambda response safely replayable.
                only_undelivered: !cursor_manages_delivery,
                limit: Some(100),
            })
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get events: {}", e)))?;

        // Return immediately if terminal state or we have events
        let is_terminal = run.status.is_terminal();
        if is_terminal || !events.is_empty() || std::time::Instant::now() >= deadline {
            // Legacy clients have no cursor, so retain their delivery marker.
            // Marking one delivered retires it for good, which is only safe
            // because `get_events` fails the poll rather than handing back an
            // event whose staged payload it could not read for a transient
            // reason. Only a payload the store reports as gone comes through,
            // marked, and it would not come back on a retry either.
            if !events.is_empty() && !cursor_manages_delivery {
                let event_ids: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
                let _ = store
                    .mark_events_delivered(&claims.run_id, &event_ids)
                    .await;
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

#[allow(clippy::too_many_arguments)]
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

    let now = chrono::Utc::now().fixed_offset();
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

async fn page_action_sealing_context(
    state: &AppState,
    claims: &ExecutionClaims,
) -> Result<Option<PageActionSealingContext>, ApiError> {
    let Some(page) = claims.page_execution.as_ref() else {
        return Ok(None);
    };

    let event_id = claims
        .event_id
        .as_deref()
        .filter(|event_id| !event_id.trim().is_empty())
        .ok_or_else(|| {
            ApiError::bad_request("A Page execution token must identify its source Event")
        })?;
    if claims.app_id.trim().is_empty()
        || claims.board_id.trim().is_empty()
        || claims.run_id.trim().is_empty()
        || page.page_id.trim().is_empty()
        || page.manifest_revision.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "The Page execution token has an incomplete sealing context",
        ));
    }

    let board_etag = page
        .board_etag
        .as_deref()
        .map(str::trim)
        .filter(|etag| !etag.is_empty());
    if !matches!(
        (page.board_version, board_etag),
        (Some(_), None) | (None, Some(_))
    ) {
        return Err(ApiError::bad_request(
            "The Page execution token has an invalid board selector",
        ));
    }

    let authority_revision = page
        .entry_authority_revision
        .as_deref()
        .map(str::trim)
        .filter(|revision| !revision.is_empty());
    let allowed_entry_nodes = if let Some(authority_revision) = authority_revision {
        let manifest = load_exact_prerun_manifest(
            state,
            &claims.app_id,
            &claims.board_id,
            page.board_version,
            board_etag,
        )
        .await?;
        if manifest.signature != authority_revision
            || manifest.entry_node_ids.is_empty()
            || manifest
                .entry_node_ids
                .iter()
                .any(|node_id| node_id.trim().is_empty())
        {
            return Err(ApiError::bad_request(
                "The Page execution token has stale or invalid entry-node authority",
            ));
        }
        let allowed = manifest
            .entry_node_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if page
            .target_node_id
            .as_deref()
            .is_none_or(|target| !allowed.contains(target))
        {
            return Err(ApiError::bad_request(
                "The Page execution token target is outside its entry-node authority",
            ));
        }
        allowed
    } else if page.allowed_entry_node_ids.is_empty() && page.board_version.is_some() {
        // Compatibility for pinned Page executor JWTs minted before the
        // signed allow-list field existed. Pinned boards are immutable, so
        // reloading the selected version reproduces the original authority.
        let cached = state
            .master_board_shared(&claims.app_id, &claims.board_id, state, page.board_version)
            .await
            .map_err(|error| {
                tracing::error!(
                    error = %error,
                    app_id = %claims.app_id,
                    board_id = %claims.board_id,
                    version = ?page.board_version,
                    "Failed to load the pinned board for Page action sealing"
                );
                ApiError::internal_error(anyhow!(
                    "Failed to validate the Page execution board context"
                ))
            })?;
        let board = cached.board.as_ref();
        if board.id != claims.board_id
            || page
                .board_version
                .is_some_and(|expected| board.version != expected)
        {
            return Err(ApiError::bad_request(
                "The Page execution board context is invalid",
            ));
        }
        board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
            .filter(|node| node.start == Some(true))
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>()
    } else if page.allowed_entry_node_ids.is_empty() {
        return Err(ApiError::bad_request(
            "The Latest Page execution token is missing its entry-node authority",
        ));
    } else {
        if page
            .allowed_entry_node_ids
            .iter()
            .any(|node_id| node_id.trim().is_empty())
        {
            return Err(ApiError::bad_request(
                "The Page execution token has an invalid entry-node authority",
            ));
        }
        let allowed = page
            .allowed_entry_node_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if page
            .target_node_id
            .as_deref()
            .is_some_and(|target| !allowed.contains(target))
        {
            return Err(ApiError::bad_request(
                "The Page execution token target is outside its entry-node authority",
            ));
        }
        allowed
    };

    Ok(Some(PageActionSealingContext {
        sub: claims.sub.clone(),
        technical_user_id: claims.technical_user_id.clone(),
        source_app_id: claims.app_id.clone(),
        source_event_id: event_id.to_string(),
        source_page_id: page.page_id.clone(),
        source_manifest_revision: page.manifest_revision.clone(),
        target_app_id: claims.app_id.clone(),
        target_board_id: claims.board_id.clone(),
        target_board_version: page.board_version,
        target_board_etag: page.board_etag.clone(),
        wasm_authority_revision: page.wasm_authority_revision.clone(),
        origin_run_id: claims.run_id.clone(),
        allowed_entry_nodes,
    }))
}

fn seal_page_event_payloads(
    context: &PageActionSealingContext,
    events: &mut [ExecutionEventInput],
) {
    for (batch_index, event) in events.iter_mut().enumerate() {
        let message_id = page_event_message_id(event, batch_index);
        let report = context.seal_payload(&event.event_type, &message_id, &mut event.payload);
        if report.sealed > 0 || report.rejected > 0 {
            tracing::debug!(
                message_id,
                sealed = report.sealed,
                rejected = report.rejected,
                "Sealed dynamic Page actions in an executor event"
            );
        }
    }
}

fn page_event_message_id(event: &ExecutionEventInput, batch_index: usize) -> String {
    event
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .sequence
                .map(|sequence| format!("sequence:{sequence}"))
        })
        .unwrap_or_else(|| format!("batch-index:{batch_index}"))
}

pub(super) fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventIdentityMode {
    Stable,
    Legacy,
}

fn event_identity_mode(
    events: &[ExecutionEventInput],
    stable_required: bool,
) -> Result<EventIdentityMode, ApiError> {
    let mut saw_stable = false;
    let mut saw_legacy = false;
    for event in events {
        match (&event.id, event.sequence) {
            (Some(_), Some(_)) => saw_stable = true,
            (None, None) => saw_legacy = true,
            _ => {
                return Err(ApiError::bad_request(
                    "execution event id and sequence must be supplied together",
                ));
            }
        }
    }

    if saw_stable && saw_legacy {
        return Err(ApiError::bad_request(
            "an execution event batch cannot mix stable and legacy identities",
        ));
    }
    if stable_required && saw_legacy {
        return Err(ApiError::bad_request(
            "strict queue events require stable ids and sequences",
        ));
    }

    if saw_stable || stable_required {
        Ok(EventIdentityMode::Stable)
    } else {
        Ok(EventIdentityMode::Legacy)
    }
}

fn stable_event_inputs(
    run_id: &str,
    events: &[ExecutionEventInput],
    expires_at: i64,
) -> Result<(Vec<CreateEventInput>, i32), ApiError> {
    let mut previous_sequence = None;
    let mut inputs = Vec::with_capacity(events.len());
    for event in events {
        let id = event
            .id
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("execution events require a stable id"))?;
        validate_lease_component("event id", id)?;
        let sequence = event
            .sequence
            .ok_or_else(|| ApiError::bad_request("execution events require a stable sequence"))?;
        if sequence < 0 || previous_sequence.is_some_and(|previous| sequence <= previous) {
            return Err(ApiError::bad_request(
                "execution event sequences must be non-negative and strictly increasing",
            ));
        }
        if id != execution_event_id(run_id, sequence) {
            return Err(ApiError::bad_request(
                "execution event id does not match its run and sequence",
            ));
        }
        previous_sequence = Some(sequence);
        inputs.push(CreateEventInput {
            id: id.to_string(),
            run_id: run_id.to_string(),
            sequence,
            event_type: event.event_type.clone(),
            payload: event.payload.clone(),
            expires_at,
        });
    }

    let next = previous_sequence.map_or(0, |sequence| sequence.saturating_add(1));
    Ok((inputs, next))
}

fn map_lease_error(error: StateStoreError) -> ApiError {
    match error {
        StateStoreError::NotFound => ApiError::NOT_FOUND,
        StateStoreError::LeaseConflict(message) => ApiError::conflict(message),
        other => ApiError::internal_error(anyhow!("Execution lease operation failed: {other}")),
    }
}

fn backend_requires_queue_lease(backend: &str) -> bool {
    matches!(backend, "cosmos" | "dynamodb" | "firestore" | "redis")
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

    config = config.with_content_store(state.content_bucket.clone());

    #[cfg(feature = "s3")]
    {
        config = config.with_meta_store(state.meta_bucket.clone());
    }

    crate::execution::state::create_state_store(config)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to create state store: {}", e)))
}

#[cfg(test)]
mod page_action_delivery_tests {
    use super::*;

    fn callback_event(run_id: &str, sequence: Option<i32>) -> ExecutionEventInput {
        ExecutionEventInput {
            id: sequence.map(|sequence| execution_event_id(run_id, sequence)),
            sequence,
            event_type: "chunk".into(),
            payload: serde_json::json!({"value": sequence}),
        }
    }

    #[test]
    fn stateless_lambda_nonlease_callbacks_accept_canonical_identity() {
        let events = vec![
            callback_event("run-1", Some(0)),
            callback_event("run-1", Some(1)),
        ];
        assert_eq!(
            event_identity_mode(&events, false).expect("stable callback should be accepted"),
            EventIdentityMode::Stable
        );
        let (stored, next) =
            stable_event_inputs("run-1", &events, 1_900_000_000_000).expect("valid identity");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id, execution_event_id("run-1", 0));
        assert_eq!(stored[1].sequence, 1);
        assert_eq!(next, 2);
    }

    #[test]
    fn stateless_lambda_cloud_queue_backends_require_lease_proof() {
        for backend in ["cosmos", "dynamodb", "firestore", "redis"] {
            assert!(backend_requires_queue_lease(backend));
        }
        for backend in ["postgres", "object_storage"] {
            assert!(!backend_requires_queue_lease(backend));
        }
    }

    #[test]
    fn legacy_callbacks_remain_compatible_but_cannot_mix_identity_modes() {
        let legacy = vec![callback_event("run-1", None)];
        assert_eq!(
            event_identity_mode(&legacy, false).expect("old executor should remain compatible"),
            EventIdentityMode::Legacy
        );
        assert!(event_identity_mode(&legacy, true).is_err());

        let mixed = vec![
            callback_event("run-1", Some(0)),
            callback_event("run-1", None),
        ];
        assert!(event_identity_mode(&mixed, false).is_err());
        let partial = vec![ExecutionEventInput {
            id: Some(execution_event_id("run-1", 0)),
            sequence: None,
            event_type: "chunk".into(),
            payload: serde_json::Value::Null,
        }];
        assert!(event_identity_mode(&partial, false).is_err());
    }

    #[test]
    fn canonical_identity_rejects_wrong_run_and_nonmonotonic_sequences() {
        let wrong_run = vec![callback_event("other-run", Some(0))];
        assert!(stable_event_inputs("run-1", &wrong_run, 1_900_000_000_000).is_err());

        let duplicate = vec![
            callback_event("run-1", Some(0)),
            callback_event("run-1", Some(0)),
        ];
        assert!(stable_event_inputs("run-1", &duplicate, 1_900_000_000_000).is_err());
        let negative = vec![callback_event("run-1", Some(-1))];
        assert!(stable_event_inputs("run-1", &negative, 1_900_000_000_000).is_err());
    }

    #[test]
    fn stateless_lambda_first_poll_starts_before_executor_sequence_zero() {
        assert_eq!(poll_after_sequence(None), -1);
        assert_eq!(poll_after_sequence(Some(0)), 0);
        assert_eq!(poll_after_sequence(Some(41)), 41);
        assert_eq!(poll_after_sequence(Some(i32::MIN)), -1);
        assert_eq!(poll_after_sequence(Some(i32::MAX)), i32::MAX - 1);
        assert!(!cursor_manages_delivery(None));
        assert!(cursor_manages_delivery(Some(-1)));
        assert!(cursor_manages_delivery(Some(41)));
    }

    fn unpinned_context() -> PageActionSealingContext {
        PageActionSealingContext {
            sub: "user-1".into(),
            technical_user_id: None,
            source_app_id: "app-1".into(),
            source_event_id: "event-1".into(),
            source_page_id: "page-1".into(),
            source_manifest_revision: "revision-1".into(),
            target_app_id: "app-1".into(),
            target_board_id: "board-1".into(),
            target_board_version: None,
            target_board_etag: None,
            wasm_authority_revision: None,
            origin_run_id: "run-1".into(),
            allowed_entry_nodes: HashSet::from(["entry-1".into()]),
        }
    }

    #[test]
    fn page_event_message_identity_prefers_id_then_sequence_then_batch_index() {
        let with_id = ExecutionEventInput {
            id: Some("executor-event-1".into()),
            sequence: Some(41),
            event_type: "chunk".into(),
            payload: serde_json::Value::Null,
        };
        let with_sequence = ExecutionEventInput {
            id: None,
            sequence: Some(42),
            event_type: "chunk".into(),
            payload: serde_json::Value::Null,
        };
        let without_identity = ExecutionEventInput {
            id: None,
            sequence: None,
            event_type: "chunk".into(),
            payload: serde_json::Value::Null,
        };

        assert_eq!(page_event_message_id(&with_id, 3), "executor-event-1");
        assert_eq!(page_event_message_id(&with_sequence, 3), "sequence:42");
        assert_eq!(page_event_message_id(&without_identity, 3), "batch-index:3");
    }

    #[test]
    fn unpinned_page_strips_routes_without_changing_event_order_or_identity() {
        let mut events = vec![
            ExecutionEventInput {
                id: Some("event-a".into()),
                sequence: Some(7),
                event_type: "a2ui".into(),
                payload: serde_json::json!({
                    "type": "surfaceUpdate",
                    "ordinal": 1,
                    "components": [{"id": "first", "component": {
                        "actions": [{
                            "name": "workflow_event",
                            "context": {"nodeId": "entry-1", "payload": "kept"}
                        }]
                    }}]
                }),
            },
            ExecutionEventInput {
                id: Some("event-b".into()),
                sequence: Some(8),
                event_type: "a2ui".into(),
                payload: serde_json::json!({
                    "type": "surfaceUpdate",
                    "ordinal": 2,
                    "components": [{"id": "second", "component": {
                        "actions": [{
                            "name": "workflow_event",
                            "context": {"nodeId": "entry-1"}
                        }]
                    }}]
                }),
            },
        ];

        seal_page_event_payloads(&unpinned_context(), &mut events);

        assert_eq!(events[0].id.as_deref(), Some("event-a"));
        assert_eq!(events[0].sequence, Some(7));
        assert_eq!(events[0].event_type, "a2ui");
        assert_eq!(events[0].payload["ordinal"], 1);
        assert!(
            events[0].payload["components"][0]["component"]["actions"][0]["context"]
                .get("nodeId")
                .is_none()
        );
        assert!(
            events[0].payload["components"][0]["component"]["actions"][0]
                .get("pageAction")
                .is_none()
        );
        assert_eq!(
            events[0].payload["components"][0]["component"]["actions"][0]["context"]["payload"],
            "kept"
        );

        assert_eq!(events[1].id.as_deref(), Some("event-b"));
        assert_eq!(events[1].sequence, Some(8));
        assert_eq!(events[1].event_type, "a2ui");
        assert_eq!(events[1].payload["ordinal"], 2);
        assert!(
            events[1].payload["components"][0]["component"]["actions"][0]["context"]
                .get("nodeId")
                .is_none()
        );
        assert!(
            events[1].payload["components"][0]["component"]["actions"][0]
                .get("pageAction")
                .is_none()
        );
    }
}

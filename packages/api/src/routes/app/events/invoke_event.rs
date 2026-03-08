//! Invoke event execution endpoint
//!
//! This endpoint triggers synchronous execution of an event workflow.
//! The execution runs in an isolated container (executor service, Lambda, etc.)
//! and streams results back to the user via SSE.
//!
//! Flow:
//! 1. Check user access permissions
//! 2. Look up the event to get the associated board
//! 3. Create a run record in the database
//! 4. Create scoped credentials based on user permissions
//! 5. Call executor service via HTTP streaming
//! 6. Proxy SSE events back to the user
//!
//! Query Parameters:
//! - `local=true`: Track run in DB only, no remote execution (returns JSON)
//! - `isolated=true`: Use isolated K8s job instead of pool (Kubernetes only)

use crate::{
    ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        ByteStream, DispatchRequest, ExecutionBackend, ExecutionJwtParams, TokenType,
        fetch_profile_for_dispatch, is_jwt_configured, payload_storage, proxy_sse_response,
        resolve_wasm_packages, sign_execution_jwt,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use flow_like_types::{anyhow, create_id, tokio};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::db::get_event_from_db;

/// Query parameters for event invocation
#[derive(Clone, Debug, Deserialize, Default, ToSchema)]
pub struct InvokeEventQuery {
    /// Track run locally only - no remote execution
    #[serde(default)]
    pub local: bool,
    /// Use isolated execution (K8s job instead of pool)
    #[serde(default)]
    pub isolated: bool,
}

/// Request body for event invocation
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct InvokeEventRequest {
    /// Optional board version to execute (defaults to latest)
    pub version: Option<String>,
    /// Input payload for the execution
    pub payload: Option<serde_json::Value>,
    /// User's auth token to pass to the flow
    pub token: Option<String>,
    /// OAuth tokens keyed by provider name
    pub oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Runtime-configured variables to override board variables
    #[schema(value_type = Option<Object>)]
    pub runtime_variables:
        Option<std::collections::HashMap<String, flow_like::flow::variable::Variable>>,
    /// Optional profile ID to select a specific user profile for execution
    pub profile_id: Option<String>,
}

/// Response from event invocation
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct InvokeEventResponse {
    /// Unique run ID
    pub run_id: String,
    /// Current status
    pub status: String,
    /// Message
    pub message: Option<String>,
    /// User JWT for polling (only for async/local mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_token: Option<String>,
}

/// Get credentials access for invoke - always InvokeWrite since
/// server-side execution is scoped through workflow logic
fn get_credentials_access() -> crate::credentials::CredentialsAccess {
    crate::credentials::CredentialsAccess::InvokeWrite
}

/// POST /apps/{app_id}/events/{event_id}/invoke
///
/// Invoke event execution. Use `?local=true` to track locally without dispatch.
/// Use `?isolated=true` for isolated K8s job execution (Kubernetes only).
///
/// Returns SSE stream for remote execution or JSON for local mode.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/invoke",
    tag = "events",
    description = "Invoke an event and stream execution results.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("local" = bool, Query, description = "Track locally without dispatch"),
        ("isolated" = bool, Query, description = "Use isolated execution")
    ),
    request_body = InvokeEventRequest,
    responses(
        (status = 200, description = "SSE stream or JSON", body = String, content_type = "text/event-stream"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/invoke",
    skip(state, user, params)
)]
pub async fn invoke_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<InvokeEventQuery>,
    Json(params): Json<InvokeEventRequest>,
) -> Result<Response, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    // Get event from database
    let event = get_event_from_db(&state.db, &event_id).await?;
    let board_id = event.board_id.clone();
    let event_json =
        serde_json::to_string(&event).map_err(|e| anyhow!("Failed to serialize event: {}", e))?;

    let run_id = create_id();
    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);

    let input_payload_len = params
        .payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    // Determine run mode
    let run_mode = if query.local {
        RunMode::Local
    } else if query.isolated {
        RunMode::KubernetesIsolated
    } else {
        RunMode::Http
    };

    // Store payload in object storage if present (for remote runs only - enables re-run)
    let input_payload_key = if !query.local {
        if let Some(ref payload) = params.payload {
            let payload_bytes = serde_json::to_vec(payload).map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to serialize payload: {}", e))
            })?;
            let master_creds = state.master_credentials().await.map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to get master credentials: {}", e))
            })?;
            let store = master_creds.to_store(false).await.map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to get object store: {}", e))
            })?;
            let stored = payload_storage::store_payload(
                store.as_generic(),
                &app_id,
                &run_id,
                &payload_bytes,
            )
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to store payload: {}", e)))?;
            Some(stored.key)
        } else {
            None
        }
    } else {
        None
    };

    // Build run record (insert happens later - sync for local/isolated, parallel for HTTP)
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(params.version.clone()),
        event_id: Set(Some(event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(run_mode.clone()),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(input_payload_key),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(Some(sub.clone())),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };

    // For local mode, insert synchronously and return JSON - no dispatch needed
    if query.local {
        run.insert(&state.db).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to create run record");
            ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
        })?;

        let poll_token = sign_execution_jwt(ExecutionJwtParams {
            user_id: sub.clone(),
            run_id: run_id.clone(),
            app_id: app_id.clone(),
            board_id: board_id.clone(),
            event_id: Some(event_id),
            callback_url: String::new(),
            token_type: TokenType::User,
            ttl_seconds: Some(60 * 60),
        })
        .ok();

        return Ok(Json(InvokeEventResponse {
            run_id,
            status: "pending".to_string(),
            message: Some("Run tracked locally - no remote execution".to_string()),
            poll_token,
        })
        .into_response());
    }

    // Check JWT signing is configured for remote execution
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing EXECUTION_KEY/EXECUTION_PUB env vars)"
        )));
    }

    // Get scoped credentials based on user permissions
    let access = get_credentials_access();
    let credentials = state.scoped_credentials(&sub, &app_id, access).await?;

    // Convert to SharedCredentials for runtime compatibility
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| anyhow!("Failed to serialize credentials: {}", e))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: sub.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to sign executor JWT");
        ApiError::internal_error(anyhow!("Failed to sign executor JWT: {}", e))
    })?;

    let profile =
        fetch_profile_for_dispatch(&state.db, &sub, params.profile_id.as_deref(), &app_id).await;

    let wasm_packages = resolve_wasm_packages(&state.db, &state.wasm_registry, &app_id).await;

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id,
        board_version: event.board_version,
        node_id: event.node_id.clone(),
        event_json: Some(event_json),
        payload: params.payload,
        user_id: sub,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token: params.token,
        oauth_tokens: params.oauth_tokens,
        stream_state: false,
        runtime_variables: params.runtime_variables,
        user_context: Some(permission.to_user_context()),
        profile,
        wasm_packages,
    };

    // For isolated K8s jobs, insert run record and dispatch async
    if query.isolated {
        run.insert(&state.db).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to create run record");
            ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
        })?;

        let response = state
            .dispatcher
            .dispatch_with_backend(ExecutionBackend::KubernetesJob, request)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to dispatch job");
                ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
            })?;

        return Ok(Json(InvokeEventResponse {
            run_id,
            status: response.status,
            message: Some(format!("Job dispatched via {} backend", response.backend)),
            poll_token: None,
        })
        .into_response());
    }

    // Determine the streaming dispatch method based on backend configuration
    let backend = state.dispatcher.backend();
    tracing::info!(run_id = %run_id, ?backend, "Dispatching streaming execution for event");

    // Create run record in DB (can happen in parallel with dispatch)
    let db_clone = state.db.clone();
    let run_id_clone = run_id.clone();
    let db_insert_handle = tokio::spawn(async move {
        run.insert(&db_clone).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to create run record");
            e
        })
    });

    // Dispatch based on the configured backend
    match backend {
        ExecutionBackend::LambdaStream => {
            // Use Lambda SDK streaming
            let (_dispatch_response, byte_stream) = state
                .dispatcher
                .dispatch_streaming(request)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to dispatch Lambda streaming job");
                    ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
                })?;

            // Wait for DB insert to complete
            if let Err(e) = db_insert_handle.await {
                tracing::error!(run_id = %run_id_clone, error = ?e, "DB insert task failed");
            }

            tracing::info!(run_id = %run_id, "Got Lambda response, starting stream proxy");

            Ok(proxy_lambda_sse_response(
                byte_stream,
                run_id,
                Some(std::sync::Arc::new(state.db.clone())),
            )
            .into_response())
        }
        _ => {
            // Use HTTP SSE for all other backends (Http, etc.)
            let (_dispatch_response, executor_response) = state
                .dispatcher
                .dispatch_http_sse(request)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to dispatch HTTP SSE job");
                    ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
                })?;

            // Wait for DB insert to complete
            if let Err(e) = db_insert_handle.await {
                tracing::error!(run_id = %run_id_clone, error = ?e, "DB insert task failed");
            }

            tracing::info!(run_id = %run_id, "Got executor response, starting stream proxy");

            Ok(proxy_sse_response(
                executor_response,
                run_id,
                Some(std::sync::Arc::new(state.db.clone())),
            )
            .into_response())
        }
    }
}

/// Create an SSE stream from a Lambda ByteStream response
fn proxy_lambda_sse_response(
    stream: ByteStream,
    run_id: String,
    db: Option<std::sync::Arc<sea_orm::DatabaseConnection>>,
) -> axum::response::sse::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::StreamExt;
    use std::time::Duration;

    let stream = async_stream::stream! {
        let mut byte_stream = stream;
        let mut buffer = Vec::new();

        while let Some(result) = byte_stream.next().await {
            match result {
                Ok(bytes) => {
                    // Append bytes to buffer
                    buffer.extend_from_slice(&bytes);

                    // Try to parse complete SSE events from buffer
                    while let Some(event) = extract_sse_event(&mut buffer) {
                        // Check if this is a completed event and update the database
                        if let Some(db) = &db
                            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data)
                                && let Some(event_type) = parsed.get("event_type").and_then(|v| v.as_str())
                                    && event_type == "completed" {
                                        let log_level = parsed.get("payload")
                                            .and_then(|p| p.get("log_level"))
                                            .and_then(|l| l.as_i64())
                                            .unwrap_or(0) as i32;
                                        let status = parsed.get("payload")
                                            .and_then(|p| p.get("status"))
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("Completed");

                                        let run_status = match status {
                                            "Failed" => RunStatus::Failed,
                                            "Cancelled" => RunStatus::Cancelled,
                                            "Timeout" => RunStatus::Timeout,
                                            _ => RunStatus::Completed,
                                        };

                                        let db = db.clone();
                                        let run_id_clone = run_id.clone();
                                        tokio::spawn(async move {
                                            use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
                                            use crate::entity::prelude::*;
                                            if let Ok(Some(run)) = ExecutionRun::find_by_id(&run_id_clone).one(db.as_ref()).await {
                                                let mut run: execution_run::ActiveModel = run.into();
                                                run.status = Set(run_status);
                                                run.log_level = Set(log_level);
                                                run.updated_at = Set(chrono::Utc::now().naive_utc());
                                                if let Err(e) = run.update(db.as_ref()).await {
                                                    tracing::error!(run_id = %run_id_clone, error = %e, "Failed to update run on completion");
                                                }
                                            }
                                        });
                                    }

                        let sse_event = Event::default()
                            .event(&event.event_type)
                            .data(event.data);
                        yield Ok(sse_event);
                    }
                }
                Err(e) => {
                    tracing::warn!(run_id = %run_id, error = %e, "Lambda stream error");
                    let error_event = Event::default()
                        .event("error")
                        .data(format!(r#"{{"error":"{}"}}"#, e));
                    yield Ok(error_event);
                    break;
                }
            }
        }

        tracing::debug!(run_id = %run_id, "Lambda SSE stream ended");
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(1)),
    )
}

/// Parsed SSE event
struct ParsedSseEvent {
    event_type: String,
    data: String,
}

/// Extract a complete SSE event from the buffer, if available
fn extract_sse_event(buffer: &mut Vec<u8>) -> Option<ParsedSseEvent> {
    // Look for double newline which marks end of SSE event
    let s = String::from_utf8_lossy(buffer);
    if let Some(end_pos) = s.find("\n\n") {
        let event_str = &s[..end_pos];
        let remainder = &s[end_pos + 2..];

        let mut event_type = "message".to_string();
        let mut data_parts = Vec::new();

        for line in event_str.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event_type = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data_parts.push(value.trim_start().to_string());
            }
        }

        // Update buffer with remainder
        *buffer = remainder.as_bytes().to_vec();

        if !data_parts.is_empty() {
            return Some(ParsedSseEvent {
                event_type,
                data: data_parts.join("\n"),
            });
        }
    }
    None
}

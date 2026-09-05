//! Invoke board execution endpoint
//!
//! This endpoint triggers synchronous execution of a board workflow.
//! The execution runs in an isolated container (executor service, Lambda, etc.)
//! and streams results back to the user via SSE.
//!
//! Flow:
//! 1. Check user access permissions
//! 2. Create a run record in the database
//! 3. Create scoped credentials based on user permissions
//! 4. Call executor service via HTTP streaming or Lambda SDK
//! 5. Proxy SSE events back to the user
//!
//! Query Parameters:
//! - `local=true`: Track run in DB only, no remote execution (returns JSON)
//! - `isolated=true`: Use isolated K8s job instead of pool (Kubernetes only)

use crate::{
    ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus, RunVariant},
    },
    error::ApiError,
    execution::{
        ByteStream, DispatchRequest, DispatchTrigger, ExecutionBackend, ExecutionJwtParams,
        TokenType, completed_run_status, fetch_profile_for_dispatch, format_run_version,
        is_jwt_configured, payload_storage, proxy_sse_response, resolve_wasm_packages,
        sign_execution_jwt, update_run_on_completion,
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
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Query parameters for board invocation
#[derive(Clone, Debug, Deserialize, Default, IntoParams, ToSchema)]
pub struct InvokeBoardQuery {
    /// Track run locally only - no remote execution
    #[serde(default)]
    pub local: bool,
    /// Use isolated execution (K8s job instead of pool)
    #[serde(default)]
    pub isolated: bool,
}

/// Request body for board invocation
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct InvokeBoardRequest {
    /// Node ID to start execution from (required)
    pub node_id: String,
    /// Optional board version as tuple (major, minor, patch) - defaults to latest
    pub version: Option<(u32, u32, u32)>,
    /// Input payload for the execution
    #[schema(value_type = Option<Object>)]
    pub payload: Option<serde_json::Value>,
    /// User's auth token to pass to the flow
    pub token: Option<String>,
    /// OAuth tokens keyed by provider name
    #[schema(value_type = Option<Object>)]
    pub oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Whether to stream node state updates (true for boards, false for events)
    #[serde(default = "default_stream_state")]
    pub stream_state: bool,
    /// Runtime-configured variables to override board variables
    #[schema(value_type = Option<Object>)]
    pub runtime_variables:
        Option<std::collections::HashMap<String, flow_like::flow::variable::Variable>>,
    /// Optional profile ID to select a specific user profile for execution
    pub profile_id: Option<String>,
}

fn default_stream_state() -> bool {
    true
}

/// Response from board invocation
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct InvokeBoardResponse {
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

/// Get credentials access for remote server-side execution.
fn get_credentials_access() -> crate::credentials::CredentialsAccess {
    crate::credentials::CredentialsAccess::ServerExecute
}

/// POST /apps/{app_id}/board/{board_id}/invoke
///
/// Invoke board execution. Use `?local=true` to track locally without dispatch.
/// Use `?isolated=true` for isolated K8s job execution (Kubernetes only).
///
/// Returns SSE stream for remote execution or JSON for local mode.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/invoke",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        InvokeBoardQuery
    ),
    request_body = InvokeBoardRequest,
    responses(
        (status = 200, description = "Board invocation started, returns SSE stream or JSON", body = InvokeBoardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/invoke",
    skip(state, user, query, params)
)]
pub async fn invoke_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<InvokeBoardQuery>,
    Json(params): Json<InvokeBoardRequest>,
) -> Result<Response, ApiError> {
    super::ensure_connected_app_board_invoke_denied(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteBoards);
    let sub = permission.effective_user_id().map_err(|_| {
        crate::error::ApiError::forbidden(
            "Invoking requires a caller that is linked to a user account",
        )
    })?;
    let technical_user_id = permission.technical_user_id().map(ToOwned::to_owned);
    let caller_app_chain = match &user {
        AppUser::ConnectedApp(connected) => Some(connected.app_chain.clone()),
        _ => None,
    };
    let parent_run_id = match &user {
        AppUser::ConnectedApp(connected) => connected.run_id.clone(),
        _ => None,
    };
    let inherited_correlation = match &user {
        AppUser::ConnectedApp(connected) => connected.correlation.clone(),
        _ => None,
    };

    let run_id = create_id();
    let expires_at = chrono::Utc::now().fixed_offset() + chrono::Duration::hours(24);

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

    // Inherit the trace root & business keys from the caller, else this is root.
    let mut correlation = inherited_correlation.unwrap_or_default();
    if correlation.trace_id.is_none() {
        correlation.trace_id = parent_run_id.clone().or_else(|| Some(run_id.clone()));
    }
    let correlation_keys = correlation.keys_json();

    let version_label = params.version.map(format_run_version);

    // Build run record (insert happens later - sync for local, parallel for HTTP)
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(version_label.clone()),
        event_id: Set(None),
        node_id: Set(Some(params.node_id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(run_mode.clone()),
        run_variant: Set(RunVariant::Primary),
        variant_name: Set(None),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
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
        technical_user_id: Set(technical_user_id.clone()),
        caller_app_chain: Set(caller_app_chain.clone().map(Into::into)),
        trace_id: Set(correlation.trace_id.clone()),
        parent_run_id: Set(parent_run_id.clone()),
        correlation_keys: Set(correlation_keys.clone()),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
    };
    let execution_audit = crate::audit::ExecutionAudit {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: None,
        node_id: Some(params.node_id.clone()),
        version: version_label,
        board_etag: None,
        mode: run_mode.clone(),
        status: RunStatus::Pending,
        input_payload_len,
        technical_user_id: technical_user_id.clone(),
    };

    // For local mode, insert synchronously and return JSON - no dispatch needed
    if query.local {
        crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create run record");
                ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
            })?;
        crate::audit::record_execution_start(&state, &user, execution_audit).await;

        println!("Tracking local run ID: {}", run_id);
        let poll_token = sign_execution_jwt(ExecutionJwtParams {
            user_id: sub.clone(),
            technical_user_id: technical_user_id.clone(),
            run_id: run_id.clone(),
            app_id: app_id.clone(),
            board_id: board_id.clone(),
            event_id: None,
            app_chain: caller_app_chain.clone(),
            correlation: None,
            callback_url: String::new(),
            token_type: TokenType::User,
            ttl_seconds: Some(60 * 60),
            shadow: None,
        })
        .ok();

        return Ok(Json(InvokeBoardResponse {
            run_id,
            status: "pending".to_string(),
            message: Some("Run tracked locally - no remote execution".to_string()),
            poll_token,
        })
        .into_response());
    }

    // Check JWT signing is configured for remote execution
    if !is_jwt_configured() {
        println!("Execution JWT signing not configured");
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }

    // Resolve independent dispatch inputs concurrently: STS scoped
    // credentials, user profile, and WASM package URLs do not depend on each
    // other and are all on the hot invoke path.
    let access = get_credentials_access();
    let (credentials_result, profile, wasm_packages) = {
        use flow_like_types::tokio;
        tokio::join!(
            state.scoped_credentials(&sub, &app_id, access),
            fetch_profile_for_dispatch(&state, &sub, params.profile_id.as_deref(), &app_id, true),
            resolve_wasm_packages(&state, &app_id),
        )
    };
    let credentials = credentials_result?;

    // Convert to SharedCredentials for runtime compatibility
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| anyhow!("Failed to serialize credentials: {}", e))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: sub.clone(),
        technical_user_id: technical_user_id.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: None,
        app_chain: caller_app_chain.clone(),
        correlation: correlation.clone().into_option(),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
        shadow: None,
    })
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to sign executor JWT");
        ApiError::internal_error(anyhow!("Failed to sign executor JWT: {}", e))
    })?;

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id,
        board_version: params.version,
        board_etag: None,
        node_id: params.node_id.clone(),
        event_json: None,
        payload: params.payload,
        user_id: sub,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token: params.token,
        oauth_tokens: params.oauth_tokens,
        stream_state: params.stream_state,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Sync),
        runtime_variables: params.runtime_variables,
        user_context: Some(permission.to_user_context()),
        profile,
        wasm_packages,
        channel: None,
        trigger: DispatchTrigger::User,
        shadow: false,
        artifact: None,
    };

    // For isolated K8s jobs, insert run record and dispatch async
    if query.isolated {
        // Insert synchronously for K8s jobs (returns immediately anyway)
        crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create run record");
                ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
            })?;
        crate::audit::record_execution_start(&state, &user, execution_audit).await;

        let response = match state
            .dispatcher
            .dispatch_with_backend(ExecutionBackend::KubernetesJob, request)
            .await {
                Ok(response) => Ok(response),
                Err(error) => {
                    if let Err(audit_error) = crate::audit::record_execution_dispatch_failure(&state, &run_id, "dispatcher").await {
                        tracing::error!(run_id = %run_id, %audit_error, "Failed to record dispatch failure");
                    }
                    Err(error)
                }
            }
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to dispatch job");
                ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
            })?;

        return Ok(Json(InvokeBoardResponse {
            run_id,
            status: response.status,
            message: Some(format!("Job dispatched via {} backend", response.backend)),
            poll_token: None,
        })
        .into_response());
    }

    // Determine the streaming dispatch method based on backend configuration
    let backend = state.dispatcher.backend();
    tracing::info!(run_id = %run_id, ?backend, "Dispatching streaming execution");

    // Persist the run record BEFORE dispatch so infrastructure failures
    // (executor crashes, network drops, timeouts) leave a visible Pending
    // row that can be reconciled, rather than a silently lost workflow.
    crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
        .await
        .map_err(|e| {
            tracing::error!(run_id = %run_id, error = %e, "Failed to create run record");
            ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
        })?;
    crate::audit::record_execution_start(&state, &user, execution_audit).await;

    // Dispatch based on the configured backend
    match backend {
        ExecutionBackend::LambdaStream => {
            // Use Lambda SDK streaming
            let (_dispatch_response, byte_stream) = match state
                .dispatcher
                .dispatch_streaming(request)
                .await {
                Ok(response) => Ok(response),
                Err(error) => {
                    if let Err(audit_error) = crate::audit::record_execution_dispatch_failure(&state, &run_id, "dispatcher").await {
                        tracing::error!(run_id = %run_id, %audit_error, "Failed to record dispatch failure");
                    }
                    Err(error)
                }
            }
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to dispatch Lambda streaming job");
                    ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
                })?;

            tracing::info!(run_id = %run_id, "Got Lambda response, starting stream proxy");

            Ok(proxy_lambda_sse_response(
                byte_stream,
                run_id,
                Some(crate::audit::ExecutionAuditContext::from(&state)),
            )
            .into_response())
        }
        _ => {
            // Use HTTP SSE for all other backends (Http, etc.)
            let (_dispatch_response, executor_response) = match state
                .dispatcher
                .dispatch_http_sse(request)
                .await {
                Ok(response) => Ok(response),
                Err(error) => {
                    if let Err(audit_error) = crate::audit::record_execution_dispatch_failure(&state, &run_id, "dispatcher").await {
                        tracing::error!(run_id = %run_id, %audit_error, "Failed to record dispatch failure");
                    }
                    Err(error)
                }
            }
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to dispatch HTTP SSE job");
                    ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
                })?;

            tracing::info!(run_id = %run_id, "Got executor response, starting stream proxy");

            Ok(proxy_sse_response(
                executor_response,
                run_id,
                Some(crate::audit::ExecutionAuditContext::from(&state)),
            )
            .into_response())
        }
    }
}

/// Create an SSE stream from a Lambda ByteStream response
fn proxy_lambda_sse_response(
    stream: ByteStream,
    run_id: String,
    db: Option<crate::audit::ExecutionAuditContext>,
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
                                            .and_then(|s| s.as_str());

                                        let run_status = completed_run_status(status);

                                        if let Err(e) = update_run_on_completion(db, &run_id, run_status, log_level).await {
                                            tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                                        }
                                    }

                        let sse_event = Event::default()
                            .event(&event.event_type)
                            .data(event.data);
                        yield Ok(sse_event);
                    }
                }
                Err(e) => {
                    tracing::warn!(run_id = %run_id, error = %e, "Lambda stream error");
                    if let Some(context) = &db
                        && let Err(error) = update_run_on_completion(context, &run_id, RunStatus::Failed, 0).await {
                            tracing::error!(run_id = %run_id, %error, "Failed to record Lambda stream failure");
                        }
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

/// Extract the next complete SSE event carrying data from the buffer, if
/// available. Data-less frames (comment keep-alives such as `:ping`) are
/// consumed and skipped rather than ending the caller's drain loop — returning
/// `None` for them would strand every complete frame still behind them.
fn extract_sse_event(buffer: &mut Vec<u8>) -> Option<ParsedSseEvent> {
    loop {
        // Find the double newline that terminates a complete SSE frame by scanning
        // raw bytes. Decoding the whole (partial) buffer here would replace the lead
        // bytes of a multi-byte codepoint straddling a chunk boundary with U+FFFD
        // and persist that corruption back into the tail.
        let end_pos = buffer.windows(2).position(|window| window == b"\n\n")?;

        // Split the complete frame off the buffer, keeping the raw undecoded tail.
        let tail = buffer.split_off(end_pos + 2);
        let frame = std::mem::replace(buffer, tail);

        // Only the complete frame is decoded — it is a whole, valid UTF-8 region.
        let event_str = String::from_utf8_lossy(&frame[..end_pos]);

        let mut event_type = "message".to_string();
        let mut data_parts = Vec::new();

        for line in event_str.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event_type = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data_parts.push(value.trim_start().to_string());
            }
        }

        if !data_parts.is_empty() {
            return Some(ParsedSseEvent {
                event_type,
                data: data_parts.join("\n"),
            });
        }
    }
}

//! Async event execution endpoint
//!
//! This endpoint triggers async execution of an event workflow via queue.
//! The job is dispatched to the configured queue backend (Redis, SQS, Kafka)
//! and returns immediately with a run_id for tracking.
//!
//! Flow:
//! 1. Check user access permissions
//! 2. Look up the event to get the associated board
//! 3. Create a run record in the database
//! 4. Create scoped credentials based on user permissions
//! 5. Dispatch to queue (Redis/SQS/Kafka based on EXECUTION_BACKEND env)
//! 6. Return run_id and poll_token for tracking progress

use crate::{
    ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        DispatchRequest, ExecutionJwtParams, TokenType, fetch_profile_for_dispatch,
        is_jwt_configured, payload_storage, resolve_wasm_packages, sign_execution_jwt,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::db::get_event_from_db;

/// Request body for async event invocation
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct InvokeEventAsyncRequest {
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

/// Response from async event invocation
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct InvokeEventAsyncResponse {
    /// Unique run ID (use this to track progress)
    pub run_id: String,
    /// Current status
    pub status: String,
    /// User JWT for long polling (use in Authorization header)
    pub poll_token: String,
    /// Backend used for dispatch
    pub backend: String,
}

/// Get credentials access for remote server-side execution.
fn get_credentials_access() -> crate::credentials::CredentialsAccess {
    crate::credentials::CredentialsAccess::ServerExecute
}

/// POST /apps/{app_id}/events/{event_id}/invoke/async
///
/// Invoke async execution of an event workflow via queue.
/// Uses EXECUTION_BACKEND env var to determine queue (redis, sqs, kafka).
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/invoke/async",
    tag = "events",
    description = "Invoke an event asynchronously via queue.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = InvokeEventAsyncRequest,
    responses(
        (status = 200, description = "Async invocation result", body = InvokeEventAsyncResponse),
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
    name = "POST /apps/{app_id}/events/{event_id}/invoke/async",
    skip(state, user, params)
)]
pub async fn invoke_event_async(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(params): Json<InvokeEventAsyncRequest>,
) -> Result<Json<InvokeEventAsyncResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
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

    // Get the event from database (validates event belongs to this app)
    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    let board_id = event.board_id.clone();
    let event_json =
        serde_json::to_string(&event).map_err(|e| anyhow!("Failed to serialize event: {}", e))?;

    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing EXECUTION_KEY/EXECUTION_PUB env vars)"
        )));
    }

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

    // Store payload in object storage if present (enables re-run)
    let input_payload_key = if let Some(ref payload) = params.payload {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize payload: {}", e)))?;
        let master_creds = state.master_credentials().await.map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to get master credentials: {}", e))
        })?;
        let store = master_creds
            .to_store(false)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to get object store: {}", e)))?;
        let stored =
            payload_storage::store_payload(store.as_generic(), &app_id, &run_id, &payload_bytes)
                .await
                .map_err(|e| ApiError::internal_error(anyhow!("Failed to store payload: {}", e)))?;
        Some(stored.key)
    } else {
        None
    };

    // Async always uses queue mode
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(params.version.clone()),
        event_id: Set(Some(event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Queue),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(input_payload_key),
        output_payload_len: Set(0),
        log_level: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(Some(sub.clone())),
        technical_user_id: Set(technical_user_id.clone()),
        caller_app_chain: Set(caller_app_chain.clone()),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };
    let execution_audit = crate::audit::ExecutionAudit {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        node_id: Some(event.id.clone()),
        version: params.version.clone(),
        mode: RunMode::Queue,
        status: RunStatus::Pending,
        input_payload_len,
        technical_user_id: technical_user_id.clone(),
    };

    run.insert(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
    })?;
    crate::audit::record_execution_start(&state, &user, execution_audit).await;

    let poll_token = sign_execution_jwt(ExecutionJwtParams {
        user_id: sub.clone(),
        technical_user_id: technical_user_id.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        app_chain: caller_app_chain.clone(),
        callback_url: String::new(),
        token_type: TokenType::User,
        ttl_seconds: Some(60 * 60),
    })
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to sign user JWT");
        ApiError::internal_error(anyhow!("Failed to sign user JWT: {}", e))
    })?;

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
        technical_user_id: technical_user_id.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        app_chain: caller_app_chain.clone(),
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

    let wasm_packages = resolve_wasm_packages(&state, &app_id).await;

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
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: params.runtime_variables,
        user_context: Some(permission.to_user_context()),
        profile,
        wasm_packages,
    };

    let response = state
        .dispatcher
        .dispatch_async(request)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to dispatch job to queue");
            ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
        })?;

    Ok(Json(InvokeEventAsyncResponse {
        run_id,
        status: response.status,
        poll_token,
        backend: response.backend,
    }))
}

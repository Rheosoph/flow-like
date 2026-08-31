//! Async board execution endpoint
//!
//! This endpoint triggers async execution of a board workflow via queue.
//! The job is dispatched to the configured queue backend (Redis, SQS, Kafka)
//! and returns immediately with a run_id for tracking.
//!
//! Flow:
//! 1. Check user access permissions
//! 2. Create a run record in the database
//! 3. Create scoped credentials based on user permissions
//! 4. Dispatch to queue (Redis/SQS/Kafka based on EXECUTION_BACKEND env)
//! 5. Return run_id and poll_token for tracking progress

use crate::{
    ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        DispatchRequest, DispatchTrigger, ExecutionJwtParams, TokenType,
        fetch_profile_for_dispatch, is_jwt_configured, payload_storage, resolve_wasm_packages,
        sign_execution_jwt,
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

/// Request body for async board invocation
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct InvokeBoardAsyncRequest {
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
    /// Runtime-configured variables to override board variables
    #[schema(value_type = Option<Object>)]
    pub runtime_variables:
        Option<std::collections::HashMap<String, flow_like::flow::variable::Variable>>,
    /// Optional profile ID to select a specific user profile for execution
    pub profile_id: Option<String>,
}

/// Response from async board invocation
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct InvokeBoardAsyncResponse {
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

/// POST /apps/{app_id}/board/{board_id}/invoke/async
///
/// Invoke async execution of a board workflow via queue.
/// Uses EXECUTION_BACKEND env var to determine queue (redis, sqs, kafka).
#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/invoke/async",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = InvokeBoardAsyncRequest,
    responses(
        (status = 200, description = "Async invocation started", body = InvokeBoardAsyncResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 500, description = "JWT signing not configured")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/invoke/async",
    skip(state, user, params)
)]
pub async fn invoke_board_async(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<InvokeBoardAsyncRequest>,
) -> Result<Json<InvokeBoardAsyncResponse>, ApiError> {
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

    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
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

    // Inherit the trace root & business keys from the caller, else this is root.
    let mut correlation = inherited_correlation.unwrap_or_default();
    if correlation.trace_id.is_none() {
        correlation.trace_id = parent_run_id.clone().or_else(|| Some(run_id.clone()));
    }
    let correlation_keys = correlation.keys_json();

    // Async always uses queue mode
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(params
            .version
            .map(|(maj, min, pat)| format!("{}.{}.{}", maj, min, pat))),
        event_id: Set(None),
        node_id: Set(Some(params.node_id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Queue),
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
        caller_app_chain: Set(caller_app_chain.clone()),
        trace_id: Set(correlation.trace_id.clone()),
        parent_run_id: Set(parent_run_id.clone()),
        correlation_keys: Set(correlation_keys.clone()),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };
    let execution_audit = crate::audit::ExecutionAudit {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: None,
        node_id: Some(params.node_id.clone()),
        version: params
            .version
            .map(|(maj, min, pat)| format!("{}.{}.{}", maj, min, pat)),
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
        event_id: None,
        app_chain: caller_app_chain.clone(),
        correlation: None,
        callback_url: String::new(),
        token_type: TokenType::User,
        ttl_seconds: Some(60 * 60),
    })
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to sign user JWT");
        ApiError::internal_error(anyhow!("Failed to sign user JWT: {}", e))
    })?;

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
        node_id: params.node_id.clone(),
        event_json: None,
        payload: params.payload,
        user_id: sub,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token: params.token,
        oauth_tokens: params.oauth_tokens,
        stream_state: true,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Async),
        runtime_variables: params.runtime_variables,
        user_context: Some(permission.to_user_context()),
        profile,
        wasm_packages,
        channel: None,
        trigger: DispatchTrigger::User,
    };

    let response = state
        .dispatcher
        .dispatch_async(request)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to dispatch job to queue");
            ApiError::internal_error(anyhow!("Failed to dispatch job: {}", e))
        })?;

    Ok(Json(InvokeBoardAsyncResponse {
        run_id,
        status: response.status,
        poll_token,
        backend: response.backend,
    }))
}

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
    ensure_fresh_permission, ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        ByteStream, DispatchError, DispatchRequest, DispatchTrigger, ExecutionBackend,
        ExecutionJwtParams, PageActionSealingContext, PageExecutionJwtContext, TokenType,
        completed_run_status, fetch_profile_for_dispatch, is_jwt_configured, payload_storage,
        proxy_sse_response_with_page_actions, rejection, resolve_wasm_packages, sign_execution_jwt,
        sign_execution_jwt_with_page_context,
        state::{PostgresStateStore, RunStatus as StateRunStatus, UpdateRunInput},
        update_run_on_completion,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::execution::progress::get_state_store,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::db::get_event_from_db;

fn sync_dispatch_failure_message(error: &DispatchError) -> String {
    match error {
        DispatchError::Artifact(_) => format!("Failed exact artifact preflight: {error}"),
        _ => format!("Failed to dispatch job: {error}"),
    }
}

fn sql_dispatch_failure_update(
    run_id: &str,
    app_id: &str,
    now: sea_orm::prelude::DateTime,
    error_message: String,
) -> sea_orm::UpdateMany<execution_run::Entity> {
    execution_run::Entity::update_many()
        .set(execution_run::ActiveModel {
            status: Set(RunStatus::Failed),
            completed_at: Set(Some(now)),
            updated_at: Set(now),
            error_message: Set(Some(error_message)),
            ..Default::default()
        })
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(execution_run::Column::AppId.eq(app_id))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
}

/// Terminalize the just-inserted run row after any sync dispatch failure.
/// Mirrors `invoke_event_async::mark_async_dispatch_failure`: app-scoped,
/// terminal-monotonic, with the SQL fallback when the state backend is down.
async fn mark_sync_dispatch_failure(
    state: &AppState,
    run_id: &str,
    app_id: &str,
    error: &DispatchError,
) {
    let now = chrono::Utc::now();
    let error_message = sync_dispatch_failure_message(error);
    let update = UpdateRunInput {
        status: Some(StateRunStatus::Failed),
        error_message: Some(error_message.clone()),
        completed_at: Some(now.timestamp_millis()),
        ..Default::default()
    };

    match get_state_store(state).await {
        Ok(store) => {
            let terminal = match store.get_run_for_app(run_id, app_id).await {
                Ok(Some(run)) if run.status.is_terminal() => Ok(run),
                Ok(Some(_)) => store.update_run(run_id, update).await,
                Ok(None) => Err(crate::execution::StateStoreError::NotFound),
                Err(error) => Err(error),
            };
            match terminal {
                Ok(_) if store.backend_name() == "postgres" => return,
                Ok(run) => {
                    match PostgresStateStore::new(std::sync::Arc::new(state.db.clone()))
                        .mirror_run_update(&run)
                        .await
                    {
                        Ok(()) => return,
                        Err(error) => tracing::error!(
                            run_id,
                            app_id,
                            error = %error,
                            "Failed to mirror the terminal dispatch failure into SQL"
                        ),
                    }
                }
                Err(error) => tracing::error!(
                    run_id,
                    app_id,
                    backend = store.backend_name(),
                    error = %error,
                    "Failed to terminalize the shared execution state after dispatch failure"
                ),
            }
        }
        Err(error) => tracing::error!(
            run_id,
            app_id,
            error = %error,
            "Failed to open the execution state store after dispatch failure"
        ),
    }

    // The state backend may be unavailable. Preserve any terminal winner and
    // scope the fallback to this app instead of overwriting a completed run.
    if let Err(update_error) =
        sql_dispatch_failure_update(run_id, app_id, now.naive_utc(), error_message)
            .exec(&state.db)
            .await
    {
        tracing::error!(
            run_id,
            app_id,
            error = %update_error,
            "Failed to mark the SQL run as failed after dispatch error"
        );
    }
}

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
    /// Business/object correlation keys (e.g. `{"order_id": "1234"}`) tagging
    /// the process case this run belongs to. Used for process mining.
    #[serde(default)]
    pub correlation: Option<std::collections::HashMap<String, String>>,
    /// Governed Page action or lifecycle trigger. This capability is request
    /// data only; normal user authentication remains in the Authorization
    /// header.
    #[serde(default)]
    pub page_trigger: Option<super::page_trigger::PageTrigger>,
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

/// Get credentials access for remote server-side execution.
fn get_credentials_access() -> crate::credentials::CredentialsAccess {
    crate::credentials::CredentialsAccess::ServerExecute
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
    skip(state, user, query, params)
)]
pub async fn invoke_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<InvokeEventQuery>,
    Json(params): Json<InvokeEventRequest>,
) -> Result<Response, ApiError> {
    invoke_event_impl(state, user, app_id, event_id, query, params, None, false).await
}

/// Invokes an event that has already been resolved through another governed
/// contract (for example an ontology action). This is deliberately crate-only:
/// public callers must not use it to bypass the generic connected-app event
/// policy with an arbitrary board or node target.
pub(crate) async fn invoke_resolved_event(
    state: AppState,
    user: AppUser,
    app_id: String,
    event: flow_like::flow::event::Event,
    query: InvokeEventQuery,
    params: InvokeEventRequest,
) -> Result<Response, ApiError> {
    let event_id = event.id.clone();
    invoke_event_impl(
        state,
        user,
        app_id,
        event_id,
        query,
        params,
        Some(event),
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn invoke_event_impl(
    state: AppState,
    user: AppUser,
    app_id: String,
    event_id: String,
    query: InvokeEventQuery,
    params: InvokeEventRequest,
    resolved_event: Option<flow_like::flow::event::Event>,
    governed_connected_app_call: bool,
) -> Result<Response, ApiError> {
    let permission = if params.page_trigger.is_some() {
        ensure_fresh_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents)
    } else {
        ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents)
    };
    let sub = permission.effective_user_id().map_err(|_| {
        crate::error::ApiError::forbidden(
            "Invoking requires a caller that is linked to a user account",
        )
    })?;
    let technical_user_id = permission.technical_user_id().map(ToOwned::to_owned);
    // If invoked through an app connection, keep the caller app chain so the
    // run (and any tokens it mints) stays attributable across apps.
    let caller_app_chain = match &user {
        AppUser::ConnectedApp(connected) => Some(connected.app_chain.clone()),
        _ => None,
    };
    // Process-mining correlation: the caller's run id is the parent; a run with
    // no parent is a root of its causal tree and owns the trace id.
    let parent_run_id = match &user {
        AppUser::ConnectedApp(connected) => connected.run_id.clone(),
        _ => None,
    };
    let inherited_correlation = match &user {
        AppUser::ConnectedApp(connected) => connected.correlation.clone(),
        _ => None,
    };

    // A pre-resolved event is accepted only from another crate-private,
    // governed contract. Generic event invocation always loads from the DB and
    // applies the direct connected-app surface policy.
    let event = match resolved_event {
        Some(event) => event,
        None => get_event_from_db(&state.db, &event_id, &app_id).await?,
    };
    if !governed_connected_app_call {
        super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;
    }
    let resolved_page_trigger = match (event.default_page_id.as_ref(), params.page_trigger.as_ref())
    {
        (Some(_), Some(trigger)) => Some(
            super::page_trigger::resolve_page_trigger(
                &state,
                &permission,
                &app_id,
                &event,
                trigger,
            )
            .await?,
        ),
        (Some(_), None) => {
            return Err(ApiError::bad_request(
                "Invoking a Page Event requires page_trigger",
            ));
        }
        (None, Some(_)) => {
            return Err(ApiError::bad_request(
                "page_trigger is valid only for an Event that owns a Page",
            ));
        }
        (None, None) => None,
    };
    let board_id = resolved_page_trigger
        .as_ref()
        .map(|resolved| resolved.board_id.clone())
        .unwrap_or_else(|| event.board_id.clone());
    let board_version = resolved_page_trigger
        .as_ref()
        .map(|resolved| resolved.board_version)
        .unwrap_or(event.board_version);
    let board_etag = resolved_page_trigger
        .as_ref()
        .and_then(|resolved| resolved.board_etag.clone());
    let node_id = resolved_page_trigger
        .as_ref()
        .map(|resolved| resolved.node_id.clone())
        .unwrap_or_else(|| event.node_id.clone());
    let resolved_version = board_version
        .map(|(major, minor, patch)| format!("{major}_{minor}_{patch}"))
        .or_else(|| board_etag.as_ref().map(|etag| format!("etag:{etag}")));
    let event_json =
        serde_json::to_string(&event).map_err(|e| anyhow!("Failed to serialize event: {}", e))?;

    let wasm_packages = resolve_wasm_packages(&state, &app_id).await;
    let wasm_authority_revision =
        flow_like_types::dispatch::wasm_package_set_revision(wasm_packages.as_ref());
    if resolved_page_trigger
        .as_ref()
        .and_then(|resolved| resolved.wasm_authority_revision.as_deref())
        .is_some_and(|expected| expected != wasm_authority_revision)
    {
        return Err(ApiError::bad_request(
            "The Page action WASM package set changed; reload the Page action",
        ));
    }
    let run_id = create_id();
    let page_action_sealing = resolved_page_trigger.as_ref().map(|resolved| {
        std::sync::Arc::new(PageActionSealingContext {
            sub: sub.clone(),
            technical_user_id: technical_user_id.clone(),
            source_app_id: app_id.clone(),
            source_event_id: event_id.clone(),
            source_page_id: resolved.page_id.clone(),
            source_manifest_revision: resolved.manifest_revision.clone(),
            target_app_id: app_id.clone(),
            target_board_id: resolved.board_id.clone(),
            target_board_version: resolved.board_version,
            target_board_etag: resolved.board_etag.clone(),
            wasm_authority_revision: Some(wasm_authority_revision.clone()),
            origin_run_id: run_id.clone(),
            allowed_entry_nodes: resolved.entry_node_ids.clone(),
        })
    });
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

    // Resolve this run's correlation: inherit the trace root & business keys
    // from the caller, otherwise this run is the root of its own trace.
    let mut correlation = inherited_correlation.unwrap_or_default();
    if correlation.trace_id.is_none() {
        correlation.trace_id = parent_run_id.clone().or_else(|| Some(run_id.clone()));
    }
    // Auto-extract business keys via the event's correlation mappings, then
    // let explicitly passed keys win on conflict.
    if let (Some(mappings), Some(payload)) = (
        event
            .correlation_mappings
            .as_ref()
            .filter(|mappings| !mappings.is_empty()),
        params.payload.as_ref(),
    ) {
        let extracted = crate::correlation::extract_mapped_keys(payload, mappings);
        if !extracted.is_empty() {
            correlation = correlation.with_keys(&extracted);
        }
    }
    if let Some(keys) = params.correlation.as_ref().filter(|keys| !keys.is_empty()) {
        crate::correlation::validate_business_keys(keys).map_err(ApiError::bad_request)?;
        correlation = correlation.with_keys(keys);
    }
    let correlation_keys = correlation.keys_json();

    // Build run record (insert happens later - sync for local/isolated, parallel for HTTP)
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(resolved_version.clone()),
        event_id: Set(Some(event_id.clone())),
        node_id: Set(Some(node_id.clone())),
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
        event_id: Some(event_id.clone()),
        node_id: Some(node_id.clone()),
        version: resolved_version.clone(),
        board_etag: board_etag.clone(),
        mode: run_mode.clone(),
        status: RunStatus::Pending,
        input_payload_len,
        technical_user_id: technical_user_id.clone(),
    };

    // For local mode, insert synchronously and return JSON - no dispatch needed
    if query.local {
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
            event_id: Some(event_id),
            app_chain: caller_app_chain.clone(),
            correlation: None,
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

    // Remote dispatch always runs the event's configured board version. A
    // request asking for a different version cannot be honored here (there is no
    // validation against the app's available board versions), so reject it
    // rather than silently executing a different version than the caller asked
    // for. A malformed version string is likewise a bad request.
    if let Some(requested) = params.version.as_deref() {
        let reason = match super::parse_version_tuple(requested) {
            Some(parsed) if board_version == Some(parsed) => None,
            Some(_) => Some(
                "Executing a board version other than the event's configured version is not supported"
                    .to_string(),
            ),
            None => Some(format!(
                "Invalid version '{requested}': expected MAJOR_MINOR_PATCH"
            )),
        };

        if let Some(reason) = reason {
            let context = rejection::RejectedRunContext::new(
                app_id.clone(),
                rejection::RejectionStage::Payload,
                reason.clone(),
            )
            .with_run_id(run_id.clone())
            .with_event_definition(&event)
            .with_mode(run_mode.clone())
            .with_actor(Some(sub.clone()), technical_user_id.clone())
            .with_credential_subject(sub.clone())
            .with_payload(params.payload.clone());
            rejection::record(&state, context).await;
            return Err(ApiError::bad_request(reason));
        }
    }

    // Check JWT signing is configured for remote execution
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
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

    let executor_jwt_params = ExecutionJwtParams {
        user_id: sub.clone(),
        technical_user_id: technical_user_id.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        app_chain: caller_app_chain.clone(),
        correlation: correlation.clone().into_option(),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    };
    let executor_jwt = if let Some(resolved) = &resolved_page_trigger {
        sign_execution_jwt_with_page_context(
            executor_jwt_params,
            PageExecutionJwtContext {
                page_id: resolved.page_id.clone(),
                manifest_revision: resolved.manifest_revision.clone(),
                board_version: resolved.board_version,
                board_etag: resolved.board_etag.clone(),
                target_node_id: Some(resolved.node_id.clone()),
                entry_authority_revision: resolved.entry_authority_revision.clone(),
                wasm_authority_revision: Some(wasm_authority_revision.clone()),
                allowed_entry_node_ids: if resolved.entry_authority_revision.is_none() {
                    let mut ids = resolved.entry_node_ids.iter().cloned().collect::<Vec<_>>();
                    ids.sort();
                    ids
                } else {
                    Vec::new()
                },
            },
        )
    } else {
        sign_execution_jwt(executor_jwt_params)
    }
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to sign executor JWT");
        ApiError::internal_error(anyhow!("Failed to sign executor JWT: {}", e))
    })?;

    let profile =
        fetch_profile_for_dispatch(&state, &sub, params.profile_id.as_deref(), &app_id, true).await;

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id,
        board_version,
        board_etag,
        node_id,
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
        channel: None,
        trigger: DispatchTrigger::User,
    };

    // For isolated K8s jobs, insert run record and dispatch async
    if query.isolated {
        run.insert(&state.db).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to create run record");
            ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
        })?;
        crate::audit::record_execution_start(&state, &user, execution_audit).await;

        let response = match state
            .dispatcher
            .dispatch_with_backend(ExecutionBackend::KubernetesJob, request)
            .await
        {
            Ok(response) => response,
            Err(e) => {
                mark_sync_dispatch_failure(&state, &run_id, &app_id, &e).await;
                tracing::error!(error = %e, "Failed to dispatch job");
                return Err(ApiError::internal_error(anyhow!(
                    "Failed to dispatch job: {}",
                    e
                )));
            }
        };

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

    // Persist the run record BEFORE dispatch so infrastructure failures
    // (executor crashes, network drops, timeouts) leave a visible Pending
    // row that can be reconciled, rather than a silently lost workflow.
    run.insert(&state.db).await.map_err(|e| {
        tracing::error!(run_id = %run_id, error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
    })?;
    crate::audit::record_execution_start(&state, &user, execution_audit).await;

    // Dispatch based on the configured backend
    match backend {
        ExecutionBackend::LambdaStream => {
            // Use Lambda SDK streaming
            let (_dispatch_response, byte_stream) =
                match state.dispatcher.dispatch_streaming(request).await {
                    Ok(response) => response,
                    Err(e) => {
                        mark_sync_dispatch_failure(&state, &run_id, &app_id, &e).await;
                        tracing::error!(error = %e, "Failed to dispatch Lambda streaming job");
                        return Err(ApiError::internal_error(anyhow!(
                            "Failed to dispatch job: {}",
                            e
                        )));
                    }
                };

            tracing::info!(run_id = %run_id, "Got Lambda response, starting stream proxy");

            Ok(proxy_lambda_sse_response(
                byte_stream,
                run_id,
                Some(std::sync::Arc::new(state.db.clone())),
                page_action_sealing,
            )
            .into_response())
        }
        _ => {
            // Use HTTP SSE for all other backends (Http, etc.)
            let (_dispatch_response, executor_response) =
                match state.dispatcher.dispatch_http_sse(request).await {
                    Ok(response) => response,
                    Err(e) => {
                        mark_sync_dispatch_failure(&state, &run_id, &app_id, &e).await;
                        tracing::error!(error = %e, "Failed to dispatch HTTP SSE job");
                        return Err(ApiError::internal_error(anyhow!(
                            "Failed to dispatch job: {}",
                            e
                        )));
                    }
                };

            tracing::info!(run_id = %run_id, "Got executor response, starting stream proxy");

            Ok(proxy_sse_response_with_page_actions(
                executor_response,
                run_id,
                Some(std::sync::Arc::new(state.db.clone())),
                page_action_sealing,
            )
            .into_response())
        }
    }
}

fn seal_lambda_page_actions(
    data: &str,
    run_id: &str,
    event_ordinal: u64,
    context: Option<&PageActionSealingContext>,
) -> String {
    let Some(context) = context else {
        return data.to_string();
    };
    let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(data) else {
        return data.to_string();
    };
    let Some(envelope_object) = envelope.as_object_mut() else {
        return data.to_string();
    };
    let message_id = ["event_id", "eventId", "id"]
        .iter()
        .find_map(|key| {
            envelope_object
                .get(*key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| format!("{run_id}:lambda:{event_ordinal}"));
    let event_type = envelope_object
        .get("event_type")
        .or_else(|| envelope_object.get("eventType"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let report = envelope_object
        .get_mut("payload")
        .map(|payload| context.seal_payload(&event_type, &message_id, payload))
        .unwrap_or_default();
    if report.rejected > 0 {
        tracing::warn!(
            run_id,
            message_id,
            rejected = report.rejected,
            "stripped rejected dynamic Page action targets from Lambda output"
        );
    }
    serde_json::to_string(&envelope).unwrap_or_else(|_| data.to_string())
}

/// Create an SSE stream from a Lambda ByteStream response
fn proxy_lambda_sse_response(
    stream: ByteStream,
    run_id: String,
    db: Option<std::sync::Arc<sea_orm::DatabaseConnection>>,
    page_actions: Option<std::sync::Arc<PageActionSealingContext>>,
) -> axum::response::sse::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::StreamExt;
    use std::time::Duration;

    let stream = async_stream::stream! {
        let mut byte_stream = stream;
        let mut buffer = Vec::new();
        let mut event_ordinal = 0_u64;

        while let Some(result) = byte_stream.next().await {
            match result {
                Ok(bytes) => {
                    // Append bytes to buffer
                    buffer.extend_from_slice(&bytes);

                    // Try to parse complete SSE events from buffer
                    while let Some(event) = extract_sse_event(&mut buffer) {
                        let event_data = seal_lambda_page_actions(
                            &event.data,
                            &run_id,
                            event_ordinal,
                            page_actions.as_deref(),
                        );
                        event_ordinal = event_ordinal.saturating_add(1);
                        // Check if this is a completed event and update the database
                        if let Some(db) = &db
                            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event_data)
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

                                        if let Err(e) = update_run_on_completion(db.as_ref(), &run_id, run_status, log_level).await {
                                            tracing::error!(run_id = %run_id, error = %e, "Failed to update run on completion");
                                        }
                                    }

                        let sse_event = Event::default()
                            .event(&event.event_type)
                            .data(event_data);
                        yield Ok(sse_event);
                    }
                }
                Err(e) => {
                    tracing::warn!(run_id = %run_id, error = %e, "Lambda stream error");
                    let error_event = Event::default()
                        .event("error")
                        .data(flow_like_types::json::json!({ "error": e.to_string() }).to_string());
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
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn sync_dispatch_failure_keeps_artifact_specific_messaging() {
        let artifact = DispatchError::Artifact("missing artifact".into());
        assert_eq!(
            sync_dispatch_failure_message(&artifact),
            "Failed exact artifact preflight: Compiled artifact error: missing artifact"
        );

        let network = DispatchError::Network("connection refused".into());
        assert_eq!(
            sync_dispatch_failure_message(&network),
            "Failed to dispatch job: Network error: connection refused"
        );
    }

    #[test]
    fn sync_dispatch_failure_fallback_is_app_scoped_and_terminal_monotonic() {
        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .naive_utc();
        let statement =
            sql_dispatch_failure_update("run-1", "app-1", now, "Failed to dispatch job".into())
                .build(DatabaseBackend::Postgres)
                .to_string();

        assert!(statement.contains("\"ExecutionRun\".\"id\" = 'run-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"appId\" = 'app-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"status\" IN"));
        assert!(statement.contains("'PENDING'"));
        assert!(statement.contains("'RUNNING'"));
    }
}

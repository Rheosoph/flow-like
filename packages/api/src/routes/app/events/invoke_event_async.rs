//! Async event execution endpoint
//!
//! This endpoint triggers async execution of an event workflow via queue.
//! The job is dispatched to the configured queue backend (Redis, SQS, Kafka)
//! and returns immediately with a run_id for tracking.
//!
//! Flow:
//! 1. Check user access permissions
//! 2. Look up the event to get the associated board
//! 3. Prepare scoped credentials and the executor request
//! 4. Create a run record in the database
//! 5. Dispatch to queue (Redis/SQS/Kafka based on EXECUTION_BACKEND env)
//! 6. Return run_id and poll_token for tracking progress

use crate::{
    ensure_fresh_permission, ensure_permission,
    entity::{
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus, RunVariant},
    },
    error::ApiError,
    execution::{
        DispatchRequest, DispatchTrigger, ExecutionJwtParams, PageExecutionJwtContext, TokenType,
        fetch_profile_for_dispatch, format_run_version, is_jwt_configured, payload_storage,
        rejection, resolve_wasm_packages, sign_execution_jwt, sign_execution_jwt_with_page_context,
        state::{PostgresStateStore, RunStatus as StateRunStatus, UpdateRunInput},
        variant,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::execution::progress::get_state_store,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::db::get_event_from_db;

fn poll_token_ttl_seconds(is_governed_page: bool) -> i64 {
    if is_governed_page {
        24 * 60 * 60
    } else {
        60 * 60
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

async fn mark_async_dispatch_failure(
    state: &AppState,
    run_id: &str,
    app_id: &str,
    error: &crate::execution::DispatchError,
) {
    let now = chrono::Utc::now();
    let error_message = format!("Failed to dispatch job: {error}");
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
                    match PostgresStateStore::new(Arc::new(state.db.clone()))
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

/// Query parameters for async event invocation
#[derive(Clone, Debug, Deserialize, Default, ToSchema)]
pub struct InvokeEventAsyncQuery {
    /// Pin execution to a named live variant instead of the weighted split
    #[serde(default, rename = "__variant")]
    pub variant: Option<String>,
}

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
    /// Business/object correlation keys tagging the process case this run
    /// belongs to (e.g. `{"order_id": "1234"}`).
    #[serde(default)]
    pub correlation: Option<std::collections::HashMap<String, String>>,
    /// Governed Page action or lifecycle trigger. This capability is request
    /// data only; normal user authentication remains in the Authorization
    /// header.
    #[serde(default)]
    pub page_trigger: Option<super::page_trigger::PageTrigger>,
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
        ("event_id" = String, Path, description = "Event ID"),
        ("__variant" = Option<String>, Query, description = "Pin execution to a named live variant"),
        ("x-flow-like-variant" = Option<String>, Header, description = "Pin execution to a named live variant")
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
    skip(state, user, query, headers, params)
)]
pub async fn invoke_event_async(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<InvokeEventAsyncQuery>,
    headers: HeaderMap,
    Json(params): Json<InvokeEventAsyncRequest>,
) -> Result<Json<InvokeEventAsyncResponse>, ApiError> {
    let variant_pin = variant::pin_from_request(&headers, query.variant.clone());
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

    // Get the event from database (validates event belongs to this app)
    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;

    let resolved_page_trigger = match (event.default_page_id.as_ref(), params.page_trigger.as_ref())
    {
        (Some(_), Some(trigger)) => Some(
            super::page_trigger::resolve_page_trigger(
                &state,
                &permission,
                &app_id,
                &event,
                trigger,
                variant_pin.as_deref(),
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
    let run_id = create_id();
    // Live-variant split, resolved before the run row and the executor JWT so
    // both carry the same board as the dispatch payload. A page trigger already
    // bound its target (the bootstrap-served variant or the primary, WP6b) and
    // is carried through as-is.
    let variant_target = match &resolved_page_trigger {
        Some(resolved) => Some(resolved.target.clone()),
        None => Some(
            variant::resolve_invoke_target(
                &event,
                variant_pin.as_deref(),
                params
                    .version
                    .as_deref()
                    .and_then(super::parse_version_tuple),
                &variant::SplitKeyRequest {
                    pinned_variant: None,
                    idempotency_key: None,
                    parent_run_id: parent_run_id.as_deref(),
                    trace_id: inherited_correlation
                        .as_ref()
                        .and_then(|correlation| correlation.trace_id.as_deref()),
                    caller_subject: Some(&sub),
                    run_id: &run_id,
                },
            )
            .map_err(ApiError::bad_request)?,
        ),
    };
    let variant_name = variant_target
        .as_ref()
        .and_then(|target| target.variant_name.clone());
    let (board_id, board_version, node_id) = match (&resolved_page_trigger, &variant_target) {
        (Some(resolved), _) => (
            resolved.board_id.clone(),
            resolved.board_version,
            resolved.node_id.clone(),
        ),
        (None, Some(target)) => (
            target.board_id.clone(),
            target.board_version,
            target.node_id.clone(),
        ),
        (None, None) => (
            event.board_id.clone(),
            event.board_version,
            event.node_id.clone(),
        ),
    };
    let board_etag = resolved_page_trigger
        .as_ref()
        .and_then(|resolved| resolved.board_etag.clone());
    let resolved_version = board_version
        .map(format_run_version)
        .or_else(|| board_etag.as_ref().map(|etag| format!("etag:{etag}")));

    // Async dispatch runs the resolved target's configured board version: a
    // request version naming the primary's (or a live variant's pinned)
    // version already selected that target above, so anything else cannot be
    // honored here (there is no validation against the app's available board
    // versions) and is rejected rather than silently executing a different
    // version than the caller asked for. A malformed version string is
    // likewise a bad request.
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
            .with_event_definition(&event)
            .with_mode(RunMode::Queue)
            .with_actor(Some(sub.clone()), technical_user_id.clone())
            .with_credential_subject(sub.clone())
            .with_payload(params.payload.clone());
            rejection::record(&state, context).await;
            return Err(ApiError::bad_request(reason));
        }
    }

    let event_json = match &variant_target {
        Some(target) => variant::dispatch_event_json(&event, target),
        None => serde_json::to_string(&event),
    }
    .map_err(|e| anyhow!("Failed to serialize event: {}", e))?;

    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }

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

    // Resolve this run's correlation (inherit trace root & keys, else self).
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

    // Async always uses queue mode
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(resolved_version.clone()),
        event_id: Set(Some(event_id.clone())),
        node_id: Set(Some(node_id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Queue),
        run_variant: Set(variant_target
            .as_ref()
            .map_or(RunVariant::Primary, |target| target.run_variant())),
        variant_name: Set(variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
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
        caller_app_chain: Set(caller_app_chain.clone().map(Into::into)),
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
        version: resolved_version,
        board_etag: board_etag.clone(),
        mode: RunMode::Queue,
        status: RunStatus::Pending,
        input_payload_len,
        technical_user_id: technical_user_id.clone(),
    };

    let poll_token = sign_execution_jwt(ExecutionJwtParams {
        user_id: sub.clone(),
        technical_user_id: technical_user_id.clone(),
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        app_chain: caller_app_chain.clone(),
        correlation: None,
        callback_url: String::new(),
        token_type: TokenType::User,
        ttl_seconds: Some(poll_token_ttl_seconds(resolved_page_trigger.is_some())),
        shadow: None,
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
        shadow: None,
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
        shadow: false,
        artifact: None,
    };

    // No executor can observe this run before dispatch. Insert only after all
    // fallible request preparation so an earlier failure cannot strand a
    // canonical Pending row.
    crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create run record");
            ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
        })?;
    crate::audit::record_execution_start(&state, &user, execution_audit).await;

    let response = match state.dispatcher.dispatch_async(request).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!(error = %e, "Failed to dispatch job to queue");
            mark_async_dispatch_failure(&state, &run_id, &app_id, &e).await;
            return Err(ApiError::internal_error(anyhow!(
                "Failed to dispatch job: {}",
                e
            )));
        }
    };

    Ok(Json(InvokeEventAsyncResponse {
        run_id,
        status: response.status,
        poll_token,
        backend: response.backend,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn stateless_lambda_dispatch_failure_fallback_is_app_scoped_and_terminal_monotonic() {
        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .naive_utc();
        let statement = sql_dispatch_failure_update(
            "run-1",
            "app-1",
            now,
            "Failed exact artifact preflight".into(),
        )
        .build(DatabaseBackend::Postgres)
        .to_string();

        assert!(statement.contains("\"ExecutionRun\".\"id\" = 'run-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"appId\" = 'app-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"status\" IN"));
        assert!(statement.contains("'PENDING'"));
        assert!(statement.contains("'RUNNING'"));
    }

    #[test]
    fn stateless_lambda_governed_page_poll_token_lives_as_long_as_its_async_run() {
        assert_eq!(poll_token_ttl_seconds(true), 24 * 60 * 60);
        assert_eq!(poll_token_ttl_seconds(false), 60 * 60);
    }
}

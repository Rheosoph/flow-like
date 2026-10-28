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
        event_sink, execution_run,
        sea_orm_active_enums::{RunMode, RunStatus, RunVariant},
    },
    error::ApiError,
    execution::{
        ByteStream, CompletionWrites, DispatchError, DispatchRequest, DispatchTrigger,
        ExecutionBackend, ExecutionJwtParams, PageActionSealingContext, PageExecutionJwtContext,
        TokenType, caller_dispatch_profile, fail_run_on_stream_error, format_run_version,
        is_jwt_configured, payload_storage, proxy_sse_response_with_page_actions, rejection,
        resolve_wasm_packages, sign_execution_jwt, sign_execution_jwt_with_page_context,
        sink_dispatch_profile,
        state::{PostgresStateStore, RunStatus as StateRunStatus, UpdateRunInput},
        stream_failure_event,
        subject::account_user_id,
        variant,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::execution::progress::get_state_store,
    routes::sink::trigger::{
        OAuthTokens, PinnedFire, SinkUse, pin_fire_for, recheck_fire_for, run_oauth_tokens,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use flow_like::flow::execution::UserExecutionContext;
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use utoipa::ToSchema;

use super::db::get_event_from_db;

macro_rules! invoke_span {
    ($name:literal) => {
        tracing::info_span!(target: "flow_like::observability", $name)
    };
}

fn sync_dispatch_failure_message(error: &DispatchError) -> String {
    match error {
        DispatchError::Artifact(_) => format!("Failed exact artifact preflight: {error}"),
        DispatchError::LocalOnly(_) => error.to_string(),
        _ => format!("Failed to dispatch job: {error}"),
    }
}

fn sql_dispatch_failure_update(
    run_id: &str,
    app_id: &str,
    now: sea_orm::prelude::DateTimeWithTimeZone,
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
                Ok(_) if store.backend_name() == "postgres" => {
                    if let Err(error) =
                        crate::audit::record_execution_outcome(state, run_id, "dispatcher").await
                    {
                        tracing::error!(run_id, %error, "Failed to audit dispatch outcome");
                    }
                    return;
                }
                Ok(run) => {
                    match PostgresStateStore::new(std::sync::Arc::new(state.db.clone()))
                        .mirror_run_update(&run)
                        .await
                    {
                        Ok(()) => {
                            if let Err(error) =
                                crate::audit::record_execution_outcome(state, run_id, "dispatcher")
                                    .await
                            {
                                tracing::error!(run_id, %error, "Failed to audit dispatch outcome");
                            }
                            return;
                        }
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
        sql_dispatch_failure_update(run_id, app_id, now.fixed_offset(), error_message)
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
    if let Err(error) = crate::audit::record_execution_outcome(state, run_id, "dispatcher").await {
        tracing::error!(run_id, %error, "Failed to audit dispatch outcome");
    }
}

/// A run rejected before its admission committed has no row yet. It gets one
/// first, so every dispatch failure stays visible as a failed run.
async fn record_dispatch_failure(
    state: &AppState,
    run_id: &str,
    app_id: &str,
    record: crate::quota::RunRecord,
    admitted: bool,
    error: &DispatchError,
) {
    if !admitted
        && let Err(insert_error) =
            crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, record.run).await
    {
        tracing::error!(run_id, error = %insert_error, "Failed to create the record of a rejected run");
    }
    mark_sync_dispatch_failure(state, run_id, app_id, error).await;
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
    /// Pin execution to a named live variant instead of the weighted split
    #[serde(default, rename = "__variant")]
    pub variant: Option<String>,
}

/// Request body for event invocation
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct InvokeEventRequest {
    /// Optional board version to execute (defaults to latest)
    pub version: Option<String>,
    /// Input payload for the execution
    pub payload: Option<serde_json::Value>,
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

/// Encrypts and stores a run's input payload so the run can be re-run, and
/// returns the key for the run row. `None` without a payload.
pub(super) async fn store_input_payload(
    state: &AppState,
    app_id: &str,
    run_id: &str,
    payload: Option<&serde_json::Value>,
) -> Result<Option<String>, ApiError> {
    let Some(payload) = payload else {
        return Ok(None);
    };
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize payload: {}", e)))?;
    let master_creds = state.master_credentials().await.map_err(|e| {
        ApiError::internal_error(anyhow!("Failed to get master credentials: {}", e))
    })?;
    let store = master_creds
        .to_store(false)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get object store: {}", e)))?;
    let stored = payload_storage::store_payload(store.as_generic(), app_id, run_id, &payload_bytes)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to store payload: {}", e)))?;
    Ok(Some(stored.key))
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
        ("isolated" = bool, Query, description = "Use isolated execution"),
        ("__variant" = Option<String>, Query, description = "Pin execution to a named live variant"),
        ("x-flow-like-variant" = Option<String>, Header, description = "Pin execution to a named live variant")
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
    skip(state, user, query, headers, params)
)]
pub async fn invoke_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(mut query): Query<InvokeEventQuery>,
    headers: HeaderMap,
    Json(params): Json<InvokeEventRequest>,
) -> Result<Response, ApiError> {
    query.variant = variant::pin_from_request(&headers, query.variant.take());
    invoke_event_impl(
        state, user, app_id, event_id, query, params, None, false, None,
    )
    .await
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
        None,
    )
    .await
}

/// Execute the exact public Event authorized by the hosted frontend route.
/// The visitor stays the run's identity and gains no app membership. A
/// signed-in visitor runs as themselves, like a member who invokes the Event in
/// the app: their profile and private Bits serve the run, it signs in with the
/// OAuth tokens the visitor sends and, for any other provider, with the Event's
/// saved sign-ins, and its callbacks hold the visitor's own standing in the app.
/// A pause because the Event's run-as user lost their standing does not stop
/// them. An anonymous visitor runs as the Event's run-as user, checked on every
/// visit as on every sink fire: that user's profile, private Bits and OAuth
/// grants serve the run, and its callbacks never exceed that user's role. A
/// run-as refused on an anonymous visit pauses the Event for every anonymous
/// visitor and answers 403, never 401.
pub(crate) async fn invoke_hosted_event(
    state: AppState,
    app_id: String,
    event: flow_like::flow::event::Event,
    caller: Option<AppUser>,
    anonymous_session_id: &str,
    query: InvokeEventQuery,
    mut params: InvokeEventRequest,
) -> Result<Response, ApiError> {
    let user = caller.unwrap_or(AppUser::Unauthorized);
    validate_hosted_invocation(&query, &params, &user)?;
    let subject = hosted_subject(&event.id, &user, anonymous_session_id)?;
    if let AppUser::OpenID(visitor) = &user {
        crate::routes::user::ensure_user_exists(&state, &visitor.sub).await?;
    }
    let hosted = hosted_invocation(&state, &app_id, &event.id, subject).await?;
    crate::routes::oauth::authorize_incoming_oauth_tokens(&state, &user, &mut params.oauth_tokens)
        .await?;
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
        Some(hosted),
    )
    .await
}

struct HostedInvocation {
    subject: String,
    /// The Event's sink as the visit read it. An anonymous visit read it when it checked the
    /// user the sink runs as; a signed-in visit borrows only its saved OAuth sign-ins.
    sink: Option<event_sink::Model>,
    profile_owner: Option<String>,
}

/// Whether a hosted visit by `subject` runs on the Event's sink credentials: the user the
/// sink runs as, that user's profile and private Bits, and the sink's OAuth grants. Only an
/// anonymous visit, whose subject is a pseudonym, does; a signed-in visitor runs as themselves.
fn uses_sink_credentials(subject: &str) -> bool {
    account_user_id(subject).is_none()
}

/// How a hosted visit by `subject` uses the Event's sink: an anonymous visit as its run-as
/// user, a signed-in visit as the visitor.
fn hosted_sink_use(subject: &str) -> SinkUse {
    if uses_sink_credentials(subject) {
        SinkUse::RunAs
    } else {
        SinkUse::Person
    }
}

/// What the Event lends a hosted visit. An anonymous visit runs as the checked run-as user of
/// the Event's sink: that user's profile and private Bits and the sink's OAuth grants, which
/// belong to the same user, serve the run once [`HostedInvocation::pin`] fixed what it runs. A
/// signed-in visit checks no run-as user and may only borrow the sink's saved OAuth sign-ins.
/// An anonymous visit needs the Event's sink, if it has one, to be on; a signed-in visit is
/// stopped only by a sink a person switched off, not by a release's pause.
async fn hosted_invocation(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    subject: String,
) -> Result<HostedInvocation, ApiError> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(&state.db)
        .await?;
    if sink
        .as_ref()
        .is_some_and(|sink| !hosted_sink_use(&subject).admits(sink))
    {
        return Err(ApiError::not_found("Event sink is inactive"));
    }
    let profile_owner = match sink.as_ref().filter(|_| uses_sink_credentials(&subject)) {
        Some(sink) => crate::routes::sink::trigger::resolve_sink_run_as(state, sink).await?,
        None => None,
    };
    Ok(HostedInvocation {
        subject,
        sink,
        profile_owner,
    })
}

/// A visit once what it runs is fixed: the draft a Latest target runs, the OAuth sign-ins it
/// takes, and for an anonymous visit the profile of the user it runs as.
struct PinnedVisit {
    board_etag: Option<String>,
    oauth_tokens: Option<OAuthTokens>,
    profile: Option<serde_json::Value>,
}

impl HostedInvocation {
    /// Fixes what the visit runs before it borrows anything from the Event's sink, as a sink
    /// fire does: a Latest target pins its draft, and a Page trigger fixed its board's ETag when
    /// it resolved. The sink read again must still run as the user the visit read, else the
    /// visit is refused as retryable. An anonymous visit then takes the sink's OAuth grants and
    /// the profile of the user it runs as. A signed-in visit takes the visitor's own sign-ins,
    /// `supplied`, and the saved grants of every other provider. `None` when the visit borrows
    /// nothing: an Event without a sink, or a signed-in visit of a sink without saved sign-ins.
    async fn pin(
        &self,
        state: &AppState,
        page_trigger: Option<&super::page_trigger::ResolvedPageTrigger>,
        target: &variant::ResolvedTarget,
        supplied: Option<&OAuthTokens>,
    ) -> Result<Option<PinnedVisit>, ApiError> {
        let sink_use = hosted_sink_use(&self.subject);
        let Some(sink) = self.sink.clone().filter(|sink| sink_use.borrows_from(sink)) else {
            return Ok(None);
        };
        let PinnedFire { sink, board_etag } = self.fix(state, sink, page_trigger, target).await?;
        let anonymous = uses_sink_credentials(&self.subject);
        let supplied = supplied.filter(|_| !anonymous).cloned();
        let oauth_tokens = run_oauth_tokens(state, Some(&sink), sink_use, supplied).await?;
        let profile = if anonymous {
            sink_dispatch_profile(state, &sink, self.profile_owner.as_deref()).await
        } else {
            None
        };
        Ok(Some(PinnedVisit {
            board_etag,
            oauth_tokens,
            profile,
        }))
    }

    /// The sink once the draft the visit runs is fixed, read again as the visit uses it.
    async fn fix(
        &self,
        state: &AppState,
        sink: event_sink::Model,
        page_trigger: Option<&super::page_trigger::ResolvedPageTrigger>,
        target: &variant::ResolvedTarget,
    ) -> Result<PinnedFire, ApiError> {
        let sink_use = hosted_sink_use(&self.subject);
        Ok(match page_trigger {
            None => pin_fire_for(state, sink, target, sink_use).await?,
            Some(resolved) => PinnedFire {
                sink: match resolved.board_etag {
                    Some(_) => recheck_fire_for(state, &sink, sink_use).await?,
                    None => sink,
                },
                board_etag: resolved.board_etag.clone(),
            },
        })
    }
}

pub(crate) fn hosted_subject(
    event_id: &str,
    caller: &AppUser,
    anonymous_session_id: &str,
) -> Result<String, ApiError> {
    match caller {
        AppUser::OpenID(user) if !user.sub.is_empty() => Ok(user.sub.clone()),
        AppUser::Unauthorized => {
            if !(16..=128).contains(&anonymous_session_id.len())
                || !anonymous_session_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ApiError::bad_request(
                    "A valid frontend session ID is required",
                ));
            }
            Ok(format!("frontend:{event_id}:{anonymous_session_id}"))
        }
        _ => Err(ApiError::unauthorized(
            "Hosted frontends require a platform user token",
        )),
    }
}

/// A hosted visit sends its payload and Page trigger, and a signed-in visitor may add their
/// own OAuth sign-ins. Nobody picks the version, runtime variables or profile, and an
/// anonymous visitor brings no sign-ins.
fn validate_hosted_invocation(
    query: &InvokeEventQuery,
    params: &InvokeEventRequest,
    visitor: &AppUser,
) -> Result<(), ApiError> {
    if query.local || query.isolated {
        return Err(ApiError::bad_request(
            "Hosted frontends require streaming remote execution",
        ));
    }
    let anonymous_sign_ins =
        params.oauth_tokens.is_some() && !matches!(visitor, AppUser::OpenID(_));
    if params.version.is_some()
        || anonymous_sign_ins
        || params.runtime_variables.is_some()
        || params.profile_id.is_some()
    {
        return Err(ApiError::bad_request(
            "Hosted frontends accept a payload, Page triggers and a signed-in visitor's own OAuth sign-ins, without other execution credential or configuration overrides",
        ));
    }
    Ok(())
}

async fn record_invocation_start(
    state: &AppState,
    user: &AppUser,
    execution: crate::audit::ExecutionAudit,
    hosted_subject: Option<&str>,
) {
    if let (AppUser::Unauthorized, Some(subject)) = (user, hosted_subject) {
        crate::audit::record_entry_once(
            state,
            crate::audit::AuditRecordInput {
                actor_id: subject.to_string(),
                actor_type: crate::entity::sea_orm_active_enums::AuditActorType::System,
                actor_ip: crate::audit::request::actor_ip(),
                action: "execution.event.start".to_string(),
                resource_type: "ExecutionRun".to_string(),
                resource_id: execution.run_id,
                scope: Some(execution.app_id),
                details: Some(serde_json::json!({
                    "board_id": execution.board_id,
                    "event_id": execution.event_id,
                    "node_id": execution.node_id,
                    "version": execution.version,
                    "board_etag": execution.board_etag,
                    "mode": format!("{:?}", execution.mode),
                    "status": format!("{:?}", execution.status),
                    "input_payload_len": execution.input_payload_len,
                    "technical_user_id": execution.technical_user_id,
                    "frontend": true,
                })),
            },
        )
        .await;
    } else {
        crate::audit::record_execution_start(state, user, execution).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn invoke_event_impl(
    state: AppState,
    user: AppUser,
    app_id: String,
    event_id: String,
    query: InvokeEventQuery,
    mut params: InvokeEventRequest,
    resolved_event: Option<flow_like::flow::event::Event>,
    governed_connected_app_call: bool,
    hosted_invocation: Option<HostedInvocation>,
) -> Result<Response, ApiError> {
    let hosted = hosted_invocation.is_some();
    let hosted_subject = hosted_invocation
        .as_ref()
        .map(|context| context.subject.as_str());
    let caller = async {
        if let Some(subject) = hosted_subject {
            let user_context = match &user {
                AppUser::OpenID(_) => {
                    Some(match user.app_permission_fresh(&app_id, &state).await {
                        Ok(permission) => permission.to_user_context(),
                        Err(error)
                            if matches!(
                                error.status(),
                                axum::http::StatusCode::FORBIDDEN
                                    | axum::http::StatusCode::UNAUTHORIZED
                            ) =>
                        {
                            UserExecutionContext::new(subject)
                        }
                        Err(error) => return Err(error),
                    })
                }
                _ => None,
            };
            let actor_user_id = account_user_id(subject).map(ToOwned::to_owned);
            return Ok((subject.to_string(), None, user_context, actor_user_id));
        }
        let permission = if params.page_trigger.is_some() {
            ensure_fresh_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents)
        } else {
            ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents)
        };
        let sub = permission.effective_user_id().map_err(|_| {
            ApiError::forbidden("Invoking requires a caller that is linked to a user account")
        })?;
        let technical_user_id = permission.technical_user_id().map(ToOwned::to_owned);
        let actor_user_id = account_user_id(&sub).map(ToOwned::to_owned);
        Ok((
            sub,
            technical_user_id,
            Some(permission.to_user_context()),
            actor_user_id,
        ))
    };
    // A pre-resolved event is accepted only from another crate-private,
    // governed contract. Generic event invocation always loads from the DB and
    // applies the direct connected-app surface policy. The read runs while the
    // caller is authorized; an authorization failure still wins.
    let event = async {
        match resolved_event {
            Some(event) => Ok(event),
            None => get_event_from_db(&state.db, &event_id, &app_id)
                .instrument(tracing::info_span!(
                    target: "flow_like::observability",
                    "db.query",
                    db.operation = "select",
                    db.table = "Event"
                ))
                .await
                .map_err(ApiError::from),
        }
    };
    let (caller, event) =
        flow_like_types::tokio::join!(caller.instrument(invoke_span!("invoke.authorize")), event);
    let (sub, technical_user_id, user_context, actor_user_id): (
        String,
        Option<String>,
        Option<UserExecutionContext>,
        Option<String>,
    ) = caller?;
    let event = event?;
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

    super::ensure_source_execution_allowed(&event)?;
    if !governed_connected_app_call {
        super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;
        crate::routes::oauth::authorize_incoming_oauth_tokens(
            &state,
            &user,
            &mut params.oauth_tokens,
        )
        .await?;
    }
    let resolved_page_trigger = match (event.default_page_id.as_ref(), params.page_trigger.as_ref())
    {
        (Some(_), Some(trigger)) => Some(
            super::page_trigger::resolve_authorized_page_trigger(
                &state,
                &app_id,
                &event,
                trigger,
                query.variant.as_deref(),
                &sub,
                technical_user_id.as_deref(),
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
    // is carried through as-is — for a local tracking run too, so the row
    // attributes the variant the client actually executed. Any other local
    // tracking run records the primary: nothing dispatches, so nothing splits.
    let variant_target = match &resolved_page_trigger {
        Some(resolved) => Some(resolved.target.clone()),
        None if query.local => None,
        None => Some(
            variant::resolve_invoke_target(
                &event,
                query.variant.as_deref(),
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
    let pinned_visit = match (&hosted_invocation, &variant_target) {
        (Some(hosted), Some(target)) => {
            hosted
                .pin(
                    &state,
                    resolved_page_trigger.as_ref(),
                    target,
                    params.oauth_tokens.as_ref(),
                )
                .await?
        }
        _ => None,
    };
    let (board_etag, hosted_profile) = match pinned_visit {
        Some(visit) => {
            params.oauth_tokens = visit.oauth_tokens;
            (visit.board_etag, visit.profile)
        }
        None => (
            resolved_page_trigger
                .as_ref()
                .and_then(|resolved| resolved.board_etag.clone()),
            None,
        ),
    };
    let resolved_version = board_version
        .map(format_run_version)
        .or_else(|| board_etag.as_ref().map(|etag| format!("etag:{etag}")));
    let event_json = match &variant_target {
        Some(target) => variant::dispatch_event_json(&event, target),
        None => serde_json::to_string(&event),
    }
    .map_err(|e| anyhow!("Failed to serialize event: {}", e))?;

    // A remote run's dispatch inputs do not depend on each other, so they
    // resolve together. Each result is consumed where it was awaited before,
    // which keeps the order in which failures surface. A local tracking run
    // only resolves the WASM package set.
    let remote = !query.local;
    let (wasm_packages, input_payload_key, credentials, (profile, custom_bit_owner)) = flow_like_types::tokio::join!(
        resolve_wasm_packages(&state, &app_id).instrument(invoke_span!("invoke.wasm_packages")),
        store_input_payload(
            &state,
            &app_id,
            &run_id,
            params.payload.as_ref().filter(|_| remote),
        )
        .instrument(invoke_span!("invoke.store_payload")),
        async {
            if remote {
                Some(
                    state
                        .scoped_credentials(&sub, &app_id, get_credentials_access())
                        .await,
                )
            } else {
                None
            }
        }
        .instrument(invoke_span!("invoke.scoped_credentials")),
        async {
            match &hosted_invocation {
                Some(hosted) if uses_sink_credentials(&hosted.subject) => {
                    (hosted_profile.clone(), hosted.profile_owner.clone())
                }
                _ if remote => {
                    caller_dispatch_profile(
                        &state,
                        &user,
                        &sub,
                        params.profile_id.as_deref(),
                        &app_id,
                    )
                    .await
                }
                _ => (None, None),
            }
        }
        .instrument(invoke_span!("invoke.profile")),
    );
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

    // Payload stored in object storage if present (for remote runs only - enables re-run)
    let input_payload_key = input_payload_key?;

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
        run_variant: Set(variant_target
            .as_ref()
            .map_or(RunVariant::Primary, |target| target.run_variant())),
        variant_name: Set(variant_name.clone()),
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
        user_id: Set(actor_user_id.clone()),
        technical_user_id: Set(technical_user_id.clone()),
        caller_app_chain: Set(caller_app_chain.clone().map(Into::into)),
        trace_id: Set(correlation.trace_id.clone()),
        parent_run_id: Set(parent_run_id.clone()),
        correlation_keys: Set(correlation_keys.clone()),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
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
        crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create run record");
                ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
            })?;
        record_invocation_start(&state, &user, execution_audit, hosted_subject).await;

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
            shadow: None,
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

    // Remote dispatch runs the resolved target's configured board version: a
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
            .with_run_id(run_id.clone())
            .with_event_definition(&event)
            .with_mode(run_mode.clone())
            .with_actor(actor_user_id, technical_user_id.clone())
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

    // Scoped credentials based on user permissions
    let credentials = credentials
        .transpose()?
        .ok_or_else(|| anyhow!("Scoped credentials were not resolved for a remote run"))?;

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

    let executor_jwt = if hosted {
        crate::execution::bind_hosted_frontend(&executor_jwt)
            .map_err(|error| ApiError::internal_error(anyhow!(error)))?
    } else {
        executor_jwt
    };
    let executor_jwt = match &user {
        AppUser::OpenID(oidc) if !query.isolated && !hosted => {
            crate::execution::bind_attended_payer(&executor_jwt, &oidc.sub)
                .map_err(|error| ApiError::internal_error(anyhow!(error)))?
        }
        _ => executor_jwt,
    };

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
        oauth_tokens: params.oauth_tokens,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: params.runtime_variables,
        user_context,
        profile,
        custom_bit_owner,
        wasm_packages,
        channel: None,
        trigger: if hosted {
            DispatchTrigger::System
        } else {
            DispatchTrigger::User
        },
        shadow: false,
        artifact: None,
    };

    // For isolated K8s jobs, insert run record and dispatch async
    if query.isolated {
        crate::entity::caller_apps::insert_run_with_caller_apps(&state.db, run)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create run record");
                ApiError::internal_error(anyhow!("Failed to create run record: {}", e))
            })?;
        record_invocation_start(&state, &user, execution_audit, hosted_subject).await;

        let response = match state
            .dispatcher
            .dispatch_with_backend(ExecutionBackend::KubernetesJob, request)
            .await
        {
            Ok(response) => response,
            Err(e) => {
                mark_sync_dispatch_failure(&state, &run_id, &app_id, &e).await;
                tracing::error!(error = %e, "Failed to dispatch job");
                return Err(ApiError::from(e));
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

    // The run row commits in the admission transaction, before the executor
    // starts, so infrastructure failures (executor crashes, network drops,
    // timeouts) leave a visible Pending row that can be reconciled.
    let record = crate::quota::RunRecord {
        caller_apps: crate::entity::caller_apps::caller_app_rows(
            &run_id,
            caller_app_chain.as_deref().unwrap_or_default(),
        ),
        run,
    };
    record_invocation_start(&state, &user, execution_audit, hosted_subject).await;

    // Dispatch based on the configured backend
    match backend {
        ExecutionBackend::LambdaStream => {
            // Use Lambda SDK streaming
            let (_dispatch_response, byte_stream) = match state
                .dispatcher
                .dispatch_streaming_with_run(request, Some(record.clone()))
                .await
            {
                Ok(response) => response,
                Err(failure) => {
                    record_dispatch_failure(
                        &state,
                        &run_id,
                        &app_id,
                        record,
                        failure.admitted,
                        &failure.error,
                    )
                    .await;
                    tracing::error!(error = %failure.error, "Failed to dispatch Lambda streaming job");
                    return Err(ApiError::from(failure.error));
                }
            };

            tracing::info!(run_id = %run_id, "Got Lambda response, starting stream proxy");

            Ok(proxy_lambda_sse_response(
                byte_stream,
                run_id,
                Some(crate::audit::ExecutionAuditContext::from(&state)),
                page_action_sealing,
            )
            .into_response())
        }
        _ => {
            // Use HTTP SSE for all other backends (Http, etc.)
            let (_dispatch_response, executor_response) = match state
                .dispatcher
                .dispatch_http_sse_with_run(request, Some(record.clone()))
                .await
            {
                Ok(response) => response,
                Err(failure) => {
                    record_dispatch_failure(
                        &state,
                        &run_id,
                        &app_id,
                        record,
                        failure.admitted,
                        &failure.error,
                    )
                    .await;
                    tracing::error!(error = %failure.error, "Failed to dispatch HTTP SSE job");
                    return Err(ApiError::from(failure.error));
                }
            };

            tracing::info!(run_id = %run_id, "Got executor response, starting stream proxy");

            Ok(proxy_sse_response_with_page_actions(
                executor_response,
                run_id,
                Some(crate::audit::ExecutionAuditContext::from(&state)),
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
    db: Option<crate::audit::ExecutionAuditContext>,
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
        let mut completion = CompletionWrites::default();

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
                        if let Some(db) = &db
                            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event_data)
                        {
                            completion.start(db, &run_id, &parsed);
                        }

                        let sse_event = Event::default()
                            .event(&event.event_type)
                            .data(event_data);
                        yield Ok(sse_event);
                    }
                }
                Err(e) => {
                    tracing::warn!(run_id = %run_id, error = %e, "Lambda stream error");
                    completion.finish().await;
                    if let Some(context) = &db
                        && let Err(error) = fail_run_on_stream_error(context, &run_id, &e.to_string()).await {
                            tracing::error!(run_id = %run_id, %error, "Failed to record Lambda stream failure");
                        }
                    yield Ok(stream_failure_event(e.to_string()));
                    break;
                }
            }
        }

        completion.finish().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::OpenIDUser;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn hosted_anonymous_subject_is_bound_to_event_and_browser_session() {
        let first =
            hosted_subject("event-1", &AppUser::Unauthorized, "anonymous-session-1").unwrap();
        let other_session =
            hosted_subject("event-1", &AppUser::Unauthorized, "anonymous-session-2").unwrap();
        let other_event =
            hosted_subject("event-2", &AppUser::Unauthorized, "anonymous-session-1").unwrap();
        assert_eq!(first, "frontend:event-1:anonymous-session-1");
        assert_ne!(first, other_session);
        assert_ne!(first, other_event);
        for session in ["", "short", "anonymous/../session", "anonymous:session:1"] {
            assert!(hosted_subject("event-1", &AppUser::Unauthorized, session).is_err());
        }
        assert!(hosted_subject("event-1", &AppUser::Unauthorized, &"a".repeat(129)).is_err());
    }

    #[test]
    fn hosted_authenticated_subject_keeps_the_verified_visitor() {
        let user = AppUser::OpenID(OpenIDUser {
            sub: "visitor-1".into(),
            access_token: "verified-platform-token".into(),
        });
        assert_eq!(hosted_subject("event-1", &user, "").unwrap(), "visitor-1");
        let empty_subject = AppUser::OpenID(OpenIDUser {
            sub: String::new(),
            access_token: "token".into(),
        });
        assert!(hosted_subject("event-1", &empty_subject, "anonymous-session-1").is_err());
    }

    /// A signed-in visitor may bring their own OAuth sign-ins; an anonymous visitor may not.
    /// Nobody picks the version, runtime variables or profile, or leaves streaming execution.
    #[test]
    fn hosted_execution_rejects_credential_and_configuration_overrides() {
        let signed_in = AppUser::OpenID(OpenIDUser {
            sub: "visitor-1".into(),
            access_token: "verified-platform-token".into(),
        });
        let anonymous = AppUser::Unauthorized;
        let payload = serde_json::json!({"payload": {"message": "hello"}});
        let valid: InvokeEventRequest = serde_json::from_value(payload.clone()).unwrap();
        let query = InvokeEventQuery::default();
        for visitor in [&signed_in, &anonymous] {
            assert!(validate_hosted_invocation(&query, &valid, visitor).is_ok());
        }
        let sign_ins = serde_json::json!({"google": {
            "access_token": "visitor-access",
            "refresh_token": "fl_oauth_visitor",
            "expires_at": 4_102_444_800_i64,
        }});
        for (field, value, signed_in_may) in [
            ("version", serde_json::json!("1_0_0"), false),
            ("oauth_tokens", serde_json::json!({}), true),
            ("oauth_tokens", sign_ins, true),
            ("runtime_variables", serde_json::json!({}), false),
            (
                "profile_id",
                serde_json::json!("another-users-profile"),
                false,
            ),
        ] {
            let mut request = payload.clone();
            request[field] = value;
            let params = serde_json::from_value(request).unwrap();
            assert_eq!(
                validate_hosted_invocation(&query, &params, &signed_in).is_ok(),
                signed_in_may,
                "signed in: {field}"
            );
            assert!(
                validate_hosted_invocation(&query, &params, &anonymous).is_err(),
                "anonymous: {field}"
            );
        }
        for query in [
            InvokeEventQuery {
                local: true,
                ..Default::default()
            },
            InvokeEventQuery {
                isolated: true,
                ..Default::default()
            },
        ] {
            for visitor in [&signed_in, &anonymous] {
                assert!(validate_hosted_invocation(&query, &valid, visitor).is_err());
            }
        }
    }

    /// The visitor's own sign-ins resolve as theirs before the run starts, after the visit
    /// checked the Event's sink, and the run's sign-ins come from the pinned visit.
    #[test]
    fn a_signed_in_visitors_sign_ins_resolve_as_theirs_before_the_run() {
        let source = production_source();
        let visit = function_body(source, "\npub(crate) async fn invoke_hosted_event(");
        in_order(
            visit,
            "invoke_hosted_event",
            &[
                "validate_hosted_invocation(&query, &params, &user)?;",
                "hosted_subject(&event.id, &user, anonymous_session_id)?",
                "ensure_user_exists(&state, &visitor.sub)",
                "hosted_invocation(&state, &app_id, &event.id, subject).await?",
                "authorize_incoming_oauth_tokens(&state, &user, &mut params.oauth_tokens)",
                "invoke_event_impl(",
            ],
        );
    }

    #[test]
    fn incoming_oauth_tokens_resolve_only_for_an_authorized_caller() {
        let source = include_str!("invoke_event.rs");
        let handler = |signature: &str| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            body.split_once("\n}\n").map_or(body, |(body, _)| body)
        };
        let position = |body: &str, step: &str| {
            body.find(step)
                .unwrap_or_else(|| panic!("the handler calls {step}"))
        };

        let route = handler("\npub async fn invoke_event(");
        assert!(!route.contains("authorize_incoming_oauth_tokens("));
        let invoke = handler("\nasync fn invoke_event_impl(");
        let resolved = position(invoke, "authorize_incoming_oauth_tokens(");
        for step in [
            "= caller?;",
            "ensure_connected_app_direct_event_allowed(",
            "if !governed_connected_app_call {",
        ] {
            assert!(position(invoke, step) < resolved, "{step}");
        }
    }

    fn production_source() -> &'static str {
        include_str!("invoke_event.rs")
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("the tests follow the production code")
            .0
    }

    fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let body = source
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    /// The dispatch columns of the hosted identity table; the callback column is in
    /// `middleware::jwt`. A signed-in visitor, member or not, runs as themselves: the visitor
    /// is the run's subject and owns its profile and private Bits, and a release's pause of the
    /// Event's sink does not stop them. An anonymous visitor's pseudonym owns nothing, so the
    /// user the sink runs as lends the profile and private Bits, and any pause stops the visit.
    #[test]
    fn hosted_visitors_run_as_themselves_unless_they_are_anonymous() {
        let signed_in = |sub: &str| {
            AppUser::OpenID(OpenIDUser {
                sub: sub.into(),
                access_token: "verified-platform-token".into(),
            })
        };
        for (visitor, caller, owner, sink_credentials, sink_use) in [
            (
                "member",
                signed_in("member-1"),
                Some("member-1"),
                false,
                SinkUse::Person,
            ),
            (
                "non-member",
                signed_in("guest-1"),
                Some("guest-1"),
                false,
                SinkUse::Person,
            ),
            (
                "anonymous",
                AppUser::Unauthorized,
                None,
                true,
                SinkUse::RunAs,
            ),
        ] {
            let subject = hosted_subject("event-1", &caller, "anonymous-session-1").unwrap();
            assert_eq!(account_user_id(&subject), owner, "{visitor}");
            assert_eq!(
                uses_sink_credentials(&subject),
                sink_credentials,
                "{visitor}"
            );
            assert_eq!(hosted_sink_use(&subject), sink_use, "{visitor}");
        }
    }

    /// A release's pause of the Event's sink stops anonymous visits, which run as the released
    /// user, and lets signed-in visitors through; a sink a person switched off stops both.
    #[test]
    fn a_release_stops_only_anonymous_visits_and_a_switch_off_every_visit() {
        let now = chrono::Utc::now().fixed_offset();
        let sink = |active: bool, released: bool| event_sink::Model {
            id: "sink-1".into(),
            event_id: "event-1".into(),
            app_id: "app-1".into(),
            sink_type: "chat".into(),
            active,
            path: None,
            auth_token: None,
            webhook_secret: None,
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: None,
            oauth_tokens_encrypted: None,
            profile_json: None,
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: (!released).then(|| "run-as".into()),
            run_as_released_at: released.then_some(now),
        };
        let signed_in = "visitor-1";
        let anonymous = "frontend:event-1:anonymous-session-1";
        for (case, sink, signed_in_admitted, anonymous_admitted) in [
            ("on", sink(true, false), true, true),
            ("paused by a release", sink(false, true), true, false),
            ("switched off", sink(false, false), false, false),
        ] {
            assert_eq!(
                hosted_sink_use(signed_in).admits(&sink),
                signed_in_admitted,
                "signed in: {case}"
            );
            assert_eq!(
                hosted_sink_use(anonymous).admits(&sink),
                anonymous_admitted,
                "anonymous: {case}"
            );
        }
    }

    /// Only an anonymous visit checks the Event's run-as user, before it borrows anything from
    /// the sink; a signed-in visit checks nobody and runs on the visitor's own profile and
    /// private Bits, like a member who invokes the Event in the app. A sink a person switched
    /// off refuses every visit, a release's pause only anonymous ones.
    #[test]
    fn only_an_anonymous_visit_checks_and_borrows_the_run_as_user() {
        let source = production_source();
        let visit = function_body(source, "\npub(crate) async fn invoke_hosted_event(");
        assert_eq!(
            visit.matches("Unauthorized").count(),
            1,
            "only the anonymous default may name the visitor kind"
        );
        assert!(
            visit.find("hosted_invocation(&state").unwrap()
                < visit.find("invoke_event_impl(").unwrap()
        );

        let publication = function_body(source, "\nasync fn hosted_invocation(");
        assert!(
            !publication.contains("AppUser"),
            "the subject decides, as on the run's callbacks"
        );
        in_order(
            publication,
            "hosted_invocation",
            &[
                "!hosted_sink_use(&subject).admits(sink)",
                "Event sink is inactive",
                "sink.as_ref().filter(|_| uses_sink_credentials(&subject))",
                "resolve_sink_run_as(state, sink).await?",
                "None => None,",
                "profile_owner,",
            ],
        );
        assert_eq!(publication.matches("resolve_sink_run_as(").count(), 1);
        assert!(!publication.contains("run_as_user_id"));
        assert!(!publication.contains(".active"));
        for borrowed in ["oauth_tokens", "sink_dispatch_profile("] {
            assert!(
                !publication.contains(borrowed),
                "the visit borrows {borrowed} before it fixed what it runs"
            );
        }

        let invoke = function_body(source, "\nasync fn invoke_event_impl(");
        in_order(
            invoke,
            "invoke_event_impl",
            &[
                "Some(hosted) if uses_sink_credentials(&hosted.subject) => {",
                "(hosted_profile.clone(), hosted.profile_owner.clone())",
                "_ if remote => {",
                "caller_dispatch_profile(",
                "&sub,",
                "_ => (None, None),",
            ],
        );
    }

    fn in_order(body: &str, name: &str, steps: &[&str]) {
        let mut from = 0;
        for step in steps {
            let found = body[from..]
                .find(step)
                .unwrap_or_else(|| panic!("{name} calls {step} after the steps before it"));
            from += found + step.len();
        }
    }

    /// A visit fixes what it runs before it takes anything from the sink, as a sink fire does:
    /// a Latest target pins its draft, a Latest Page trigger fixed its ETag when it resolved and
    /// reads the sink again, and the run carries that ETag. An anonymous visit then takes the
    /// sink's OAuth grants and run-as profile; a signed-in visit takes its own sign-ins and the
    /// saved grants of every other provider, and pins nothing when the sink saved none.
    #[test]
    fn a_visit_pins_what_it_runs_before_it_uses_the_sink() {
        let source = production_source();
        let pin = function_body(source, "\n    async fn pin(");
        in_order(
            pin,
            "HostedInvocation::pin and fix",
            &[
                "let sink_use = hosted_sink_use(&self.subject);",
                "self.sink.clone().filter(|sink| sink_use.borrows_from(sink))",
                "return Ok(None);",
                "self.fix(state, sink, page_trigger, target).await?",
                "let supplied = supplied.filter(|_| !anonymous).cloned();",
                "run_oauth_tokens(state, Some(&sink), sink_use, supplied).await?",
                "sink_dispatch_profile(state, &sink, self.profile_owner.as_deref())",
                "let sink_use = hosted_sink_use(&self.subject);",
                "None => pin_fire_for(state, sink, target, sink_use).await?,",
                "Some(_) => recheck_fire_for(state, &sink, sink_use).await?,",
            ],
        );
        let invoke = function_body(source, "\nasync fn invoke_event_impl(");
        in_order(
            invoke,
            "invoke_event_impl",
            &[
                "resolve_authorized_page_trigger(",
                "resolve_invoke_target(",
                "params.oauth_tokens.as_ref(),",
                "params.oauth_tokens = visit.oauth_tokens;",
                "let resolved_version",
                "hosted_profile.clone()",
                "board_etag: board_etag.clone(),",
                "oauth_tokens: params.oauth_tokens,",
            ],
        );
        assert!(invoke.contains("        board_etag,\n"));
        assert!(!invoke.contains("board_etag: None"));
    }

    #[test]
    fn pseudo_subjects_never_reach_user_foreign_keys() {
        let invoke = function_body(production_source(), "\nasync fn invoke_event_impl(");
        for unchecked in ["Set(Some(sub", ".with_actor(Some(", "Some(sub),"] {
            assert!(!invoke.contains(unchecked), "{unchecked}");
        }
        let actors: Vec<_> = invoke
            .match_indices("let actor_user_id = ")
            .map(|(at, _)| invoke[at..].split_once(';').unwrap().0)
            .collect();
        assert_eq!(actors.len(), 2, "the hosted visitor and the caller");
        for actor in actors {
            assert!(actor.contains("account_user_id("), "{actor}");
        }
        assert!(invoke.contains("user_id: Set(actor_user_id.clone()),"));
        assert!(invoke.contains(".with_actor(actor_user_id, technical_user_id.clone())"));
    }

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
            .fixed_offset();
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

    /// A SQLite stand-in for the tables a `completed` event writes.
    async fn completion_context() -> crate::audit::ExecutionAuditContext {
        use crate::entity::{execution_usage_tracking, payment_request};
        use sea_orm::{ConnectOptions, ConnectionTrait, Database, Schema, sea_query::Table};
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(execution_run::Entity),
            schema.create_table_from_entity(payment_request::Entity),
            schema.create_table_from_entity(execution_usage_tracking::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        let now = chrono::Utc::now().fixed_offset();
        db.execute_raw(sea_orm::Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            r#"INSERT INTO public."ExecutionRun" (id,"boardId","nodeId","logLevel","inputPayloadLen","outputPayloadLen",progress,"appId","createdAt","updatedAt",status,mode,"runVariant") VALUES ('run-1','board-1','node-1',0,0,0,0,'app-1',?,?,'RUNNING','HTTP','PRIMARY')"#,
            [now.into(), now.into()],
        ))
        .await
        .unwrap();
        crate::audit::ExecutionAuditContext {
            db: std::sync::Arc::new(db),
            dialect: crate::db::DbDialect::Postgres,
            enabled: false,
            state: None,
        }
    }

    async fn run_status(context: &crate::audit::ExecutionAuditContext) -> RunStatus {
        execution_run::Entity::find_by_id("run-1")
            .one(context.db.as_ref())
            .await
            .unwrap()
            .unwrap()
            .status
    }

    fn completed_frame() -> bytes::Bytes {
        bytes::Bytes::from_static(
            b"event: message\ndata: {\"event_type\":\"completed\",\"payload\":{\"status\":\"completed\"}}\n\n",
        )
    }

    #[flow_like_types::tokio::test]
    async fn completion_writes_finish_after_the_client_disconnects() {
        use futures::StreamExt;
        let context = completion_context().await;
        // The executor keeps its stream open after `completed`.
        let executor = futures::stream::iter([Ok::<_, DispatchError>(completed_frame())])
            .chain(futures::stream::pending());
        let body = proxy_lambda_sse_response(
            Box::pin(executor),
            "run-1".into(),
            Some(context.clone()),
            None,
        )
        .into_response()
        .into_body();
        let mut frames = body.into_data_stream();
        while let Some(frame) = frames.next().await {
            if String::from_utf8_lossy(&frame.unwrap()).contains("\"completed\"") {
                break;
            }
        }
        drop(frames);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while run_status(&context).await != RunStatus::Completed {
            assert!(
                std::time::Instant::now() < deadline,
                "the completion update was cancelled with the response body"
            );
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[flow_like_types::tokio::test]
    async fn the_stream_ends_only_after_its_completion_writes() {
        let context = completion_context().await;
        let executor = futures::stream::iter([Ok::<_, DispatchError>(completed_frame())]);
        let body = proxy_lambda_sse_response(
            Box::pin(executor),
            "run-1".into(),
            Some(context.clone()),
            None,
        )
        .into_response()
        .into_body();
        let text = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&text).contains("\"completed\""));
        assert_eq!(run_status(&context).await, RunStatus::Completed);

        let context = completion_context().await;
        let executor = reqwest::Response::from(axum::http::Response::new(reqwest::Body::from(
            completed_frame(),
        )));
        let body = proxy_sse_response_with_page_actions(
            executor,
            "run-1".into(),
            Some(context.clone()),
            None,
        )
        .into_response()
        .into_body();
        axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
        assert_eq!(run_status(&context).await, RunStatus::Completed);
    }
}

//! Event setup helper.
//!
//! Runs an event's workflow in "setup" mode, captures every `server_config`
//! intercom event it emits, and persists them as `EventRemoteAuth` +
//! `EventRemoteRegistration` rows so inbound REST/MCP traffic can be
//! dispatched directly to the registered nodes.
//!
//! Invoked automatically from [`super::upsert_event`] for `rest` / `mcp`
//! event types (always for the stable variant), and manually via
//! `POST /apps/{app_id}/events/{event_id}/setup` — which may name a Live
//! variant to build that variant's own registration bucket.
//!
//! Persistence is a delete-then-insert by `(app_id, event_id,
//! event_version, variant)`. Every setup writes the `(event, variant)`
//! `EventSetup` pointer row; for the stable variant the event row is
//! additionally updated with `setup_status`, `last_setup_at`,
//! `last_setup_version`, and `last_setup_error` — the back-compat pointer
//! inbound falls back to when no stable `EventSetup` row exists yet.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use eventsource_stream::Eventsource;
use flow_like::flow::{
    board::Board,
    event::{EventVariant, EventVariantMode},
    node::Node,
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use futures::StreamExt;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    sea_query::{Expr, OnConflict},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::{
    audit_branch, ensure_permission,
    entity::{
        event, event_remote_auth, event_remote_registration, event_setup,
        sea_orm_active_enums::RunMode,
    },
    error::ApiError,
    execution::{
        ByteStream, DispatchError, DispatchRequest, DispatchTrigger, ExecutionBackend,
        ExecutionJwtParams, TokenType, fetch_profile_for_dispatch, is_jwt_configured,
        resolve_wasm_packages, sign_execution_jwt,
        variant::{ResolvedTarget, STABLE_VARIANT, dispatch_event_json},
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

use super::db::{encrypt_token, get_event_from_db};

/// Default setup timeout — setup workflows are expected to finish in
/// seconds (they're emitting config, not doing real work).
const DEFAULT_SETUP_TIMEOUT_SECS: u64 = 90;

/// Intercom event type emitted by the REST/MCP server nodes during a
/// remote setup run. Kept in sync with
/// `flow_like_catalog_web::web::remote::REMOTE_SERVER_CONFIG_EVENT_TYPE`.
const SERVER_CONFIG_EVENT_TYPE: &str = "server_config";

fn is_completed_run_status(status: &str) -> bool {
    status.trim().eq_ignore_ascii_case("completed")
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct SetupEventRequest {
    /// Optional override for the setup payload sent to the workflow.
    /// Defaults to `{}` — most setup workflows only emit configuration
    /// and don't read the payload.
    pub payload: Option<serde_json::Value>,
    /// Optional profile ID for credential scoping.
    pub profile_id: Option<String>,
    /// Setup timeout in seconds (default: 90).
    pub timeout_seconds: Option<u64>,
    /// Force a setup even if another setup run is already `running`.
    /// Without this flag a parallel `POST /setup` returns `409 Conflict`.
    #[serde(default)]
    pub force: bool,
    /// Variant whose inbound surface to (re)build. Defaults to "stable"
    /// (the primary target). Naming a Live variant runs the setup against
    /// that variant's board and writes its own registration bucket; shadow
    /// variants have no inbound surface and are refused.
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SetupEventResponse {
    pub run_id: String,
    pub event_id: String,
    pub event_version: String,
    pub status: String,
    pub server_configs_received: usize,
    pub registrations_written: usize,
    pub auths_written: usize,
    pub error: Option<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/setup",
    tag = "events",
    description = "Run REST/MCP remote setup and persist inbound registrations for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
    ),
    request_body = SetupEventRequest,
    responses(
        (status = 200, description = "Setup completed", body = SetupEventResponse),
        (status = 400, description = "Setup failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Setup already running"),
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/setup",
    skip(state, user, body)
)]
pub async fn setup_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<SetupEventRequest>,
) -> Result<Json<SetupEventResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let user_context = permission.to_user_context();
    let variant = body
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(STABLE_VARIANT)
        .to_string();
    let force = body.force;

    let response = run_event_setup(
        state.clone(),
        sub,
        app_id.clone(),
        event_id.clone(),
        body,
        user_context,
    )
    .await?;
    if response.status == "ok" {
        audit_branch!(
            state,
            user,
            app_id,
            "event.setup",
            "Event",
            event_id,
            "Event setup run",
            json!({
                "variant": variant,
                "event_version": response.event_version,
                "run_id": response.run_id,
                "setup_status": response.status,
                "registrations": response.registrations_written,
                "auths": response.auths_written,
                "server_configs_received": response.server_configs_received,
                "force": force,
            })
        );
        Ok(Json(response))
    } else {
        Err(ApiError::bad_request(
            response
                .error
                .clone()
                .unwrap_or_else(|| "event setup failed".to_string()),
        ))
    }
}

#[derive(Clone, Deserialize, Debug)]
struct ServerConfigEnvelope {
    kind: String,
    node_id: String,
    config: Value,
}

fn http_response_byte_stream(response: reqwest::Response) -> ByteStream {
    Box::pin(
        response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|err| DispatchError::Network(err.to_string()))),
    )
}

async fn collect_server_config_events(
    stream: ByteStream,
) -> (Vec<ServerConfigEnvelope>, Option<String>) {
    let mut events: Vec<ServerConfigEnvelope> = Vec::new();
    let mut error: Option<String> = None;
    let mut es = stream.eventsource();

    while let Some(item) = es.next().await {
        let sse = match item {
            Ok(evt) => evt,
            Err(err) => {
                error = Some(format!("sse parse error: {err}"));
                break;
            }
        };
        let parsed: Value = match serde_json::from_str(&sse.data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = parsed
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if event_type == SERVER_CONFIG_EVENT_TYPE
            && let Some(payload) = parsed.get("payload").cloned()
            && let Ok(env) = serde_json::from_value::<ServerConfigEnvelope>(payload)
        {
            events.push(env);
        }
        if event_type == "completed" {
            let payload = parsed.get("payload");
            if let Some(status) = payload
                .and_then(|p| p.get("status"))
                .and_then(|s| s.as_str())
                && !is_completed_run_status(status)
            {
                error = Some(format!("setup run finished with status: {status}"));
            }
            break;
        }
    }

    (events, error)
}

/// Core setup logic. Invoked from background tasks spawned by
/// [`super::upsert_event`] for REST/MCP event types. The caller is
/// responsible for permission enforcement; the helper does no
/// authorization checks.
pub(crate) async fn run_event_setup(
    state: AppState,
    sub: String,
    app_id: String,
    event_id: String,
    body: SetupEventRequest,
    user_context: flow_like::flow::execution::UserExecutionContext,
) -> Result<SetupEventResponse, ApiError> {
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }

    // Load the event (validates ownership) and capture its current version.
    let core_event = get_event_from_db(&state.db, &event_id, &app_id)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    if !super::generic_event_endpoint_allowed(&core_event.event_type) {
        return Err(ApiError::forbidden(
            "Ontology action events are managed and invoked through Data Studio",
        ));
    }

    // Resolve the target surface. A non-stable setup dispatches into the
    // variant's board/version/node and writes that variant's own
    // registration bucket; the primary ("stable") keeps today's path.
    let variant_name = body
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(STABLE_VARIANT)
        .to_string();
    let target = if variant_name == STABLE_VARIANT {
        ResolvedTarget::primary(&core_event)
    } else {
        let Some(event_variant) = core_event
            .variant_set()
            .into_iter()
            .find(|variant| variant.name == variant_name)
        else {
            return Err(ApiError::not_found(format!(
                "event {event_id} has no variant named '{variant_name}'"
            )));
        };
        if matches!(event_variant.mode, EventVariantMode::Shadow { .. }) {
            return Err(ApiError::bad_request(format!(
                "variant '{variant_name}' is a shadow variant; shadow variants have no inbound surface"
            )));
        }
        ResolvedTarget::from_variant(&event_variant)
    };

    // Concurrent-setup guard. Setup writes are delete-then-insert by
    // `(app, event, version, variant)` so two parallel calls race on the
    // same rows. Reject the second unless the caller explicitly forces.
    // The persist phase additionally serializes on a row lock and derives
    // the registration prune's protect set inside its transaction.
    if !body.force {
        let running = if variant_name == STABLE_VARIANT {
            event::Entity::find_by_id(&core_event.id)
                .filter(event::Column::AppId.eq(&app_id))
                .one(&state.db)
                .await
                .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
                .is_some_and(|row| row.setup_status.as_deref() == Some("running"))
        } else {
            find_event_setup(&state.db, &app_id, &core_event.id, &variant_name)
                .await
                .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
                .is_some_and(|row| row.setup_status.as_deref() == Some("running"))
        };
        if running {
            return Err(ApiError::conflict(
                "setup already running for this event; pass `force: true` to override",
            ));
        }
    }

    let event_version = format!(
        "{}.{}.{}",
        core_event.event_version.0, core_event.event_version.1, core_event.event_version.2
    );
    // The event the executor sees carries the target: for a variant, the
    // variant's board triple plus its variables merged over the event's.
    let event_json = dispatch_event_json(&core_event, &target)
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    let board_id = target.board_id.clone();
    let board_version = target.board_version;

    // Mark setup as running before dispatch so the UI can show progress.
    // Deliberately DO NOT touch the serving pointers here —
    // `last_setup_version` (stable) and `EventSetup.eventVersion` (variants)
    // must keep pointing at the last successful setup until this completes.
    let now = chrono::Utc::now().fixed_offset();
    if variant_name == STABLE_VARIANT {
        let _ = event::ActiveModel {
            id: Set(core_event.id.clone()),
            setup_status: Set(Some("running".to_string())),
            last_setup_at: Set(Some(now)),
            last_setup_error: Set(None),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(&state.db)
        .await;
    } else if let Err(error) = write_event_setup_row(
        &state.db,
        &app_id,
        &core_event.id,
        &variant_name,
        &event_version,
        &target,
        "running",
        None,
    )
    .await
    {
        tracing::warn!(
            event_id = %core_event.id,
            variant = %variant_name,
            %error,
            "failed to mark variant setup as running"
        );
    }

    let run_id = flow_like_types::create_id();
    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: sub.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(60 * 60),
        shadow: None,
    })
    .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let credentials = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await?;
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    let profile =
        fetch_profile_for_dispatch(&state, &sub, body.profile_id.as_deref(), &app_id, true).await;
    let wasm_packages = resolve_wasm_packages(&state, &app_id).await;

    // Persist a run record. Setup runs are tracked as `Http` mode runs
    // because they go through the same dispatch path.
    let run_active = crate::entity::execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id.clone()),
        version: Set(None),
        event_id: Set(Some(event_id.clone())),
        node_id: Set(Some(target.node_id.clone())),
        status: Set(crate::entity::sea_orm_active_enums::RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(target.run_variant()),
        variant_name: Set(target.variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(0),
        input_payload_len: Set(0),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(now + chrono::Duration::hours(2))),
        user_id: Set(Some(sub.clone())),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    run_active
        .insert(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id,
        board_version,
        board_etag: None,
        node_id: target.node_id.clone(),
        event_json: Some(event_json),
        payload: body.payload.clone().or_else(|| Some(serde_json::json!({}))),
        user_id: sub,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token: None,
        oauth_tokens: None,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Event),
        runtime_variables: None,
        user_context: Some(user_context),
        profile,
        wasm_packages,
        channel: None,
        // A background setup workflow emitting config.
        trigger: DispatchTrigger::System,
        shadow: false,
        artifact: None,
    };

    let backend = state.dispatcher.backend();
    let setup_stream = match backend {
        ExecutionBackend::LambdaStream => {
            match state.dispatcher.dispatch_streaming(request).await {
                Ok((_dispatch_response, byte_stream)) => {
                    tracing::info!(run_id = %run_id, "Got Lambda setup response, collecting server config");
                    byte_stream
                }
                Err(e) => {
                    let msg = format!("dispatch failed: {e}");
                    record_setup_failure(
                        &state,
                        &app_id,
                        &core_event.id,
                        &event_version,
                        &variant_name,
                        &target,
                        &msg,
                    )
                    .await;
                    return Err(ApiError::internal_error(flow_like_types::anyhow!(msg)));
                }
            }
        }
        _ => match state.dispatcher.dispatch_http_sse(request).await {
            Ok((_dispatch_response, executor_response)) => {
                tracing::info!(run_id = %run_id, "Got executor setup response, collecting server config");
                http_response_byte_stream(executor_response)
            }
            Err(e) => {
                let msg = format!("dispatch failed: {e}");
                record_setup_failure(
                    &state,
                    &app_id,
                    &core_event.id,
                    &event_version,
                    &variant_name,
                    &target,
                    &msg,
                )
                .await;
                return Err(ApiError::internal_error(flow_like_types::anyhow!(msg)));
            }
        },
    };

    // Drain the SSE stream and collect `server_config` events. Bail out as
    // soon as we see a `completed` event so we don't hold the connection
    // longer than necessary.
    let timeout = Duration::from_secs(body.timeout_seconds.unwrap_or(DEFAULT_SETUP_TIMEOUT_SECS));
    let (collected, error) = match flow_like_types::tokio::time::timeout(
        timeout,
        collect_server_config_events(setup_stream),
    )
    .await
    {
        Ok(pair) => pair,
        Err(_) => {
            let msg = format!("setup timed out after {}s", timeout.as_secs());
            record_setup_failure(
                &state,
                &app_id,
                &core_event.id,
                &event_version,
                &variant_name,
                &target,
                &msg,
            )
            .await;
            return Err(ApiError::internal_error(flow_like_types::anyhow!(msg)));
        }
    };

    if let Some(ref err_msg) = error {
        record_setup_failure(
            &state,
            &app_id,
            &core_event.id,
            &event_version,
            &variant_name,
            &target,
            err_msg,
        )
        .await;
        return Ok(SetupEventResponse {
            run_id,
            event_id: core_event.id,
            event_version,
            status: "failed".to_string(),
            server_configs_received: collected.len(),
            registrations_written: 0,
            auths_written: 0,
            error: Some(err_msg.clone()),
        });
    }

    // Setup ran cleanly but the flow never emitted any server config.
    // For rest/mcp events this means the user forgot the corresponding
    // server node — surface that explicitly instead of silently leaving
    // the event with zero registrations (which would 404 on every
    // inbound request).
    let expected_kind = match core_event.event_type.as_str() {
        "rest" => Some("rest"),
        "mcp" => Some("mcp"),
        _ => None,
    };
    let matching_configs = expected_kind
        .map(|kind| collected.iter().filter(|env| env.kind == kind).count())
        .unwrap_or(collected.len());

    if expected_kind.is_some() && matching_configs == 0 {
        let node_label = if core_event.event_type == "rest" {
            "Run REST Server"
        } else {
            "Run MCP Server"
        };
        let msg = format!(
            "setup completed but no server configuration was emitted — \
             add a '{node_label}' node at the start of the flow so the \
             event can register its endpoints"
        );
        record_setup_failure(
            &state,
            &app_id,
            &core_event.id,
            &event_version,
            &variant_name,
            &target,
            &msg,
        )
        .await;
        return Ok(SetupEventResponse {
            run_id,
            event_id: core_event.id,
            event_version,
            status: "failed".to_string(),
            server_configs_received: 0,
            registrations_written: 0,
            auths_written: 0,
            error: Some(msg),
        });
    }

    // Persist: delete previous (app_id, event_id, event_version, variant)
    // rows, then insert the freshly collected ones in a single transaction.
    let collected_to_persist: Vec<ServerConfigEnvelope> = match expected_kind {
        Some(kind) => collected
            .iter()
            .filter(|env| env.kind == kind)
            .cloned()
            .collect(),
        None => collected.clone(),
    };

    let setup_board = if expected_kind == Some("mcp") {
        match state
            .master_board(
                "setup",
                &app_id,
                &target.board_id,
                &state,
                target.board_version,
            )
            .await
        {
            Ok(board) => Some(board),
            Err(err) => {
                tracing::warn!(
                    event_id = %core_event.id,
                    error = %err,
                    "failed to load board while expanding MCP setup metadata"
                );
                None
            }
        }
    } else {
        None
    };

    // Success marking (the `EventSetup` pointer row, plus `setup_status = "ok"`
    // and the `last_setup_version` advance for stable) happens inside
    // persist_registrations' transaction, atomically with the registration
    // rows the pointer names.
    let (registrations_written, auths_written) = match persist_registrations(
        &state,
        &app_id,
        &core_event.id,
        &event_version,
        &variant_name,
        &target,
        &collected_to_persist,
        setup_board.as_ref(),
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            let msg = match &e {
                PersistError::Parity(reason) | PersistError::VariantGone(reason) => reason.clone(),
                PersistError::Db(err) => format!("persisting registrations failed: {err}"),
                PersistError::Other(err) => format!("persisting registrations failed: {err}"),
            };
            if !matches!(e, PersistError::VariantGone(_)) {
                record_setup_failure(
                    &state,
                    &app_id,
                    &core_event.id,
                    &event_version,
                    &variant_name,
                    &target,
                    &msg,
                )
                .await;
            }
            // A parity refusal is the caller's board to fix — surface it the
            // same way as other failed-but-not-broken setups (a 400 at the
            // route), not as an internal error.
            return match e {
                PersistError::Parity(_) | PersistError::VariantGone(_) => Ok(SetupEventResponse {
                    run_id,
                    event_id: core_event.id,
                    event_version,
                    status: "failed".to_string(),
                    server_configs_received: collected.len(),
                    registrations_written: 0,
                    auths_written: 0,
                    error: Some(msg),
                }),
                PersistError::Db(err) => Err(ApiError::from(err)),
                PersistError::Other(_) => {
                    Err(ApiError::internal_error(flow_like_types::anyhow!(msg)))
                }
            };
        }
    };

    Ok(SetupEventResponse {
        run_id,
        event_id: core_event.id,
        event_version,
        status: "ok".to_string(),
        server_configs_received: collected.len(),
        registrations_written,
        auths_written,
        error: None,
    })
}

/// Mark the failed setup's status row as `"error"` — the event row for the
/// stable variant, the `(event, variant)` `EventSetup` row otherwise. Awaited
/// inline so the status is durably persisted before the API response goes
/// out — k8s/lambda shutdowns must not be able to leave the row stuck in
/// `running`. Deliberately does NOT overwrite the serving pointers
/// (`last_setup_version` / `EventSetup.eventVersion`) so inbound traffic
/// keeps routing to the last successful setup.
#[allow(clippy::too_many_arguments)]
async fn record_setup_failure(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    target: &ResolvedTarget,
    msg: &str,
) {
    let result = if variant == STABLE_VARIANT {
        let now = chrono::Utc::now().fixed_offset();
        (event::ActiveModel {
            id: Set(event_id.to_string()),
            setup_status: Set(Some("error".to_string())),
            last_setup_at: Set(Some(now)),
            last_setup_error: Set(Some(msg.to_string())),
            updated_at: Set(now),
            ..Default::default()
        })
        .update(&state.db)
        .await
        .map(|_| ())
    } else {
        write_event_setup_row(
            &state.db,
            app_id,
            event_id,
            variant,
            event_version,
            target,
            "error",
            Some(msg),
        )
        .await
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, event_id = %event_id, variant = %variant, "failed to record setup failure");
    }
}

fn format_board_version((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// The `(event, variant)` `EventSetup` pointer row, if one exists. Shared by
/// the inbound routers and the registration read surfaces.
pub(crate) async fn find_event_setup<C: ConnectionTrait>(
    conn: &C,
    app_id: &str,
    event_id: &str,
    variant: &str,
) -> Result<Option<event_setup::Model>, sea_orm::DbErr> {
    event_setup::Entity::find()
        .filter(event_setup::Column::AppId.eq(app_id))
        .filter(event_setup::Column::EventId.eq(event_id))
        .filter(event_setup::Column::Variant.eq(variant))
        .one(conn)
        .await
}

/// Upsert the `(event, variant)` `EventSetup` row. Only an `"ok"` write may
/// touch the serving-pointer columns (`eventVersion`, `boardId`,
/// `boardVersion`); `"running"`/`"error"` marks write just the status fields
/// — a first-time non-ok insert leaves the pointer columns empty — so a
/// populated pointer always names the last successful setup. Inbound treats
/// pointer presence as servability and `setup_status` as health metadata
/// only.
#[allow(clippy::too_many_arguments)]
async fn write_event_setup_row<C: ConnectionTrait>(
    conn: &C,
    app_id: &str,
    event_id: &str,
    variant: &str,
    event_version: &str,
    target: &ResolvedTarget,
    status: &str,
    error: Option<&str>,
) -> Result<(), sea_orm::DbErr> {
    let pointer_columns = [
        event_setup::Column::EventVersion,
        event_setup::Column::BoardId,
        event_setup::Column::BoardVersion,
    ];
    let status_columns = [
        event_setup::Column::SetupStatus,
        event_setup::Column::LastSetupAt,
        event_setup::Column::LastSetupError,
        event_setup::Column::UpdatedAt,
    ];
    let mut on_conflict =
        OnConflict::columns([event_setup::Column::EventId, event_setup::Column::Variant]);
    if status == "ok" {
        on_conflict.update_columns(pointer_columns);
    }
    on_conflict.update_columns(status_columns);

    let (event_version, board_id, board_version) = if status == "ok" {
        (
            event_version.to_string(),
            target.board_id.clone(),
            target.board_version.map(format_board_version),
        )
    } else {
        (String::new(), String::new(), None)
    };
    let now = chrono::Utc::now().fixed_offset();
    event_setup::Entity::insert(event_setup::ActiveModel {
        id: Set(flow_like_types::create_id()),
        app_id: Set(app_id.to_string()),
        event_id: Set(event_id.to_string()),
        variant: Set(variant.to_string()),
        event_version: Set(event_version),
        board_id: Set(board_id),
        board_version: Set(board_version),
        setup_status: Set(Some(status.to_string())),
        last_setup_at: Set(Some(now)),
        last_setup_error: Set(error.map(ToString::to_string)),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .on_conflict(on_conflict)
    .exec_without_returning(conn)
    .await?;
    Ok(())
}

/// Refresh only the status columns of an existing `(event, variant)`
/// `EventSetup` row — the stale-setup path, where the serving pointer must
/// keep naming the newer setup. A no-op when no row exists (stable served by
/// the legacy scalar only).
async fn touch_event_setup_status<C: ConnectionTrait>(
    conn: &C,
    app_id: &str,
    event_id: &str,
    variant: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), sea_orm::DbErr> {
    let now = chrono::Utc::now().fixed_offset();
    event_setup::Entity::update_many()
        .col_expr(event_setup::Column::SetupStatus, Expr::value(status))
        .col_expr(event_setup::Column::LastSetupAt, Expr::value(now))
        .col_expr(
            event_setup::Column::LastSetupError,
            Expr::value(error.map(ToString::to_string)),
        )
        .col_expr(event_setup::Column::UpdatedAt, Expr::value(now))
        .filter(event_setup::Column::AppId.eq(app_id))
        .filter(event_setup::Column::EventId.eq(event_id))
        .filter(event_setup::Column::Variant.eq(variant))
        .exec(conn)
        .await?;
    Ok(())
}

/// Persist server-config events.
///
/// For each `kind == "rest"` event we explode the `RestServerConfig` into
/// one registration row per function route × method, one per file route,
/// and one per OpenAPI route. The auth config (if present and not
/// `RestAuthConfig::None`) becomes a single `EventRemoteAuth` row that the
/// registrations link to.
///
/// For each `kind == "mcp"` event we currently store one opaque
/// `mcp_raw` registration with the full config in `extras_json` — the
/// MCP protocol handler will interpret it later. This keeps the inbound
/// path implementable without locking in MCP-specific schema details.
///
/// Rows from superseded event versions of the same variant are pruned in
/// the same transaction — only the version just written and the variant's
/// serving pointer (`last_setup_version` for stable, the `EventSetup` row
/// otherwise) survive. Every registration row is stamped with the variant,
/// and the `(event, variant)` `EventSetup` pointer row is written
/// atomically with the rows it names; only the stable variant additionally
/// advances `event.last_setup_version` + `setup_status`. A non-stable
/// setup must additionally pass the stable-parity gates (auth types, MCP
/// tool names) before anything commits.
#[allow(clippy::too_many_arguments)]
async fn persist_registrations(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    target: &ResolvedTarget,
    envelopes: &[ServerConfigEnvelope],
    setup_board: Option<&Board>,
) -> Result<(usize, usize), PersistError> {
    let inputs = Arc::new(PersistInputs {
        app_id: app_id.to_string(),
        event_id: event_id.to_string(),
        event_version: event_version.to_string(),
        variant: variant.to_string(),
        target: target.clone(),
        envelopes: envelopes.to_vec(),
        setup_board: setup_board.cloned(),
    });
    state
        .transaction(|txn| {
            let state = state.clone();
            let inputs = inputs.clone();
            Box::pin(async move { persist_registrations_in(txn, &state, &inputs).await })
        })
        .await
}

/// Everything one persist attempt reads, owned so the retried body can run
/// from scratch after a lost commit race.
struct PersistInputs {
    app_id: String,
    event_id: String,
    event_version: String,
    variant: String,
    target: ResolvedTarget,
    envelopes: Vec<ServerConfigEnvelope>,
    setup_board: Option<Board>,
}

async fn persist_registrations_in(
    txn: &DatabaseTransaction,
    state: &AppState,
    inputs: &PersistInputs,
) -> Result<(usize, usize), PersistError> {
    use sea_orm::QuerySelect;

    let PersistInputs {
        app_id,
        event_id,
        event_version,
        variant,
        target,
        envelopes,
        setup_board,
    } = inputs;
    let (app_id, event_id, event_version, variant) = (
        app_id.as_str(),
        event_id.as_str(),
        event_version.as_str(),
        variant.as_str(),
    );
    let setup_board = setup_board.as_ref();

    // Lock the event row for the whole persist phase. Two overlapping setups
    // (force, or the non-atomic running-status guard) serialize here, and the
    // protect set below is derived from the rows as committed — a snapshot
    // taken before the txn could name a version whose rows another setup just
    // pruned, leaving inbound routing at a version with zero registrations.
    let locked_row = event::Entity::find_by_id(event_id)
        .filter(event::Column::AppId.eq(app_id))
        .lock_exclusive()
        .one(txn)
        .await?;
    // Re-verify against the locked row that the variant still exists — a
    // concurrent promote/abort may have removed it (and dropped its bucket)
    // between this setup's dispatch and its persist; committing would
    // resurrect the deleted variant's rows.
    if variant != STABLE_VARIANT {
        let still_exists = locked_row.as_ref().is_some_and(|row| {
            row.variants
                .as_ref()
                .and_then(|json| serde_json::from_value::<Vec<EventVariant>>(json.clone()).ok())
                .map(|variants| variants.iter().any(|entry| entry.name == variant))
                .unwrap_or_else(|| variant == "canary" && row.canary.is_some())
        });
        if !still_exists {
            return Err(PersistError::VariantGone(format!(
                "variant '{variant}' no longer exists on event {event_id}; a concurrent promote or abort removed it, so this setup run was discarded"
            )));
        }
    }
    let stable_setup_version = locked_row.and_then(|row| row.last_setup_version);
    // The version whose rows this variant serves until this txn commits. A
    // variant row without a serving pointer (no successful setup yet) serves
    // nothing.
    let live_setup_version = if variant == STABLE_VARIANT {
        stable_setup_version.clone()
    } else {
        find_event_setup(txn, app_id, event_id, variant)
            .await?
            .map(|row| row.event_version)
            .filter(|version| !version.is_empty())
    };

    // Wipe previous rows for this (version, variant) so re-runs don't pile up
    // duplicates.
    event_remote_registration::Entity::delete_many()
        .filter(event_remote_registration::Column::AppId.eq(app_id))
        .filter(event_remote_registration::Column::EventId.eq(event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(event_version))
        .filter(event_remote_registration::Column::Variant.eq(variant))
        .exec(txn)
        .await?;
    event_remote_auth::Entity::delete_many()
        .filter(event_remote_auth::Column::AppId.eq(app_id))
        .filter(event_remote_auth::Column::EventId.eq(event_id))
        .filter(event_remote_auth::Column::EventVersion.eq(event_version))
        .filter(event_remote_auth::Column::Variant.eq(variant))
        .exec(txn)
        .await?;

    let mut reg_count = 0usize;
    let mut auth_count = 0usize;
    let now = chrono::Utc::now().fixed_offset();
    // Dedup `(variant, kind, method, path)` across all envelopes for this
    // event version. A misconfigured graph can produce duplicate routes (e.g.
    // two REST server nodes both registering `POST /webhook`). We keep the
    // first occurrence and emit a warning for the rest — inbound dispatch
    // would otherwise pick rows in DB-order, which is undefined.
    let mut seen: std::collections::HashSet<(String, String, String, String)> =
        std::collections::HashSet::new();

    for env in envelopes {
        match env.kind.as_str() {
            "rest" => {
                let (regs, auth_id) = expand_rest_config(
                    state,
                    app_id,
                    event_id,
                    event_version,
                    variant,
                    &env.node_id,
                    &env.config,
                    &mut auth_count,
                    now,
                    txn,
                )
                .await?;
                for mut reg in regs {
                    reg.auth_id = Set(auth_id.clone());
                    let kind_s = reg.kind.clone().take().unwrap_or_default();
                    let method_s = reg
                        .method
                        .clone()
                        .take()
                        .unwrap_or_default()
                        .unwrap_or_default();
                    let path_s = reg.path.clone().take().unwrap_or_default();
                    let key = (variant.to_string(), kind_s, method_s.to_uppercase(), path_s);
                    if !seen.insert(key.clone()) {
                        tracing::warn!(
                            kind = %key.1, method = %key.2, path = %key.3,
                            "duplicate inbound route within setup batch; ignoring later occurrence"
                        );
                        continue;
                    }
                    reg.insert(txn).await?;
                    reg_count += 1;
                }
            }
            "mcp" => {
                let auth_id = maybe_insert_auth_from_value(
                    state,
                    app_id,
                    event_id,
                    event_version,
                    variant,
                    &env.node_id,
                    "mcp",
                    env.config.get("auth"),
                    &mut auth_count,
                    now,
                    txn,
                )
                .await?;
                let key = (
                    variant.to_string(),
                    "mcp_raw".to_string(),
                    String::new(),
                    "/".to_string(),
                );
                if !seen.insert(key) {
                    tracing::warn!(
                        node_id = %env.node_id,
                        "duplicate mcp_raw registration within setup batch; ignoring"
                    );
                    continue;
                }
                let mut config_json = env.config.clone();
                if let Some(auth) = env.config.get("auth")
                    && let Some(obj) = config_json.as_object_mut()
                {
                    obj.insert(
                        "auth".to_string(),
                        protect_auth_config_for_storage(auth, &state.encryption_key),
                    );
                }
                event_remote_registration::ActiveModel {
                    id: Set(flow_like_types::create_id()),
                    app_id: Set(app_id.to_string()),
                    event_id: Set(event_id.to_string()),
                    event_version: Set(event_version.to_string()),
                    variant: Set(variant.to_string()),
                    kind: Set("mcp_raw".to_string()),
                    method: Set(None),
                    path: Set("/".to_string()),
                    node_id: Set(Some(env.node_id.clone())),
                    schema_json: Set(None),
                    extras_json: Set(Some(config_json)),
                    auth_id: Set(auth_id.clone()),
                    created_at: Set(now),
                }
                .insert(txn)
                .await?;
                reg_count += 1;

                for tool in mcp_tool_entries(setup_board, &env.config) {
                    let key = (
                        variant.to_string(),
                        "mcp_tool".to_string(),
                        String::new(),
                        tool.name.clone(),
                    );
                    if !seen.insert(key.clone()) {
                        tracing::warn!(
                            kind = %key.1, path = %key.3,
                            "duplicate mcp tool registration within setup batch; ignoring later occurrence"
                        );
                        continue;
                    }
                    event_remote_registration::ActiveModel {
                        id: Set(flow_like_types::create_id()),
                        app_id: Set(app_id.to_string()),
                        event_id: Set(event_id.to_string()),
                        event_version: Set(event_version.to_string()),
                        variant: Set(variant.to_string()),
                        kind: Set("mcp_tool".to_string()),
                        method: Set(None),
                        path: Set(tool.name.clone()),
                        node_id: Set(Some(tool.node_id.clone())),
                        schema_json: Set(Some(tool.schema.clone())),
                        extras_json: Set(Some(json!({
                            "name": tool.name,
                            "description": tool.description,
                            "function_ref": tool.node_id,
                        }))),
                        auth_id: Set(auth_id.clone()),
                        created_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    reg_count += 1;
                }

                if let Some(resources) = env.config.get("resources").and_then(|v| v.as_array()) {
                    for resource in resources {
                        let uri = mcp_resource_uri(resource);
                        if uri.is_empty() {
                            tracing::warn!(
                                node_id = %env.node_id,
                                "mcp resource has no uri or flow_path.path; skipping registration row"
                            );
                            continue;
                        }
                        let key = (
                            variant.to_string(),
                            "mcp_resource".to_string(),
                            String::new(),
                            uri.clone(),
                        );
                        if !seen.insert(key.clone()) {
                            tracing::warn!(
                                kind = %key.1, path = %key.3,
                                "duplicate mcp resource registration within setup batch; ignoring later occurrence"
                            );
                            continue;
                        }
                        event_remote_registration::ActiveModel {
                            id: Set(flow_like_types::create_id()),
                            app_id: Set(app_id.to_string()),
                            event_id: Set(event_id.to_string()),
                            event_version: Set(event_version.to_string()),
                            variant: Set(variant.to_string()),
                            kind: Set("mcp_resource".to_string()),
                            method: Set(None),
                            path: Set(uri),
                            node_id: Set(None),
                            schema_json: Set(None),
                            extras_json: Set(Some(resource.clone())),
                            auth_id: Set(auth_id.clone()),
                            created_at: Set(now),
                        }
                        .insert(txn)
                        .await?;
                        reg_count += 1;
                    }
                }

                if let Some(prompts) = env.config.get("prompts").and_then(|v| v.as_array()) {
                    for prompt in prompts {
                        let name = prompt
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_default();
                        if name.is_empty() {
                            tracing::warn!(
                                node_id = %env.node_id,
                                "mcp prompt has no name; skipping registration row"
                            );
                            continue;
                        }
                        let key = (
                            variant.to_string(),
                            "mcp_prompt".to_string(),
                            String::new(),
                            name.clone(),
                        );
                        if !seen.insert(key.clone()) {
                            tracing::warn!(
                                kind = %key.1, path = %key.3,
                                "duplicate mcp prompt registration within setup batch; ignoring later occurrence"
                            );
                            continue;
                        }
                        event_remote_registration::ActiveModel {
                            id: Set(flow_like_types::create_id()),
                            app_id: Set(app_id.to_string()),
                            event_id: Set(event_id.to_string()),
                            event_version: Set(event_version.to_string()),
                            variant: Set(variant.to_string()),
                            kind: Set("mcp_prompt".to_string()),
                            method: Set(None),
                            path: Set(name),
                            node_id: Set(None),
                            schema_json: Set(None),
                            extras_json: Set(Some(prompt.clone())),
                            auth_id: Set(auth_id.clone()),
                            created_at: Set(now),
                        }
                        .insert(txn)
                        .await?;
                        reg_count += 1;
                    }
                }
            }
            other => {
                tracing::warn!(kind = %other, "ignoring unknown server_config kind");
            }
        }
    }

    // Fatal parity gates for a non-stable setup, checked against the stable
    // variant's currently served rows while everything is still uncommitted.
    if variant != STABLE_VARIANT {
        enforce_stable_parity(
            app_id,
            event_id,
            event_version,
            variant,
            stable_setup_version.as_deref(),
            txn,
        )
        .await?;
    }

    // Prune this variant's superseded versions' rows. The previous successful
    // setup — the set inbound traffic is serving until this txn commits —
    // stays protected alongside the one just written. Other variants' buckets
    // are never touched.
    let protected_versions: Vec<&str> = std::iter::once(event_version)
        .chain(live_setup_version.as_deref())
        .collect();
    let pruned_registrations = event_remote_registration::Entity::delete_many()
        .filter(event_remote_registration::Column::AppId.eq(app_id))
        .filter(event_remote_registration::Column::EventId.eq(event_id))
        .filter(event_remote_registration::Column::Variant.eq(variant))
        .filter(
            event_remote_registration::Column::EventVersion
                .is_not_in(protected_versions.iter().copied()),
        )
        .exec(txn)
        .await?;
    let pruned_auths = event_remote_auth::Entity::delete_many()
        .filter(event_remote_auth::Column::AppId.eq(app_id))
        .filter(event_remote_auth::Column::EventId.eq(event_id))
        .filter(event_remote_auth::Column::Variant.eq(variant))
        .filter(
            event_remote_auth::Column::EventVersion.is_not_in(protected_versions.iter().copied()),
        )
        .exec(txn)
        .await?;
    tracing::debug!(
        event_id = %event_id,
        variant = %variant,
        registrations_pruned = pruned_registrations.rows_affected,
        auths_pruned = pruned_auths.rows_affected,
        "pruned remote registrations for superseded event versions"
    );

    // A stale forced setup can commit after a newer version's setup already
    // advanced the pointer: keep its rows (protected above) and refresh the
    // health columns, but never move the serving pointer backward.
    let stale_pointer = matches!(
        (
            super::parse_version_tuple(event_version),
            live_setup_version
                .as_deref()
                .and_then(super::parse_version_tuple),
        ),
        (Some(written), Some(served)) if written < served
    );
    if stale_pointer {
        tracing::warn!(
            event_id = %event_id,
            variant = %variant,
            written_version = %event_version,
            served_version = ?live_setup_version,
            "setup finished for an older event version than the one currently served; rows written, serving pointer not advanced"
        );
        touch_event_setup_status(txn, app_id, event_id, variant, "ok", None).await?;
        if variant == STABLE_VARIANT {
            let now = chrono::Utc::now().fixed_offset();
            event::ActiveModel {
                id: Set(event_id.to_string()),
                setup_status: Set(Some("ok".to_string())),
                last_setup_at: Set(Some(now)),
                last_setup_error: Set(None),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(txn)
            .await?;
        }
    } else {
        // Advance the serving pointer atomically with the rows it names.
        // Outside this txn a failed update could leave the pointer naming a
        // version whose rows the prune above already removed.
        write_event_setup_row(
            txn,
            app_id,
            event_id,
            variant,
            event_version,
            target,
            "ok",
            None,
        )
        .await?;
        // Back-compat pointer, stable only: inbound falls back to
        // `last_setup_version` on deployments whose stable `EventSetup` row
        // has not been backfilled yet, and three read surfaces still consume
        // it.
        if variant == STABLE_VARIANT {
            let now = chrono::Utc::now().fixed_offset();
            event::ActiveModel {
                id: Set(event_id.to_string()),
                setup_status: Set(Some("ok".to_string())),
                last_setup_at: Set(Some(now)),
                last_setup_version: Set(Some(event_version.to_string())),
                last_setup_error: Set(None),
                updated_at: Set(now),
                ..Default::default()
            }
            .update(txn)
            .await?;
        }
    }

    Ok((reg_count, auth_count))
}

/// Persist-phase failure split: a stable-parity refusal is the caller's
/// board to fix and a concurrently deleted variant is a lost race — both
/// surface as a failed setup (400 at the route) — while everything else
/// stays an internal error. `VariantGone` additionally skips the failure
/// marking, which would otherwise re-insert an `EventSetup` row for the
/// deleted variant.
#[derive(Debug)]
enum PersistError {
    Parity(String),
    VariantGone(String),
    Db(sea_orm::DbErr),
    Other(flow_like_types::Error),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Parity(reason) | PersistError::VariantGone(reason) => f.write_str(reason),
            PersistError::Db(error) => write!(f, "{error}"),
            PersistError::Other(error) => write!(f, "{error}"),
        }
    }
}

impl crate::db::AsDbConflict for PersistError {
    fn db_conflict(&self) -> Option<crate::db::DbConflict> {
        match self {
            PersistError::Db(error) => error.db_conflict(),
            _ => None,
        }
    }
}

impl From<sea_orm::DbErr> for PersistError {
    fn from(error: sea_orm::DbErr) -> Self {
        PersistError::Db(error)
    }
}

impl From<flow_like_types::Error> for PersistError {
    fn from(error: flow_like_types::Error) -> Self {
        PersistError::Other(error)
    }
}

/// One variant's inbound surface as the parity gates compare it: resolved
/// auth types per route and the MCP tool names clients see.
struct InboundSurface {
    /// `(kind, path)` → the set of resolved auth type names for that route
    /// (`"none"` when a registration links no auth row).
    auth_types: HashMap<(String, String), BTreeSet<String>>,
    /// `mcp_tool` registration paths — the `tools/list` name set.
    mcp_tool_names: BTreeSet<String>,
}

async fn load_inbound_surface<C: ConnectionTrait>(
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    txn: &C,
) -> flow_like_types::Result<InboundSurface> {
    let auth_type_by_id: HashMap<String, String> = event_remote_auth::Entity::find()
        .filter(event_remote_auth::Column::AppId.eq(app_id))
        .filter(event_remote_auth::Column::EventId.eq(event_id))
        .filter(event_remote_auth::Column::EventVersion.eq(event_version))
        .filter(event_remote_auth::Column::Variant.eq(variant))
        .all(txn)
        .await?
        .into_iter()
        .map(|auth| {
            let auth_type = auth
                .config_json
                .get("type")
                .and_then(|value| value.as_str())
                .map(canonical_rest_auth_type)
                .unwrap_or("untyped")
                .to_string();
            (auth.id, auth_type)
        })
        .collect();

    let mut auth_types: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    let mut mcp_tool_names = BTreeSet::new();
    let registrations = event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(app_id))
        .filter(event_remote_registration::Column::EventId.eq(event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(event_version))
        .filter(event_remote_registration::Column::Variant.eq(variant))
        .all(txn)
        .await?;
    for registration in registrations {
        let resolved = registration
            .auth_id
            .as_deref()
            .map(|auth_id| {
                auth_type_by_id
                    .get(auth_id)
                    .cloned()
                    .unwrap_or_else(|| "untyped".to_string())
            })
            .unwrap_or_else(|| "none".to_string());
        if registration.kind == "mcp_tool" {
            mcp_tool_names.insert(registration.path.clone());
        }
        auth_types
            .entry((registration.kind, registration.path))
            .or_default()
            .insert(resolved);
    }
    Ok(InboundSurface {
        auth_types,
        mcp_tool_names,
    })
}

/// The two fatal gates a non-stable setup must pass against the stable
/// variant's currently served rows, inside the persist transaction: (a) a
/// `(kind, path)` served by both may not resolve to a different auth type —
/// a fraction of callers would suddenly need a different credential with no
/// way to discover it; (b) the MCP tool-name set may not differ — clients
/// cache `tools/list` and would get `-32602` inside a 200. With no stable
/// setup on record there is nothing to diverge from and both gates pass.
async fn enforce_stable_parity<C: ConnectionTrait>(
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    stable_setup_version: Option<&str>,
    txn: &C,
) -> Result<(), PersistError> {
    let Some(stable_version) = stable_setup_version else {
        return Ok(());
    };
    let stable =
        load_inbound_surface(app_id, event_id, stable_version, STABLE_VARIANT, txn).await?;
    let candidate = load_inbound_surface(app_id, event_id, event_version, variant, txn).await?;

    for (key, candidate_types) in &candidate.auth_types {
        let Some(stable_types) = stable.auth_types.get(key) else {
            continue;
        };
        if stable_types != candidate_types {
            let join =
                |types: &BTreeSet<String>| types.iter().cloned().collect::<Vec<_>>().join(", ");
            return Err(PersistError::Parity(format!(
                "variant '{}' resolves a different auth type than stable for {} {}: stable uses [{}], the variant uses [{}] — a canary share must not require a different credential",
                variant,
                key.0,
                key.1,
                join(stable_types),
                join(candidate_types),
            )));
        }
    }

    if candidate.mcp_tool_names != stable.mcp_tool_names {
        let diff = |a: &BTreeSet<String>, b: &BTreeSet<String>| {
            a.difference(b).cloned().collect::<Vec<_>>().join(", ")
        };
        return Err(PersistError::Parity(format!(
            "a canary may not add, remove or rename MCP tools; clients cache tools/list (variant '{}' adds [{}] and removes [{}] relative to stable)",
            variant,
            diff(&candidate.mcp_tool_names, &stable.mcp_tool_names),
            diff(&stable.mcp_tool_names, &candidate.mcp_tool_names),
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn maybe_insert_auth_from_value<C: ConnectionTrait>(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    node_id: &str,
    kind: &str,
    auth: Option<&Value>,
    auth_count: &mut usize,
    now: chrono::DateTime<chrono::FixedOffset>,
    txn: &C,
) -> flow_like_types::Result<Option<String>> {
    // Treat missing, null, plain `"none"`, or `{ "type": "none" }` as "no auth".
    let Some(auth) = auth else { return Ok(None) };
    if auth.is_null() {
        return Ok(None);
    }
    if auth
        .as_str()
        .map(|value| value.eq_ignore_ascii_case("none"))
        .unwrap_or(false)
    {
        return Ok(None);
    }
    if auth.as_object().map(|obj| obj.is_empty()).unwrap_or(false) {
        return Ok(None);
    }
    if auth
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t.eq_ignore_ascii_case("none"))
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let id = flow_like_types::create_id();
    let config_json = protect_auth_config_for_storage(auth, &state.encryption_key);
    event_remote_auth::ActiveModel {
        id: Set(id.clone()),
        app_id: Set(app_id.to_string()),
        event_id: Set(event_id.to_string()),
        event_version: Set(event_version.to_string()),
        variant: Set(variant.to_string()),
        node_id: Set(node_id.to_string()),
        kind: Set(kind.to_string()),
        config_json: Set(config_json),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(txn)
    .await?;
    *auth_count += 1;
    Ok(Some(id))
}

fn protect_auth_config_for_storage(auth: &Value, encryption_key: &[u8; 32]) -> Value {
    let mut protected = auth.clone();
    let Some(obj) = protected.as_object_mut() else {
        return protected;
    };

    if obj
        .get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == "o_auth_bearer")
    {
        obj.insert(
            "type".to_string(),
            Value::String("oauth_bearer".to_string()),
        );
    }

    for field in ["key", "token", "password", "secret"] {
        let Some(value) = obj
            .remove(field)
            .and_then(|value| value.as_str().map(ToString::to_string))
        else {
            continue;
        };
        obj.insert(
            format!("{field}_encrypted"),
            Value::String(encrypt_token(&value, encryption_key)),
        );
    }

    protected
}

fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn rest_file_route_prefix(path: &str) -> Option<String> {
    let path = normalize_route_path(path);
    let prefix = path.strip_suffix("/{filename}")?;
    Some(if prefix.is_empty() {
        "/".to_string()
    } else {
        prefix.to_string()
    })
}

fn normalize_rest_file_mount_path(path: String) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn rest_file_mount_path(path: &str) -> String {
    normalize_rest_file_mount_path(
        rest_file_route_prefix(path).unwrap_or_else(|| normalize_route_path(path)),
    )
}

fn rest_file_is_directory_route(route: &Value) -> bool {
    let raw_path = route.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    route
        .get("directory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || rest_file_route_prefix(raw_path).is_some()
}

fn rest_file_registration_path(route: &Value) -> String {
    let raw_path = route.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    normalize_route_path(raw_path)
}

fn rest_file_openapi_path(route: &Value) -> String {
    let raw_path = route.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    if !rest_file_is_directory_route(route) {
        return normalize_route_path(raw_path);
    }
    let mount = rest_file_mount_path(raw_path);
    if mount == "/" {
        "/{filename}".to_string()
    } else {
        format!("{}/{{filename}}", mount.trim_end_matches('/'))
    }
}

fn rest_file_routes(config: &Value) -> Vec<&Value> {
    ["file_routes", "fileRoutes"]
        .into_iter()
        .filter_map(|key| config.get(key).and_then(|value| value.as_array()))
        .flat_map(|routes| routes.iter())
        .collect()
}

fn mcp_resource_uri(resource: &Value) -> String {
    resource
        .get("uri")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            resource
                .get("flow_path")
                .and_then(|v| v.get("path"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|path| format!("file://{path}"))
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
struct McpSetupToolEntry {
    name: String,
    description: Option<String>,
    schema: Value,
    node_id: String,
}

fn mcp_tool_entries(board: Option<&Board>, config: &Value) -> Vec<McpSetupToolEntry> {
    let Some(board) = board else {
        return Vec::new();
    };
    let function_refs: Vec<String> = config
        .get("function_refs")
        .and_then(|v| v.as_array())
        .map(|refs| {
            refs.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    if function_refs.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut used_names = std::collections::HashSet::new();
    for node_id in function_refs {
        let Some(node) = board.nodes.get(&node_id) else {
            tracing::warn!(
                node_id = %node_id,
                "mcp setup references a function node that is not present on the board"
            );
            continue;
        };
        let (base_name, description, schema) = mcp_tool_metadata(node, &board.refs);
        let mut name = base_name.clone();
        let mut suffix = 2u32;
        while used_names.contains(&name) {
            name = format!("{}_{}", base_name, suffix);
            suffix += 1;
        }
        used_names.insert(name.clone());
        out.push(McpSetupToolEntry {
            name,
            description,
            schema,
            node_id,
        });
    }
    out
}

fn mcp_tool_metadata(
    node: &Node,
    board_refs: &HashMap<String, String>,
) -> (String, Option<String>, Value) {
    let name_source = if node.friendly_name.trim().is_empty() {
        node.name.as_str()
    } else {
        node.friendly_name.as_str()
    };
    let name = sanitize_mcp_identifier(name_source);
    let description = resolved_mcp_description(&node.description, board_refs);
    let has_non_payload_data_pin = node.pins.values().any(|pin| {
        pin.pin_type == PinType::Output
            && pin.data_type != VariableType::Execution
            && pin.name != "payload"
            && pin.name != "_client"
    });

    let mut properties = serde_json::Map::new();
    let mut used_argument_names = std::collections::HashSet::new();
    for pin in node.pins.values() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }
        if pin.name == "_client" || (pin.name == "payload" && has_non_payload_data_pin) {
            continue;
        }
        let argument_name = unique_mcp_tool_argument_name(pin, &used_argument_names);
        used_argument_names.insert(argument_name.clone());
        let schema = pin_schema(
            &pin.data_type,
            &pin.value_type,
            pin.schema
                .as_deref()
                .map(|schema| resolve_mcp_text_ref(schema, board_refs))
                .as_deref(),
            resolved_mcp_description(&pin.description, board_refs)
                .unwrap_or_default()
                .as_str(),
        );
        properties.insert(argument_name, schema);
    }

    (
        name,
        description,
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": true
        }),
    )
}

fn sanitize_mcp_identifier(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            output.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_string();
    if output.is_empty() {
        "function".to_string()
    } else {
        output
    }
}

fn resolve_mcp_text_ref(value: &str, board_refs: &HashMap<String, String>) -> String {
    let trimmed = value.trim();
    if trimmed == "16248035215404677707" {
        return String::new();
    }
    board_refs
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

fn resolved_mcp_description(value: &str, board_refs: &HashMap<String, String>) -> Option<String> {
    let resolved = resolve_mcp_text_ref(value, board_refs);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn unique_mcp_tool_argument_name(pin: &Pin, used: &std::collections::HashSet<String>) -> String {
    let friendly = sanitize_mcp_identifier(pin.friendly_name.trim());
    let raw = sanitize_mcp_identifier(pin.name.trim());
    for candidate in [&friendly, &raw] {
        if !candidate.is_empty() && !used.contains(candidate) {
            return candidate.clone();
        }
    }
    let base = if !friendly.is_empty() {
        friendly
    } else if !raw.is_empty() {
        raw
    } else {
        "arg".to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    while used.contains(&candidate) {
        candidate = format!("{}_{}", base, suffix);
        suffix += 1;
    }
    candidate
}

fn pin_schema(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    description: &str,
) -> Value {
    let mut base = match data_type {
        VariableType::String | VariableType::PathBuf | VariableType::Date => {
            json!({"type": "string"})
        }
        VariableType::Integer | VariableType::Byte => json!({"type": "integer"}),
        VariableType::Float => json!({"type": "number"}),
        VariableType::Boolean => json!({"type": "boolean"}),
        VariableType::Struct | VariableType::Generic => schema
            .and_then(|schema| serde_json::from_str::<Value>(schema).ok())
            .unwrap_or_else(|| json!({"type": "object"})),
        VariableType::Execution => json!({"type": "null"}),
    };
    if let Some(obj) = base.as_object_mut()
        && !description.is_empty()
    {
        obj.insert("description".to_string(), json!(description));
    }
    match value_type {
        ValueType::Array | ValueType::HashSet => json!({"type": "array", "items": base}),
        ValueType::HashMap => json!({"type": "object", "additionalProperties": base}),
        ValueType::Normal => base,
    }
}

#[allow(clippy::too_many_arguments)]
async fn expand_rest_config<C: ConnectionTrait>(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    event_version: &str,
    variant: &str,
    node_id: &str,
    config: &Value,
    auth_count: &mut usize,
    now: chrono::DateTime<chrono::FixedOffset>,
    txn: &C,
) -> flow_like_types::Result<(Vec<event_remote_registration::ActiveModel>, Option<String>)> {
    let auth_id = maybe_insert_auth_from_value(
        state,
        app_id,
        event_id,
        event_version,
        variant,
        node_id,
        "rest",
        config.get("auth"),
        auth_count,
        now,
        txn,
    )
    .await?;

    let mut out: Vec<event_remote_registration::ActiveModel> = Vec::new();

    // function_routes -> rest_fn (one row per (path, method))
    if let Some(routes) = config.get("function_routes").and_then(|v| v.as_array()) {
        for route in routes {
            let path =
                normalize_route_path(route.get("path").and_then(|v| v.as_str()).unwrap_or("/"));
            // Per-route handler node — first entry in `function_refs`.
            // This is what inbound dispatch will use as the start node,
            // NOT the REST-server-config node.
            let handler_node_id = route
                .get("function_refs")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if handler_node_id.is_none() {
                tracing::warn!(
                    server_node_id = %node_id,
                    path = %path,
                    "rest function_route has no function_refs; skipping (no handler to dispatch to)"
                );
                continue;
            }
            let methods: Vec<String> = route
                .get("methods")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.as_str().map(|s| s.to_uppercase()))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vec!["ANY".to_string()]);
            for method in methods {
                out.push(event_remote_registration::ActiveModel {
                    id: Set(flow_like_types::create_id()),
                    app_id: Set(app_id.to_string()),
                    event_id: Set(event_id.to_string()),
                    event_version: Set(event_version.to_string()),
                    variant: Set(variant.to_string()),
                    kind: Set("rest_fn".to_string()),
                    method: Set(Some(method)),
                    path: Set(path.clone()),
                    node_id: Set(handler_node_id.clone()),
                    schema_json: Set(None),
                    extras_json: Set(Some(json!({
                        "route": route,
                        "server_node_id": node_id,
                    }))),
                    auth_id: Set(auth_id.clone()),
                    created_at: Set(now),
                });
            }
        }
    }

    // file_routes -> rest_file
    for route in rest_file_routes(config) {
        let path = rest_file_registration_path(route);
        out.push(event_remote_registration::ActiveModel {
            id: Set(flow_like_types::create_id()),
            app_id: Set(app_id.to_string()),
            event_id: Set(event_id.to_string()),
            event_version: Set(event_version.to_string()),
            variant: Set(variant.to_string()),
            kind: Set("rest_file".to_string()),
            method: Set(Some("GET".to_string())),
            path: Set(path.clone()),
            node_id: Set(Some(node_id.to_string())),
            schema_json: Set(None),
            extras_json: Set(Some(route.clone())),
            auth_id: Set(auth_id.clone()),
            created_at: Set(now),
        });
    }

    // openapi_routes -> rest_openapi
    if let Some(routes) = config.get("openapi_routes").and_then(|v| v.as_array()) {
        let spec = build_rest_openapi_spec(config);
        for route in routes {
            let path = normalize_route_path(
                route
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/openapi.json"),
            );
            let ui_path = match route.get("ui_path") {
                Some(Value::String(path)) => {
                    let path = path.trim();
                    if path.is_empty() {
                        None
                    } else {
                        Some(normalize_route_path(path))
                    }
                }
                Some(Value::Null) => None,
                Some(_) => None,
                None => Some("/docs".to_string()),
            };
            out.push(event_remote_registration::ActiveModel {
                id: Set(flow_like_types::create_id()),
                app_id: Set(app_id.to_string()),
                event_id: Set(event_id.to_string()),
                event_version: Set(event_version.to_string()),
                variant: Set(variant.to_string()),
                kind: Set("rest_openapi".to_string()),
                method: Set(Some("GET".to_string())),
                path: Set(path.clone()),
                node_id: Set(Some(node_id.to_string())),
                schema_json: Set(None),
                extras_json: Set(Some(json!({
                    "route": route,
                    "ui_path": ui_path,
                    "spec": spec,
                }))),
                auth_id: Set(auth_id.clone()),
                created_at: Set(now),
            });

            if let Some(ui_path) = ui_path.as_deref().filter(|ui_path| *ui_path != path) {
                out.push(event_remote_registration::ActiveModel {
                    id: Set(flow_like_types::create_id()),
                    app_id: Set(app_id.to_string()),
                    event_id: Set(event_id.to_string()),
                    event_version: Set(event_version.to_string()),
                    variant: Set(variant.to_string()),
                    kind: Set("rest_openapi_ui".to_string()),
                    method: Set(Some("GET".to_string())),
                    path: Set(ui_path.to_string()),
                    node_id: Set(Some(node_id.to_string())),
                    schema_json: Set(None),
                    extras_json: Set(Some(json!({
                        "route": route,
                        "spec_path": path,
                    }))),
                    auth_id: Set(auth_id.clone()),
                    created_at: Set(now),
                });
            }
        }
    }

    Ok((out, auth_id))
}

/// Build a minimal OpenAPI 3.1 document from a persisted REST server
/// `config`. We don't have board context here (per-pin schemas would
/// require running the flow), so request/response bodies are typed as
/// open `object`. The catalog node produces a richer in-process doc;
/// this one is the authoritative spec that inbound serves remotely.
fn build_rest_openapi_spec(config: &Value) -> Value {
    let mut paths = serde_json::Map::new();

    if let Some(routes) = config.get("function_routes").and_then(|v| v.as_array()) {
        for route in routes {
            let path =
                normalize_route_path(route.get("path").and_then(|v| v.as_str()).unwrap_or("/"));
            let methods: Vec<String> = route
                .get("methods")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.as_str().map(|s| s.to_lowercase()))
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| {
                    vec![
                        "get".to_string(),
                        "post".to_string(),
                        "put".to_string(),
                        "patch".to_string(),
                        "delete".to_string(),
                        "options".to_string(),
                        "head".to_string(),
                    ]
                });
            let entry = paths
                .entry(path.clone())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            let obj = entry.as_object_mut().expect("just inserted as object");
            for method in methods {
                let mut op = json!({
                    "operationId": format!("{}_{}", method, path.trim_start_matches('/').replace('/', "_")),
                    "summary": "REST function route",
                    "responses": {
                        "200": {
                            "description": "OK",
                            "content": {"application/json": {"schema": {"type": "object", "additionalProperties": true}}}
                        }
                    }
                });
                if method != "get" && method != "head" {
                    op["requestBody"] = json!({
                        "required": false,
                        "content": {"application/json": {"schema": {"type": "object", "additionalProperties": true}}}
                    });
                }
                obj.insert(method, op);
            }
        }
    }

    for route in rest_file_routes(config) {
        let directory = rest_file_is_directory_route(route);
        let content_type = route
            .get("content_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let openapi_path = rest_file_openapi_path(route);
        let mut op = json!({
            "operationId": format!("get_{}", openapi_path.trim_start_matches('/').replace('/', "_").replace(['{', '}'], "")),
            "summary": if directory { "Static directory file" } else { "Static file" },
            "responses": {
                "200": {
                    "description": "OK",
                    "content": {
                        content_type: {"schema": {"type": "string", "format": "binary"}}
                    }
                },
                "307": {"description": "Redirect to signed object-store URL"}
            }
        });
        if directory || openapi_path.contains("{filename}") {
            op["parameters"] = json!([{
                "name": "filename",
                "in": "path",
                "required": true,
                "schema": {"type": "string"}
            }]);
        }
        let entry = paths
            .entry(openapi_path)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        entry
            .as_object_mut()
            .expect("just inserted as object")
            .insert("get".to_string(), op);
    }

    let mut doc = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Flow Like REST Server",
            "version": "1.0.0"
        },
        "paths": paths,
    });

    // Reflect the configured auth scheme in `components.securitySchemes`.
    if let Some(auth) = config.get("auth") {
        let auth_type = auth.get("type").and_then(|v| v.as_str()).unwrap_or("none");
        let scheme = match canonical_rest_auth_type(auth_type) {
            "api_key" => Some(json!({
                "type": "apiKey",
                "in": "header",
                "name": auth.get("header").and_then(|v| v.as_str()).unwrap_or("x-api-key")
            })),
            "bearer_token" => Some(json!({"type": "http", "scheme": "bearer"})),
            "basic_auth" => Some(json!({"type": "http", "scheme": "basic"})),
            "oauth_bearer" => {
                Some(json!({"type": "http", "scheme": "bearer", "bearerFormat": "JWT"}))
            }
            "hmac_sha256" => Some(json!({
                "type": "apiKey",
                "in": "header",
                "name": auth.get("signature_header").and_then(|v| v.as_str()).unwrap_or("x-signature"),
                "description": "HMAC-SHA256 signature of `<timestamp>.<body>`"
            })),
            _ => None,
        };
        if let Some(scheme) = scheme {
            doc["components"] = json!({"securitySchemes": {"flowLikeAuth": scheme}});
            doc["security"] = json!([{"flowLikeAuth": []}]);
        }
    }

    doc
}

fn canonical_rest_auth_type(auth_type: &str) -> &str {
    match auth_type {
        "o_auth_bearer" | "oauth_bearer" => "oauth_bearer",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_rest_openapi_spec, is_completed_run_status, rest_file_routes};

    #[test]
    fn completed_run_status_is_case_insensitive() {
        assert!(is_completed_run_status("Completed"));
        assert!(is_completed_run_status("completed"));
        assert!(is_completed_run_status("COMPLETED"));
        assert!(is_completed_run_status(" completed "));
        assert!(!is_completed_run_status("Failed"));
    }

    #[test]
    fn rest_file_routes_feed_remote_openapi_spec() {
        let config = json!({
            "file_routes": [{
                "path": "/assets",
                "flow_path": {
                    "path": "storage/assets",
                    "store_ref": "dirs__storage_test",
                    "cache_store_ref": null
                },
                "directory": true,
                "content_type": "text/plain"
            }]
        });

        assert_eq!(rest_file_routes(&config).len(), 1);

        let spec = build_rest_openapi_spec(&config);
        assert_eq!(
            spec["paths"]["/assets/{filename}"]["get"]["summary"],
            json!("Static directory file")
        );
        assert!(
            spec["paths"]["/assets/{filename}"]["get"]["responses"]["200"]["content"]["text/plain"]
                .is_object()
        );
    }
}

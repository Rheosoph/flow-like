use super::{
    AuthMode, Connection, TeamsPermission, auth, config, config::ManagedConfig, endpoint, guid,
    microsoft::MicrosoftError, now, provision, store,
};
use crate::{
    audit::AuditRecordInput, ensure_fresh_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, RawQuery, State},
    http::header,
    response::{Html, IntoResponse},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use utoipa::ToSchema;

const OPERATION_MS: i64 = 300_000;
const SETUP_WINDOW_MS: i64 = 3_600_000;
const SETUP_LIMIT: u32 = 10;

#[derive(Serialize, ToSchema)]
pub struct TeamsConnectionView {
    /// Whether this server can create Flow-Like-managed bots.
    managed_available: bool,
    /// Whether the bot is connected and its credentials were verified.
    configured: bool,
    /// `not_configured`, `provisioning`, `ready`, `setup_failed`, `disconnecting`,
    /// `disconnect_failed` or `disconnected`.
    status: String,
    /// Messaging endpoint to enter in a customer-managed bot registration.
    endpoint: String,
    connection_id: String,
    mode: Option<AuthMode>,
    name: String,
    description: String,
    customer_tenant_id: String,
    home_tenant_id: String,
    client_id: String,
    secret_expires_at: Option<String>,
    allowed_responders: Vec<String>,
    /// Read access the Teams app requests through resource-specific consent.
    permissions: Vec<TeamsPermission>,
    /// Time of the last verified Teams request, in Unix milliseconds.
    last_activity_at: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamsSetupRequest {
    mode: AuthMode,
    /// Bot name shown in Teams, 1–30 characters.
    name: String,
    #[serde(default)]
    description: String,
    /// Tenant of the Teams organization whose conversations may trigger the event.
    customer_tenant_id: String,
    /// Home tenant of a customer-managed bot identity.
    #[serde(default)]
    home_tenant_id: String,
    /// Application (client) ID of a customer-managed bot.
    #[serde(default)]
    client_id: String,
    /// Client secret value of a customer-managed bot. Leave empty to keep the saved secret.
    #[serde(default)]
    client_secret: String,
    /// Entra user object IDs that may answer approvals. Empty: only the requester.
    #[serde(default)]
    allowed_responders: Vec<String>,
    /// Read access to request through resource-specific consent. Changes take effect after
    /// the updated Teams app is installed.
    #[serde(default)]
    permissions: Vec<TeamsPermission>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamsAccessStatus {
    /// The bot requests no read permissions.
    NotNeeded,
    /// Microsoft issues the bot a Microsoft Graph token for the organization.
    Ready,
    /// An administrator of the organization must approve the bot first.
    ConsentRequired,
    /// The bot is not connected.
    NotConnected,
    /// Microsoft could not confirm access; `message` says why.
    Error,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamsAccessView {
    status: TeamsAccessStatus,
    /// Admin-consent link of a Flow-Like-managed bot, sent with `consent_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    consent_url: Option<String>,
    /// What to do next.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl TeamsAccessView {
    fn of(status: TeamsAccessStatus) -> Self {
        Self {
            status,
            consent_url: None,
            message: None,
        }
    }

    fn with_message(status: TeamsAccessStatus, message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            ..Self::of(status)
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct TeamsPackage {
    filename: String,
    /// The Teams app package (ZIP), base64-encoded.
    base64: String,
}

fn connection_id(app: &str, event: &str) -> String {
    let hash = blake3::hash(format!("teams\0{app}\0{event}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn new_connection(app: &str, event: &str, mode: AuthMode) -> Connection {
    Connection {
        id: connection_id(app, event),
        app_id: app.into(),
        event_id: event.into(),
        mode,
        name: String::new(),
        description: String::new(),
        customer_tenant_id: String::new(),
        home_tenant_id: String::new(),
        client_id: String::new(),
        secret: String::new(),
        graph_object_id: None,
        azure_resource_id: None,
        secret_key_id: None,
        secret_expires_at: None,
        pending_secret_key_ids: vec![],
        status: "not_configured".into(),
        allowed_responders: vec![],
        permissions: vec![],
        operation_until: 0,
    }
}

async fn authorize(
    state: &AppState,
    user: &AppUser,
    app: &str,
    event: &str,
    write: bool,
) -> Result<(), ApiError> {
    ensure_fresh_permission!(
        user,
        app,
        state,
        if write {
            RolePermissions::WriteEvents
        } else {
            RolePermissions::ReadEvents
        }
    );
    let event = crate::routes::app::events::db::get_event_from_db(&state.db, event, app).await?;
    if event.event_type != "teams"
        || event.execution_mode != flow_like::flow::event::EventExecutionMode::Remote
    {
        return Err(ApiError::bad_request(
            "Save this event as a remote Teams bot before configuring it",
        ));
    }
    Ok(())
}

async fn view(
    state: &AppState,
    app: &str,
    event: &str,
    connection: Option<Connection>,
) -> Result<TeamsConnectionView, ApiError> {
    let id = connection_id(app, event);
    let c = connection.as_ref();
    Ok(TeamsConnectionView {
        managed_available: match ManagedConfig::load(&state.secrets).await {
            Ok(config) => config.is_some(),
            Err(error) => {
                tracing::warn!(error = %error, "Teams managed provisioning is unavailable");
                false
            }
        },
        configured: c.is_some_and(|c| c.status == "ready"),
        status: c
            .map(|c| c.status.clone())
            .unwrap_or_else(|| "not_configured".into()),
        endpoint: endpoint(state, &id).await?,
        connection_id: id,
        mode: c.map(|c| c.mode.clone()),
        name: c
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Flow-Like Bot".into()),
        description: c.map(|c| c.description.clone()).unwrap_or_default(),
        customer_tenant_id: c.map(|c| c.customer_tenant_id.clone()).unwrap_or_default(),
        home_tenant_id: c.map(|c| c.home_tenant_id.clone()).unwrap_or_default(),
        client_id: c.map(|c| c.client_id.clone()).unwrap_or_default(),
        secret_expires_at: c.and_then(|c| c.secret_expires_at.clone()),
        allowed_responders: c.map(|c| c.allowed_responders.clone()).unwrap_or_default(),
        permissions: c.map(|c| c.permissions.clone()).unwrap_or_default(),
        last_activity_at: None,
    })
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/teams",
    operation_id = "get_teams_bot",
    tag = "events",
    description = "Show the Microsoft Teams bot of a Teams event: its status, messaging endpoint and last verified activity.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Teams event ID")
    ),
    responses(
        (status = 200, description = "The event's Teams bot", body = TeamsConnectionView),
        (status = 400, description = "The event is not saved as a remote Teams bot"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    authorize(&state, &user, &app, &event, false).await?;
    let mut result = view(
        &state,
        &app,
        &event,
        store::event_connection(&state, &event).await?.map(|c| c.0),
    )
    .await?;
    if let Some((health, _)) =
        store::get::<Value>(&state, &format!("health:{}", result.connection_id)).await?
        && health["client_id"] == result.client_id
        && health["tenant_id"] == result.customer_tenant_id
    {
        result.last_activity_at = health["at"].as_i64();
    }
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/teams/access",
    operation_id = "check_teams_bot_access",
    tag = "events",
    description = "Check whether Microsoft lets the Teams bot read what its permissions request. When an administrator has to approve the bot first, a Flow-Like-managed bot gets an admin-consent link.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Teams event ID")
    ),
    responses(
        (status = 200, description = "The bot's Microsoft Graph access", body = TeamsAccessView),
        (status = 400, description = "The event is not saved as a remote Teams bot"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<TeamsAccessView>, ApiError> {
    authorize(&state, &user, &app, &event, false).await?;
    let connection = store::event_connection(&state, &event).await?.map(|c| c.0);
    Ok(Json(match connection {
        Some(c) if c.app_id != app => return Err(ApiError::FORBIDDEN),
        Some(c) if c.permissions.is_empty() => TeamsAccessView::of(TeamsAccessStatus::NotNeeded),
        Some(c) if c.status == "ready" => graph_access(&state, &c).await,
        Some(_) => TeamsAccessView::of(TeamsAccessStatus::NotConnected),
        None => TeamsAccessView::of(TeamsAccessStatus::NotNeeded),
    }))
}

async fn graph_access(state: &AppState, c: &Connection) -> TeamsAccessView {
    match auth::graph_token(state, c).await {
        Ok(_) => TeamsAccessView::of(TeamsAccessStatus::Ready),
        Err(MicrosoftError::ConsentRequired) => consent_access(state, c).await,
        Err(MicrosoftError::Unavailable(reason) | MicrosoftError::Invalid(reason)) => {
            TeamsAccessView::with_message(TeamsAccessStatus::Error, reason)
        }
        Err(error) => TeamsAccessView::with_message(
            TeamsAccessStatus::Error,
            error
                .into_api("the bot's Microsoft Graph access")
                .public_message()
                .unwrap_or("Microsoft could not confirm the bot's access. Retry shortly."),
        ),
    }
}

async fn consent_access(state: &AppState, c: &Connection) -> TeamsAccessView {
    if c.mode != AuthMode::FlowLikeManaged {
        return TeamsAccessView::with_message(
            TeamsAccessStatus::ConsentRequired,
            "An administrator of your Microsoft 365 organization must approve the bot: in Microsoft Entra ID, open the bot's app registration, choose API permissions, then select Grant admin consent.",
        );
    }
    match provision::ensure_consent_redirect(state, c).await {
        Ok(redirect) => TeamsAccessView {
            consent_url: Some(admin_consent_url(
                &c.customer_tenant_id,
                &c.client_id,
                &redirect,
            )),
            ..TeamsAccessView::with_message(
                TeamsAccessStatus::ConsentRequired,
                "An administrator of your Microsoft 365 organization must approve the bot. Open the consent link and sign in as an administrator.",
            )
        },
        Err(error) => {
            tracing::warn!(connection_id = %c.id, %error, "Could not register the Teams admin-consent redirect");
            TeamsAccessView::with_message(
                TeamsAccessStatus::Error,
                format!(
                    "Microsoft needs an administrator's approval, but Flow-Like could not prepare the consent link. {}",
                    error.public_message().unwrap_or("Retry shortly.")
                ),
            )
        }
    }
}

fn admin_consent_url(tenant: &str, client_id: &str, redirect: &str) -> String {
    format!(
        "https://login.microsoftonline.com/{}/adminconsent?client_id={}&redirect_uri={}",
        urlencoding::encode(tenant),
        urlencoding::encode(client_id),
        urlencoding::encode(redirect)
    )
}

#[utoipa::path(
    get,
    path = "/sink/trigger/teams/{connection_id}/consent",
    operation_id = "teams_admin_consent_result",
    tag = "sink",
    description = "Page Microsoft opens after an administrator answers the admin-consent request of a Flow-Like-managed Teams bot. It shows only whether consent was recorded.",
    params(
        ("connection_id" = String, Path, description = "Teams bot connection ID"),
        ("error" = Option<String>, Query, description = "Set by Microsoft when consent was not granted"),
        ("error_description" = Option<String>, Query, description = "Microsoft's reason. Never shown on the page.")
    ),
    responses(
        (status = 200, description = "Static page saying whether admin consent was recorded", content_type = "text/html", body = String)
    )
)]
pub async fn consent(RawQuery(query): RawQuery) -> impl IntoResponse {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
            ),
        ],
        Html(consent_page(!consent_failed(query.as_deref()))),
    )
}

fn consent_failed(query: Option<&str>) -> bool {
    query
        .unwrap_or_default()
        .split('&')
        .any(|pair| matches!(pair.split('=').next(), Some("error" | "error_description")))
}

/// Static text only: Microsoft's query values are never rendered.
fn consent_page(recorded: bool) -> String {
    let (title, text) = if recorded {
        (
            "Admin consent recorded",
            "You can close this tab and check access again in Flow-Like.",
        )
    } else {
        (
            "Admin consent was not granted",
            "Microsoft did not record consent for the bot. Close this tab and check access again in Flow-Like, or ask an administrator of your organization to approve the bot.",
        )
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title><style>body{{margin:0;min-height:100vh;display:grid;place-items:center;font-family:system-ui,sans-serif;background:#f7f7f8;color:#1f1f23}}@media (prefers-color-scheme:dark){{body{{background:#131316;color:#ececf1}}}}main{{max-width:30rem;padding:2rem;text-align:center;line-height:1.5}}</style></head><body><main><h1>{title}</h1><p>{text}</p></main></body></html>"#
    )
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/teams",
    operation_id = "setup_teams_bot",
    tag = "events",
    description = "Connect or update the Microsoft Teams bot of a Teams event. New credentials are verified with Microsoft before they replace the saved ones, and edits to a connected bot keep it connected.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Teams event ID")
    ),
    request_body = TeamsSetupRequest,
    responses(
        (status = 200, description = "The bot is connected", body = TeamsConnectionView),
        (status = 400, description = "Invalid settings, or Microsoft rejected the credentials"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden, or Teams bots are disabled on this server"),
        (status = 409, description = "Another Teams operation is running or the bot changed meanwhile"),
        (status = 422, description = "The app reached its Flow-Like-managed bot limit"),
        (status = 429, description = "Too many Flow-Like-managed bot setups for this app"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn setup(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
    Json(input): Json<TeamsSetupRequest>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    authorize(&state, &user, &app, &event, true).await?;
    super::ensure_enabled(&state)?;
    let (customer_tenant_id, allowed_responders, permissions) = validate_setup(&input)?;
    if input.mode == AuthMode::FlowLikeManaged
        && ManagedConfig::load(&state.secrets).await?.is_none()
    {
        return Err(ApiError::bad_request(
            "Flow-Like-managed bots are not enabled on this server. Choose a customer-managed bot or ask your administrator.",
        ));
    }
    let (previous, revision) = match store::event_connection(&state, &event).await? {
        Some((c, revision)) => {
            ensure_editable(&c, &app, &input.mode)?;
            (Some(c), revision)
        }
        None => (None, 0),
    };
    let mut c = previous
        .clone()
        .unwrap_or_else(|| new_connection(&app, &event, input.mode.clone()));
    c.name = input.name.trim().into();
    c.description = input.description.clone();
    c.customer_tenant_id = customer_tenant_id;
    c.allowed_responders = allowed_responders;
    c.permissions = permissions;
    c.mode = input.mode.clone();
    let c = if c.mode == AuthMode::FlowLikeManaged {
        setup_managed(&state, previous, c, revision).await?
    } else {
        setup_customer(&state, previous, c, revision, &input).await?
    };
    crate::audit_branch!(state, user, app, "event.teams.setup", "Event", event);
    Ok(Json(view(&state, &app, &event, Some(c)).await?))
}

/// Manifest order. The exhaustive match makes a new permission choose its place.
fn permission_rank(permission: TeamsPermission) -> u8 {
    match permission {
        TeamsPermission::ReadMessages => 0,
        TeamsPermission::MeetingDetails => 1,
        TeamsPermission::ConversationDetails => 2,
    }
}

fn canonical_permissions(permissions: &[TeamsPermission]) -> Vec<TeamsPermission> {
    let mut permissions = permissions.to_vec();
    permissions.sort_by_key(|permission| permission_rank(*permission));
    permissions.dedup();
    permissions
}

/// Returns the normalized organization tenant, approver IDs and permissions.
fn validate_setup(
    input: &TeamsSetupRequest,
) -> Result<(String, Vec<String>, Vec<TeamsPermission>), ApiError> {
    if input.name.trim().is_empty()
        || input.name.chars().count() > 30
        || input.description.chars().count() > 4000
        || input.client_secret.len() > 4096
        || input.allowed_responders.len() > 100
    {
        return Err(ApiError::bad_request(
            "Use a bot name of 1–30 characters, a description up to 4000 characters, and at most 100 approvers",
        ));
    }
    let responders = input
        .allowed_responders
        .iter()
        .map(|id| guid(id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        guid(&input.customer_tenant_id)?,
        responders,
        canonical_permissions(&input.permissions),
    ))
}

fn ensure_editable(c: &Connection, app: &str, mode: &AuthMode) -> Result<(), ApiError> {
    if c.app_id != app {
        return Err(ApiError::FORBIDDEN);
    }
    if c.operation_until > now() {
        return Err(ApiError::conflict(
            "Teams setup is already running. Refresh its status shortly.",
        ));
    }
    if c.status != "disconnected" && &c.mode != mode {
        return Err(ApiError::conflict(
            "Disconnect the existing bot before changing its management mode",
        ));
    }
    Ok(())
}

async fn persist_setup(
    state: &AppState,
    previous: Option<&Connection>,
    c: &Connection,
    revision: i32,
) -> Result<i32, ApiError> {
    if previous.is_some() {
        store::save_connection(state, c, revision).await?;
        return Ok(revision + 1);
    }
    if !store::insert_connection(state, c).await? {
        return Err(ApiError::conflict(
            "Teams setup is already running. Refresh its status.",
        ));
    }
    Ok(0)
}

/// A customer bot changes credentials only after Microsoft issued a token for the candidate,
/// so a failed edit leaves the saved bot and its status untouched.
async fn setup_customer(
    state: &AppState,
    previous: Option<Connection>,
    mut c: Connection,
    revision: i32,
    input: &TeamsSetupRequest,
) -> Result<Connection, ApiError> {
    apply_customer_credentials(&mut c, input)?;
    let verified = previous.as_ref().is_some_and(|p| {
        p.status == "ready"
            && p.home_tenant_id == c.home_tenant_id
            && p.client_id == c.client_id
            && p.secret == c.secret
    });
    if !verified {
        auth::bot_token(state, &c).await?;
    }
    c.status = "ready".into();
    c.operation_until = 0;
    persist_setup(state, previous.as_ref(), &c, revision).await?;
    Ok(c)
}

/// A secret is required for a new bot identity; an empty one keeps the saved secret.
fn apply_customer_credentials(
    c: &mut Connection,
    input: &TeamsSetupRequest,
) -> Result<(), ApiError> {
    let home = guid(&input.home_tenant_id)?;
    let id = guid(&input.client_id)?;
    let same_identity = c.home_tenant_id == home && c.client_id == id;
    if input.client_secret.is_empty() && (c.secret.is_empty() || !same_identity) {
        return Err(ApiError::bad_request(
            "Enter the client secret value for this bot",
        ));
    }
    c.home_tenant_id = home;
    c.client_id = id;
    if !input.client_secret.is_empty() {
        c.secret = input.client_secret.clone();
    }
    Ok(())
}

async fn setup_managed(
    state: &AppState,
    previous: Option<Connection>,
    mut c: Connection,
    revision: i32,
) -> Result<Connection, ApiError> {
    if let Some(ready) = previous
        .as_ref()
        .filter(|p| p.status == "ready" && p.mode == AuthMode::FlowLikeManaged)
    {
        return update_managed(state, ready, c, revision).await;
    }
    let limit = config::managed_bot_limit(&state.secrets).await?;
    if store::managed_bots(state, &c.app_id, &c.id).await? >= limit {
        return Err(ApiError::unprocessable(format!(
            "This app already has {limit} Flow-Like-managed Teams bots. Disconnect one, or connect a customer-managed bot instead."
        )));
    }
    consume_setup_budget(state, &c.app_id).await?;
    c.status = "provisioning".into();
    c.operation_until = now() + OPERATION_MS;
    let mut revision = persist_setup(state, previous.as_ref(), &c, revision).await?;
    let result = provision::provision(state, &mut c, &mut revision).await;
    c.operation_until = 0;
    c.status = if result.is_ok() {
        "ready"
    } else {
        "setup_failed"
    }
    .into();
    store::save_connection(state, &c, revision).await?;
    result?;
    Ok(c)
}

/// A connected managed bot stays `ready` while its name or description is updated in Azure;
/// if Azure refuses, the previous settings are kept.
async fn update_managed(
    state: &AppState,
    ready: &Connection,
    c: Connection,
    revision: i32,
) -> Result<Connection, ApiError> {
    if ready.name == c.name && ready.description == c.description {
        store::save_connection(state, &c, revision).await?;
        return Ok(c);
    }
    consume_setup_budget(state, &c.app_id).await?;
    let mut lease = ready.clone();
    lease.operation_until = now() + OPERATION_MS;
    store::save_connection(state, &lease, revision).await?;
    let result = provision::update_bot(state, &c).await;
    let mut saved = if result.is_ok() { c } else { lease };
    saved.operation_until = 0;
    store::save_connection(state, &saved, revision + 1).await?;
    result?;
    Ok(saved)
}

/// Managed setups create Microsoft resources, so each app gets a fixed hourly budget.
async fn consume_setup_budget(state: &AppState, app: &str) -> Result<(), ApiError> {
    let key = format!("setup-rate:{app}");
    for _ in 0..4 {
        let saved = match store::get::<u32>(state, &key).await? {
            Some((count, _)) if count >= SETUP_LIMIT => {
                return Err(ApiError::too_many_requests(format!(
                    "This app may start at most {SETUP_LIMIT} Flow-Like-managed bot setups per hour. Retry later."
                )));
            }
            Some((count, revision)) => store::update(state, &key, &(count + 1), revision).await?,
            None => {
                store::insert(
                    state,
                    &key,
                    &format!("app:{app}"),
                    &1u32,
                    now() + SETUP_WINDOW_MS,
                )
                .await?
            }
        };
        if saved {
            return Ok(());
        }
    }
    Err(ApiError::conflict("Teams setup is busy. Retry shortly."))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/teams/rotate",
    operation_id = "rotate_teams_bot_secret",
    tag = "events",
    description = "Renew the client secret of a Flow-Like-managed Teams bot. The current secret keeps working until the new one is verified.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Teams event ID")
    ),
    responses(
        (status = 200, description = "The secret was renewed", body = TeamsConnectionView),
        (status = 400, description = "The bot is customer-managed or not set up"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "The event has no Teams bot"),
        (status = 409, description = "Another Teams operation is running"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn rotate(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    authorize(&state, &user, &app, &event, true).await?;
    let (mut c, mut revision) = store::event_connection(&state, &event)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if c.app_id != app {
        return Err(ApiError::FORBIDDEN);
    }
    if c.mode != AuthMode::FlowLikeManaged {
        return Err(ApiError::bad_request(
            "Create a secret in your Entra app, then save it in the bot setup",
        ));
    }
    if c.operation_until > now() {
        return Err(ApiError::conflict("Another Teams operation is running"));
    }
    c.operation_until = now() + OPERATION_MS;
    store::save_connection(&state, &c, revision).await?;
    revision += 1;
    let result = provision::rotate(&state, &mut c, &mut revision).await;
    c.operation_until = 0;
    store::save_connection(&state, &c, revision).await?;
    result?;
    crate::audit_branch!(state, user, app, "event.teams.rotate", "Event", event);
    Ok(Json(view(&state, &app, &event, Some(c)).await?))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/events/{event_id}/teams",
    operation_id = "disconnect_teams_bot",
    tag = "events",
    description = "Disconnect the Microsoft Teams bot of an event. A Flow-Like-managed bot's Microsoft registration is deleted; a customer-managed registration stays in the customer's account.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "The bot is disconnected", body = TeamsConnectionView),
        (status = 400, description = "Microsoft refused to remove the registration"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "The event has no Teams bot"),
        (status = 409, description = "Another Teams operation is running"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn disconnect(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    // Keep cleanup available after an event changes type.
    ensure_fresh_permission!(user, &app, &state, RolePermissions::WriteEvents);
    let (mut c, mut revision) = store::event_connection(&state, &event)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if c.app_id != app {
        return Err(ApiError::FORBIDDEN);
    }
    disconnect_connection(&state, &mut c, &mut revision).await?;
    crate::audit_branch!(state, user, app, "event.teams.disconnect", "Event", event);
    Ok(Json(view(&state, &app, &event, Some(c)).await?))
}

async fn disconnect_connection(
    state: &AppState,
    c: &mut Connection,
    revision: &mut i32,
) -> Result<(), ApiError> {
    if c.status == "disconnected" {
        return Ok(());
    }
    if c.operation_until > now() {
        return Err(ApiError::conflict("Another Teams operation is running"));
    }
    // Any setup attempt may have created an application before its id was persisted.
    let recover = c.status != "not_configured";
    c.status = "disconnecting".into();
    c.operation_until = now() + OPERATION_MS;
    store::save_connection(state, c, *revision).await?;
    *revision += 1;
    let result = if c.mode == AuthMode::FlowLikeManaged {
        provision::remove(state, c, recover).await
    } else {
        Ok(())
    };
    c.operation_until = 0;
    if result.is_ok() {
        c.secret.clear();
        c.client_id.clear();
        c.graph_object_id = None;
        c.azure_resource_id = None;
        c.secret_key_id = None;
        c.pending_secret_key_ids.clear();
        c.secret_expires_at = None;
        c.status = "disconnected".into();
    } else {
        c.status = "disconnect_failed".into();
    }
    store::save_connection(state, c, *revision).await?;
    result?;
    Ok(())
}

/// Deleting an event or app never waits on Microsoft: when the registration cannot be
/// removed, the local connection is still deleted and the orphan is logged and audited
/// with the ids needed to remove it by hand.
pub(crate) async fn cleanup_event(state: &AppState, event: &str) -> Result<(), ApiError> {
    let Some((mut c, mut revision)) = store::event_connection(state, event).await? else {
        return Ok(());
    };
    if let Err(error) = disconnect_connection(state, &mut c, &mut revision).await {
        let details = json!({
            "connection_id": c.id,
            "mode": c.mode,
            "home_tenant_id": c.home_tenant_id,
            "client_id": c.client_id,
            "graph_object_id": c.graph_object_id,
            "azure_resource_id": c.azure_resource_id,
            "recovery_tag": provision::recovery_tag(&c.id),
            "error": error.to_string(),
        });
        tracing::error!(app_id = %c.app_id, event_id = %c.event_id, %details, "Orphaned Teams bot registration: Microsoft cleanup failed while its event was deleted");
        crate::audit::record_entry(
            state,
            AuditRecordInput::system("teams", "event.teams.orphaned", "Event", &c.event_id)
                .on_scope(&c.app_id)
                .with_details(details),
        )
        .await;
    }
    store::remove_connection(state, &c.id).await
}

/// Changing a Teams event's type would strand its bot, so a connected bot must be
/// disconnected first. Only saved Teams events can own a connection.
pub(crate) async fn validate_event_type(
    state: &AppState,
    saved: Option<&flow_like::flow::event::Event>,
    event: &flow_like::flow::event::Event,
) -> Result<(), ApiError> {
    if event.event_type == "teams" || saved.is_none_or(|saved| saved.event_type != "teams") {
        return Ok(());
    }
    if let Some((c, _)) = store::event_connection(state, &event.id).await?
        && c.status != "disconnected"
    {
        return Err(ApiError::conflict(
            "Disconnect the Teams bot before changing this event type",
        ));
    }
    Ok(())
}

fn icon(size: u32, outline: bool) -> Result<Vec<u8>, ApiError> {
    let mut image = image::RgbaImage::from_pixel(
        size,
        size,
        if outline {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([91, 83, 174, 255])
        },
    );
    for y in size / 4..size * 3 / 4 {
        for x in size / 5..size * 4 / 5 {
            if !outline
                || x < size / 5 + 2
                || x >= size * 4 / 5 - 2
                || y < size / 4 + 2
                || y >= size * 3 / 4 - 2
            {
                image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|_| ApiError::internal("Cannot create Teams icon"))?;
    Ok(out.into_inner())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/teams/package",
    operation_id = "download_teams_app_package",
    tag = "events",
    description = "Download the Teams app package (manifest and icons) to upload in Microsoft Teams. It contains no credentials.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Teams event ID")
    ),
    responses(
        (status = 200, description = "The Teams app package", body = TeamsPackage),
        (status = 400, description = "Bot setup is not complete"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "The event has no Teams bot")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<TeamsPackage>, ApiError> {
    authorize(&state, &user, &app, &event, false).await?;
    let (c, revision) = store::event_connection(&state, &event)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if c.app_id != app {
        return Err(ApiError::FORBIDDEN);
    }
    if c.status != "ready" {
        return Err(ApiError::bad_request(
            "Complete bot setup before downloading the Teams app",
        ));
    }
    let filename = package_filename(&c.name);
    let bytes = app_package(c, revision)?;
    Ok(Json(TeamsPackage {
        filename,
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }))
}

fn package_filename(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    match slug.trim_end_matches('-') {
        "" => "flow-like-teams.zip".into(),
        slug => format!("{slug}.zip"),
    }
}

/// Resource-specific consent entries for the manifest's `authorization.permissions`.
fn resource_specific(permissions: &[TeamsPermission]) -> Vec<Value> {
    canonical_permissions(permissions)
        .into_iter()
        .flat_map(TeamsPermission::rsc)
        .map(|name| json!({"name":name,"type":"Application"}))
        .collect()
}

fn app_package(c: Connection, revision: i32) -> Result<Vec<u8>, ApiError> {
    let mut manifest = json!({
        "$schema":"https://developer.microsoft.com/json-schemas/teams/v1.21/MicrosoftTeams.schema.json",
        "manifestVersion":"1.21","version":format!("1.0.{}",revision.max(0)),"id":c.id,
        "developer":{"name":"Flow-Like","websiteUrl":"https://flow-like.com","privacyUrl":"https://flow-like.com/privacy-policy","termsOfUseUrl":"https://flow-like.com/eula"},
        "name":{"short":c.name,"full":c.name},
        "description":{"short":if c.description.is_empty(){c.name.clone()}else{c.description.chars().take(80).collect::<String>()},"full":if c.description.is_empty(){format!("{} powered by Flow-Like",c.name)}else{c.description}},
        "icons":{"color":"color.png","outline":"outline.png"},"accentColor":"#5B53AE",
        "bots":[{"botId":c.client_id,"scopes":["personal","team","groupchat"],"isNotificationOnly":false,"supportsFiles":true}],
        "validDomains":[]
    });
    if !c.permissions.is_empty() {
        manifest["webApplicationInfo"] =
            json!({"id":c.client_id,"resource":format!("api://botid-{}",c.client_id)});
        manifest["authorization"] =
            json!({"permissions":{"resourceSpecific":resource_specific(&c.permissions)}});
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let files = [
        (
            "manifest.json",
            serde_json::to_vec_pretty(&manifest)
                .map_err(|_| ApiError::internal("Cannot encode Teams manifest"))?,
        ),
        ("color.png", icon(192, false)?),
        ("outline.png", icon(32, true)?),
    ];
    for (name, bytes) in files {
        zip.start_file(name, options)
            .map_err(|_| ApiError::internal("Cannot package Teams app"))?;
        zip.write_all(&bytes)
            .map_err(|_| ApiError::internal("Cannot package Teams app"))?;
    }
    let bytes = zip
        .finish()
        .map_err(|_| ApiError::internal("Cannot finish Teams package"))?
        .into_inner();
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use TeamsPermission::*;

    fn packaged_connection() -> Connection {
        serde_json::from_value(json!({
            "id":"12345678-1234-1234-1234-123456789012", "app_id":"app", "event_id":"event", "mode":"customer_teams",
            "name":"Support", "description":"Support in Teams", "customer_tenant_id":"tenant", "home_tenant_id":"home",
            "client_id":"22345678-1234-1234-1234-123456789012", "secret":"NEVER-IN-PACKAGE", "status":"ready", "allowed_responders":[]
        })).unwrap()
    }

    fn packaged_manifest(c: Connection) -> Value {
        let bytes = app_package(c, 3).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap()
    }

    fn setup_request(permissions: Value) -> TeamsSetupRequest {
        serde_json::from_value(json!({
            "mode":"customer_teams", "name":"Support",
            "customer_tenant_id":"32345678-1234-1234-1234-123456789012",
            "permissions":permissions
        }))
        .unwrap()
    }

    #[test]
    fn package_contains_installable_metadata_and_icons_without_credentials() {
        let bytes = app_package(packaged_connection(), 12).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("NEVER-IN-PACKAGE"));
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 3);
        let manifest: Value =
            serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["version"], "1.0.12");
        assert_eq!(
            manifest["bots"][0]["scopes"],
            json!(["personal", "team", "groupchat"])
        );
        assert_eq!(
            manifest["bots"][0]["botId"],
            "22345678-1234-1234-1234-123456789012"
        );
        assert_eq!(manifest["bots"][0]["supportsFiles"], true);
        assert!(manifest.get("webApplicationInfo").is_none());
        assert!(manifest.get("authorization").is_none());
        for (name, size) in [("color.png", 192), ("outline.png", 32)] {
            use std::io::Read;
            let mut bytes = Vec::new();
            archive
                .by_name(name)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            let image =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
            assert_eq!((image.width(), image.height()), (size, size));
        }
    }

    #[test]
    fn package_declares_resource_specific_consent_in_permission_order() {
        let mut c = packaged_connection();
        c.permissions = vec![
            ConversationDetails,
            ReadMessages,
            MeetingDetails,
            ReadMessages,
        ];
        let manifest = packaged_manifest(c);
        assert_eq!(
            manifest["webApplicationInfo"],
            json!({
                "id":"22345678-1234-1234-1234-123456789012",
                "resource":"api://botid-22345678-1234-1234-1234-123456789012"
            })
        );
        assert_eq!(
            manifest["authorization"],
            json!({"permissions":{"resourceSpecific":[
                {"name":"ChannelMessage.Read.Group","type":"Application"},
                {"name":"ChatMessage.Read.Chat","type":"Application"},
                {"name":"OnlineMeeting.ReadBasic.Chat","type":"Application"},
                {"name":"ChannelMeeting.ReadBasic.Group","type":"Application"},
                {"name":"TeamSettings.Read.Group","type":"Application"},
                {"name":"ChannelSettings.Read.Group","type":"Application"},
                {"name":"ChatSettings.Read.Chat","type":"Application"}
            ]}})
        );

        let mut c = packaged_connection();
        c.permissions = vec![MeetingDetails];
        assert_eq!(
            packaged_manifest(c)["authorization"]["permissions"]["resourceSpecific"],
            json!([
                {"name":"OnlineMeeting.ReadBasic.Chat","type":"Application"},
                {"name":"ChannelMeeting.ReadBasic.Group","type":"Application"}
            ])
        );
    }

    #[test]
    fn setup_permissions_are_optional_deduplicated_and_ordered() {
        let (_, _, permissions) = validate_setup(&setup_request(json!([
            "conversation_details",
            "read_messages",
            "conversation_details"
        ])))
        .unwrap();
        assert_eq!(permissions, vec![ReadMessages, ConversationDetails]);

        let request: TeamsSetupRequest = serde_json::from_value(json!({
            "mode":"customer_teams", "name":"Support",
            "customer_tenant_id":"32345678-1234-1234-1234-123456789012"
        }))
        .unwrap();
        assert!(validate_setup(&request).unwrap().2.is_empty());
        assert!(
            serde_json::from_value::<TeamsSetupRequest>(json!({
                "mode":"customer_teams", "name":"Support", "customer_tenant_id":"t",
                "permissions":["write_messages"]
            }))
            .is_err()
        );
        assert!(
            new_connection("app", "event", AuthMode::CustomerTeams)
                .permissions
                .is_empty()
        );
    }

    #[test]
    fn access_views_omit_what_does_not_apply() {
        assert_eq!(
            serde_json::to_value(TeamsAccessView::of(TeamsAccessStatus::NotNeeded)).unwrap(),
            json!({"status":"not_needed"})
        );
        let view = TeamsAccessView {
            consent_url: Some("https://login.microsoftonline.com/t/adminconsent".into()),
            ..TeamsAccessView::with_message(TeamsAccessStatus::ConsentRequired, "Approve it.")
        };
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            json!({
                "status":"consent_required",
                "consent_url":"https://login.microsoftonline.com/t/adminconsent",
                "message":"Approve it."
            })
        );
    }

    #[test]
    fn admin_consent_url_targets_the_customer_tenant_and_encodes_the_redirect() {
        assert_eq!(
            admin_consent_url(
                "32345678-1234-1234-1234-123456789012",
                "22345678-1234-1234-1234-123456789012",
                "https://api.flow-like.com/api/v1/sink/trigger/teams/c/consent"
            ),
            "https://login.microsoftonline.com/32345678-1234-1234-1234-123456789012/adminconsent?client_id=22345678-1234-1234-1234-123456789012&redirect_uri=https%3A%2F%2Fapi.flow-like.com%2Fapi%2Fv1%2Fsink%2Ftrigger%2Fteams%2Fc%2Fconsent"
        );
    }

    #[tokio::test]
    async fn consent_page_is_static_uncached_html_that_never_echoes_the_query() {
        use axum::{body::to_bytes, routing::get};
        use tower::ServiceExt;

        let router = axum::Router::new().route("/consent", get(consent));
        for (query, recorded) in [
            ("", true),
            ("?admin_consent=True&tenant=t", true),
            (
                "?error=access_denied&error_description=%3Cscript%3Ealert(1)%3C%2Fscript%3E",
                false,
            ),
            ("?error_description=%3Cb%3Einjected%3C%2Fb%3E", false),
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/consent{query}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let headers = response.headers();
            assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body.contains("Admin consent recorded"), recorded, "{query}");
            assert!(!body.contains("script") && !body.contains("injected"));
            assert!(!body.contains("access_denied"));
        }
        assert!(!consent_failed(Some("admin_consent=True&errors=0")));
    }

    #[test]
    fn package_filename_is_a_slug_of_the_bot_name() {
        assert_eq!(package_filename("Support Bot"), "support-bot.zip");
        assert_eq!(
            package_filename("  HR / Payroll -- Q&A! "),
            "hr-payroll-q-a.zip"
        );
        assert_eq!(package_filename("Zoë's Bot"), "zo-s-bot.zip");
        assert_eq!(package_filename("日本"), "flow-like-teams.zip");
        assert_eq!(package_filename("../../etc"), "etc.zip");
    }

    #[test]
    fn only_active_managed_bots_hold_a_quota_slot() {
        let mut c = new_connection("app", "event", AuthMode::FlowLikeManaged);
        assert!(!c.holds_managed_slot());
        for status in ["provisioning", "ready", "setup_failed", "disconnect_failed"] {
            c.status = status.into();
            assert!(c.holds_managed_slot(), "{status}");
        }
        c.status = "disconnected".into();
        assert!(!c.holds_managed_slot());
        c.status = "ready".into();
        c.mode = AuthMode::CustomerTeams;
        assert!(!c.holds_managed_slot());
    }
}

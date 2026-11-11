//! A bot's connectors: the MCP servers and OAuth accounts its owner attached. Owner only; the
//! bot row is checked in SQL on every call. The account grant is stored sealed and never
//! returned. Refreshing a server's tool list is network work, so it runs as a job on the
//! executor like a bot turn's connector calls.

use std::collections::BTreeMap;
use std::time::Duration;

use super::{lock_bot, owned_bot, workspace};
use crate::{
    bot_tools::connectors::{
        AUTH_NONE, AUTH_OAUTH, AUTH_SECRET, authorization_of, entry_of, seal_grant, to_contract,
    },
    entity::{bot_connector, bot_secret},
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::Utc;
use flow_like_bot_tools_host::{JobConnector, JobScope, ToolJob};
use flow_like_types::bot_connector::{
    BotConnector, BotConnectorList, ConnectorAuth, ConnectorKind, ConnectorPolicy,
    ConnectorToolInfo, CreateBotConnector, MAX_CONNECTORS_PER_BOT, MAX_SCOPES, MAX_TOOLS_SNAPSHOT,
    MAX_URL_CHARS, UpdateBotConnector, validate_label, validate_tool_policies,
};
use flow_like_types::data_classification::DataClassification;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter,
};
use serde::Serialize;
use utoipa::ToSchema;

const LIMIT_REACHED: &str = "BOT_CONNECTOR_LIMIT_REACHED";
const HANDLE_PREFIX: &str = "fl_oauth_";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(90);

/// Documentation shape of [`BotConnector`]; the wire format is the contract type's.
#[derive(Serialize, ToSchema)]
pub struct BotConnectorSchema {
    pub id: String,
    pub bot_id: String,
    /// `oauth` or `mcp`.
    pub kind: String,
    pub label: String,
    pub provider_id: Option<String>,
    pub url: Option<String>,
    #[schema(value_type = Object)]
    pub auth: serde_json::Value,
    /// A sealed grant is stored; the grant itself is never returned.
    pub has_grant: bool,
    pub scopes: Vec<String>,
    /// `read`, `ask` or `never`.
    pub default_policy: String,
    #[schema(value_type = Object)]
    pub tool_policies: serde_json::Value,
    #[schema(value_type = Vec<Object>)]
    pub tools_snapshot: serde_json::Value,
    pub updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct BotConnectorListSchema {
    pub connectors: Vec<BotConnectorSchema>,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request(message.into())
}

fn url_of(raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim();
    let url = flow_like_types::reqwest::Url::parse(raw)
        .map_err(|error| invalid(format!("The server URL is invalid: {error}")))?;
    if raw.chars().count() > MAX_URL_CHARS
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(invalid(
            "The server URL must be an https URL without a user name or password",
        ));
    }
    Ok(url.to_string())
}

fn grant_of(grant: Option<&str>) -> Result<Option<String>, ApiError> {
    match grant.map(str::trim).filter(|grant| !grant.is_empty()) {
        Some(handle) if handle.starts_with(HANDLE_PREFIX) && handle.len() < 200 => {
            Ok(Some(handle.to_string()))
        }
        Some(_) => Err(invalid(
            "Connect the account again: only a stored sign-in handle can be attached",
        )),
        None => Ok(None),
    }
}

fn scopes_of(scopes: Vec<String>) -> Result<Vec<String>, ApiError> {
    if scopes.len() > MAX_SCOPES || scopes.iter().any(|scope| scope.len() > 300) {
        return Err(invalid(format!(
            "At most {MAX_SCOPES} scopes of 300 characters"
        )));
    }
    Ok(scopes)
}

pub(crate) async fn owned_connector<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    connector_id: &str,
) -> Result<bot_connector::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    bot_connector::Entity::find_by_id(connector_id)
        .filter(bot_connector::Column::BotId.eq(bot_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Connector '{connector_id}' not found")))
}

fn info(row: bot_connector::Model) -> Result<BotConnector, ApiError> {
    let id = row.id.clone();
    to_contract(&row).ok_or_else(|| {
        ApiError::internal(format!(
            "Connector '{id}' was stored with an unknown kind or policy"
        ))
    })
}

async fn label_free<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    label: &str,
    except: Option<&str>,
) -> Result<(), ApiError> {
    let taken = bot_connector::Entity::find()
        .filter(bot_connector::Column::BotId.eq(bot_id))
        .all(db)
        .await?
        .iter()
        .any(|row| row.label.eq_ignore_ascii_case(label) && Some(row.id.as_str()) != except);
    if taken {
        return Err(ApiError::conflict(format!(
            "The bot already has a connector called '{label}'"
        )));
    }
    Ok(())
}

async fn secret_exists<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    name: &str,
) -> Result<bool, ApiError> {
    Ok(
        bot_secret::Entity::find_by_id((bot_id.to_string(), name.to_string()))
            .one(db)
            .await?
            .is_some(),
    )
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/connectors",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The MCP servers and accounts attached to your bot, with their policies and tool lists; never a sign-in", body = BotConnectorListSchema),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/connectors", skip(state, user))]
pub async fn list_bot_connectors(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotConnectorList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let connectors = crate::bot_tools::connectors::rows_of(&state, &bot_id)
        .await?
        .iter()
        .filter_map(to_contract)
        .collect();
    Ok(Json(BotConnectorList { connectors }))
}

#[derive(Clone)]
pub(crate) struct NewConnector {
    pub(super) kind: ConnectorKind,
    pub(super) label: String,
    pub(super) url: Option<String>,
    pub(super) provider_id: Option<String>,
    pub(super) auth_kind: &'static str,
    pub(super) auth_ref: Option<String>,
    pub(super) grant_encrypted: Option<String>,
    pub(super) scopes: Vec<String>,
    pub(super) policy: ConnectorPolicy,
}

/// Inserts the connector once the bot is the caller's and has room.
pub(crate) async fn create_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    new: NewConnector,
) -> Result<bot_connector::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    if new.auth_kind == AUTH_SECRET
        && !secret_exists(db, bot_id, new.auth_ref.as_deref().unwrap_or_default()).await?
    {
        return Err(invalid(
            "The bot has no secret with that name; store it first",
        ));
    }
    let count = bot_connector::Entity::find()
        .filter(bot_connector::Column::BotId.eq(bot_id))
        .count(db)
        .await?;
    if count >= MAX_CONNECTORS_PER_BOT as u64 {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            LIMIT_REACHED,
            format!("A bot has at most {MAX_CONNECTORS_PER_BOT} connectors"),
        ));
    }
    label_free(db, bot_id, &new.label, None).await?;
    let now = Utc::now().fixed_offset();
    Ok(bot_connector::ActiveModel {
        id: Set(format!("bc_{}", uuid::Uuid::new_v4().simple())),
        bot_id: Set(bot_id.to_string()),
        kind: Set(new.kind.column().to_string()),
        label: Set(new.label),
        provider_id: Set(new.provider_id),
        url: Set(new.url),
        auth_kind: Set(new.auth_kind.to_string()),
        auth_ref: Set(new.auth_ref),
        grant_encrypted: Set(new.grant_encrypted),
        scopes: Set(serde_json::json!(new.scopes)),
        default_policy: Set(new.policy.column().to_string()),
        tool_policies: Set(serde_json::json!({})),
        tools_snapshot: Set(serde_json::json!([])),
        confirmed_hashes: Set(serde_json::json!({})),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?)
}

fn checked_new(state: &AppState, body: CreateBotConnector) -> Result<NewConnector, ApiError> {
    let label = validate_label(&body.label).map_err(invalid)?;
    let grant = grant_of(body.grant.as_deref())?;
    let scopes = scopes_of(body.scopes)?;
    let policy = body.default_policy.unwrap_or(ConnectorPolicy::Ask);
    let (auth_kind, auth_ref, provider_id) = match &body.auth {
        ConnectorAuth::None => (AUTH_NONE, None, None),
        ConnectorAuth::Secret { name } => {
            if !flow_like_types::bot_message::is_secret_name(name) {
                return Err(invalid("The secret name is invalid"));
            }
            (AUTH_SECRET, Some(name.clone()), None)
        }
        ConnectorAuth::Oauth { provider_id } => {
            if !state.oauth_providers.contains_key(provider_id) {
                return Err(invalid(format!(
                    "'{provider_id}' is not a configured provider"
                )));
            }
            (
                AUTH_OAUTH,
                Some(provider_id.clone()),
                Some(provider_id.clone()),
            )
        }
    };
    let url = match body.kind {
        ConnectorKind::Mcp => {
            let url = url_of(
                body.url
                    .as_deref()
                    .ok_or_else(|| invalid("An MCP server needs a URL"))?,
            )?;
            if let Some(provider) = &provider_id {
                let trusted = state
                    .oauth_providers
                    .get(provider)
                    .and_then(|config| config.mcp_url.as_deref());
                if trusted != Some(url.as_str()) {
                    return Err(invalid(
                        "A provider's sign-in goes only to the MCP endpoint the hub configured for it",
                    ));
                }
            }
            Some(url)
        }
        ConnectorKind::Oauth => {
            if provider_id.is_none() {
                return Err(invalid("An account needs a provider: use auth type oauth"));
            }
            None
        }
    };
    if auth_kind == AUTH_OAUTH && grant.is_none() {
        return Err(invalid("Connect the account first, then attach it"));
    }
    Ok(NewConnector {
        kind: body.kind,
        label,
        url,
        provider_id,
        auth_kind,
        auth_ref,
        grant_encrypted: grant.map(|handle| seal_grant(state, &handle)),
        scopes,
        policy,
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/connectors",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body(content = Object, description = "kind (mcp or oauth), label, url (https, for MCP), auth ({type: none | secret | oauth}), grant (the stored sign-in handle of an account), scopes, default_policy"),
    responses(
        (status = 201, description = "The connector was attached; its tool list is fetched with a refresh", body = BotConnectorSchema),
        (status = 400, description = "The label, URL, secret name, provider or sign-in is invalid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_CONNECTOR_LIMIT_REACHED: a bot has at most 20 connectors; or the label is taken")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/connectors", skip(state, user, body))]
pub async fn create_bot_connector(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<CreateBotConnector>,
) -> Result<(StatusCode, Json<BotConnector>), ApiError> {
    let sub = user.sub()?;
    let new = checked_new(&state, body)?;
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, new) = (sub.clone(), bot_id.clone(), new.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                create_in(txn, &sub, &bot_id, new).await
            })
        })
        .await?;
    Ok((StatusCode::CREATED, Json(info(row)?)))
}

/// The confirmed hashes after an update: a tool whose policy the owner just set, or that they
/// confirmed, takes its current schema hash; every other tool keeps what it had.
fn confirmed_after(current: &BotConnector, body: &UpdateBotConnector) -> BTreeMap<String, String> {
    let mut hashes = current.confirmed_hashes.clone();
    let hash_of = |name: &str| {
        current
            .tools_snapshot
            .iter()
            .find(|tool| tool.name == name)
            .map(|tool| tool.schema_hash.clone())
    };
    if let Some(policies) = &body.tool_policies {
        for (name, policy) in policies {
            if current.tool_policies.get(name) != Some(policy)
                && let Some(hash) = hash_of(name)
            {
                hashes.insert(name.clone(), hash);
            }
        }
        hashes.retain(|name, _| policies.contains_key(name));
    }
    for name in &body.confirm_tools {
        if let Some(hash) = hash_of(name) {
            hashes.insert(name.clone(), hash);
        }
    }
    hashes
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/connectors/{connector_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("connector_id" = String, Path, description = "The connector's id")
    ),
    request_body(content = Object, description = "Any of label, default_policy, tool_policies (tool name, or read / write for accounts, to read | ask | never), grant, scopes"),
    responses(
        (status = 200, description = "The updated connector", body = BotConnectorSchema),
        (status = 400, description = "A field is invalid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or connector"),
        (status = 409, description = "The label is taken")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/connectors/{connector_id}",
    skip(state, user, body)
)]
pub async fn update_bot_connector(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, connector_id)): Path<(String, String)>,
    Json(body): Json<UpdateBotConnector>,
) -> Result<Json<BotConnector>, ApiError> {
    let sub = user.sub()?;
    let row = owned_connector(&state.db, &sub, &bot_id, &connector_id).await?;
    let mut active: bot_connector::ActiveModel = row.clone().into();
    if let Some(label) = &body.label {
        let label = validate_label(label).map_err(invalid)?;
        label_free(&state.db, &bot_id, &label, Some(&connector_id)).await?;
        active.label = Set(label);
    }
    if let Some(policy) = body.default_policy {
        active.default_policy = Set(policy.column().to_string());
    }
    let current = to_contract(&row)
        .ok_or_else(|| ApiError::internal("The connector has an unknown kind or policy"))?;
    if let Some(policies) = &body.tool_policies {
        validate_tool_policies(policies).map_err(invalid)?;
        let stored: BTreeMap<&String, &str> = policies
            .iter()
            .map(|(name, policy)| (name, policy.column()))
            .collect();
        active.tool_policies = Set(serde_json::json!(stored));
    }
    if body.tool_policies.is_some() || !body.confirm_tools.is_empty() {
        let hashes = confirmed_after(&current, &body);
        active.confirmed_hashes = Set(serde_json::json!(hashes));
    }
    if let Some(grant) = grant_of(body.grant.as_deref())? {
        if row.auth_kind != AUTH_OAUTH {
            return Err(invalid("Only an account takes a sign-in"));
        }
        active.grant_encrypted = Set(Some(seal_grant(&state, &grant)));
    }
    if let Some(scopes) = body.scopes {
        active.scopes = Set(serde_json::json!(scopes_of(scopes)?));
    }
    active.updated_at = Set(Utc::now().fixed_offset());
    Ok(Json(info(active.update(&state.db).await?)?))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/connectors/{connector_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("connector_id" = String, Path, description = "The connector's id")
    ),
    responses(
        (status = 204, description = "The connector and its stored sign-in are gone"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or connector")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /bots/{bot_id}/connectors/{connector_id}",
    skip(state, user)
)]
pub async fn delete_bot_connector(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, connector_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_connector(&state.db, &sub, &bot_id, &connector_id).await?;
    bot_connector::Entity::delete_many()
        .filter(bot_connector::Column::Id.eq(&connector_id))
        .filter(bot_connector::Column::BotId.eq(&bot_id))
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn tools_from(output: &str) -> Result<Vec<ConnectorToolInfo>, ApiError> {
    let envelope: serde_json::Value = serde_json::from_str(output).map_err(|_| {
        ApiError::service_unavailable("The executor answered with something unreadable")
    })?;
    if envelope["status"] != "ok" {
        let message = envelope["message"]
            .as_str()
            .or_else(|| envelope["error"]["message"].as_str())
            .unwrap_or("The server could not be reached");
        return Err(ApiError::coded(
            StatusCode::BAD_GATEWAY,
            "BOT_CONNECTOR_UNREACHABLE",
            message.to_string(),
        ));
    }
    serde_json::from_value(envelope["result"]["tools"].clone())
        .map_err(|_| ApiError::service_unavailable("The tool list was unreadable"))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/connectors/{connector_id}/refresh",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("connector_id" = String, Path, description = "The connector's id")
    ),
    responses(
        (status = 200, description = "The MCP server was asked for its tools; the connector holds the new list", body = BotConnectorSchema),
        (status = 400, description = "Only MCP servers have a tool list"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or connector"),
        (status = 409, description = "The bot cannot have a workspace (Flux), or its workspace is being deleted"),
        (status = 502, description = "BOT_CONNECTOR_UNREACHABLE: the server did not answer"),
        (status = 503, description = "This server has no executor for connector calls")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/connectors/{connector_id}/refresh",
    skip(state, user)
)]
pub async fn refresh_bot_connector(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, connector_id)): Path<(String, String)>,
) -> Result<Json<BotConnector>, ApiError> {
    let sub = user.sub()?;
    let row = owned_connector(&state.db, &sub, &bot_id, &connector_id).await?;
    if row.kind != ConnectorKind::Mcp.column() {
        return Err(invalid("Only an MCP server has a tool list to refresh"));
    }
    let executor = crate::bot_tools::executor_for(&state).ok_or_else(|| {
        ApiError::service_unavailable("This server has no executor, so it cannot reach MCP servers")
    })?;
    let workspace = workspace::ensure(&state, &sub, &bot_id).await?.app_id;
    let entry = entry_of(&state, &row)
        .ok_or_else(|| ApiError::internal("The connector has an unknown kind or policy"))?;
    let authorization = authorization_of(&state, &sub, &row)
        .await
        .map_err(|message| {
            ApiError::coded(StatusCode::CONFLICT, "BOT_CONNECTOR_NO_SIGN_IN", message)
        })?;
    let credentials = crate::bot_tools::owner_credentials(&state, &sub, &workspace)
        .await
        .map_err(|error| ApiError::internal(format!("Cannot scope credentials: {error}")))?;
    let job = ToolJob {
        tool: flow_like_bot_tools::services::CONNECTOR_REFRESH.to_string(),
        args: serde_json::json!({ "connector": connector_id }),
        owner: sub.clone(),
        bot_id: bot_id.clone(),
        turn_id: format!("connector-refresh-{}", uuid::Uuid::new_v4().simple()),
        workspace: workspace.clone(),
        scope: JobScope {
            storage: false,
            data_connections: false,
            classification: DataClassification::Public,
            apps: Vec::new(),
        },
        credentials: BTreeMap::from([(
            workspace.clone(),
            serde_json::to_value(credentials).map_err(|error| {
                ApiError::internal(format!("Cannot encode credentials: {error}"))
            })?,
        )]),
        models: Default::default(),
        allow_expensive_documents: false,
        secrets: Default::default(),
        connectors: vec![JobConnector {
            entry,
            authorization,
        }],
        skill_call: None,
        files: Vec::new(),
    };
    let principal = BotUser {
        bot_id: bot_id.clone(),
        turn_id: job.turn_id.clone(),
        conv_id: None,
    };
    let text = serde_json::to_string(&job)
        .map_err(|error| ApiError::internal(format!("Cannot encode the job: {error}")))?;
    let reply = executor
        .run(&sub, &principal, &workspace, text, REFRESH_TIMEOUT)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "[bot_connectors] the tool list refresh failed on the executor");
            ApiError::coded(
                StatusCode::BAD_GATEWAY,
                "BOT_CONNECTOR_UNREACHABLE",
                "The executor could not reach the server",
            )
        })?;
    let mut tools = tools_from(&reply.output)?;
    tools.truncate(MAX_TOOLS_SNAPSHOT);
    let mut active: bot_connector::ActiveModel = row.into();
    active.tools_snapshot = Set(serde_json::json!(tools));
    active.updated_at = Set(Utc::now().fixed_offset());
    Ok(Json(info(active.update(&state.db).await?)?))
}

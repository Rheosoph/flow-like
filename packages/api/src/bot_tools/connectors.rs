//! The bot's connectors on the server. The API holds the trusted half: it reads the settings,
//! resolves the credential (the owner's OAuth grant, a bot secret, or a CalDAV password) and
//! hands both to the executor job that does the network call. Nothing here talks to a
//! connector's host: connector calls and calendar calls (`super::calendar_jobs`) run on the
//! executor.

use std::collections::BTreeMap;

use async_trait::async_trait;
use flow_like_bot_tools::calendar::caldav::{account_hosts, basic_credential};
use flow_like_bot_tools::calendar::{CalendarJobs, calendar_provider};
use flow_like_bot_tools::services::{
    CallAudit, CallBudget, ConnectorEntry, ConnectorSource, Credential,
};
use flow_like_types::bot_connector::{
    BotConnector, CalDavCalendarInfo, ConnectorAuth, ConnectorKind, ConnectorPolicy,
    ConnectorToolInfo, api_hosts_for,
};
use flow_like_types::{Result, anyhow};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::calendar_jobs::ExecutorCalendars;
use crate::{
    entity::bot_connector,
    middleware::jwt::BotUser,
    routes::{bots::secrets::open_for_bot, oauth::resolve_sink_oauth_token},
    state::AppState,
};

pub(crate) const AUTH_NONE: &str = "NONE";
pub(crate) const AUTH_SECRET: &str = "SECRET";
pub(crate) const AUTH_OAUTH: &str = "OAUTH";
pub(crate) const AUTH_BASIC: &str = "BASIC";

pub(crate) fn auth_of(row: &bot_connector::Model) -> ConnectorAuth {
    match (row.auth_kind.as_str(), row.auth_ref.clone()) {
        (AUTH_SECRET, Some(name)) => ConnectorAuth::Secret { name },
        (AUTH_OAUTH, Some(provider_id)) => ConnectorAuth::Oauth { provider_id },
        (AUTH_BASIC, Some(username)) => ConnectorAuth::Basic { username },
        _ => ConnectorAuth::None,
    }
}

/// The sealed form of a CalDAV password.
pub(crate) fn seal_password(state: &AppState, password: &str) -> String {
    let grant = serde_json::json!({ "password": password });
    crate::utils::crypto::encrypt_secret(&grant.to_string(), &state.encryption_key)
}

/// The CalDAV password a row keeps sealed.
pub(crate) fn open_password(
    state: &AppState,
    row: &bot_connector::Model,
) -> Result<String, String> {
    let sealed = row.grant_encrypted.as_deref().ok_or_else(|| {
        format!(
            "'{}' has no password stored; the owner must enter it again",
            row.label
        )
    })?;
    crate::utils::crypto::decrypt_secret(sealed, &state.encryption_key)
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|grant| grant.get("password")?.as_str().map(str::to_string))
        .ok_or_else(|| {
            format!(
                "The password of '{}' cannot be read; the owner must enter it again",
                row.label
            )
        })
}

pub(crate) fn calendars_of(row: &bot_connector::Model) -> Vec<CalDavCalendarInfo> {
    row.calendars
        .as_ref()
        .map(parse::<Vec<CalDavCalendarInfo>>)
        .unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned + Default>(value: &serde_json::Value) -> T {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// The settings row as the owner sees it; `None` for a row with an unknown kind or policy.
pub(crate) fn to_contract(row: &bot_connector::Model) -> Option<BotConnector> {
    Some(BotConnector {
        id: row.id.clone(),
        bot_id: row.bot_id.clone(),
        kind: ConnectorKind::from_column(&row.kind)?,
        label: row.label.clone(),
        provider_id: row.provider_id.clone(),
        url: row.url.clone(),
        auth: auth_of(row),
        has_grant: row.grant_encrypted.is_some(),
        scopes: parse::<Vec<String>>(&row.scopes),
        default_policy: ConnectorPolicy::from_column(&row.default_policy)?,
        tool_policies: parse::<BTreeMap<String, ConnectorPolicy>>(&row.tool_policies),
        tools_snapshot: parse::<Vec<ConnectorToolInfo>>(&row.tools_snapshot),
        confirmed_hashes: parse::<BTreeMap<String, String>>(&row.confirmed_hashes),
        calendar_url: row.calendar_url.clone(),
        calendars: calendars_of(row),
        updated_at: row.updated_at.to_rfc3339(),
    })
}

pub(crate) async fn rows_of(
    state: &AppState,
    bot_id: &str,
) -> Result<Vec<bot_connector::Model>, sea_orm::DbErr> {
    bot_connector::Entity::find()
        .filter(bot_connector::Column::BotId.eq(bot_id))
        .order_by_asc(bot_connector::Column::CreatedAt)
        .all(&state.db)
        .await
}

/// What a bot run checks connector calls against: ids, labels and policies, nothing secret.
pub(crate) async fn rules_of<C: sea_orm::ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<flow_like_types::bot_connector::ConnectorRule>, sea_orm::DbErr> {
    Ok(bot_connector::Entity::find()
        .filter(bot_connector::Column::BotId.eq(bot_id))
        .order_by_asc(bot_connector::Column::CreatedAt)
        .all(db)
        .await?
        .iter()
        .filter_map(to_contract)
        .map(|connector| connector.rule())
        .collect())
}

pub(crate) fn entry_of(state: &AppState, row: &bot_connector::Model) -> Option<ConnectorEntry> {
    let connector = to_contract(row)?;
    let provider_id = row.provider_id.clone();
    let api_hosts = match (&provider_id, connector.kind) {
        (Some(provider), ConnectorKind::Oauth) => {
            let configured = state
                .oauth_providers
                .get(provider)
                .map(|config| config.api_hosts.clone())
                .unwrap_or_default();
            api_hosts_for(provider, &configured)
        }
        (_, ConnectorKind::Caldav) => account_hosts(connector.url.as_deref().unwrap_or_default()),
        _ => Vec::new(),
    };
    Some(entry_from(connector, provider_id, api_hosts))
}

fn entry_from(
    connector: BotConnector,
    provider_id: Option<String>,
    api_hosts: Vec<String>,
) -> ConnectorEntry {
    ConnectorEntry {
        rule: connector.rule(),
        kind: connector.kind,
        calendar: connector.calendar().cloned(),
        url: connector.url,
        provider_id,
        api_hosts,
    }
}

/// The provider of the calendar this connector connects (`google`, `microsoft`, `caldav`), when
/// the bot's calendar tools would read it.
pub(crate) fn calendar_provider_of(row: &bot_connector::Model) -> Option<&'static str> {
    let connector = to_contract(row)?;
    calendar_provider(entry_from(connector, row.provider_id.clone(), Vec::new()))
}

/// The `Authorization` value for one call, resolved now. Errors never carry a token.
pub(crate) async fn authorization_of(
    state: &AppState,
    owner: &str,
    row: &bot_connector::Model,
) -> Result<Option<String>, String> {
    match row.auth_kind.as_str() {
        AUTH_SECRET => {
            let name = row.auth_ref.clone().unwrap_or_default();
            let value = open_for_bot(&state.db, &state.encryption_key, &row.bot_id, &name)
                .await
                .map_err(|error| format!("Cannot read the secret '{name}': {error}"))?
                .ok_or_else(|| {
                    format!("The bot has no secret named '{name}' for '{}'", row.label)
                })?;
            Ok(Credential::from_secret(&value).header().map(str::to_string))
        }
        AUTH_OAUTH => {
            let provider = row.provider_id.clone().unwrap_or_default();
            let sealed = row.grant_encrypted.as_deref().ok_or_else(|| {
                format!(
                    "'{}' has no connected account; the owner must connect one",
                    row.label
                )
            })?;
            let grant = crate::utils::crypto::decrypt_secret(sealed, &state.encryption_key)
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                .ok_or_else(|| {
                    format!(
                        "The account grant of '{}' cannot be read; reconnect it",
                        row.label
                    )
                })?;
            let resolved = resolve_sink_oauth_token(state, &provider, &grant, Some(owner))
                .await
                .map_err(|_| {
                    format!(
                        "The account of '{}' needs to be reconnected by its owner",
                        row.label
                    )
                })?;
            let token = resolved
                .get("access_token")
                .and_then(serde_json::Value::as_str)
                .filter(|token| !token.is_empty())
                .ok_or_else(|| format!("The account of '{}' has no usable token", row.label))?;
            Ok(Some(format!("Bearer {token}")))
        }
        AUTH_BASIC => {
            let username = row.auth_ref.clone().unwrap_or_default();
            let password = open_password(state, row)?;
            Ok(basic_credential(&username, &password)
                .header()
                .map(str::to_string))
        }
        _ => Ok(None),
    }
}

pub(crate) fn seal_grant(state: &AppState, handle: &str) -> String {
    let grant = serde_json::json!({ "refresh_token": handle });
    crate::utils::crypto::encrypt_secret(&grant.to_string(), &state.encryption_key)
}

/// One turn's connectors, or a bot's outside a turn. Listing answers from the stored snapshot:
/// asking the servers is the executor's job, and so is every calendar call.
pub(crate) struct ApiConnectors {
    state: AppState,
    owner: String,
    bot_id: String,
    budget: CallBudget,
    calendars: ExecutorCalendars,
}

impl ApiConnectors {
    pub(crate) fn new(state: &AppState, owner: &str, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            owner: owner.to_string(),
            bot_id: bot_id.to_string(),
            budget: CallBudget::default(),
            calendars: ExecutorCalendars::unbound(state, owner, bot_id),
        }
    }

    /// Runs this source's calendar jobs under a turn's identity and workspace.
    pub(crate) fn in_turn(mut self, principal: BotUser, workspace: &str) -> Self {
        self.calendars = ExecutorCalendars::in_turn(&self.state, &self.owner, principal, workspace);
        self
    }
}

#[async_trait]
impl ConnectorSource for ApiConnectors {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        let rows = rows_of(&self.state, &self.bot_id).await?;
        Ok(rows
            .iter()
            .filter_map(|row| entry_of(&self.state, row))
            .collect())
    }

    async fn credential(&self, connector_id: &str) -> Result<Credential> {
        let rows = rows_of(&self.state, &self.bot_id).await?;
        let row = rows
            .iter()
            .find(|row| row.id == connector_id)
            .ok_or_else(|| anyhow!("No connector '{connector_id}'"))?;
        let header = authorization_of(&self.state, &self.owner, row)
            .await
            .map_err(|message| anyhow!("{message}"))?;
        Ok(header.map_or_else(Credential::none, Credential::authorization))
    }

    fn budget(&self) -> &CallBudget {
        &self.budget
    }

    fn live_listing(&self) -> bool {
        false
    }

    fn audit(&self, entry: &CallAudit) {
        tracing::info!(
            bot_id = %self.bot_id,
            connector = %entry.connector_id,
            operation = %entry.operation,
            effect = entry.effect,
            ok = entry.ok,
            "[bot_tools] connector call"
        );
    }

    /// Always the executor: this process never calls a calendar provider.
    fn calendar_jobs(&self) -> Option<&dyn CalendarJobs> {
        Some(&self.calendars)
    }
}

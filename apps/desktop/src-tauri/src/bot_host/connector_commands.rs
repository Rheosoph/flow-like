//! Tauri commands for the connectors of bots kept on this device. The calls themselves run in
//! `functions::ai::bot_tools::connectors`; these edit the settings, refresh an MCP server's tool
//! list and find a CalDAV account's calendars in this process (a server on this network too).

use crate::functions::ai::bot_tools::connectors::{LocalSecrets, credential_for};
use crate::functions::local_bots::{LocalBots, on_store};
use crate::native_auth::{SessionBinding, oauth::provider_hosts};
use flow_like_bot_tools::calendar::caldav::{HttpDavTransport, basic_credential, discover};
use flow_like_bot_tools::{Environment, services};
use flow_like_types::bot_connector::{
    BotConnector, CalDavCalendarInfo, ConnectorAuth, ConnectorKind, CreateBotConnector,
    UpdateBotConnector,
};
use tauri::{AppHandle, State};

fn failed(error: impl ToString) -> String {
    error.to_string()
}

/// The calendars of a CalDAV account, found from this device.
async fn calendars_of(
    label: &str,
    url: &str,
    username: &str,
    password: &str,
) -> Result<Vec<CalDavCalendarInfo>, String> {
    discover(
        label,
        url,
        &basic_credential(username, password),
        &HttpDavTransport(Environment::Local),
    )
    .await
}

fn username_of(auth: &ConnectorAuth) -> &str {
    match auth {
        ConnectorAuth::Basic { username } => username.trim(),
        _ => "",
    }
}

#[tauri::command]
pub async fn local_bot_connectors(
    bots: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<BotConnector>, String> {
    on_store(&bots, move |store| store.connectors(&bot_id))
        .await
        .map_err(failed)
}

/// Attaches a connector. A CalDAV calendar is signed in first: its calendars are found before it
/// is kept, and a discovery that fails attaches nothing.
#[tauri::command]
pub async fn local_bot_create_connector(
    bots: State<'_, LocalBots>,
    bot_id: String,
    request: CreateBotConnector,
) -> Result<BotConnector, String> {
    let calendars = match request.kind {
        ConnectorKind::Caldav => Some(
            calendars_of(
                &request.label,
                request.url.as_deref().unwrap_or_default(),
                username_of(&request.auth),
                request.grant.as_deref().unwrap_or_default(),
            )
            .await?,
        ),
        _ => None,
    };
    let wanted = request.calendar_url.clone();
    on_store(&bots, move |store| {
        let made = store.create_connector(&bot_id, request)?;
        match calendars {
            Some(calendars) => {
                store.set_connector_calendars(&bot_id, &made.id, calendars, wanted.as_deref())
            }
            None => Ok(made),
        }
    })
    .await
    .map_err(failed)
}

#[tauri::command]
pub async fn local_bot_update_connector(
    bots: State<'_, LocalBots>,
    bot_id: String,
    connector_id: String,
    request: UpdateBotConnector,
) -> Result<BotConnector, String> {
    on_store(&bots, move |store| {
        store.update_connector(&bot_id, &connector_id, request)
    })
    .await
    .map_err(failed)
}

#[tauri::command]
pub async fn local_bot_delete_connector(
    bots: State<'_, LocalBots>,
    bot_id: String,
    connector_id: String,
) -> Result<(), String> {
    on_store(&bots, move |store| {
        store.delete_connector(&bot_id, &connector_id)
    })
    .await
    .map_err(failed)
}

/// Asks the MCP server for its tools, or the CalDAV server for its calendars, and keeps the list
/// with the connector. A bot secret named in `auth` signs an MCP request in; an OAuth-signed
/// server needs the signed-in `profile_id` and `hub`, and the person's consent for the bot's
/// workspace; a CalDAV calendar signs in with its stored password.
#[tauri::command]
pub async fn local_bot_refresh_connector(
    app: AppHandle,
    bots: State<'_, LocalBots>,
    bot_id: String,
    connector_id: String,
    profile_id: Option<String>,
    hub: Option<String>,
) -> Result<BotConnector, String> {
    let lookup = bot_id.clone();
    let (bot, connectors) = on_store(&bots, move |store| {
        Ok((store.get_bot(&lookup)?, store.connectors(&lookup)?))
    })
    .await
    .map_err(failed)?;
    let connector = connectors
        .into_iter()
        .find(|connector| connector.id == connector_id)
        .ok_or("This connector does not exist")?;
    if connector.kind == ConnectorKind::Caldav {
        let (owner, id) = (bot_id.clone(), connector.id.clone());
        let password = on_store(&bots, move |store| store.connector_password(&owner, &id))
            .await
            .map_err(failed)?;
        let calendars = calendars_of(
            &connector.label,
            connector.url.as_deref().unwrap_or_default(),
            username_of(&connector.auth),
            &password,
        )
        .await?;
        return on_store(&bots, move |store| {
            store.set_connector_calendars(&bot_id, &connector.id, calendars, None)
        })
        .await
        .map_err(failed);
    }
    let url = connector
        .url
        .clone()
        .ok_or("Only an MCP server has a tool list to refresh")?;
    let binding = match (profile_id, hub) {
        (Some(profile_id), Some(hub)) => SessionBinding::capture(&profile_id, &hub).await?,
        _ => None,
    };
    let hosts = match &binding {
        Some(binding) => provider_hosts(binding).await.ok(),
        None => None,
    };
    let hosts: Option<&'static _> = hosts.map(|hosts| &*Box::leak(Box::new(hosts)));
    let workspace = bot.and_then(|bot| bot.workspace_app_id).unwrap_or_default();
    let secrets = LocalSecrets {
        app: app.clone(),
        bot_id: bot_id.clone(),
    };
    let credential = credential_for(
        &app,
        binding.as_ref(),
        &workspace,
        &connector,
        &secrets,
        hosts,
    )
    .await
    .map_err(failed)?;
    let tools = services::fetch_tools(&services::McpEndpoint {
        url,
        authorization: credential.header().map(str::to_string),
        environment: Environment::Local,
    })
    .await?;
    on_store(&bots, move |store| {
        store.set_connector_tools(&bot_id, &connector.id, tools)
    })
    .await
    .map_err(failed)
}

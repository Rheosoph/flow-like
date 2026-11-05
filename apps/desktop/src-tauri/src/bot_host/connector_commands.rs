//! Tauri commands for the connectors of bots kept on this device. The calls themselves run in
//! `functions::ai::bot_tools::connectors`; these edit the settings and refresh an MCP server's
//! tool list in this process.

use crate::functions::local_bots::{LocalBots, on_store};
use flow_like_bot_tools::{Environment, services};
use flow_like_types::bot_connector::{
    BotConnector, CreateBotConnector, UpdateBotConnector,
};
use crate::functions::ai::bot_tools::connectors::{LocalSecrets, credential_for};
use crate::native_auth::{SessionBinding, oauth::provider_hosts};
use tauri::{AppHandle, State};

fn failed(error: impl ToString) -> String {
    error.to_string()
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

#[tauri::command]
pub async fn local_bot_create_connector(
    bots: State<'_, LocalBots>,
    bot_id: String,
    request: CreateBotConnector,
) -> Result<BotConnector, String> {
    on_store(&bots, move |store| store.create_connector(&bot_id, request))
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

/// Asks the MCP server for its tools and keeps the list with the connector. A bot secret named
/// in `auth` signs the request in; an OAuth-signed server needs the signed-in `profile_id` and
/// `hub`, and the person's consent for the bot's workspace.
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
    let workspace = bot
        .and_then(|bot| bot.workspace_app_id)
        .unwrap_or_default();
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

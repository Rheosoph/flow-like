//! Tauri commands for the connectors of bots kept on this device. The calls themselves run in
//! `functions::ai::bot_tools::connectors`; these edit the settings and refresh an MCP server's
//! tool list in this process.

use crate::functions::local_bots::{LocalBots, on_store};
use flow_like_bot_tools::{Environment, services};
use flow_like_types::bot_connector::{
    BotConnector, ConnectorAuth, CreateBotConnector, UpdateBotConnector,
};
use tauri::State;

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
/// in `auth` signs the request in.
#[tauri::command]
pub async fn local_bot_refresh_connector(
    bots: State<'_, LocalBots>,
    bot_id: String,
    connector_id: String,
) -> Result<BotConnector, String> {
    let lookup = bot_id.clone();
    let connector = on_store(&bots, move |store| store.connectors(&lookup))
        .await
        .map_err(failed)?
        .into_iter()
        .find(|connector| connector.id == connector_id)
        .ok_or("This connector does not exist")?;
    let url = connector
        .url
        .clone()
        .ok_or("Only an MCP server has a tool list to refresh")?;
    let authorization = match &connector.auth {
        ConnectorAuth::Secret { name } => {
            let (lookup, name) = (bot_id.clone(), name.clone());
            let opened = on_store(&bots, move |store| store.open_secrets(&lookup))
                .await
                .map_err(failed)?;
            let value = opened
                .readable
                .iter()
                .find(|secret| secret.name == name)
                .map(|secret| secret.value.to_string())
                .ok_or_else(|| format!("The bot has no secret named '{name}'"))?;
            services::Credential::from_secret(&value)
                .header()
                .map(str::to_string)
        }
        ConnectorAuth::Oauth { .. } => {
            return Err("Refresh an OAuth-signed MCP server from a bot turn".to_string());
        }
        ConnectorAuth::None => None,
    };
    let tools = services::fetch_tools(&services::McpEndpoint {
        url,
        authorization,
        environment: Environment::Local,
    })
    .await?;
    on_store(&bots, move |store| {
        store.set_connector_tools(&bot_id, &connector.id, tools)
    })
    .await
    .map_err(failed)
}

//! The desktop's connectors for a bot's turn. A local bot's live in the local store, an account
//! bot's at the hub. The calls run in this process: an OAuth account's sign-in comes from the
//! native OAuth store under the bot's workspace app id (the person consented there), an MCP
//! server's from a bot secret. Localhost MCP servers are reachable; the token still goes only to
//! the provider's API hosts.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::flow::compiled::prerun::PrerunOAuthRequirement;
use flow_like_bot_tools::services::{
    CallAudit, CallBudget, ConnectorEntry, ConnectorSource, Credential,
};
use flow_like_types::bot_connector::{BotConnector, ConnectorAuth, ConnectorKind, api_hosts_for};
use flow_like_types::{Result, anyhow};
use reqwest::Method;
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::local_bots::{LocalBots, on_store};
use crate::native_auth::SessionBinding;

/// Reads a bot secret by name: the device's store for a local bot, the hub's for an account bot.
#[async_trait]
pub(crate) trait SecretReader: Send + Sync {
    async fn secret(&self, name: &str) -> Result<Option<String>>;
}

pub(crate) struct DesktopConnectors {
    app: AppHandle,
    binding: Option<SessionBinding>,
    workspace: String,
    bot_id: String,
    local: bool,
    api: Arc<dyn TaskApi>,
    secrets: Arc<dyn SecretReader>,
    budget: CallBudget,
}

impl DesktopConnectors {
    pub(crate) async fn new(
        app: &AppHandle,
        binding: Option<SessionBinding>,
        workspace: &str,
        bot_id: &str,
        api: Arc<dyn TaskApi>,
        secrets: Arc<dyn SecretReader>,
    ) -> Self {
        Self {
            app: app.clone(),
            binding,
            workspace: workspace.to_string(),
            bot_id: bot_id.to_string(),
            local: super::held_locally(app, bot_id).await,
            api,
            secrets,
            budget: CallBudget::default(),
        }
    }

    async fn connectors(&self) -> Result<Vec<BotConnector>> {
        if self.local {
            let bots = self
                .app
                .try_state::<LocalBots>()
                .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
            let bot_id = self.bot_id.clone();
            return on_store(&bots, move |store| store.connectors(&bot_id))
                .await
                .map_err(|error| anyhow!(error.to_string()));
        }
        let listed = self
            .api
            .request(
                Method::GET,
                &format!(
                    "bots/{}/connectors",
                    crate::functions::ai::native_platform_tools::segment(&self.bot_id)
                ),
                None,
            )
            .await
            .map_err(|error| anyhow!(error))?;
        serde_json::from_value(listed.get("connectors").cloned().unwrap_or_default())
            .map_err(|error| anyhow!("The hub sent unusable connectors: {error}"))
    }
}

fn entry_of(connector: &BotConnector) -> ConnectorEntry {
    let api_hosts = match (&connector.provider_id, connector.kind) {
        (Some(provider), ConnectorKind::Oauth) => api_hosts_for(provider, &[]),
        _ => Vec::new(),
    };
    ConnectorEntry {
        rule: connector.rule(),
        kind: connector.kind,
        url: connector.url.clone(),
        provider_id: connector.provider_id.clone(),
        api_hosts,
    }
}

#[async_trait]
impl ConnectorSource for DesktopConnectors {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        Ok(self.connectors().await?.iter().map(entry_of).collect())
    }

    async fn credential(&self, connector_id: &str) -> Result<Credential> {
        let connectors = self.connectors().await?;
        let connector = connectors
            .iter()
            .find(|connector| connector.id == connector_id)
            .ok_or_else(|| anyhow!("No connector '{connector_id}'"))?;
        match &connector.auth {
            ConnectorAuth::None => Ok(Credential::none()),
            ConnectorAuth::Secret { name } => {
                let value = self.secrets.secret(name).await?.ok_or_else(|| {
                    anyhow!(
                        "The bot has no secret named '{name}' for '{}'",
                        connector.label
                    )
                })?;
                Ok(Credential::from_secret(&value))
            }
            ConnectorAuth::Oauth { provider_id } => {
                let requirement = PrerunOAuthRequirement {
                    provider_id: provider_id.clone(),
                    scopes: connector.scopes.clone(),
                };
                let mut tokens = crate::native_auth::oauth::resolve(
                    &self.app,
                    self.binding.as_ref(),
                    &self.workspace,
                    &[requirement],
                )
                .await
                .map_err(|error| anyhow!(error))?;
                let token = tokens
                    .remove(provider_id)
                    .ok_or_else(|| anyhow!("'{}' needs to be reconnected", connector.label))?;
                Ok(Credential::authorization(format!(
                    "Bearer {}",
                    token.access_token
                )))
            }
        }
    }

    fn budget(&self) -> &CallBudget {
        &self.budget
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
}

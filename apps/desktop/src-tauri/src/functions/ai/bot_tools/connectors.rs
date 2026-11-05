//! The desktop's connectors for a bot's turn. A local bot's live in the local store, an account
//! bot's at the hub. The calls run in this process: an OAuth account's sign-in comes from the
//! native OAuth store under the bot's workspace app id (the person consented there), an MCP
//! server's from a bot secret. Localhost MCP servers are reachable; the token still goes only to
//! the provider's API hosts.

use std::collections::HashMap;
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
use crate::native_auth::oauth::{ProviderHosts, provider_hosts};

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
    hub_hosts: tokio::sync::OnceCell<&'static HashMap<String, ProviderHosts>>,
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
            hub_hosts: tokio::sync::OnceCell::new(),
        }
    }

    /// The hub's configured hosts, read once per turn; the built-in list when it can't be read.
    async fn hosts(&self) -> HubHosts {
        let binding = self.binding.as_ref()?;
        let hosts = self
            .hub_hosts
            .get_or_init(|| async {
                match provider_hosts(binding).await {
                    Ok(hosts) => Box::leak(Box::new(hosts)) as &'static HashMap<_, _>,
                    Err(error) => {
                        tracing::warn!(%error, "The hub's connector hosts are unavailable; using the built-in list");
                        Box::leak(Box::new(HashMap::new())) as &'static HashMap<_, _>
                    }
                }
            })
            .await;
        Some(*hosts)
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

pub(crate) type HubHosts = Option<&'static HashMap<String, ProviderHosts>>;

fn entry_of(connector: &BotConnector, hosts: HubHosts) -> ConnectorEntry {
    let api_hosts = match (&connector.provider_id, connector.kind) {
        (Some(provider), ConnectorKind::Oauth) => api_hosts_for(
            provider,
            hosts
                .and_then(|hosts| hosts.get(provider))
                .map(|hosts| hosts.api_hosts.as_slice())
                .unwrap_or_default(),
        ),
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
        let hosts = self.hosts().await;
        Ok(self
            .connectors()
            .await?
            .iter()
            .map(|connector| entry_of(connector, hosts))
            .collect())
    }

    async fn credential(&self, connector_id: &str) -> Result<Credential> {
        let connectors = self.connectors().await?;
        let connector = connectors
            .iter()
            .find(|connector| connector.id == connector_id)
            .ok_or_else(|| anyhow!("No connector '{connector_id}'"))?;
        credential_for(
            &self.app,
            self.binding.as_ref(),
            &self.workspace,
            connector,
            self.secrets.as_ref(),
            self.hosts().await,
        )
        .await
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

/// The `Authorization` value for one connector. An OAuth sign-in comes from the native store
/// under `workspace` (the person consented there); an MCP server signed in by OAuth is only
/// trusted at the endpoint the hub configured for that provider.
pub(crate) async fn credential_for(
    app: &AppHandle,
    binding: Option<&SessionBinding>,
    workspace: &str,
    connector: &BotConnector,
    secrets: &dyn SecretReader,
    hosts: HubHosts,
) -> Result<Credential> {
    match &connector.auth {
        ConnectorAuth::None => Ok(Credential::none()),
        ConnectorAuth::Secret { name } => {
            let value = secrets.secret(name).await?.ok_or_else(|| {
                anyhow!("The bot has no secret named '{name}' for '{}'", connector.label)
            })?;
            Ok(Credential::from_secret(&value))
        }
        ConnectorAuth::Oauth { provider_id } => {
            if connector.kind == ConnectorKind::Mcp {
                let trusted = hosts
                    .and_then(|hosts| hosts.get(provider_id))
                    .and_then(|hosts| hosts.mcp_url.as_deref());
                if trusted != connector.url.as_deref() || trusted.is_none() {
                    return Err(anyhow!(
                        "'{}' signs in with {provider_id}, but the hub configured no such MCP endpoint",
                        connector.label
                    ));
                }
            }
            let requirement = PrerunOAuthRequirement {
                provider_id: provider_id.clone(),
                scopes: connector.scopes.clone(),
            };
            let mut tokens =
                crate::native_auth::oauth::resolve(app, binding, workspace, &[requirement])
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

/// Reads bot secrets from this device's store, for the settings commands.
pub(crate) struct LocalSecrets {
    pub(crate) app: AppHandle,
    pub(crate) bot_id: String,
}

#[async_trait]
impl SecretReader for LocalSecrets {
    async fn secret(&self, name: &str) -> Result<Option<String>> {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        let bot_id = self.bot_id.clone();
        let opened = on_store(&bots, move |store| store.open_secrets(&bot_id))
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(opened
            .readable
            .iter()
            .find(|secret| secret.name == name)
            .map(|secret| secret.value.to_string()))
    }
}

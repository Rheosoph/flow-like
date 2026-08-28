//! Azure Web PubSub transport: HS256 client access tokens with one literal per-group role per
//! side, and the REST `:send` forwarder for pushes that arrive on the HTTP fallback.

use std::sync::Arc;

use flow_like_channels::ChannelForwarder;
use flow_like_channels::azure::{
    AzureWebPubSubForwarder, client_access_token, client_roles, client_ws_url, executor_roles,
    group_for, normalize_endpoint,
};
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use flow_like_types::channel::{ChannelClientDescriptor, ChannelExecutorGrant, now_unix};
use flow_like_types::{Result, anyhow, bail};

use super::issuer::{MintedDescriptor, MintedExecutor};

pub const ENDPOINT_ENV: &str = "CHANNEL_WEBPUBSUB_ENDPOINT";
pub const HUB_ENV: &str = "CHANNEL_WEBPUBSUB_HUB";
pub const ACCESS_KEY_SECRET: &str = "CHANNEL_WEBPUBSUB_ACCESS_KEY";
pub const DEFAULT_HUB: &str = "channels";

#[derive(Clone, Debug)]
pub struct AzureChannelConfig {
    pub endpoint: String,
    pub hub: String,
}

impl AzureChannelConfig {
    pub fn from_env() -> Result<Self> {
        let endpoint = std::env::var(ENDPOINT_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("{ENDPOINT_ENV} is required for the azure_web_pubsub channel transport")
            })?;
        if !endpoint.starts_with("https://") {
            bail!("{ENDPOINT_ENV} must be an https:// origin, got '{endpoint}'");
        }
        let hub = std::env::var(HUB_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_HUB.to_string());
        Ok(Self {
            endpoint: normalize_endpoint(&endpoint).to_string(),
            hub,
        })
    }
}

pub struct AzureChannelRuntime {
    config: AzureChannelConfig,
    access_key: String,
    forwarder: Arc<AzureWebPubSubForwarder>,
}

impl AzureChannelRuntime {
    pub async fn from_env(secrets: &SecretStore) -> Result<Self> {
        let config = AzureChannelConfig::from_env()?;
        let access_key = secrets
            .get_secret_string(&SecretRef::new(ACCESS_KEY_SECRET))
            .await
            .map_err(|e| anyhow!("secret {ACCESS_KEY_SECRET} could not be resolved: {e}"))?
            .expose_secret()
            .trim()
            .to_string();
        if access_key.is_empty() {
            bail!("secret {ACCESS_KEY_SECRET} is empty");
        }
        let forwarder = Arc::new(AzureWebPubSubForwarder::new(
            config.endpoint.clone(),
            config.hub.clone(),
            access_key.clone(),
        ));
        Ok(Self {
            config,
            access_key,
            forwarder,
        })
    }

    pub fn forwarder(&self) -> Arc<dyn ChannelForwarder> {
        self.forwarder.clone()
    }

    /// Join the channel's group on connect, nothing else.
    pub fn executor(&self, channel_id: &str, ttl_secs: i64) -> Result<MintedExecutor> {
        let group = group_for(channel_id)?;
        let token = client_access_token(
            &self.config.endpoint,
            &self.config.hub,
            &self.access_key,
            &format!("svc:{channel_id}"),
            &executor_roles(&group),
            std::slice::from_ref(&group),
            ttl_secs,
        )?;
        Ok(MintedExecutor {
            expires_at: now_unix() + ttl_secs,
            grant: ChannelExecutorGrant::AzureWebPubSub {
                url: client_ws_url(&self.config.endpoint, &self.config.hub, &token),
                group,
            },
        })
    }

    /// Send into the channel's group, never join or read it.
    pub fn client(&self, channel_id: &str, sub: &str, ttl_secs: i64) -> Result<MintedDescriptor> {
        let group = group_for(channel_id)?;
        let token = client_access_token(
            &self.config.endpoint,
            &self.config.hub,
            &self.access_key,
            sub,
            &client_roles(&group),
            &[],
            ttl_secs,
        )?;
        let expires_at = now_unix() + ttl_secs;
        Ok(MintedDescriptor {
            expires_at,
            descriptor: ChannelClientDescriptor::AzureWebPubSub {
                url: client_ws_url(&self.config.endpoint, &self.config.hub, &token),
                group,
                expires_at,
            },
        })
    }
}

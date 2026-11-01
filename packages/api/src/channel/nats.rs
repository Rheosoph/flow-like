//! Scoped NATS credentials for self-hosted run channels.

use std::sync::Arc;

use flow_like_channels::{
    ChannelForwarder,
    nats::{NatsForwarder, Role, mint_user_token, mint_user_token_until, subject_for},
};
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use flow_like_types::{
    Result, anyhow, bail,
    channel::{ChannelClientDescriptor, ChannelExecutorGrant, now_unix},
};

use super::issuer::{MintedDescriptor, MintedExecutor};

pub const URL_ENV: &str = "CHANNEL_NATS_URL";
pub const WS_URL_ENV: &str = "CHANNEL_NATS_WS_URL";
pub const ACCOUNT_SEED_SECRET: &str = "CHANNEL_NATS_ACCOUNT_SEED";

pub struct NatsChannelRuntime {
    url: String,
    ws_url: String,
    account_seed: String,
    forwarder: Arc<NatsForwarder>,
}

fn endpoint(value: &str, browser: bool) -> Result<String> {
    let url = reqwest::Url::parse(value).map_err(|_| anyhow!("invalid NATS endpoint URL"))?;
    let schemes: &[&str] = if browser {
        &["ws", "wss"]
    } else {
        &["nats", "tls", "ws", "wss"]
    };
    if !schemes.contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!matches!(url.scheme(), "ws" | "wss") && !matches!(url.path(), "" | "/"))
    {
        bail!(
            "NATS endpoint must use an allowed scheme without embedded credentials, query or fragment"
        );
    }
    Ok(value.to_owned())
}

impl NatsChannelRuntime {
    pub async fn from_env(secrets: &SecretStore) -> Result<Self> {
        let required = |name| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow!("{name} is required for the nats channel transport"))
        };
        let url = endpoint(required(URL_ENV)?.trim(), false)?;
        let ws_url = endpoint(required(WS_URL_ENV)?.trim(), true)?;
        let account_seed = secrets
            .get_secret_string(&SecretRef::new(ACCOUNT_SEED_SECRET))
            .await
            .map_err(|_| anyhow!("secret {ACCOUNT_SEED_SECRET} could not be resolved"))?
            .expose_secret()
            .trim()
            .to_owned();
        let forwarder = Arc::new(NatsForwarder::new(url.clone(), account_seed.clone())?);
        // Validate the key before dispatch, without needing a broker connection at boot.
        mint_user_token(&account_seed, "startup-probe", Role::Client, 1)?;
        Ok(Self {
            url,
            ws_url,
            account_seed,
            forwarder,
        })
    }

    pub fn forwarder(&self) -> Arc<dyn ChannelForwarder> {
        self.forwarder.clone()
    }

    pub fn client(&self, channel_id: &str, ttl_secs: i64) -> Result<MintedDescriptor> {
        let expires_at = now_unix() + ttl_secs;
        self.client_until(channel_id, expires_at)
    }

    pub fn client_until(&self, channel_id: &str, expires_at: i64) -> Result<MintedDescriptor> {
        Ok(MintedDescriptor {
            expires_at,
            descriptor: ChannelClientDescriptor::Nats {
                url: self.ws_url.clone(),
                subject: subject_for(channel_id)?,
                token: mint_user_token_until(
                    &self.account_seed,
                    channel_id,
                    Role::Client,
                    expires_at,
                )?,
                expires_at,
            },
        })
    }

    pub fn executor(&self, channel_id: &str, ttl_secs: i64) -> Result<MintedExecutor> {
        Ok(MintedExecutor {
            expires_at: now_unix() + ttl_secs,
            grant: ChannelExecutorGrant::Nats {
                url: self.url.clone(),
                subject: subject_for(channel_id)?,
                token: mint_user_token(&self.account_seed, channel_id, Role::Executor, ttl_secs)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_reject_credentials_and_non_websocket_browser_urls() {
        assert!(endpoint("nats://nats:4222", false).is_ok());
        assert!(endpoint("wss://channels.example.test/nats", true).is_ok());
        for url in [
            "nats://nats:4222",
            "https://nats",
            "wss://user:secret@nats",
            "wss://nats?token=secret",
            "wss://nats#fragment",
        ] {
            assert!(endpoint(url, true).is_err(), "{url}");
        }
    }
}

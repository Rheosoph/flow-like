//! Firebase Realtime Database transport: RS256 custom tokens for both sides (signed with the
//! channel service account) and the rules-bypassing forwarder for pushes on the HTTP fallback.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_like_channels::ChannelForwarder;
use flow_like_channels::gcp::{
    AccessTokenProvider, EXECUTOR_UID, FirebaseRtdbForwarder, MAX_CUSTOM_TOKEN_TTL_SECS,
    client_claims, custom_token, executor_claims, inbound_path, inbox_path,
};
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use flow_like_types::channel::{ChannelClientDescriptor, ChannelExecutorGrant, now_unix};
use flow_like_types::sync::Mutex;
use flow_like_types::{Result, anyhow, bail};
use futures_util::future::BoxFuture;

use super::issuer::{MintedDescriptor, MintedExecutor};
use crate::push_notifications::{GoogleServiceAccount, exchange_google_service_account};

pub const DATABASE_URL_ENV: &str = "CHANNEL_FIREBASE_DATABASE_URL";
pub const API_KEY_ENV: &str = "CHANNEL_FIREBASE_API_KEY";
pub const PROJECT_ID_ENV: &str = "CHANNEL_FIREBASE_PROJECT_ID";
pub const SERVICE_ACCOUNT_SECRET: &str = "CHANNEL_FIREBASE_SERVICE_ACCOUNT";
/// Scopes the forwarder's OAuth token needs to write with `?access_token=`.
pub const DATABASE_SCOPES: &str = "https://www.googleapis.com/auth/firebase.database https://www.googleapis.com/auth/userinfo.email";
const ACCESS_TOKEN_CACHE: Duration = Duration::from_secs(55 * 60);

#[derive(Clone, Debug)]
pub struct GcpChannelConfig {
    pub database_url: String,
    pub api_key: String,
    pub project_id: String,
}

impl GcpChannelConfig {
    pub fn from_env() -> Result<Self> {
        let database_url = required(DATABASE_URL_ENV)?;
        if !database_url.starts_with("https://") {
            bail!("{DATABASE_URL_ENV} must be an https:// database url, got '{database_url}'");
        }
        Ok(Self {
            database_url: database_url.trim_end_matches('/').to_string(),
            api_key: required(API_KEY_ENV)?,
            project_id: optional(PROJECT_ID_ENV)
                .or_else(|| optional("GCP_PROJECT_ID"))
                .ok_or_else(|| {
                    anyhow!("{PROJECT_ID_ENV} (or GCP_PROJECT_ID) is required for the gcp_firebase_rtdb channel transport")
                })?,
        })
    }
}

fn required(name: &str) -> Result<String> {
    optional(name)
        .ok_or_else(|| anyhow!("{name} is required for the gcp_firebase_rtdb channel transport"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub struct GcpChannelRuntime {
    config: GcpChannelConfig,
    account: GoogleServiceAccount,
    forwarder: Arc<FirebaseRtdbForwarder>,
}

impl GcpChannelRuntime {
    pub async fn from_env(secrets: &SecretStore) -> Result<Self> {
        let config = GcpChannelConfig::from_env()?;
        let service_account_json = secrets
            .get_secret_string(&SecretRef::new(SERVICE_ACCOUNT_SECRET))
            .await
            .map_err(|e| anyhow!("secret {SERVICE_ACCOUNT_SECRET} could not be resolved: {e}"))?
            .expose_secret()
            .to_string();
        let account = GoogleServiceAccount::parse(&service_account_json)
            .map_err(|e| anyhow!("secret {SERVICE_ACCOUNT_SECRET}: {e}"))?;
        let forwarder = Arc::new(FirebaseRtdbForwarder::new(
            &config.database_url,
            access_token_provider(service_account_json),
        )?);
        Ok(Self {
            config,
            account,
            forwarder,
        })
    }

    pub fn forwarder(&self) -> Arc<dyn ChannelForwarder> {
        self.forwarder.clone()
    }

    fn token(&self, uid: &str, claims: serde_json::Value, ttl_secs: i64) -> Result<String> {
        custom_token(
            &self.account.client_email,
            &self.account.private_key,
            self.account.private_key_id.as_deref(),
            uid,
            claims,
            ttl_secs,
        )
    }

    fn expiry(ttl_secs: i64) -> i64 {
        now_unix() + ttl_secs.clamp(1, MAX_CUSTOM_TOKEN_TTL_SECS)
    }

    /// `role = server` token for this run only, plus the `meta` node the sweeper keys on.
    pub async fn executor(
        &self,
        channel_id: &str,
        sub: &str,
        ttl_secs: i64,
    ) -> Result<MintedExecutor> {
        let inbox_path = inbox_path(channel_id)?;
        let inbound_path = inbound_path(channel_id)?;
        let token = self.token(EXECUTOR_UID, executor_claims(channel_id), ttl_secs)?;
        self.forwarder.create_channel_meta(channel_id, sub).await?;
        Ok(MintedExecutor {
            expires_at: Self::expiry(ttl_secs),
            grant: ChannelExecutorGrant::GcpFirebaseRtdb {
                database_url: self.config.database_url.clone(),
                api_key: self.config.api_key.clone(),
                custom_token: token,
                inbox_path,
                inbound_path,
            },
        })
    }

    /// `role = client` token bound to this run; the rules only let it create single-`payload`
    /// children under the run's `inbox` / `inbound`.
    pub fn client(&self, channel_id: &str, sub: &str, ttl_secs: i64) -> Result<MintedDescriptor> {
        let inbox_path = inbox_path(channel_id)?;
        let inbound_path = inbound_path(channel_id)?;
        let token = self.token(sub, client_claims(channel_id), ttl_secs)?;
        let expires_at = Self::expiry(ttl_secs);
        Ok(MintedDescriptor {
            expires_at,
            descriptor: ChannelClientDescriptor::GcpFirebaseRtdb {
                database_url: self.config.database_url.clone(),
                api_key: self.config.api_key.clone(),
                project_id: self.config.project_id.clone(),
                custom_token: token,
                inbox_path,
                inbound_path,
                expires_at,
            },
        })
    }
}

/// OAuth token for the forwarder's rules-bypassing writes; cached for 55 minutes like the
/// push-notification token. The service-account key never leaves this closure.
fn access_token_provider(service_account_json: String) -> AccessTokenProvider {
    let service_account_json = Arc::new(service_account_json);
    let cache: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
    Arc::new(move || -> BoxFuture<'static, Result<String>> {
        let service_account_json = service_account_json.clone();
        let cache = cache.clone();
        Box::pin(async move {
            let mut cached = cache.lock().await;
            if let Some((token, expires_at)) = cached.as_ref()
                && Instant::now() < *expires_at
            {
                return Ok(token.clone());
            }
            let token =
                exchange_google_service_account(&service_account_json, DATABASE_SCOPES).await?;
            *cached = Some((token.clone(), Instant::now() + ACCESS_TOKEN_CACHE));
            Ok(token)
        })
    })
}

mod cloudflare;

use async_trait::async_trait;
use cloudflare::CloudflareRealtimeIceProvider;
use flow_like::hub::{RealtimeConfig, RealtimeIceConfig};
use flow_like_secrets::SecretStore;
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use utoipa::ToSchema;

const MAX_ICE_SERVERS: usize = 16;
const MAX_URLS_PER_SERVER: usize = 16;
const MAX_ICE_URL_LENGTH: usize = 2_048;
const MAX_CREDENTIAL_LENGTH: usize = 4_096;

/// Browser-ready STUN or TURN configuration returned by the realtime endpoint.
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct RealtimeIceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl fmt::Debug for RealtimeIceServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealtimeIceServer")
            .field("urls", &self.urls)
            .field("username", &self.username.as_ref().map(|_| "[REDACTED]"))
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedRealtimeIceServers {
    pub ice_servers: Vec<RealtimeIceServer>,
    /// Unix timestamp in seconds. The client refreshes before this time.
    pub expires_at: i64,
}

/// Issues client-scoped ICE configuration without exposing a provider's
/// long-lived credential to the browser.
#[async_trait]
pub trait RealtimeIceProvider: Send + Sync {
    async fn issue(&self, issuance_id: &str) -> Result<IssuedRealtimeIceServers>;
}

#[derive(Clone, Default)]
pub struct RealtimeIceService {
    provider: Option<Arc<dyn RealtimeIceProvider>>,
}

impl RealtimeIceService {
    /// Builds the selected provider without resolving its secret values.
    pub fn from_config(config: &RealtimeConfig, secrets: Arc<SecretStore>) -> Result<Self> {
        let Some(config) = config.ice.as_ref() else {
            return Ok(Self::default());
        };

        let provider: Arc<dyn RealtimeIceProvider> = match config {
            RealtimeIceConfig::Cloudflare {
                turn_key_id_secret_ref,
                turn_key_api_token_secret_ref,
                ttl_seconds,
            } => Arc::new(CloudflareRealtimeIceProvider::new(
                secrets,
                turn_key_id_secret_ref,
                turn_key_api_token_secret_ref,
                *ttl_seconds,
            )?),
        };

        Ok(Self {
            provider: Some(provider),
        })
    }

    pub async fn issue(&self, issuance_id: &str) -> Result<Option<IssuedRealtimeIceServers>> {
        match self.provider.as_ref() {
            Some(provider) => provider.issue(issuance_id).await.map(Some),
            None => Ok(None),
        }
    }
}

fn validate_ice_servers(ice_servers: &[RealtimeIceServer]) -> Result<()> {
    if ice_servers.is_empty() || ice_servers.len() > MAX_ICE_SERVERS {
        return Err(anyhow!(
            "realtime ICE response must contain between 1 and {MAX_ICE_SERVERS} servers"
        ));
    }

    for server in ice_servers {
        if server.urls.is_empty() || server.urls.len() > MAX_URLS_PER_SERVER {
            return Err(anyhow!(
                "realtime ICE server must contain between 1 and {MAX_URLS_PER_SERVER} URLs"
            ));
        }
        for url in &server.urls {
            if url.is_empty() || url.len() > MAX_ICE_URL_LENGTH {
                return Err(anyhow!("realtime ICE server URL has an invalid length"));
            }
            let scheme = url.split_once(':').map(|(scheme, _)| scheme);
            if !matches!(scheme, Some("stun" | "stuns" | "turn" | "turns")) {
                return Err(anyhow!("realtime ICE server URL uses an invalid scheme"));
            }
        }

        let uses_turn = server.urls.iter().any(|url| {
            matches!(
                url.split_once(':').map(|(scheme, _)| scheme),
                Some("turn" | "turns")
            )
        });
        match (server.username.as_deref(), server.credential.as_deref()) {
            (Some(username), Some(credential)) => {
                if username.trim().is_empty() || credential.trim().is_empty() {
                    return Err(anyhow!("realtime ICE credential is empty"));
                }
                if username.len() > MAX_CREDENTIAL_LENGTH
                    || credential.len() > MAX_CREDENTIAL_LENGTH
                {
                    return Err(anyhow!("realtime ICE credential is too long"));
                }
            }
            (None, None) if !uses_turn => {}
            _ => {
                return Err(anyhow!(
                    "realtime TURN server must contain a username and credential"
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_service_omits_ice_servers() {
        let issued = RealtimeIceService::default().issue("issuance-id").await;
        assert_eq!(issued.unwrap(), None);
    }

    #[test]
    fn rejects_turn_configuration_without_credentials() {
        let malformed = vec![RealtimeIceServer {
            urls: vec!["turn:turn.cloudflare.com:3478".to_string()],
            username: None,
            credential: None,
        }];

        assert!(validate_ice_servers(&malformed).is_err());
    }

    #[test]
    fn debug_output_redacts_turn_credentials() {
        let server = RealtimeIceServer {
            urls: vec!["turn:turn.cloudflare.com:3478".to_string()],
            username: Some("sensitive-user".to_string()),
            credential: Some("sensitive-password".to_string()),
        };
        let debug = format!("{server:?}");

        assert!(!debug.contains("sensitive-user"));
        assert!(!debug.contains("sensitive-password"));
        assert!(debug.contains("[REDACTED]"));
    }
}

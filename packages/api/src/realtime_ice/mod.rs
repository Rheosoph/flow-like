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
/// Bounded issuance never mints a relay credential shorter than this.
pub const MIN_BOUNDED_TTL_SECONDS: u32 = 5 * 60;
const MAX_REUSED_SUBJECTS: u64 = 10_000;

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
    /// `max_ttl_seconds` only ever shortens the configured credential lifetime.
    async fn issue(
        &self,
        issuance_id: &str,
        max_ttl_seconds: Option<u32>,
    ) -> Result<IssuedRealtimeIceServers>;
}

#[derive(Clone)]
struct ReusedCredential {
    issued: IssuedRealtimeIceServers,
    /// A provider TTL below the request caps the lifetime actually granted.
    lifetime_seconds: i64,
}

#[derive(Clone)]
pub struct RealtimeIceService {
    provider: Option<Arc<dyn RealtimeIceProvider>>,
    reused: moka::sync::Cache<String, ReusedCredential>,
}

impl Default for RealtimeIceService {
    fn default() -> Self {
        Self::with_provider(None)
    }
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

        Ok(Self::with_provider(Some(provider)))
    }

    fn with_provider(provider: Option<Arc<dyn RealtimeIceProvider>>) -> Self {
        Self {
            provider,
            reused: moka::sync::Cache::builder()
                .max_capacity(MAX_REUSED_SUBJECTS)
                .time_to_live(std::time::Duration::from_secs(48 * 60 * 60))
                .build(),
        }
    }

    pub async fn issue(&self, issuance_id: &str) -> Result<Option<IssuedRealtimeIceServers>> {
        match self.provider.as_ref() {
            Some(provider) => provider.issue(issuance_id, None).await.map(Some),
            None => Ok(None),
        }
    }

    /// Relay credentials cannot be revoked, so a subject's credential lapses with
    /// its authorization and renewals reuse it instead of minting another one.
    /// A credential is reused while at least half of its granted lifetime remains
    /// and it does not outlive the subject's current authorization.
    pub async fn issue_bounded(
        &self,
        subject: &str,
        issuance_id: &str,
        max_ttl_seconds: u32,
        now: i64,
    ) -> Result<Option<IssuedRealtimeIceServers>> {
        let Some(provider) = self.provider.as_ref() else {
            return Ok(None);
        };
        if max_ttl_seconds < MIN_BOUNDED_TTL_SECONDS {
            return Ok(None);
        }
        if let Some(reused) = self
            .reused
            .get(subject)
            .filter(|reused| reusable(reused, max_ttl_seconds, now))
        {
            return Ok(Some(reused.issued));
        }
        let issued = provider.issue(issuance_id, Some(max_ttl_seconds)).await?;
        self.reused.insert(
            subject.to_owned(),
            ReusedCredential {
                lifetime_seconds: issued.expires_at.saturating_sub(now),
                issued: issued.clone(),
            },
        );
        Ok(Some(issued))
    }
}

/// Providers stamp expiry from their own clock read, which can land after the caller's `now`.
const REUSE_BOUND_SLACK_SECONDS: i64 = 5;

fn reusable(reused: &ReusedCredential, max_ttl_seconds: u32, now: i64) -> bool {
    let bound = now
        .saturating_add(i64::from(max_ttl_seconds))
        .saturating_add(REUSE_BOUND_SLACK_SECONDS);
    let expires_at = reused.issued.expires_at;
    expires_at <= bound && expires_at.saturating_sub(now) >= reused.lifetime_seconds / 2
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

    struct CountingProvider {
        ttl_seconds: u32,
        issued: std::sync::Mutex<Vec<Option<u32>>>,
    }

    #[async_trait]
    impl RealtimeIceProvider for CountingProvider {
        async fn issue(
            &self,
            _issuance_id: &str,
            max_ttl_seconds: Option<u32>,
        ) -> Result<IssuedRealtimeIceServers> {
            self.issued.lock().unwrap().push(max_ttl_seconds);
            let ttl = max_ttl_seconds.map_or(self.ttl_seconds, |max| max.min(self.ttl_seconds));
            Ok(IssuedRealtimeIceServers {
                ice_servers: vec![RealtimeIceServer {
                    urls: vec!["turn:turn.example:3478".into()],
                    username: Some(format!("user-{}", self.issued.lock().unwrap().len())),
                    credential: Some("credential".into()),
                }],
                expires_at: chrono::Utc::now().timestamp() + i64::from(ttl),
            })
        }
    }

    #[tokio::test]
    async fn bounded_issuance_caps_lifetime_and_reuses_a_subjects_credential() {
        let provider = Arc::new(CountingProvider {
            ttl_seconds: 4 * 60 * 60,
            issued: Default::default(),
        });
        let service = RealtimeIceService::with_provider(Some(provider.clone()));
        let now = chrono::Utc::now().timestamp();
        let first = service
            .issue_bounded("device:1:device:a", "one", 3600, now)
            .await
            .unwrap()
            .unwrap();
        assert!(first.expires_at <= now + 3600 + 1);
        let renewed = service
            .issue_bounded("device:1:device:a", "two", 3600, now + 240)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renewed, first);
        let other = service
            .issue_bounded("device:1:controller:b", "three", 3600, now)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(other, first);
        // A shortened grant must not keep receiving the longer credential.
        let narrowed = service
            .issue_bounded("device:1:device:a", "four", 900, now + 300)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(narrowed, first);
        assert!(narrowed.expires_at <= chrono::Utc::now().timestamp() + 900);
        assert_eq!(
            service
                .issue_bounded(
                    "device:1:device:a",
                    "five",
                    MIN_BOUNDED_TTL_SECONDS - 1,
                    now
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            *provider.issued.lock().unwrap(),
            vec![Some(3600), Some(3600), Some(900)]
        );
    }

    #[tokio::test]
    async fn reuse_follows_the_granted_lifetime_and_tolerates_a_later_provider_clock() {
        let provider = Arc::new(CountingProvider {
            ttl_seconds: 600,
            issued: Default::default(),
        });
        let service = RealtimeIceService::with_provider(Some(provider.clone()));
        // The provider reads its clock a second after the caller did.
        let now = chrono::Utc::now().timestamp() - 1;
        let first = service
            .issue_bounded("device:1:device:a", "one", 3600, now)
            .await
            .unwrap()
            .unwrap();
        // A provider TTL below half the requested bound still allows renewals to reuse.
        for (issuance, at) in [("two", now), ("three", now + 235)] {
            let reused = service
                .issue_bounded("device:1:device:a", issuance, 3600, at)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(reused, first);
        }
        let lapsing = service
            .issue_bounded("device:1:device:a", "four", 3600, now + 302)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(lapsing, first);
        let bounded = service
            .issue_bounded("device:1:controller:b", "five", 600, now)
            .await
            .unwrap()
            .unwrap();
        let reticketed = service
            .issue_bounded("device:1:controller:b", "six", 600, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reticketed, bounded);
        assert_eq!(
            *provider.issued.lock().unwrap(),
            vec![Some(3600), Some(3600), Some(600)]
        );
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

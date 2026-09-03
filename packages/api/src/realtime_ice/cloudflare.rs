use super::{
    IssuedRealtimeIceServers, RealtimeIceProvider, RealtimeIceServer, validate_ice_servers,
};
use async_trait::async_trait;
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore, SecretString};
use flow_like_types::{Result, anyhow, tokio::sync::OnceCell};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

const TURN_API_ORIGIN: &str = "https://rtc.live.cloudflare.com";
const MIN_TTL_SECONDS: u32 = 5 * 60;
const MAX_TTL_SECONDS: u32 = 48 * 60 * 60;
const MAX_RESPONSE_BYTES: usize = 64 * 1_024;

struct CloudflareCredentials {
    endpoint: String,
    turn_key_api_token: Arc<SecretString>,
}

pub(super) struct CloudflareRealtimeIceProvider {
    client: reqwest::Client,
    secrets: Arc<SecretStore>,
    turn_key_id_secret_ref: SecretRef,
    turn_key_api_token_secret_ref: SecretRef,
    /// Coalesces concurrent first reads and retains successful values for this process.
    credentials: OnceCell<CloudflareCredentials>,
    ttl_seconds: u32,
}

#[derive(Serialize)]
struct CloudflareCredentialRequest {
    ttl: u32,
}

#[derive(Deserialize)]
struct CloudflareCredentialResponse {
    #[serde(rename = "iceServers")]
    ice_servers: Vec<RealtimeIceServer>,
}

impl CloudflareRealtimeIceProvider {
    pub(super) fn new(
        secrets: Arc<SecretStore>,
        turn_key_id_secret_ref: &str,
        turn_key_api_token_secret_ref: &str,
        ttl_seconds: u32,
    ) -> Result<Self> {
        if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl_seconds) {
            return Err(anyhow!(
                "Cloudflare realtime ICE ttl_seconds must be between {MIN_TTL_SECONDS} and {MAX_TTL_SECONDS}"
            ));
        }

        let turn_key_id_secret_ref = parse_secret_ref(turn_key_id_secret_ref)?;
        let turn_key_api_token_secret_ref = parse_secret_ref(turn_key_api_token_secret_ref)?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "flow-like-realtime-ice/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| anyhow!("failed to construct realtime ICE HTTP client: {error}"))?;

        Ok(Self {
            client,
            secrets,
            turn_key_id_secret_ref,
            turn_key_api_token_secret_ref,
            credentials: OnceCell::new(),
            ttl_seconds,
        })
    }

    async fn credentials(&self) -> Result<&CloudflareCredentials> {
        self.credentials
            .get_or_try_init(|| async {
                let turn_key_id = self
                    .read_secret(&self.turn_key_id_secret_ref)
                    .await?;
                let turn_key_api_token = self
                    .read_secret(&self.turn_key_api_token_secret_ref)
                    .await?;
                let turn_key_id = turn_key_id.expose_secret().trim();
                if turn_key_id.len() != 32
                    || !turn_key_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return Err(anyhow!(
                        "Cloudflare realtime ICE TURN key identifier is malformed"
                    ));
                }

                let turn_key_api_token = turn_key_api_token.expose_secret().trim();
                if turn_key_api_token.is_empty() {
                    return Err(anyhow!(
                        "Cloudflare realtime ICE TURN key API token is empty"
                    ));
                }

                Ok(CloudflareCredentials {
                    endpoint: format!(
                        "{TURN_API_ORIGIN}/v1/turn/keys/{turn_key_id}/credentials/generate-ice-servers"
                    ),
                    turn_key_api_token: Arc::new(SecretString::from(
                        turn_key_api_token.to_string(),
                    )),
                })
            })
            .await
    }

    async fn read_secret(&self, reference: &SecretRef) -> Result<Arc<SecretString>> {
        self.secrets
            .get_secret_string(reference)
            .await
            .map_err(|_| anyhow!("configured realtime ICE secret is unavailable"))
    }

    fn parse_response(body: &[u8]) -> Result<Vec<RealtimeIceServer>> {
        let response: CloudflareCredentialResponse = serde_json::from_slice(body)
            .map_err(|error| anyhow!("Cloudflare realtime ICE response is malformed: {error}"))?;
        validate_ice_servers(&response.ice_servers)?;
        Ok(response.ice_servers)
    }
}

#[async_trait]
impl RealtimeIceProvider for CloudflareRealtimeIceProvider {
    async fn issue(&self, _issuance_id: &str) -> Result<IssuedRealtimeIceServers> {
        let credentials = self.credentials().await?;
        let issued_at = chrono::Utc::now().timestamp();
        let mut response = self
            .client
            .post(&credentials.endpoint)
            .bearer_auth(credentials.turn_key_api_token.expose_secret())
            .json(&CloudflareCredentialRequest {
                ttl: self.ttl_seconds,
            })
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "Cloudflare realtime ICE request failed: {}",
                    error.without_url()
                )
            })?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Cloudflare realtime ICE request returned HTTP {}",
                response.status().as_u16()
            ));
        }

        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(anyhow!(
                "Cloudflare realtime ICE response exceeds the size limit"
            ));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            anyhow!(
                "failed to read Cloudflare realtime ICE response: {}",
                error.without_url()
            )
        })? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(anyhow!(
                    "Cloudflare realtime ICE response exceeds the size limit"
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let ice_servers = Self::parse_response(&body)?;
        let expires_at = issued_at.saturating_add(i64::from(self.ttl_seconds));

        Ok(IssuedRealtimeIceServers {
            ice_servers,
            expires_at,
        })
    }
}

fn parse_secret_ref(reference: &str) -> Result<SecretRef> {
    SecretRef::try_from(reference.trim())
        .map_err(|_| anyhow!("realtime ICE secret-store reference is invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretStoreConfig};

    #[test]
    fn parses_cloudflare_browser_configuration() {
        let response = br#"{
            "iceServers": [
                {"urls": ["stun:stun.cloudflare.com:3478"]},
                {
                    "urls": [
                        "turn:turn.cloudflare.com:3478?transport=udp",
                        "turns:turn.cloudflare.com:443?transport=tcp"
                    ],
                    "username": "temporary-user",
                    "credential": "temporary-password"
                }
            ]
        }"#;

        let servers = CloudflareRealtimeIceProvider::parse_response(response).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(
            servers[0].urls,
            vec!["stun:stun.cloudflare.com:3478".to_string()]
        );
        assert_eq!(servers[1].username.as_deref(), Some("temporary-user"));
    }

    #[test]
    fn rejects_invalid_urls_and_unsupported_ttls() {
        let malformed = br#"{"iceServers":[{"urls":["https://example.com"]}]}"#;
        assert!(CloudflareRealtimeIceProvider::parse_response(malformed).is_err());

        for ttl_seconds in [MIN_TTL_SECONDS - 1, MAX_TTL_SECONDS + 1] {
            let result = CloudflareRealtimeIceProvider::new(
                empty_secret_store(),
                "turn-key-id",
                "turn-key-api-token",
                ttl_seconds,
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn accepts_supported_ttl_boundaries_without_loading_secrets() {
        for ttl_seconds in [MIN_TTL_SECONDS, MAX_TTL_SECONDS] {
            let provider = CloudflareRealtimeIceProvider::new(
                empty_secret_store(),
                "turn-key-id",
                "turn-key-api-token",
                ttl_seconds,
            )
            .unwrap();
            assert!(provider.credentials.get().is_none());
        }
    }

    #[tokio::test]
    async fn loads_secrets_once_on_first_use() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("turn-key-id"), "a".repeat(32)).unwrap();
        std::fs::write(directory.path().join("turn-key-api-token"), "token").unwrap();
        let secrets = Arc::new(
            SecretStore::new(
                SecretStoreConfig::default()
                    .with_allow_env_override(false)
                    .with_provider(ProviderConfig::File(FileProviderConfig {
                        root_path: directory.path().to_path_buf(),
                        trim_trailing_newline: true,
                    })),
            )
            .unwrap(),
        );
        let provider = CloudflareRealtimeIceProvider::new(
            secrets,
            "secret://file/turn-key-id",
            "secret://file/turn-key-api-token",
            MIN_TTL_SECONDS,
        )
        .unwrap();

        assert!(provider.credentials.get().is_none());
        let first = provider.credentials().await.unwrap();
        let second = provider.credentials().await.unwrap();

        assert!(std::ptr::eq(first, second));
    }

    fn empty_secret_store() -> Arc<SecretStore> {
        Arc::new(
            SecretStore::new(SecretStoreConfig::default().with_allow_env_override(false)).unwrap(),
        )
    }
}

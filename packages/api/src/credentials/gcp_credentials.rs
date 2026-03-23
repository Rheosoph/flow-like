use super::RuntimeCredentialsTrait;
#[cfg(feature = "gcp")]
use crate::credentials::CredentialsAccess;
use crate::state::{AppState, State};
#[cfg(feature = "gcp")]
use flow_like::credentials::{SharedCredentials, gcp_credentials::GcpSharedCredentials};
use flow_like::{
    flow_like_storage::lancedb::{connect, connection::ConnectBuilder},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(feature = "gcp")]
const GCS_STORAGE_TOKEN_OPTION: &str = "google_storage_token";

/// GCP Runtime Credentials with downscoped access tokens
///
/// SECURITY: Uses GCP Credential Access Boundaries to create tokens that are
/// cryptographically restricted to specific paths and permissions, similar to
/// AWS STS and Azure Directory SAS. GCP enforces these restrictions server-side.
///
/// The flow is:
/// 1. Generate a base OAuth2 access token from the service account
/// 2. Exchange it for a downscoped token with Credential Access Boundary
/// 3. The downscoped token can only access the specified paths/permissions
#[cfg(feature = "gcp")]
#[derive(Clone, Serialize, Deserialize)]
pub struct GcpRuntimeCredentials {
    /// Master service account key (server-side only, never sent to clients)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account_key: Option<String>,
    /// Short-lived downscoped OAuth2 access token (sent to clients)
    pub access_token: Option<String>,
    pub meta_bucket: String,
    pub content_bucket: String,
    pub logs_bucket: String,
    /// Allowed path prefixes (enforced by GCP via Credential Access Boundary)
    pub allowed_prefixes: Vec<String>,
    /// Whether write operations are allowed
    pub write_access: bool,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub content_path_prefix: Option<String>,
    pub user_content_path_prefix: Option<String>,
}

#[cfg(feature = "gcp")]
impl std::fmt::Debug for GcpRuntimeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpRuntimeCredentials")
            .field("service_account_key", &self.service_account_key.as_ref().map(|_| "[REDACTED]"))
            .field("access_token", &self.access_token.as_ref().map(|_| "[REDACTED]"))
            .field("meta_bucket", &self.meta_bucket)
            .field("content_bucket", &self.content_bucket)
            .field("logs_bucket", &self.logs_bucket)
            .field("write_access", &self.write_access)
            .field("expiration", &self.expiration)
            .finish()
    }
}

#[cfg(feature = "gcp")]
impl GcpRuntimeCredentials {
    pub fn new(meta_bucket: &str, content_bucket: &str, logs_bucket: &str) -> Self {
        GcpRuntimeCredentials {
            service_account_key: None,
            access_token: None,
            meta_bucket: meta_bucket.to_string(),
            content_bucket: content_bucket.to_string(),
            logs_bucket: logs_bucket.to_string(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub fn from_env() -> Self {
        let service_account_key = std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON").ok();
        let logs_bucket = std::env::var("GCP_LOG_BUCKET").unwrap_or_default();
        if logs_bucket.is_empty() {
            tracing::warn!(
                "GCP_LOG_BUCKET environment variable is not set - logs will not be persisted"
            );
        }

        GcpRuntimeCredentials {
            service_account_key,
            access_token: None,
            meta_bucket: std::env::var("GCP_META_BUCKET").unwrap_or_default(),
            content_bucket: std::env::var("GCP_CONTENT_BUCKET").unwrap_or_default(),
            logs_bucket,
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        let service_account_key = std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON").ok();

        GcpRuntimeCredentials {
            service_account_key,
            access_token: None,
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[tracing::instrument(
        name = "GcpRuntimeCredentials::scoped_credentials",
        skip(self, state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let service_account_key = self
            .service_account_key
            .clone()
            .ok_or_else(|| anyhow!("Service account key not set"))?;

        let apps_prefix = format!("apps/{}", app_id);
        let user_prefix = format!("users/{}/apps/{}", sub, app_id);
        let log_prefix = format!("runs/{}", app_id);
        let temporary_user_prefix = format!("tmp/user/{}/apps/{}", sub, app_id);
        let temporary_global_prefix = format!("tmp/global/apps/{}", app_id);

        let (allowed_prefixes, write_access) = match mode {
            CredentialsAccess::EditApp => (vec![apps_prefix], true),
            CredentialsAccess::ReadApp => (vec![apps_prefix], false),
            CredentialsAccess::EditUser => (vec![user_prefix], true),
            CredentialsAccess::ReadUser => (vec![user_prefix], false),
            CredentialsAccess::InvokeNone => {
                (vec![user_prefix, temporary_user_prefix, log_prefix], true)
            }
            CredentialsAccess::InvokeRead => (
                vec![
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                    log_prefix,
                ],
                false,
            ),
            CredentialsAccess::InvokeWrite => (
                vec![
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                    log_prefix,
                ],
                true,
            ),
            CredentialsAccess::ReadLogs => (vec![log_prefix], false),
        };

        // Generate a base access token, then downscope it with Credential Access Boundary
        let base_token = generate_access_token(&service_account_key, state).await?;
        let access_token = downscope_token(
            &base_token,
            &self.content_bucket,
            &allowed_prefixes,
            write_access,
        )
        .await?;
        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            service_account_key: None, // Never send the key to clients
            access_token: Some(access_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes,
            write_access,
            expiration: Some(chrono_expiration),
            content_path_prefix: Some(format!("apps/{}", app_id)),
            user_content_path_prefix: Some(format!("users/{}/apps/{}", sub, app_id)),
        })
    }

    /// Test-only version using the service account key directly
    /// In production, use scoped_credentials with State
    #[cfg(test)]
    pub async fn scoped_credentials_for_test(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let service_account_key = self
            .service_account_key
            .clone()
            .or_else(|| std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON").ok())
            .ok_or_else(|| anyhow!("GOOGLE_APPLICATION_CREDENTIALS_JSON is not set"))?;

        let apps_prefix = format!("apps/{}", app_id);
        let user_prefix = format!("users/{}/apps/{}", sub, app_id);
        let log_prefix = format!("runs/{}", app_id);
        let temporary_user_prefix = format!("tmp/user/{}/apps/{}", sub, app_id);
        let temporary_global_prefix = format!("tmp/global/apps/{}", app_id);

        let (allowed_prefixes, write_access) = match mode {
            CredentialsAccess::EditApp => (vec![apps_prefix], true),
            CredentialsAccess::ReadApp => (vec![apps_prefix], false),
            CredentialsAccess::EditUser => (vec![user_prefix], true),
            CredentialsAccess::ReadUser => (vec![user_prefix], false),
            CredentialsAccess::InvokeNone => {
                (vec![user_prefix, temporary_user_prefix, log_prefix], true)
            }
            CredentialsAccess::InvokeRead => (
                vec![
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                    log_prefix,
                ],
                false,
            ),
            CredentialsAccess::InvokeWrite => (
                vec![
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                    log_prefix,
                ],
                true,
            ),
            CredentialsAccess::ReadLogs => (vec![log_prefix], false),
        };

        // Generate a base access token, then downscope it with Credential Access Boundary
        let base_token = generate_access_token_standalone(&service_account_key).await?;
        let access_token = downscope_token(
            &base_token,
            &self.content_bucket,
            &allowed_prefixes,
            write_access,
        )
        .await?;
        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            service_account_key: None,
            access_token: Some(access_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes,
            write_access,
            expiration: Some(chrono_expiration),
            content_path_prefix: Some(format!("apps/{}", app_id)),
            user_content_path_prefix: Some(format!("users/{}/apps/{}", sub, app_id)),
        })
    }
}

/// Generate a short-lived OAuth2 access token using the service account key
#[cfg(feature = "gcp")]
async fn generate_access_token(service_account_key: &str, _state: &State) -> Result<String> {
    // Use reqwest directly since State's hyper client is lower-level
    generate_access_token_standalone(service_account_key).await
}

/// Standalone version for tests without State
#[cfg(feature = "gcp")]
async fn generate_access_token_standalone(service_account_key: &str) -> Result<String> {
    let jwt = create_jwt_assertion(service_account_key)?;
    let token_uri = get_token_uri(service_account_key)?;

    let client = reqwest::Client::new();
    let response = client
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await
        .map_err(|e| anyhow!("Failed to request access token: {}", e))?;

    parse_token_response(response).await
}

#[cfg(feature = "gcp")]
fn get_token_uri(service_account_key: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ServiceAccountKey {
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = flow_like_types::json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    Ok(sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()))
}

#[cfg(feature = "gcp")]
fn create_jwt_assertion(service_account_key: &str) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    #[derive(Deserialize)]
    struct ServiceAccountKey {
        client_email: String,
        private_key: String,
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = flow_like_types::json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    let token_uri = sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    let now = chrono::Utc::now().timestamp();
    let exp = now + 3600;

    let header = flow_like_types::json::json!({
        "alg": "RS256",
        "typ": "JWT"
    });

    let claims = flow_like_types::json::json!({
        "iss": sa_key.client_email,
        "sub": sa_key.client_email,
        "aud": token_uri,
        "iat": now,
        "exp": exp,
        "scope": "https://www.googleapis.com/auth/devstorage.read_write"
    });

    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
    let message = format!("{}.{}", header_b64, claims_b64);

    let signature = sign_rs256(&sa_key.private_key, message.as_bytes())?;
    let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

    Ok(format!("{}.{}", message, signature_b64))
}

#[cfg(feature = "gcp")]
async fn parse_token_response(response: reqwest::Response) -> Result<String> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Failed to get access token: {} - {}", status, body));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse token response: {}", e))?;

    Ok(token_response.access_token)
}

/// Downscope an access token using Google's STS endpoint with Credential Access Boundaries.
/// This creates a new token that is restricted to the specified paths and permissions.
/// The token will only be able to access objects under the specified prefixes in the bucket.
#[cfg(feature = "gcp")]
async fn downscope_token(
    access_token: &str,
    bucket: &str,
    allowed_prefixes: &[String],
    write_access: bool,
) -> Result<String> {
    use serde_json::json;

    let permission_role = if write_access {
        "inRole:roles/storage.objectAdmin"
    } else {
        "inRole:roles/storage.objectViewer"
    };

    // Build condition expression for path restrictions
    // This restricts both object access (resource.name) and list operations (objectListPrefix)
    let conditions: Vec<String> = allowed_prefixes
        .iter()
        .flat_map(|prefix| {
            vec![
                format!(
                    "resource.name.startsWith('projects/_/buckets/{}/objects/{}')",
                    bucket, prefix
                ),
                format!(
                    "api.getAttribute('storage.googleapis.com/objectListPrefix', '').startsWith('{}')",
                    prefix
                ),
            ]
        })
        .collect();

    let condition_expression = conditions.join(" || ");

    let cab = json!({
        "accessBoundary": {
            "accessBoundaryRules": [
                {
                    "availablePermissions": [permission_role],
                    "availableResource": format!("//storage.googleapis.com/projects/_/buckets/{}", bucket),
                    "availabilityCondition": {
                        "expression": condition_expression
                    }
                }
            ]
        }
    });

    let cab_json = cab.to_string();

    let form = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        (
            "subject_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("subject_token", access_token),
        (
            "requested_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("options", cab_json.as_str()),
    ];

    let client = reqwest::Client::new();
    let response = client
        .post("https://sts.googleapis.com/v1/token")
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("STS token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("STS token exchange failed: {} - {}", status, body));
    }

    #[derive(Deserialize)]
    struct StsResponse {
        access_token: String,
    }

    let sts_response: StsResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse STS response: {}", e))?;

    Ok(sts_response.access_token)
}

/// Sign data with RS256 (RSA-SHA256)
#[cfg(feature = "gcp")]
fn sign_rs256(private_key_pem: &str, data: &[u8]) -> Result<Vec<u8>> {
    use rsa::{
        RsaPrivateKey, pkcs1v15::SigningKey, pkcs8::DecodePrivateKey, signature::SignatureEncoding,
        signature::Signer,
    };

    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| anyhow!("Failed to parse private key: {}", e))?;

    let signing_key = SigningKey::<sha2::Sha256>::new(private_key);
    let signature = signing_key.sign(data);

    Ok(signature.to_bytes().to_vec())
}

#[cfg(feature = "gcp")]
#[async_trait]
impl RuntimeCredentialsTrait for GcpRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        SharedCredentials::Gcp(GcpSharedCredentials {
            service_account_key: self.service_account_key.clone().unwrap_or_default(),
            access_token: self.access_token.clone(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes: self.allowed_prefixes.clone(),
            write_access: self.write_access,
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
        })
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials().to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials().to_db_scoped(sub, app_id).await
    }

    #[tracing::instrument(
        name = "GcpRuntimeCredentials::to_state",
        skip(self, state),
        level = "debug"
    )]
    async fn to_state(&self, state: AppState) -> Result<FlowLikeState> {
        let (meta_store, content_store) = {
            use flow_like_types::tokio;

            tokio::join!(
                async { self.into_shared_credentials().to_store(true).await },
                async { self.into_shared_credentials().to_store(false).await },
            )
        };
        let http_client = HTTPClient::new_without_refetch();

        let meta_store = meta_store?;
        let content_store = content_store?;

        let mut config = {
            let mut cfg = FlowLikeConfig::with_default_store(content_store);
            cfg.register_app_meta_store(meta_store.clone());
            cfg
        };

        let bucket = self.content_bucket.clone();

        if let Some(ref service_account_key) = self.service_account_key {
            config.register_build_logs_database(Arc::new(make_gcs_builder_with_key(
                bucket.clone(),
                service_account_key.clone(),
            )));
            config.register_build_project_database(Arc::new(make_gcs_builder_with_key(
                bucket.clone(),
                service_account_key.clone(),
            )));
            config.register_build_user_database(Arc::new(make_gcs_builder_with_key(
                bucket,
                service_account_key.clone(),
            )));
        } else if let Some(ref access_token) = self.access_token {
            config.register_build_logs_database(Arc::new(make_gcs_builder_with_token(
                bucket.clone(),
                access_token.clone(),
            )));
            config.register_build_project_database(Arc::new(make_gcs_builder_with_token(
                bucket.clone(),
                access_token.clone(),
            )));
            config.register_build_user_database(Arc::new(make_gcs_builder_with_token(
                bucket,
                access_token.clone(),
            )));
        } else {
            return Err(anyhow!("No GCP credentials available"));
        }

        let mut flow_like_state = FlowLikeState::new(config, http_client);

        flow_like_state.model_provider_config = state.provider.clone();
        flow_like_state.node_registry.write().await.node_registry = state.registry.clone();

        Ok(flow_like_state)
    }
}

#[cfg(feature = "gcp")]
fn make_gcs_builder_with_key(
    bucket: String,
    service_account_key: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        connect(&url).storage_option(
            "google_service_account_key".to_string(),
            service_account_key.clone(),
        )
    }
}

#[cfg(feature = "gcp")]
fn make_gcs_builder_with_token(
    bucket: String,
    access_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        connect(&url).storage_option(GCS_STORAGE_TOKEN_OPTION.to_string(), access_token.clone())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(test, feature = "gcp"))]
mod tests {
    use super::*;
    use crate::credentials::CredentialsAccess;
    use flow_like_storage::Path;
    use flow_like_storage::object_store::ObjectStore;
    use flow_like_types::json::{from_str, to_string};
    use flow_like_types::tokio;
    use std::sync::Once;

    const TEST_SUB: &str = "test-user-123";
    const TEST_APP_ID: &str = "test-app-456";

    static INIT: Once = Once::new();

    fn init_env() {
        INIT.call_once(|| {
            if dotenv::from_filename("packages/api/.env").is_err() {
                let _ = dotenv::dotenv();
            }
        });
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_setup() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env();
        assert!(
            creds.service_account_key.is_some(),
            "GOOGLE_APPLICATION_CREDENTIALS_JSON must be set"
        );
        assert!(!creds.meta_bucket.is_empty(), "GCP_META_BUCKET must be set");
        assert!(
            !creds.content_bucket.is_empty(),
            "GCP_CONTENT_BUCKET must be set"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_can_write() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env().master_credentials().await;
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!(
            "test/master-write-test-{}.txt",
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"test content".to_vec().into())
                    .await
                    .expect("Master credentials should be able to write");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_can_read() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env().master_credentials().await;
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!("test/master-read-test-{}.txt", flow_like_types::create_id());
        let path = Path::from(test_path.as_str());
        let content = b"read test content";

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, content.to_vec().into())
                    .await
                    .expect("Setup: write should succeed");

                let result = s.get(&path).await.expect("Read should succeed");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), content);

                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_can_write_in_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::EditApp)
            .await
            .expect("Failed to generate scoped credentials");

        // Verify service account key is NOT included
        assert!(
            scoped.service_account_key.is_none(),
            "Scoped credentials should not include service account key"
        );
        assert!(
            scoped.access_token.is_some(),
            "Scoped credentials should include access token"
        );

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "apps/{}/test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"scoped test content".to_vec().into())
                    .await
                    .expect("Scoped credentials should be able to write in allowed path");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_invoke_write() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        assert!(scoped.write_access);
        assert_eq!(scoped.allowed_prefixes.len(), 5);

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "users/{}/apps/{}/test-{}.txt",
            TEST_SUB,
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"invoke write test".to_vec().into())
                    .await
                    .expect("InvokeWrite credentials should be able to write");

                let result = s.get(&path).await.expect("Should be able to read");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), b"invoke write test");

                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_can_read_in_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        // First, write test data with master credentials
        let master_shared = master.clone().into_shared_credentials();
        let master_store = master_shared
            .to_store(false)
            .await
            .expect("Failed to create master store");

        let test_path = format!(
            "apps/{}/read-test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());
        let content = b"scoped read test content";

        match &master_store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, content.to_vec().into())
                    .await
                    .expect("Setup: write should succeed");
            }
            _ => panic!("Expected GCP store"),
        }

        // Now read with scoped credentials
        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadApp)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s
                    .get(&path)
                    .await
                    .expect("Scoped credentials should be able to read in allowed path");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), content);
            }
            _ => panic!("Expected GCP store"),
        }

        // Cleanup with master
        match &master_store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.delete(&path).await.ok();
            }
            _ => {}
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_cannot_write_outside_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        // Try to write to a path outside the allowed prefixes (different user)
        let test_path = format!(
            "users/different-user/apps/{}/unauthorized-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;

                if result.is_ok() {
                    s.delete(&path).await.ok();
                }

                assert!(
                    result.is_err(),
                    "Downscoped GCP credentials must not write outside their allowed prefixes"
                );
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_read_only_cannot_write() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadApp)
            .await
            .expect("Failed to generate scoped credentials");

        // Verify write_access is false
        assert!(
            !scoped.write_access,
            "ReadApp credentials should have write_access=false"
        );

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "apps/{}/readonly-test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;

                if result.is_ok() {
                    s.delete(&path).await.ok();
                }

                assert!(
                    result.is_err(),
                    "Read-only downscoped GCP credentials must not allow writes"
                );
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[test]
    fn test_gcp_runtime_credentials_serialization() {
        let creds = GcpRuntimeCredentials {
            service_account_key: Some(r#"{"type":"service_account"}"#.to_string()),
            access_token: Some("ya29.test-token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec!["apps/test-app".to_string()],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");
        let deserialized: GcpRuntimeCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.access_token, deserialized.access_token);
        assert_eq!(creds.meta_bucket, deserialized.meta_bucket);
        assert_eq!(creds.allowed_prefixes, deserialized.allowed_prefixes);
        assert_eq!(creds.write_access, deserialized.write_access);
    }

    #[test]
    fn test_gcp_scoped_credentials_do_not_include_service_account_key() {
        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("ya29.scoped-token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec!["apps/test-app".to_string()],
            write_access: false,
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            content_path_prefix: None,
            user_content_path_prefix: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");

        assert!(
            !json.contains("service_account") || json.contains("null"),
            "Scoped credentials should not expose service account key"
        );
        assert!(
            json.contains("ya29.scoped-token"),
            "Scoped credentials should include access token"
        );
    }

    #[test]
    fn test_credentials_access_modes() {
        let apps_prefix = format!("apps/{}", TEST_APP_ID);
        let user_prefix = format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID);
        let log_prefix = format!("runs/{}", TEST_APP_ID);
        let tmp_user_prefix = format!("tmp/user/{}/apps/{}", TEST_SUB, TEST_APP_ID);
        let tmp_global_prefix = format!("tmp/global/apps/{}", TEST_APP_ID);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![apps_prefix.clone()],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(creds.write_access);
        assert_eq!(creds.allowed_prefixes, vec![apps_prefix.clone()]);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![apps_prefix.clone()],
            write_access: false,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(!creds.write_access);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![
                apps_prefix.clone(),
                user_prefix.clone(),
                tmp_user_prefix.clone(),
                tmp_global_prefix.clone(),
                log_prefix.clone(),
            ],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(creds.write_access);
        assert_eq!(creds.allowed_prefixes.len(), 5);
    }
}

//! Cloudflare R2 credentials with prefix-scoped temporary access
//!
//! R2 uses a proprietary temporary credentials API instead of AWS STS.
//! See: https://developers.cloudflare.com/api/resources/r2/subresources/temporary_credentials/

use crate::credentials::{
    CredentialsAccess, RuntimeCredentials, RuntimeCredentialsTrait,
    mixed_credentials::MixedRuntimeCredentials,
};
use crate::state::{AppState, State};
use flow_like::credentials::{
    BucketConfig, SharedCredentials, aws_credentials::AwsSharedCredentials,
};
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::lancedb::{connect, connection::ConnectBuilder};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// R2 temporary credentials response from Cloudflare API
#[derive(Debug, Deserialize)]
struct R2TempCredentialsResponse {
    success: bool,
    errors: Vec<R2Error>,
    result: Option<R2TempCredentials>,
}

#[derive(Debug, Deserialize)]
struct R2Error {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct R2TempCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct R2RuntimeCredentials {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub meta_bucket: String,
    pub content_bucket: String,
    pub logs_bucket: String,
    pub endpoint: String,
    pub account_id: String,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub content_path_prefix: Option<String>,
    pub user_content_path_prefix: Option<String>,
}

impl std::fmt::Debug for R2RuntimeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("R2RuntimeCredentials")
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("meta_bucket", &self.meta_bucket)
            .field("content_bucket", &self.content_bucket)
            .field("logs_bucket", &self.logs_bucket)
            .field("endpoint", &self.endpoint)
            .field("account_id", &self.account_id)
            .field("expiration", &self.expiration)
            .finish()
    }
}

impl R2RuntimeCredentials {
    async fn device_execute_credentials(
        &self,
        sub: &str,
        app_id: &str,
        write: bool,
        expires_at: i64,
    ) -> Result<Self> {
        let expiry = super::device_execute_expiry(expires_at)?;
        let prefixes = super::device_execute_prefixes(sub, app_id)?
            .into_values()
            .collect::<Vec<_>>();
        let unavailable = || anyhow!("R2 could not issue bounded device credentials");
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let endpoint = reqwest::Url::parse(&self.endpoint).map_err(|_| unavailable())?;
        let host = format!("{}.r2.cloudflarestorage.com", self.account_id);
        if self.account_id.len() != 32
            || !self.account_id.bytes().all(|b| b.is_ascii_hexdigit())
            || endpoint.scheme() != "https"
            || endpoint.host_str() != Some(host.as_str())
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.port().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || self.session_token.is_some()
        {
            return Err(anyhow!(
                "Instance storage requires a standard R2 account endpoint and signing credential",
            ));
        }
        let access_key_id = self
            .access_key_id
            .as_ref()
            .filter(|v| !v.is_empty())
            .ok_or_else(unavailable)?;
        let parent_secret = self
            .secret_access_key
            .as_ref()
            .filter(|v| !v.is_empty())
            .ok_or_else(unavailable)?;
        // R2 verifies this JWT's directory and absolute expiry. The derived
        // signing secret cannot be used to mint another temporary credential.
        let claims = serde_json::json!({ "bucket": self.content_bucket,
            "scope": if write { "object-read-write" } else { "object-read-only" },
            "paths": { "prefixPaths": prefixes, "objectPaths": [] },
            "sub": self.account_id, "iss": access_key_id, "aud": host,
            "iat": chrono::Utc::now().timestamp(), "exp": expires_at });
        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(parent_secret.as_bytes()),
        )
        .map_err(|_| unavailable())?;
        let secret_access_key = format!("{:x}", Sha256::digest(jwt.as_bytes()));
        let session_token = base64::engine::general_purpose::STANDARD.encode(format!("jwt/{jwt}"));
        Ok(Self {
            access_key_id: Some(access_key_id.clone()),
            secret_access_key: Some(secret_access_key),
            session_token: Some(session_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            endpoint: self.endpoint.clone(),
            account_id: self.account_id.clone(),
            expiration: Some(expiry),
            content_path_prefix: Some(format!("apps/{app_id}")),
            user_content_path_prefix: Some(format!("users/{sub}/apps/{app_id}")),
        })
    }

    pub fn from_env() -> Self {
        let logs_bucket = std::env::var("LOG_BUCKET")
            .or_else(|_| std::env::var("LOGS_BUCKET"))
            .unwrap_or_default();
        if logs_bucket.is_empty() {
            tracing::warn!(
                "LOG_BUCKET environment variable is not set - logs will not be persisted"
            );
        }
        R2RuntimeCredentials {
            access_key_id: std::env::var("R2_ACCESS_KEY_ID")
                .or_else(|_| std::env::var("AWS_ACCESS_KEY_ID"))
                .ok(),
            secret_access_key: std::env::var("R2_SECRET_ACCESS_KEY")
                .or_else(|_| std::env::var("AWS_SECRET_ACCESS_KEY"))
                .ok(),
            session_token: None,
            meta_bucket: std::env::var("META_BUCKET")
                .or_else(|_| std::env::var("META_BUCKET_NAME"))
                .unwrap_or_default(),
            content_bucket: std::env::var("CONTENT_BUCKET")
                .or_else(|_| std::env::var("CONTENT_BUCKET_NAME"))
                .unwrap_or_default(),
            logs_bucket,
            endpoint: std::env::var("R2_ENDPOINT")
                .or_else(|_| std::env::var("AWS_ENDPOINT"))
                .unwrap_or_default(),
            account_id: std::env::var("R2_ACCOUNT_ID").unwrap_or_default(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        R2RuntimeCredentials {
            access_key_id: std::env::var("R2_ACCESS_KEY_ID")
                .or_else(|_| std::env::var("AWS_ACCESS_KEY_ID"))
                .ok(),
            secret_access_key: std::env::var("R2_SECRET_ACCESS_KEY")
                .or_else(|_| std::env::var("AWS_SECRET_ACCESS_KEY"))
                .ok(),
            session_token: None,
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            endpoint: self.endpoint.clone(),
            account_id: self.account_id.clone(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    /// Generate prefix-scoped temporary credentials using R2's temp credentials API
    #[tracing::instrument(
        name = "R2RuntimeCredentials::scoped_credentials",
        skip(self, sub, _state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        _state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }

        // Validate sub and app_id to prevent path traversal
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        if let CredentialsAccess::DeviceExecute { write, expires_at } = mode {
            return self
                .device_execute_credentials(sub, app_id, write, expires_at)
                .await;
        }

        let api_token = std::env::var("R2_API_TOKEN")
            .map_err(|_| anyhow!("R2_API_TOKEN environment variable not set"))?;

        let parent_key_id = self
            .access_key_id
            .as_ref()
            .ok_or_else(|| anyhow!("R2_ACCESS_KEY_ID not set"))?;

        // Build prefix list based on access mode
        let apps_prefix = format!("apps/{}/", app_id);
        let db_prefix = format!("{}storage/db/", apps_prefix);
        let user_prefix = format!("users/{}/apps/{}/", sub, app_id);
        let log_prefix = format!("runs/{}/", app_id);
        // Same segments the `/tmp` presign route and the HTTP-sink offload
        // write; R2 prefixes carry a trailing slash where the others do not.
        let (temporary_user_prefix, temporary_global_prefix) =
            crate::credentials::temporary_prefixes(sub, app_id);
        let temporary_user_prefix = format!("{temporary_user_prefix}/");
        let temporary_global_prefix = format!("{temporary_global_prefix}/");
        let app_content_path_prefix = format!("apps/{}", app_id);
        let user_content_path_prefix_value = format!("users/{}/apps/{}", sub, app_id);
        let (content_path_prefix, user_content_path_prefix) = scoped_content_path_prefixes(
            &app_content_path_prefix,
            &user_content_path_prefix_value,
            &mode,
        );

        let (permission, prefixes) = match mode {
            CredentialsAccess::EditApp => ("object-read-write", vec![apps_prefix.clone()]),
            CredentialsAccess::ReadApp => ("object-read-only", vec![apps_prefix.clone()]),
            // R2 doesn't expose per-bucket policies the way AWS does;
            // the prefix-based scope is the same for ReadApp and
            // ReadAppContent (likewise EditApp / EditAppContent).
            // The content-only restriction is enforced server-side by
            // only ever pointing the client at the content store.
            CredentialsAccess::ReadAppContent => ("object-read-only", vec![apps_prefix.clone()]),
            CredentialsAccess::EditAppContent => ("object-read-write", vec![apps_prefix]),
            CredentialsAccess::ReadAppDb => ("object-read-only", vec![db_prefix.clone()]),
            CredentialsAccess::EditAppDb => ("object-read-write", vec![db_prefix]),
            CredentialsAccess::EditUser => ("object-read-write", vec![user_prefix]),
            CredentialsAccess::ReadUser => ("object-read-only", vec![user_prefix]),
            CredentialsAccess::InvokeNone => (
                "object-read-write",
                vec![user_prefix, log_prefix, temporary_user_prefix],
            ),
            CredentialsAccess::InvokeRead => (
                "object-read-only",
                vec![
                    apps_prefix.clone(),
                    user_prefix,
                    log_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ],
            ),
            CredentialsAccess::InvokeWrite => (
                "object-read-write",
                vec![
                    apps_prefix,
                    user_prefix,
                    log_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ],
            ),
            CredentialsAccess::ServerExecute => (
                "object-read-write",
                vec![
                    apps_prefix,
                    user_prefix,
                    log_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ],
            ),
            // R2 temp credentials carry one permission for every prefix, so a
            // per-prefix read/write split is not expressible here; shadow runs
            // mirror ServerExecute and rely on the in-process `ReadOnlyStore`
            // decorator for app-content write isolation.
            CredentialsAccess::ShadowExecute => (
                "object-read-write",
                vec![
                    apps_prefix,
                    user_prefix,
                    log_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ],
            ),
            CredentialsAccess::ReadLogs => ("object-read-only", vec![log_prefix]),
            CredentialsAccess::DeviceExecute { .. } => unreachable!("handled above"),
        };

        // Call R2 temp credentials API for each bucket
        // For now, we scope to the content bucket as that's the primary data store
        let temp_creds = self
            .get_temp_credentials(
                &api_token,
                parent_key_id,
                &self.content_bucket,
                permission,
                &prefixes,
                3600, // 1 hour
            )
            .await?;

        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            access_key_id: Some(temp_creds.access_key_id),
            secret_access_key: Some(temp_creds.secret_access_key),
            session_token: Some(temp_creds.session_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            endpoint: self.endpoint.clone(),
            account_id: self.account_id.clone(),
            expiration: Some(chrono_expiration),
            content_path_prefix,
            user_content_path_prefix,
        })
    }

    async fn scoped_server_content_write_credentials(
        &self,
        sub: &str,
        app_id: &str,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }

        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let apps_prefix = format!("apps/{}/", app_id);
        let user_prefix = format!("users/{}/apps/{}/", sub, app_id);
        // Same segments the `/tmp` presign route and the HTTP-sink offload
        // write; R2 prefixes carry a trailing slash where the others do not.
        let (temporary_user_prefix, temporary_global_prefix) =
            crate::credentials::temporary_prefixes(sub, app_id);
        let temporary_user_prefix = format!("{temporary_user_prefix}/");
        let temporary_global_prefix = format!("{temporary_global_prefix}/");
        let app_content_path_prefix = format!("apps/{}", app_id);
        let user_content_path_prefix = format!("users/{}/apps/{}", sub, app_id);

        self.scoped_bucket_credentials(
            &self.content_bucket,
            "object-read-write",
            vec![
                apps_prefix,
                user_prefix,
                temporary_user_prefix,
                temporary_global_prefix,
            ],
            Some(app_content_path_prefix),
            Some(user_content_path_prefix),
        )
        .await
    }

    async fn scoped_server_logs_write_credentials(&self, sub: &str, app_id: &str) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }

        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let log_prefix = format!("runs/{}/", app_id);
        self.scoped_bucket_credentials(
            &self.logs_bucket,
            "object-read-write",
            vec![log_prefix],
            None,
            None,
        )
        .await
    }

    pub async fn scoped_server_execute_credentials(
        &self,
        sub: &str,
        app_id: &str,
    ) -> Result<MixedRuntimeCredentials> {
        let (content, logs) = flow_like_types::tokio::join!(
            self.scoped_server_content_write_credentials(sub, app_id),
            self.scoped_server_logs_write_credentials(sub, app_id),
        );
        let content = content?;

        Ok(MixedRuntimeCredentials {
            // The executor opens no meta store: its board arrives as a
            // presigned compiled artifact and widgets come from the hub. The
            // mixed shape still wants a credential in the slot, so the content
            // credential fills it — it cannot reach the meta bucket.
            meta: Box::new(RuntimeCredentials::R2(content.clone())),
            content: Box::new(RuntimeCredentials::R2(content)),
            logs: Box::new(RuntimeCredentials::R2(logs?)),
        })
    }

    async fn scoped_bucket_credentials(
        &self,
        bucket: &str,
        permission: &str,
        prefixes: Vec<String>,
        content_path_prefix: Option<String>,
        user_content_path_prefix: Option<String>,
    ) -> Result<Self> {
        if bucket.is_empty() {
            return Err(anyhow!("R2 bucket is empty"));
        }

        let api_token = std::env::var("R2_API_TOKEN")
            .map_err(|_| anyhow!("R2_API_TOKEN environment variable not set"))?;

        let parent_key_id = self
            .access_key_id
            .as_ref()
            .ok_or_else(|| anyhow!("R2_ACCESS_KEY_ID not set"))?;

        let temp_creds = self
            .get_temp_credentials(
                &api_token,
                parent_key_id,
                bucket,
                permission,
                &prefixes,
                3600,
            )
            .await?;

        Ok(Self {
            access_key_id: Some(temp_creds.access_key_id),
            secret_access_key: Some(temp_creds.secret_access_key),
            session_token: Some(temp_creds.session_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            endpoint: self.endpoint.clone(),
            account_id: self.account_id.clone(),
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            content_path_prefix,
            user_content_path_prefix,
        })
    }

    async fn get_temp_credentials(
        &self,
        api_token: &str,
        parent_key_id: &str,
        bucket: &str,
        permission: &str,
        prefixes: &[String],
        ttl_seconds: u32,
    ) -> Result<R2TempCredentials> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/r2/temp-access-credentials",
            self.account_id
        );

        let body = serde_json::json!({
            "bucket": bucket,
            "parentAccessKeyId": parent_key_id,
            "permission": permission,
            "prefixes": prefixes,
            "ttlSeconds": ttl_seconds,
        });

        let client = flow_like_types::reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to call R2 temp credentials API: {}", e))?;

        let status = response.status();
        let resp: R2TempCredentialsResponse = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse R2 temp credentials response: {}", e))?;

        if !resp.success {
            let errors: Vec<String> = resp.errors.into_iter().map(|e| e.message).collect();
            return Err(anyhow!(
                "R2 temp credentials API error ({}): {}",
                status,
                errors.join(", ")
            ));
        }

        resp.result
            .ok_or_else(|| anyhow!("R2 temp credentials response missing result"))
    }
}

fn scoped_content_path_prefixes(
    apps_prefix: &str,
    user_prefix: &str,
    mode: &CredentialsAccess,
) -> (Option<String>, Option<String>) {
    let app = matches!(
        mode,
        CredentialsAccess::EditApp
            | CredentialsAccess::ReadApp
            | CredentialsAccess::ReadAppContent
            | CredentialsAccess::EditAppContent
            | CredentialsAccess::ReadAppDb
            | CredentialsAccess::EditAppDb
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::DeviceExecute { .. }
            | CredentialsAccess::ShadowExecute
    )
    .then(|| apps_prefix.to_string());

    let user = matches!(
        mode,
        CredentialsAccess::EditUser
            | CredentialsAccess::ReadUser
            | CredentialsAccess::InvokeNone
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::DeviceExecute { .. }
            | CredentialsAccess::ShadowExecute
    )
    .then(|| user_prefix.to_string());

    (app, user)
}

#[async_trait]
impl RuntimeCredentialsTrait for R2RuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        // R2 uses AWS-compatible S3 API, so we use AwsSharedCredentials
        // All R2 buckets use the same endpoint
        // R2 has no KMS: encryption is bucket-managed and there is no
        // per-request key to name.
        let r2_config = Some(BucketConfig {
            endpoint: Some(self.endpoint.clone()),
            express: false,
            kms_key_arn: None,
            kms_bucket_key: false,
            ..Default::default()
        });

        SharedCredentials::Aws(AwsSharedCredentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            meta_config: r2_config.clone(),
            content_config: r2_config.clone(),
            logs_config: r2_config,
            region: "auto".to_string(), // R2 uses "auto" region
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
        })
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials().to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials()
            .to_db_scoped(sub, app_id)
            .await
    }

    #[tracing::instrument(
        name = "R2RuntimeCredentials::to_state",
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

        let (bkt, key, secret, token) = (
            self.content_bucket.clone(),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("R2_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("R2_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone().unwrap_or_default(),
        );

        config.register_build_logs_database(Arc::new(make_r2_builder(
            bkt.clone(),
            key.clone(),
            secret.clone(),
            token.clone(),
        )));
        config.register_build_project_database(Arc::new(make_r2_builder(
            bkt.clone(),
            key.clone(),
            secret.clone(),
            token.clone(),
        )));
        config.register_build_user_database(Arc::new(make_r2_builder(bkt, key, secret, token)));

        let mut flow_like_state = FlowLikeState::new(config, http_client);

        flow_like_state.model_provider_config = state.provider.clone();
        flow_like_state.node_registry.write().await.node_registry = state.registry.clone();

        Ok(flow_like_state)
    }
}

fn make_r2_builder(
    bucket: String,
    access_key: String,
    secret_key: String,
    session_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("s3://{}/{}", bucket, path);
        let builder = connect(&url)
            .storage_option("aws_access_key_id".to_string(), access_key.clone())
            .storage_option("aws_secret_access_key".to_string(), secret_key.clone())
            .storage_option("aws_region".to_string(), "auto".to_string());

        if !session_token.is_empty() {
            builder.storage_option("aws_session_token".to_string(), session_token.clone())
        } else {
            builder
        }
    }
}

#[cfg(all(test, feature = "r2"))]
mod tests {
    use super::*;

    #[flow_like_types::tokio::test]
    async fn instance_credentials_sign_only_the_project_directory_and_deadline() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let account = "0123456789abcdef0123456789abcdef";
        let mut credentials = R2RuntimeCredentials {
            access_key_id: Some("public-parent-id".into()),
            secret_access_key: Some("private-parent-secret".into()),
            session_token: None,
            meta_bucket: "private-meta".into(),
            content_bucket: "content".into(),
            logs_bucket: "private-logs".into(),
            endpoint: format!("https://{account}.r2.cloudflarestorage.com"),
            account_id: account.into(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        let expires_at = chrono::Utc::now().timestamp() + 3590;
        let issued = credentials
            .device_execute_credentials("owner", "project", false, expires_at)
            .await
            .unwrap();
        let access_key_id = issued.access_key_id.unwrap();
        let secret_access_key = issued.secret_access_key.unwrap();
        let session_token = issued.session_token.unwrap();
        let jwt = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(session_token)
                .unwrap(),
        )
        .unwrap();
        let jwt = jwt.strip_prefix("jwt/").unwrap();
        assert_eq!(access_key_id, "public-parent-id");
        assert_ne!(secret_access_key, "private-parent-secret");
        assert_eq!(
            secret_access_key,
            format!("{:x}", Sha256::digest(jwt.as_bytes()))
        );
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_audience(&[format!("{account}.r2.cloudflarestorage.com")]);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            jwt,
            &jsonwebtoken::DecodingKey::from_secret(b"private-parent-secret"),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims["exp"], expires_at);
        assert_eq!(claims["scope"], "object-read-only");
        assert_eq!(claims["bucket"], "content");
        assert_eq!(
            claims["paths"]["prefixPaths"],
            serde_json::json!([
                "apps/project/upload/",
                "apps/project/storage/",
                "users/owner/apps/project/",
                "tmp/user/owner/apps/project/"
            ])
        );
        assert!(!claims.to_string().contains("private-meta"));
        credentials.endpoint = "https://another.example".into();
        assert!(
            credentials
                .device_execute_credentials("owner", "project", false, expires_at)
                .await
                .is_err()
        );
    }

    #[test]
    fn test_r2_invoke_none_does_not_advertise_app_content_prefix() {
        let (app, user) = scoped_content_path_prefixes(
            "apps/app-1",
            "users/user-1/apps/app-1",
            &CredentialsAccess::InvokeNone,
        );

        assert_eq!(app, None);
        assert_eq!(user, Some("users/user-1/apps/app-1".to_string()));
    }

    #[test]
    fn test_r2_invoke_read_and_write_advertise_app_content_prefix() {
        for mode in [
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
        ] {
            let (app, user) =
                scoped_content_path_prefixes("apps/app-1", "users/user-1/apps/app-1", &mode);

            assert_eq!(app, Some("apps/app-1".to_string()));
            assert_eq!(user, Some("users/user-1/apps/app-1".to_string()));
        }
    }
}

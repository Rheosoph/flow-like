use crate::credentials::{LogsDbBuilder, SharedCredentialsTrait, StoreType, db_path_from_base};
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_storage::object_store::StaticCredentialProvider;
use flow_like_storage::object_store::gcp::{GcpCredential, GoogleCloudStorageBuilder};
use flow_like_storage::object_store;
use flow_like_storage::{files::store::FlowLikeStore, lancedb};
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const GCS_STORAGE_TOKEN_OPTION: &str = "google_storage_token";

/// GCP Shared Credentials that can use either service account key or access token
///
/// SECURITY: Scoped credentials should only contain an access_token, never service_account_key.
/// The access_token is short-lived (1 hour) and server-generated, preventing client tampering.
#[derive(Clone, Serialize, Deserialize)]
pub struct GcpSharedCredentials {
    /// Full service account key (only for master credentials)
    /// SECURITY: Never serialize to prevent leaking master credentials to clients
    #[serde(default, skip_serializing)]
    pub service_account_key: String,
    /// Short-lived OAuth2 access token (for scoped credentials)
    #[serde(default)]
    pub access_token: Option<String>,
    pub meta_bucket: String,
    pub content_bucket: String,
    pub logs_bucket: String,
    /// Allowed path prefixes for this credential (informational, enforcement is server-side)
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
    /// Whether write operations are allowed
    #[serde(default = "default_write_access")]
    pub write_access: bool,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    /// App-level content path prefix (e.g., "apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_path_prefix: Option<String>,
    /// User-level content path prefix (e.g., "users/{sub}/apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content_path_prefix: Option<String>,
}

impl std::fmt::Debug for GcpSharedCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpSharedCredentials")
            .field("service_account_key", &if self.service_account_key.is_empty() { "empty" } else { "[REDACTED]" })
            .field("access_token", &self.access_token.as_ref().map(|_| "[REDACTED]"))
            .field("meta_bucket", &self.meta_bucket)
            .field("content_bucket", &self.content_bucket)
            .field("logs_bucket", &self.logs_bucket)
            .field("write_access", &self.write_access)
            .field("expiration", &self.expiration)
            .finish()
    }
}

fn default_write_access() -> bool {
    true
}

#[async_trait]
impl SharedCredentialsTrait for GcpSharedCredentials {
    #[tracing::instrument(name = "GcpSharedCredentials::to_store", skip(self, meta), fields(meta = meta), level="debug")]
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        self.to_store_type(if meta {
            StoreType::Meta
        } else {
            StoreType::Content
        })
        .await
    }

    #[tracing::instrument(name = "GcpSharedCredentials::to_store_type", skip(self), fields(store_type = ?store_type), level="debug")]
    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        use flow_like_types::tokio;

        let bucket = match store_type {
            StoreType::Meta => self.meta_bucket.clone(),
            StoreType::Content => self.content_bucket.clone(),
            StoreType::Logs => self.logs_bucket.clone(),
        };

        // Prefer access token for scoped credentials, fall back to service account key
        if let Some(ref access_token) = self.access_token
            && !access_token.is_empty()
        {
            let token = access_token.clone();
            let bucket = bucket.clone();
            let store = tokio::task::spawn_blocking(move || {
                let credential = GcpCredential { bearer: token };
                let provider = StaticCredentialProvider::new(credential);
                GoogleCloudStorageBuilder::new()
                    .with_bucket_name(bucket)
                    .with_credentials(Arc::new(provider))
                    .build()
            })
            .await
            .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??;

            return Ok(FlowLikeStore::Google(Arc::new(store)));
        }

        // Fall back to service account key (master credentials)
        if !self.service_account_key.is_empty() {
            let service_account_key = self.service_account_key.clone();
            let store = tokio::task::spawn_blocking(move || {
                GoogleCloudStorageBuilder::new()
                    .with_bucket_name(bucket)
                    .with_service_account_key(&service_account_key)
                    .build()
            })
            .await
            .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??;

            return Ok(FlowLikeStore::Google(Arc::new(store)));
        }

        Err(anyhow!(
            "No GCP credentials available (neither access token nor service account key)"
        ))
    }

    #[tracing::instrument(name = "GcpSharedCredentials::to_db", skip(self), level = "debug")]
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = self
            .content_path_prefix
            .clone()
            .or_else(|| {
                self.allowed_prefixes
                    .iter()
                    .find(|prefix| prefix.starts_with("apps/"))
                    .cloned()
            })
            .unwrap_or_else(|| format!("apps/{}", app_id));
        let path = db_path_from_base(&base_path);

        // Prefer access token for scoped credentials
        if let Some(ref access_token) = self.access_token
            && !access_token.is_empty()
        {
            let connection =
                make_gcs_builder_with_token(self.content_bucket.clone(), access_token.clone());
            return Ok(connection(path.clone()));
        }

        // Fall back to service account key
        if !self.service_account_key.is_empty() {
            let connection = make_gcs_builder_with_key(
                self.content_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(connection(path.clone()));
        }

        Err(anyhow!("No GCP credentials available"))
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = format!("users/{}/apps/{}", sub, app_id);
        let path = db_path_from_base(&base_path);

        if let Some(ref access_token) = self.access_token
            && !access_token.is_empty()
        {
            let connection =
                make_gcs_builder_with_token(self.content_bucket.clone(), access_token.clone());
            return Ok(connection(path.clone()));
        }

        if !self.service_account_key.is_empty() {
            let connection = make_gcs_builder_with_key(
                self.content_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(connection(path.clone()));
        }

        Err(anyhow!("No GCP credentials available"))
    }

    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        if self.logs_bucket.is_empty() {
            return Err(anyhow!(
                "logs_bucket is empty - cannot create logs database builder"
            ));
        }

        // Prefer access token for scoped credentials
        if let Some(ref access_token) = self.access_token
            && !access_token.is_empty()
        {
            let builder =
                make_gcs_builder_with_token(self.logs_bucket.clone(), access_token.clone());
            return Ok(Arc::new(builder));
        }

        // Fall back to service account key
        if !self.service_account_key.is_empty() {
            let builder = make_gcs_builder_with_key(
                self.logs_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(Arc::new(builder));
        }

        Err(anyhow!("No GCP credentials available"))
    }
}

fn make_gcs_builder_with_key(
    bucket: String,
    service_account_key: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        lancedb::connect(&url).storage_option(
            "google_service_account_key".to_string(),
            service_account_key.clone(),
        )
    }
}

fn make_gcs_builder_with_token(
    bucket: String,
    access_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        lancedb::connect(&url)
            .storage_option(GCS_STORAGE_TOKEN_OPTION.to_string(), access_token.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    fn sample_service_account_key() -> String {
        r#"{"type":"service_account","project_id":"my-project","private_key_id":"abc123","private_key":"-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\n","client_email":"test@my-project.iam.gserviceaccount.com","client_id":"123456789","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token"}"#.to_string()
    }

    fn sample_credentials() -> GcpSharedCredentials {
        GcpSharedCredentials {
            service_account_key: sample_service_account_key(),
            access_token: None,
            meta_bucket: "my-meta-bucket".to_string(),
            content_bucket: "my-content-bucket".to_string(),
            logs_bucket: "my-logs-bucket".to_string(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    fn sample_scoped_credentials() -> GcpSharedCredentials {
        GcpSharedCredentials {
            service_account_key: String::new(),
            access_token: Some("ya29.test-access-token".to_string()),
            meta_bucket: "my-meta-bucket".to_string(),
            content_bucket: "my-content-bucket".to_string(),
            logs_bucket: "my-logs-bucket".to_string(),
            allowed_prefixes: vec!["apps/test-app".to_string()],
            write_access: false,
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            content_path_prefix: Some("apps/test-app".to_string()),
            user_content_path_prefix: None,
        }
    }

    #[test]
    fn test_gcp_credentials_serialization() {
        let creds = sample_credentials();
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("my-meta-bucket"));
        assert!(json.contains("my-content-bucket"));
        // service_account_key is skip_serializing for security — must NOT appear
        assert!(!json.contains("service_account"));
    }

    #[test]
    fn test_gcp_scoped_credentials_serialization() {
        let creds = sample_scoped_credentials();
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("ya29.test-access-token"));
        assert!(json.contains("apps/test-app"));
        assert!(json.contains("\"write_access\":false"));
    }

    #[test]
    fn test_gcp_credentials_deserialization() {
        let sa_key = sample_service_account_key().replace('\"', "\\\"");
        let json = format!(
            r#"{{
            "service_account_key": "{}",
            "meta_bucket": "test-meta",
            "content_bucket": "test-content",
            "logs_bucket": "test-logs",
            "expiration": null
        }}"#,
            sa_key
        );

        let creds: GcpSharedCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.meta_bucket, "test-meta");
        assert_eq!(creds.content_bucket, "test-content");
        assert!(creds.service_account_key.contains("service_account"));
        assert!(creds.expiration.is_none());
        assert!(creds.write_access); // default is true
    }

    #[test]
    fn test_gcp_credentials_roundtrip() {
        let original = sample_credentials();
        let json = to_string(&original).expect("Failed to serialize");
        let deserialized: GcpSharedCredentials = from_str(&json).expect("Failed to deserialize");

        // service_account_key is skip_serializing for security — not preserved in roundtrip
        assert!(deserialized.service_account_key.is_empty());
        assert_eq!(original.meta_bucket, deserialized.meta_bucket);
        assert_eq!(original.content_bucket, deserialized.content_bucket);
    }

    #[test]
    fn test_gcp_credentials_with_expiration() {
        let sa_key = sample_service_account_key().replace('\"', "\\\"");
        let json = format!(
            r#"{{
            "service_account_key": "{}",
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs",
            "expiration": "2025-01-15T12:00:00Z"
        }}"#,
            sa_key
        );

        let creds: GcpSharedCredentials = from_str(&json).expect("Failed to deserialize");
        assert!(creds.expiration.is_some());
    }

    #[test]
    fn test_gcp_service_account_key_contains_required_fields() {
        let creds = sample_credentials();
        let key = &creds.service_account_key;

        assert!(key.contains("type"));
        assert!(key.contains("project_id"));
        assert!(key.contains("private_key"));
        assert!(key.contains("client_email"));
    }

    #[test]
    fn test_gcp_scoped_credentials_with_access_token() {
        let json = r#"{
            "service_account_key": "",
            "access_token": "ya29.scoped-token",
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs",
            "allowed_prefixes": ["apps/my-app", "users/user1/apps/my-app"],
            "write_access": true,
            "expiration": "2025-12-16T12:00:00Z"
        }"#;

        let creds: GcpSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert_eq!(creds.access_token, Some("ya29.scoped-token".to_string()));
        assert_eq!(creds.allowed_prefixes.len(), 2);
        assert!(creds.write_access);
    }

    #[test]
    fn test_gcp_credentials_defaults() {
        let json = r#"{
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs"
        }"#;

        let creds: GcpSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert!(creds.service_account_key.is_empty());
        assert!(creds.access_token.is_none());
        assert!(creds.allowed_prefixes.is_empty());
        assert!(creds.write_access);
        assert!(creds.expiration.is_none());
    }
}

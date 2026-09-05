#[cfg(feature = "flow-runtime")]
use crate::credentials::{LogsDbBuilder, db_path_from_base};
use crate::credentials::{SharedCredentialsTrait, StoreType};
use flow_like_storage::files::store::FlowLikeStore;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb::connection::ConnectBuilder;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::object_store;
use flow_like_storage::object_store::StaticCredentialProvider;
use flow_like_storage::object_store::gcp::{GcpCredential, GoogleCloudStorageBuilder};
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(feature = "flow-runtime")]
const GCS_STORAGE_TOKEN_OPTION: &str = "google_storage_token";
#[cfg(feature = "flow-runtime")]
const GCS_SERVICE_ACCOUNT_KEY_OPTION: &str = "google_service_account_key";

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
            .field(
                "service_account_key",
                &if self.service_account_key.is_empty() {
                    "empty"
                } else {
                    "[REDACTED]"
                },
            )
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[REDACTED]"),
            )
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

impl GcpSharedCredentials {
    /// Whether this credential set came from `scoped_credentials` rather than
    /// from server configuration. Every scoped credential carries at least one
    /// allowed prefix and an expiry; master credentials carry neither.
    fn is_scoped(&self) -> bool {
        !self.allowed_prefixes.is_empty()
            || self.expiration.is_some()
            || self.content_path_prefix.is_some()
            || self.user_content_path_prefix.is_some()
    }

    /// Gate in front of every keyless (ADC) fallback.
    ///
    /// Reaching ADC means object_store resolves the workload's own runtime
    /// service account, which carries no Credential Access Boundary and is
    /// therefore unrestricted across the entire bucket. That is the correct
    /// identity for master credentials under Workload Identity, and precisely
    /// the wrong one for a scoped credential: prefix isolation on GCP is
    /// enforced *only* by the downscoped token, so falling back here would
    /// trade tenant isolation for availability without a word in the logs. A
    /// scoped credential that arrives with no usable token is a bug upstream or
    /// a forged dispatch payload, and both have to fail closed.
    fn ensure_keyless_allowed(&self) -> Result<()> {
        if self.is_scoped() {
            return Err(anyhow!(
                "scoped GCP credentials carry no usable access token (prefixes: {:?}, expiration: {:?}) - \
                 refusing to fall back to the ambient runtime identity, which enforces no prefix restriction",
                self.allowed_prefixes,
                self.expiration
            ));
        }
        Ok(())
    }
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
            // The downscoped token already carries the `tmp/*` prefixes on the
            // content bucket, so no separate credential is needed.
            StoreType::Tmp => self.content_bucket.clone(),
        };

        // Prefer access token for scoped credentials, fall back to service account key
        if let Some(ref access_token) = self.access_token
            && !access_token.trim().is_empty()
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
        if !self.service_account_key.trim().is_empty() {
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

        // Keyless. On Cloud Run / GKE Workload Identity neither a scoped token
        // nor a key JSON exists — the runtime service account is bound to the
        // workload and there is no key material to carry — so erroring here
        // failed every executor on its first object access. Building the store
        // bare hands the problem to object_store's own credential chain:
        // service-account key -> ADC file -> InstanceCredentialProvider against
        // the metadata server. Master credentials only: see
        // `ensure_keyless_allowed`.
        //
        // V4 signed URLs on this path resolve to InstanceSigningCredentialProvider,
        // which signs through iamcredentials.signBlob instead of a local private
        // key. That requires roles/iam.serviceAccountTokenCreator on the runtime
        // service account, granted to itself — a Terraform-side grant the runtime
        // module must make. Without it presigning fails with a 403 that only
        // surfaces when a client tries to fetch the URL, long after the code path
        // that produced it has returned successfully.
        //
        // `spawn_blocking` because the ADC resolution reads a credentials file
        // from disk, exactly as the key path parses a key.
        self.ensure_keyless_allowed()?;
        let store = tokio::task::spawn_blocking(move || {
            GoogleCloudStorageBuilder::new()
                .with_bucket_name(bucket)
                .build()
        })
        .await
        .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??;

        Ok(FlowLikeStore::Google(Arc::new(store)))
    }

    #[tracing::instrument(name = "GcpSharedCredentials::to_db", skip(self), level = "debug")]
    #[cfg(feature = "flow-runtime")]
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
            && !access_token.trim().is_empty()
        {
            let connection =
                make_gcs_builder_with_token(self.content_bucket.clone(), access_token.clone());
            return Ok(connection(path.clone()));
        }

        // Fall back to service account key
        if !self.service_account_key.trim().is_empty() {
            let connection = make_gcs_builder_with_key(
                self.content_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(connection(path.clone()));
        }

        // Keyless: let object_store resolve ADC down to the metadata server.
        // See `to_store_type` for why this branch exists and what it costs, and
        // `ensure_keyless_allowed` for why a scoped credential may not take it.
        self.ensure_keyless_allowed()?;
        let connection = make_gcs_builder_adc(self.content_bucket.clone());
        Ok(connection(path.clone()))
    }

    #[cfg(feature = "flow-runtime")]
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = format!("users/{}/apps/{}", sub, app_id);
        let path = db_path_from_base(&base_path);

        if let Some(ref access_token) = self.access_token
            && !access_token.trim().is_empty()
        {
            let connection =
                make_gcs_builder_with_token(self.content_bucket.clone(), access_token.clone());
            return Ok(connection(path.clone()));
        }

        if !self.service_account_key.trim().is_empty() {
            let connection = make_gcs_builder_with_key(
                self.content_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(connection(path.clone()));
        }

        // Keyless: see `to_store_type` and `ensure_keyless_allowed`. This is the
        // per-user database path, so an unrestricted fallback here would hand a
        // run the whole content bucket instead of one user's prefix.
        self.ensure_keyless_allowed()?;
        let connection = make_gcs_builder_adc(self.content_bucket.clone());
        Ok(connection(path.clone()))
    }

    #[cfg(feature = "flow-runtime")]
    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        if self.logs_bucket.is_empty() {
            return Err(anyhow!(
                "logs_bucket is empty - cannot create logs database builder"
            ));
        }

        // Prefer access token for scoped credentials
        if let Some(ref access_token) = self.access_token
            && !access_token.trim().is_empty()
        {
            let builder =
                make_gcs_builder_with_token(self.logs_bucket.clone(), access_token.clone());
            return Ok(Arc::new(builder));
        }

        // Fall back to service account key
        if !self.service_account_key.trim().is_empty() {
            let builder = make_gcs_builder_with_key(
                self.logs_bucket.clone(),
                self.service_account_key.clone(),
            );
            return Ok(Arc::new(builder));
        }

        // Keyless: see `to_store_type` and `ensure_keyless_allowed`.
        self.ensure_keyless_allowed()?;
        Ok(Arc::new(make_gcs_builder_adc(self.logs_bucket.clone())))
    }
}

/// Build a LanceDB connection factory over a GCS bucket.
///
/// `credential` is `None` only on the deliberate keyless path. A blank
/// credential is never normalised to `None` here: object_store rejects a
/// present-but-blank key, and that rejection is the outcome we want. Dropping
/// the option instead would resolve ADC and quietly substitute the unrestricted
/// runtime identity for the credential that was supposed to constrain the
/// caller.
#[cfg(feature = "flow-runtime")]
fn make_gcs_builder(
    bucket: String,
    credential: Option<(&'static str, String)>,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        let builder = lancedb::connect(&url);
        match &credential {
            Some((option, value)) => builder.storage_option(option.to_string(), value.clone()),
            None => builder,
        }
    }
}

#[cfg(feature = "flow-runtime")]
fn make_gcs_builder_with_key(
    bucket: String,
    service_account_key: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    make_gcs_builder(
        bucket,
        Some((GCS_SERVICE_ACCOUNT_KEY_OPTION, service_account_key)),
    )
}

#[cfg(feature = "flow-runtime")]
fn make_gcs_builder_with_token(
    bucket: String,
    access_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    make_gcs_builder(bucket, Some((GCS_STORAGE_TOKEN_OPTION, access_token)))
}

/// Application Default Credentials: no explicit credential, so object_store
/// resolves service-account key -> ADC file -> `InstanceCredentialProvider` on
/// the metadata server. This is the keyless equivalent of the two builders
/// above, and the path V4 signing reaches through
/// `InstanceSigningCredentialProvider` — see `to_store_type` for the IAM grant
/// that path requires.
#[cfg(feature = "flow-runtime")]
fn make_gcs_builder_adc(
    bucket: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    make_gcs_builder(bucket, None)
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

    /// A scoped credential whose token has gone missing. Both credential fields
    /// are `#[serde(default)]`, so this is the shape a truncated or forged
    /// dispatch payload deserializes to.
    fn scoped_credentials_missing_token() -> GcpSharedCredentials {
        GcpSharedCredentials {
            access_token: None,
            ..sample_scoped_credentials()
        }
    }

    /// The keyless path must stay closed to scoped credentials. ADC resolves the
    /// workload's runtime service account, which carries no Credential Access
    /// Boundary — falling back to it would hand a run the whole bucket instead
    /// of the prefixes it was scoped to.
    #[tokio::test]
    async fn scoped_credentials_without_token_refuse_keyless_fallback() {
        let creds = scoped_credentials_missing_token();

        #[cfg(feature = "flow-runtime")]
        {
            assert!(creds.to_db("test-app").await.is_err());
            assert!(creds.to_db_scoped("user-1", "test-app").await.is_err());
            assert!(creds.to_logs_db_builder().is_err());
        }
        assert!(creds.to_store(false).await.is_err());
    }

    /// A blank token is a broken credential, not a keyless one.
    #[cfg(feature = "flow-runtime")]
    #[tokio::test]
    async fn blank_scoped_token_refuses_keyless_fallback() {
        let creds = GcpSharedCredentials {
            access_token: Some("   ".to_string()),
            ..sample_scoped_credentials()
        };

        assert!(creds.to_db("test-app").await.is_err());
        assert!(creds.to_logs_db_builder().is_err());
    }

    /// Workload Identity: no key, no token, and nothing scoped about it. This is
    /// the shape the keyless branch exists for and it must keep working.
    #[cfg(feature = "flow-runtime")]
    #[tokio::test]
    async fn keyless_master_credentials_still_reach_adc() {
        let creds = GcpSharedCredentials {
            service_account_key: String::new(),
            access_token: None,
            ..sample_credentials()
        };

        assert!(!creds.is_scoped());
        assert!(creds.to_db("test-app").await.is_ok());
        assert!(creds.to_logs_db_builder().is_ok());
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

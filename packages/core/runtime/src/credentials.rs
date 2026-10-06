use aws_credentials::AwsSharedCredentials;
use azure_credentials::AzureSharedCredentials;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_types::Result;
use flow_like_types::async_trait;
use gcp_credentials::GcpSharedCredentials;
use mixed_credentials::MixedSharedCredentials;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod aws_credentials;
pub mod azure_credentials;
pub mod gcp_credentials;
pub mod mixed_credentials;
pub mod renewable;

pub use aws_credentials::BucketConfig;

/// Type alias for the logs database builder callback
#[cfg(feature = "flow-runtime")]
pub type LogsDbBuilder = Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>;

#[cfg(feature = "flow-runtime")]
pub(crate) fn db_path_from_base(base_path: &str) -> Path {
    let base = Path::from(base_path);

    if base_path.starts_with("users/") {
        return base.join("db");
    }

    base.join("storage").join("db")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreType {
    Meta,
    Content,
    Logs,
    /// Short-lived request/upload scratch space under `tmp/user/{sub}/apps/{app_id}`.
    ///
    /// AWS and GCP express this as an extra prefix on the content bucket inside the
    /// same credential, so it resolves to the content store there. Azure signs one
    /// directory per SAS and therefore needs a token of its own.
    Tmp,
}

#[async_trait]
pub trait SharedCredentialsTrait {
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore>;
    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore>;
    #[cfg(feature = "flow-runtime")]
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder>;
    #[cfg(feature = "flow-runtime")]
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder>;
    #[cfg(feature = "flow-runtime")]
    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SharedCredentials {
    Aws(AwsSharedCredentials),
    Azure(AzureSharedCredentials),
    Gcp(GcpSharedCredentials),
    Mixed(MixedSharedCredentials),
    /// Runtime capability; never serialize a refresh source or its session authority.
    #[serde(skip)]
    Renewable(Arc<renewable::RenewableSharedCredentials>),
}

impl SharedCredentials {
    /// Identity of a fixed content-storage lease, including its tokens, paths,
    /// endpoints and permissions. Renewable and ambient credentials stay with
    /// their owning runtime instead of entering a process-wide connection cache.
    #[cfg(feature = "flow-runtime")]
    pub fn database_cache_identity(&self) -> Result<Option<([u8; 32], std::time::SystemTime)>> {
        let mut credentials = self;
        for _ in 0..8 {
            match credentials {
                Self::Mixed(mixed) => credentials = &mixed.content,
                _ => break,
            }
        }
        let expiry = match credentials {
            Self::Aws(value)
                if value.access_key_id.as_ref().is_some_and(|v| !v.is_empty())
                    && value
                        .secret_access_key
                        .as_ref()
                        .is_some_and(|v| !v.is_empty()) =>
            {
                value.expiration
            }
            Self::Azure(value) if value.account_key.is_none() => value.expiration,
            Self::Gcp(value)
                if value
                    .access_token
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty()) =>
            {
                value.expiration
            }
            _ => return Ok(None),
        };
        let Some(expiry) = expiry else {
            return Ok(None);
        };
        let expires_at = std::time::SystemTime::from(expiry);
        if expires_at <= std::time::SystemTime::now() {
            return Err(flow_like_types::anyhow!(
                "Database storage credentials have expired"
            ));
        }
        // Keep secrets out of keys and diagnostics; token rotation or a change
        // to any serialized authority field always selects another connection.
        let mut hasher = blake3::Hasher::new();
        serde_json::to_writer(&mut hasher, credentials)?;
        Ok(Some((*hasher.finalize().as_bytes(), expires_at)))
    }

    pub async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_store(meta).await,
            SharedCredentials::Azure(azure) => azure.to_store(meta).await,
            SharedCredentials::Gcp(gcp) => gcp.to_store(meta).await,
            SharedCredentials::Mixed(mixed) => mixed.to_store(meta).await,
            SharedCredentials::Renewable(credentials) => credentials.to_store(meta).await,
        }
    }

    pub async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_store_type(store_type).await,
            SharedCredentials::Azure(azure) => azure.to_store_type(store_type).await,
            SharedCredentials::Gcp(gcp) => gcp.to_store_type(store_type).await,
            SharedCredentials::Mixed(mixed) => mixed.to_store_type(store_type).await,
            SharedCredentials::Renewable(credentials) => {
                credentials.to_store_type(store_type).await
            }
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_db(app_id).await,
            SharedCredentials::Azure(azure) => azure.to_db(app_id).await,
            SharedCredentials::Gcp(gcp) => gcp.to_db(app_id).await,
            SharedCredentials::Mixed(mixed) => mixed.to_db(app_id).await,
            SharedCredentials::Renewable(credentials) => credentials.to_db(app_id).await,
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_db_scoped(sub, app_id).await,
            SharedCredentials::Azure(azure) => azure.to_db_scoped(sub, app_id).await,
            SharedCredentials::Gcp(gcp) => gcp.to_db_scoped(sub, app_id).await,
            SharedCredentials::Mixed(mixed) => mixed.to_db_scoped(sub, app_id).await,
            SharedCredentials::Renewable(credentials) => {
                credentials.to_db_scoped(sub, app_id).await
            }
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_logs_db_builder(),
            SharedCredentials::Azure(azure) => azure.to_logs_db_builder(),
            SharedCredentials::Gcp(gcp) => gcp.to_logs_db_builder(),
            SharedCredentials::Mixed(mixed) => mixed.to_logs_db_builder(),
            SharedCredentials::Renewable(credentials) => credentials.to_logs_db_builder(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    #[test]
    #[cfg(feature = "flow-runtime")]
    fn test_db_path_from_base_for_app_scope() {
        let path = db_path_from_base("apps/test-app");

        assert_eq!(path.to_string(), "apps/test-app/storage/db");
    }

    #[test]
    #[cfg(feature = "flow-runtime")]
    fn test_db_path_from_base_for_user_scope() {
        let path = db_path_from_base("users/test-user/apps/test-app");

        assert_eq!(path.to_string(), "users/test-user/apps/test-app/db");
    }

    fn sample_aws() -> AwsSharedCredentials {
        AwsSharedCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("secret".to_string()),
            session_token: Some("token".to_string()),
            meta_bucket: "aws-meta".to_string(),
            content_bucket: "aws-content".to_string(),
            logs_bucket: "aws-logs".to_string(),
            meta_config: None,
            content_config: None,
            logs_config: None,
            region: "us-west-2".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    fn sample_azure() -> AzureSharedCredentials {
        AzureSharedCredentials {
            meta_sas_token: Some("?sv=2022-11-02&ss=b&sig=meta".to_string()),
            content_sas_token: Some("?sv=2022-11-02&ss=b&sig=content".to_string()),
            user_content_sas_token: None,
            logs_sas_token: Some("?sv=2022-11-02&ss=b&sig=logs".to_string()),
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: "azure-meta".to_string(),
            content_container: "azure-content".to_string(),
            logs_container: "azure-logs".to_string(),
            account_name: "mystorageaccount".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    fn sample_gcp() -> GcpSharedCredentials {
        GcpSharedCredentials {
            service_account_key: r#"{"type":"service_account","project_id":"test"}"#.to_string(),
            access_token: None,
            meta_bucket: "gcp-meta".to_string(),
            content_bucket: "gcp-content".to_string(),
            logs_bucket: "gcp-logs".to_string(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[test]
    #[cfg(feature = "flow-runtime")]
    fn database_cache_identity_tracks_storage_authority_and_expiry() {
        let mut base = sample_aws();
        base.expiration = Some(chrono::Utc::now() + chrono::Duration::hours(1));
        let identity = SharedCredentials::Aws(base.clone())
            .database_cache_identity()
            .unwrap()
            .unwrap();
        assert_eq!(
            identity,
            SharedCredentials::Aws(base.clone())
                .database_cache_identity()
                .unwrap()
                .unwrap()
        );
        let mut variants = Vec::new();
        let mut changed = base.clone();
        changed.session_token = Some("different-grant".into());
        variants.push(changed);
        let mut changed = base.clone();
        changed.secret_access_key = Some("rotated-key".into());
        variants.push(changed);
        let mut changed = base.clone();
        changed.content_bucket = "other-tenant".into();
        variants.push(changed);
        let mut changed = base.clone();
        changed.content_path_prefix = Some("apps/other-tenant".into());
        variants.push(changed);
        let mut changed = base.clone();
        changed.content_config = Some(BucketConfig {
            endpoint: Some("https://other-storage.example".into()),
            ..Default::default()
        });
        variants.push(changed);
        for changed in variants {
            assert_ne!(
                identity.0,
                SharedCredentials::Aws(changed)
                    .database_cache_identity()
                    .unwrap()
                    .unwrap()
                    .0
            );
        }
        base.expiration = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
        assert!(
            SharedCredentials::Aws(base.clone())
                .database_cache_identity()
                .is_err()
        );
        base.expiration = None;
        assert!(
            SharedCredentials::Aws(base)
                .database_cache_identity()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    #[cfg(feature = "flow-runtime")]
    fn database_cache_identity_separates_permissions_and_mixed_content() {
        let mut gcp = sample_gcp();
        gcp.access_token = Some("scoped-token".into());
        gcp.expiration = Some(chrono::Utc::now() + chrono::Duration::hours(1));
        let content = SharedCredentials::Gcp(gcp.clone());
        let base = content.database_cache_identity().unwrap().unwrap();
        gcp.write_access = false;
        assert_ne!(
            base.0,
            SharedCredentials::Gcp(gcp)
                .database_cache_identity()
                .unwrap()
                .unwrap()
                .0
        );
        let mixed = SharedCredentials::Mixed(MixedSharedCredentials {
            meta: Box::new(SharedCredentials::Aws(sample_aws())),
            content: Box::new(content),
            logs: Box::new(SharedCredentials::Azure(sample_azure())),
        });
        assert_eq!(base, mixed.database_cache_identity().unwrap().unwrap());
        let mut azure = sample_azure();
        azure.account_key = Some("unserialized-master-key".into());
        azure.expiration = Some(chrono::Utc::now() + chrono::Duration::hours(1));
        assert!(
            SharedCredentials::Azure(azure)
                .database_cache_identity()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_shared_credentials_aws_serialization() {
        let creds = SharedCredentials::Aws(sample_aws());
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("Aws"));
        assert!(json.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(json.contains("aws-meta"));
    }

    #[test]
    fn test_shared_credentials_azure_serialization() {
        let creds = SharedCredentials::Azure(sample_azure());
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("Azure"));
        assert!(json.contains("mystorageaccount"));
        assert!(json.contains("azure-meta"));
    }

    #[test]
    fn test_shared_credentials_gcp_serialization() {
        let creds = SharedCredentials::Gcp(sample_gcp());
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("Gcp"));
        assert!(json.contains("gcp-meta"));
    }

    #[test]
    fn test_shared_credentials_aws_deserialization() {
        let json = r#"{"Aws":{"access_key_id":"AKIA123","secret_access_key":"secret","session_token":"token","meta_bucket":"meta","content_bucket":"content","logs_bucket":"logs","region":"us-east-1","expiration":null}}"#;
        let creds: SharedCredentials = from_str(json).expect("Failed to deserialize");

        match creds {
            SharedCredentials::Aws(aws) => {
                assert_eq!(aws.access_key_id, Some("AKIA123".to_string()));
                assert_eq!(aws.region, "us-east-1");
            }
            _ => panic!("Expected AWS credentials"),
        }
    }

    #[test]
    fn test_shared_credentials_azure_deserialization() {
        let json = r#"{"Azure":{"sas_token":"?sv=test","meta_container":"meta","content_container":"content","logs_container":"logs","account_name":"storage","expiration":null}}"#;
        let creds: SharedCredentials = from_str(json).expect("Failed to deserialize");

        match creds {
            SharedCredentials::Azure(azure) => {
                assert_eq!(azure.account_name, "storage");
                assert_eq!(azure.meta_container, "meta");
            }
            _ => panic!("Expected Azure credentials"),
        }
    }

    #[test]
    fn test_shared_credentials_gcp_deserialization() {
        let json = r#"{"Gcp":{"service_account_key":"{\"type\":\"service_account\"}","meta_bucket":"meta","content_bucket":"content","logs_bucket":"logs","allowed_prefixes":[],"write_access":true,"expiration":null}}"#;
        let creds: SharedCredentials = from_str(json).expect("Failed to deserialize");

        match creds {
            SharedCredentials::Gcp(gcp) => {
                assert_eq!(gcp.meta_bucket, "meta");
                assert!(gcp.service_account_key.contains("service_account"));
            }
            _ => panic!("Expected GCP credentials"),
        }
    }

    #[test]
    fn test_shared_credentials_roundtrip_all_variants() {
        let variants: Vec<SharedCredentials> = vec![
            SharedCredentials::Aws(sample_aws()),
            SharedCredentials::Azure(sample_azure()),
            SharedCredentials::Gcp(sample_gcp()),
        ];

        for creds in variants {
            let json = to_string(&creds).expect("Failed to serialize");
            let deserialized: SharedCredentials = from_str(&json).expect("Failed to deserialize");

            match (&creds, &deserialized) {
                (SharedCredentials::Aws(a), SharedCredentials::Aws(b)) => {
                    assert_eq!(a.access_key_id, b.access_key_id);
                    assert_eq!(a.meta_bucket, b.meta_bucket);
                }
                (SharedCredentials::Azure(a), SharedCredentials::Azure(b)) => {
                    assert_eq!(a.account_name, b.account_name);
                    assert_eq!(a.meta_container, b.meta_container);
                }
                (SharedCredentials::Gcp(a), SharedCredentials::Gcp(b)) => {
                    assert_eq!(a.meta_bucket, b.meta_bucket);
                }
                _ => panic!("Variant mismatch after roundtrip"),
            }
        }
    }

    #[test]
    fn test_shared_credentials_debug_impl() {
        let creds = SharedCredentials::Aws(sample_aws());
        let debug_str = format!("{:?}", creds);
        assert!(debug_str.contains("Aws"));
    }

    #[test]
    fn test_shared_credentials_clone() {
        let original = SharedCredentials::Azure(sample_azure());
        let cloned = original.clone();

        match (original, cloned) {
            (SharedCredentials::Azure(a), SharedCredentials::Azure(b)) => {
                assert_eq!(a.account_name, b.account_name);
            }
            _ => panic!("Clone should preserve variant"),
        }
    }
}

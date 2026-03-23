use aws_credentials::AwsSharedCredentials;
use azure_credentials::AzureSharedCredentials;
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
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

pub use aws_credentials::BucketConfig;

/// Type alias for the logs database builder callback
pub type LogsDbBuilder = Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>;

pub(crate) fn db_path_from_base(base_path: &str) -> Path {
    let base = Path::from(base_path);

    if base_path.starts_with("users/") {
        return base.child("db");
    }

    base.child("storage").child("db")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreType {
    Meta,
    Content,
    Logs,
}

#[async_trait]
pub trait SharedCredentialsTrait {
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore>;
    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore>;
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder>;
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder>;
    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SharedCredentials {
    Aws(AwsSharedCredentials),
    Azure(AzureSharedCredentials),
    Gcp(GcpSharedCredentials),
    Mixed(MixedSharedCredentials),
}

impl SharedCredentials {
    pub async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_store(meta).await,
            SharedCredentials::Azure(azure) => azure.to_store(meta).await,
            SharedCredentials::Gcp(gcp) => gcp.to_store(meta).await,
            SharedCredentials::Mixed(mixed) => mixed.to_store(meta).await,
        }
    }

    pub async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_store_type(store_type).await,
            SharedCredentials::Azure(azure) => azure.to_store_type(store_type).await,
            SharedCredentials::Gcp(gcp) => gcp.to_store_type(store_type).await,
            SharedCredentials::Mixed(mixed) => mixed.to_store_type(store_type).await,
        }
    }

    pub async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_db(app_id).await,
            SharedCredentials::Azure(azure) => azure.to_db(app_id).await,
            SharedCredentials::Gcp(gcp) => gcp.to_db(app_id).await,
            SharedCredentials::Mixed(mixed) => mixed.to_db(app_id).await,
        }
    }

    pub async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_db_scoped(sub, app_id).await,
            SharedCredentials::Azure(azure) => azure.to_db_scoped(sub, app_id).await,
            SharedCredentials::Gcp(gcp) => gcp.to_db_scoped(sub, app_id).await,
            SharedCredentials::Mixed(mixed) => mixed.to_db_scoped(sub, app_id).await,
        }
    }

    pub fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        match self {
            SharedCredentials::Aws(aws) => aws.to_logs_db_builder(),
            SharedCredentials::Azure(azure) => azure.to_logs_db_builder(),
            SharedCredentials::Gcp(gcp) => gcp.to_logs_db_builder(),
            SharedCredentials::Mixed(mixed) => mixed.to_logs_db_builder(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    #[test]
    fn test_db_path_from_base_for_app_scope() {
        let path = db_path_from_base("apps/test-app");

        assert_eq!(path.to_string(), "apps/test-app/storage/db");
    }

    #[test]
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
            meta_container: "azure-meta".to_string(),
            content_container: "azure-content".to_string(),
            logs_container: "azure-logs".to_string(),
            account_name: "mystorageaccount".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
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

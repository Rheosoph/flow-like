use crate::credentials::{LogsDbBuilder, SharedCredentials, SharedCredentialsTrait, StoreType};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_types::{Result, async_trait};
use serde::{Deserialize, Serialize};

/// Mixed-provider shared credentials that allow different cloud providers per bucket.
///
/// Each bucket type (meta, content, logs) carries its own [`SharedCredentials`],
/// enabling configurations like S3 directory buckets for metadata and R2 for content.
///
/// Old clients that cannot parse `Mixed` will never see it — the server only
/// emits this variant when per-bucket providers actually differ.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixedSharedCredentials {
    pub meta: Box<SharedCredentials>,
    pub content: Box<SharedCredentials>,
    pub logs: Box<SharedCredentials>,
}

#[async_trait]
impl SharedCredentialsTrait for MixedSharedCredentials {
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        if meta {
            self.meta.to_store(true).await
        } else {
            self.content.to_store(false).await
        }
    }

    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        match store_type {
            StoreType::Meta => self.meta.to_store_type(StoreType::Meta).await,
            StoreType::Content => self.content.to_store_type(StoreType::Content).await,
            StoreType::Logs => self.logs.to_store_type(StoreType::Logs).await,
        }
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.content.to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.content.to_db_scoped(sub, app_id).await
    }

    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        self.logs.to_logs_db_builder()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::aws_credentials::AwsSharedCredentials;
    use flow_like_types::json::{from_str, to_string};

    fn sample_aws(bucket_prefix: &str) -> AwsSharedCredentials {
        AwsSharedCredentials {
            access_key_id: Some("AKIATEST".to_string()),
            secret_access_key: Some("secret".to_string()),
            session_token: Some("token".to_string()),
            meta_bucket: format!("{}-meta", bucket_prefix),
            content_bucket: format!("{}-content", bucket_prefix),
            logs_bucket: format!("{}-logs", bucket_prefix),
            meta_config: None,
            content_config: None,
            logs_config: None,
            region: "us-east-1".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[test]
    fn test_mixed_serialization_roundtrip() {
        let mixed = MixedSharedCredentials {
            meta: Box::new(SharedCredentials::Aws(sample_aws("s3"))),
            content: Box::new(SharedCredentials::Aws(sample_aws("r2"))),
            logs: Box::new(SharedCredentials::Aws(sample_aws("r2"))),
        };

        let creds = SharedCredentials::Mixed(mixed);
        let json = to_string(&creds).expect("serialize");
        let roundtrip: SharedCredentials = from_str(&json).expect("deserialize");

        match roundtrip {
            SharedCredentials::Mixed(m) => {
                match m.meta.as_ref() {
                    SharedCredentials::Aws(aws) => assert_eq!(aws.meta_bucket, "s3-meta"),
                    _ => panic!("expected Aws for meta"),
                }
                match m.content.as_ref() {
                    SharedCredentials::Aws(aws) => assert_eq!(aws.content_bucket, "r2-content"),
                    _ => panic!("expected Aws for content"),
                }
            }
            _ => panic!("expected Mixed variant"),
        }
    }

    #[test]
    fn test_mixed_deserialization_legacy_unaffected() {
        let json = r#"{"Aws":{"access_key_id":"AKIA123","secret_access_key":"secret","session_token":"token","meta_bucket":"meta","content_bucket":"content","logs_bucket":"logs","region":"us-east-1","expiration":null}}"#;
        let creds: SharedCredentials = from_str(json).expect("deserialize");
        assert!(matches!(creds, SharedCredentials::Aws(_)));
    }
}

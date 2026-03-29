use crate::credentials::{LogsDbBuilder, SharedCredentialsTrait, StoreType, db_path_from_base};
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_storage::object_store;
use flow_like_storage::object_store::aws::AmazonS3Builder;
use flow_like_storage::{files::store::FlowLikeStore, lancedb};
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Additional bucket configuration for S3-compatible storage
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BucketConfig {
    /// Custom endpoint URL (for R2, MinIO, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Whether this is an S3 Express One Zone bucket
    #[serde(default)]
    pub express: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AwsSharedCredentials {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    /// Meta bucket name
    pub meta_bucket: String,
    /// Content bucket name
    pub content_bucket: String,
    /// Logs bucket name
    pub logs_bucket: String,
    /// Optional meta bucket config (endpoint, express)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_config: Option<BucketConfig>,
    /// Optional content bucket config (endpoint, express)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_config: Option<BucketConfig>,
    /// Optional logs bucket config (endpoint, express)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_config: Option<BucketConfig>,
    pub region: String,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    /// App-level content path prefix (e.g., "apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_path_prefix: Option<String>,
    /// User-level content path prefix (e.g., "users/{sub}/apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content_path_prefix: Option<String>,
}

impl std::fmt::Debug for AwsSharedCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsSharedCredentials")
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
            .field("region", &self.region)
            .field("expiration", &self.expiration)
            .finish()
    }
}

impl AwsSharedCredentials {
    fn get_bucket_info(&self, store_type: StoreType) -> (&str, Option<&BucketConfig>) {
        match store_type {
            StoreType::Meta => (&self.meta_bucket, self.meta_config.as_ref()),
            StoreType::Content => (&self.content_bucket, self.content_config.as_ref()),
            StoreType::Logs => (&self.logs_bucket, self.logs_config.as_ref()),
        }
    }
}

#[async_trait]
impl SharedCredentialsTrait for AwsSharedCredentials {
    #[tracing::instrument(name = "AwsSharedCredentials::to_store", skip(self, meta), fields(meta = meta), level="debug")]
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        self.to_store_type(if meta {
            StoreType::Meta
        } else {
            StoreType::Content
        })
        .await
    }

    #[tracing::instrument(name = "AwsSharedCredentials::to_store_type", skip(self), fields(store_type = ?store_type), level="debug")]
    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        use flow_like_types::tokio;

        let (bucket_name, bucket_config) = self.get_bucket_info(store_type);

        let builder = {
            let mut builder = AmazonS3Builder::new()
                .with_access_key_id(
                    self.access_key_id
                        .clone()
                        .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
                )
                .with_secret_access_key(
                    self.secret_access_key
                        .clone()
                        .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
                )
                .with_token(
                    self.session_token
                        .clone()
                        .ok_or(anyhow!("SESSION TOKEN is not set"))?,
                )
                .with_bucket_name(bucket_name)
                .with_region(&self.region);

            if let Some(config) = bucket_config {
                if let Some(endpoint) = &config.endpoint {
                    builder = builder.with_endpoint(endpoint);
                }
                if config.express {
                    builder = builder.with_s3_express(true);
                }
            }
            builder
        };

        let store = tokio::task::spawn_blocking(move || builder.build())
            .await
            .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??;
        Ok(FlowLikeStore::AWS(Arc::new(store)))
    }

    #[tracing::instrument(name = "AwsSharedCredentials::to_db", skip(self), level = "debug")]
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = self
            .content_path_prefix
            .clone()
            .unwrap_or_else(|| format!("apps/{}", app_id));
        let path = db_path_from_base(&base_path);
        let connection = make_s3_builder(
            &self.content_bucket,
            self.content_config
                .as_ref()
                .and_then(|c| c.endpoint.clone()),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = format!("users/{}/apps/{}", sub, app_id);
        let path = db_path_from_base(&base_path);
        let connection = make_s3_builder(
            &self.content_bucket,
            self.content_config
                .as_ref()
                .and_then(|c| c.endpoint.clone()),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        if self.logs_bucket.is_empty() {
            return Err(anyhow!(
                "logs_bucket is empty - cannot create logs database builder"
            ));
        }
        tracing::debug!(
            logs_bucket = %self.logs_bucket,
            has_access_key = self.access_key_id.is_some(),
            has_secret_key = self.secret_access_key.is_some(),
            has_session_token = self.session_token.is_some(),
            "Building logs database connection"
        );
        let builder = make_s3_builder(
            &self.logs_bucket,
            self.logs_config.as_ref().and_then(|c| c.endpoint.clone()),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
        );
        Ok(Arc::new(builder))
    }
}

fn make_s3_builder(
    bucket: &str,
    endpoint: Option<String>,
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    let bucket = bucket.to_string();
    move |path| {
        let url = format!("s3://{}/{}", bucket, path);
        let mut builder = lancedb::connect(&url)
            .storage_option("aws_access_key_id".to_string(), access_key.clone())
            .storage_option("aws_secret_access_key".to_string(), secret_key.clone());

        if let Some(ref token) = session_token {
            builder = builder.storage_option("aws_session_token".to_string(), token.clone());
        }

        if let Some(ref ep) = endpoint {
            builder = builder.storage_option("aws_endpoint".to_string(), ep.clone());
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    fn sample_credentials() -> AwsSharedCredentials {
        AwsSharedCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: Some("FwoGZXIvYXdzEBYaDJ...".to_string()),
            meta_bucket: "my-meta-bucket--usw2-az1--x-s3".to_string(),
            content_bucket: "my-content-bucket".to_string(),
            logs_bucket: "my-logs-bucket".to_string(),
            meta_config: Some(BucketConfig {
                endpoint: None,
                express: true,
            }),
            content_config: None,
            logs_config: None,
            region: "us-west-2".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[test]
    fn test_aws_credentials_serialization() {
        let creds = sample_credentials();
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(json.contains("my-meta-bucket--usw2-az1--x-s3"));
        assert!(json.contains("my-content-bucket"));
        assert!(json.contains("us-west-2"));
    }

    #[test]
    fn test_aws_credentials_deserialization_legacy() {
        // Test backward compatibility - old format without *_config fields
        let json = r#"{
            "access_key_id": "AKIAIOSFODNN7EXAMPLE",
            "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "session_token": "FwoGZXIvYXdzEBYaDJ...",
            "meta_bucket": "test-meta",
            "content_bucket": "test-content",
            "logs_bucket": "test-logs",
            "region": "eu-west-1",
            "expiration": null
        }"#;

        let creds: AwsSharedCredentials = from_str(json).expect("Failed to deserialize");

        assert_eq!(
            creds.access_key_id,
            Some("AKIAIOSFODNN7EXAMPLE".to_string())
        );
        assert_eq!(creds.meta_bucket, "test-meta");
        assert_eq!(creds.content_bucket, "test-content");
        assert_eq!(creds.region, "eu-west-1");
        assert!(creds.meta_config.is_none());
        assert!(creds.content_config.is_none());
        assert!(creds.expiration.is_none());
    }

    #[test]
    fn test_aws_credentials_deserialization_with_config() {
        let json = r#"{
            "access_key_id": "AKIAIOSFODNN7EXAMPLE",
            "secret_access_key": "secret",
            "session_token": "token",
            "meta_bucket": "test-meta",
            "content_bucket": "test-content",
            "logs_bucket": "test-logs",
            "meta_config": { "endpoint": "https://r2.example.com", "express": false },
            "content_config": { "express": true },
            "region": "eu-west-1",
            "expiration": null
        }"#;

        let creds: AwsSharedCredentials = from_str(json).expect("Failed to deserialize");

        assert_eq!(creds.meta_bucket, "test-meta");
        assert_eq!(
            creds.meta_config.as_ref().unwrap().endpoint,
            Some("https://r2.example.com".to_string())
        );
        assert!(creds.content_config.as_ref().unwrap().express);
        assert!(creds.logs_config.is_none());
    }

    #[test]
    fn test_aws_credentials_roundtrip() {
        let original = sample_credentials();
        let json = to_string(&original).expect("Failed to serialize");
        let deserialized: AwsSharedCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(original.access_key_id, deserialized.access_key_id);
        assert_eq!(original.secret_access_key, deserialized.secret_access_key);
        assert_eq!(original.session_token, deserialized.session_token);
        assert_eq!(original.meta_bucket, deserialized.meta_bucket);
        assert_eq!(original.content_bucket, deserialized.content_bucket);
        assert_eq!(original.region, deserialized.region);
    }

    #[test]
    fn test_aws_credentials_with_expiration() {
        let json = r#"{
            "access_key_id": "AKIAIOSFODNN7EXAMPLE",
            "secret_access_key": "secret",
            "session_token": "token",
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs",
            "region": "us-east-1",
            "expiration": "2025-01-15T12:00:00Z"
        }"#;

        let creds: AwsSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert!(creds.expiration.is_some());
    }

    #[test]
    fn test_aws_credentials_optional_fields() {
        let json = r#"{
            "access_key_id": null,
            "secret_access_key": null,
            "session_token": null,
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs",
            "region": "us-east-1",
            "expiration": null
        }"#;

        let creds: AwsSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert!(creds.access_key_id.is_none());
        assert!(creds.secret_access_key.is_none());
        assert!(creds.session_token.is_none());
    }

    #[test]
    fn test_bucket_config_with_endpoint() {
        let json = r#"{
            "access_key_id": "key",
            "secret_access_key": "secret",
            "session_token": "token",
            "meta_bucket": "meta",
            "content_bucket": "content",
            "logs_bucket": "logs",
            "meta_config": { "endpoint": "https://account.r2.cloudflarestorage.com", "express": false },
            "content_config": { "endpoint": "http://localhost:9000", "express": false },
            "region": "auto",
            "expiration": null
        }"#;

        let creds: AwsSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert_eq!(
            creds.meta_config.as_ref().unwrap().endpoint,
            Some("https://account.r2.cloudflarestorage.com".to_string())
        );
        assert_eq!(
            creds.content_config.as_ref().unwrap().endpoint,
            Some("http://localhost:9000".to_string())
        );
        assert!(creds.logs_config.is_none());
    }
}

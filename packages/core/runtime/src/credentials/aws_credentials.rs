#[cfg(feature = "flow-runtime")]
use crate::credentials::{LogsDbBuilder, db_path_from_base};
use crate::credentials::{SharedCredentialsTrait, StoreType};
#[cfg(feature = "flow-runtime")]
use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_storage::files::store::FlowLikeStore;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb::connection::ConnectBuilder;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::object_store;
use flow_like_storage::object_store::aws::{
    AmazonS3, AmazonS3Builder, AmazonS3ConfigKey, S3CopyIfNotExists,
};
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Building a client costs a fresh connection pool and TLS handshake, so one
/// client serves every caller holding the same credentials for the same bucket.
static STORES: LazyLock<moka::sync::Cache<[u8; 32], Arc<AmazonS3>>> = LazyLock::new(|| {
    moka::sync::Cache::builder()
        .max_capacity(256)
        .time_to_idle(Duration::from_secs(15 * 60))
        .build()
});

/// A session this close to expiry gets a private client and is never cached.
const STORE_CACHE_EXPIRY_MARGIN: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// Additional bucket configuration for S3-compatible storage
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BucketConfig {
    /// Custom endpoint URL (for R2, MinIO, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Whether this is an S3 Express One Zone bucket
    #[serde(default)]
    pub express: bool,
    /// Send bucket names in the request path for compatible S3 endpoints.
    #[serde(default)]
    pub use_path_style: bool,
    /// Permit plain HTTP on an explicitly configured private endpoint.
    #[serde(default)]
    pub allow_http: bool,
    /// SSE-KMS customer-managed key (ARN, key id or alias) to send with every
    /// write to this bucket.
    ///
    /// A bucket whose *default* encryption already names the key needs none of
    /// this — S3 applies the key server-side and only the IAM grant matters
    /// (see the KMS statements the API attaches to scoped credentials). Set it
    /// when the bucket policy denies writes that arrive without an explicit
    /// `x-amz-server-side-encryption` header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_arn: Option<String>,
    /// Request an S3 Bucket Key, which collapses the per-object KMS call into
    /// one call per bucket-key lifetime. Only meaningful alongside
    /// `kms_key_arn`.
    #[serde(default)]
    pub kms_bucket_key: bool,
}

impl BucketConfig {
    /// The SSE-KMS key to send headers for, if any.
    ///
    /// Directory (S3 Express One Zone) buckets take their key from the bucket
    /// itself and reject per-request encryption headers, so they always
    /// resolve to `None`.
    fn sse_kms_key(&self) -> Option<&str> {
        if self.express {
            return None;
        }
        self.kms_key_arn.as_deref()
    }
}

/// Apply the bucket's SSE-KMS configuration to an object-store builder.
fn apply_sse_kms(
    mut builder: AmazonS3Builder,
    config: Option<&BucketConfig>,
) -> Result<AmazonS3Builder> {
    let Some(config) = config else {
        return Ok(builder);
    };
    let Some(key) = config.sse_kms_key() else {
        return Ok(builder);
    };

    builder = builder.with_sse_kms_encryption(key);
    if config.kms_bucket_key {
        let bucket_key: AmazonS3ConfigKey = "aws_sse_bucket_key_enabled"
            .parse()
            .map_err(|e| anyhow!("aws_sse_bucket_key_enabled is not a known S3 option: {e}"))?;
        builder = builder.with_config(bucket_key, "true");
    }
    Ok(builder)
}

/// Lance talks to S3 through `storage_option` strings rather than a builder, so
/// the SSE-KMS settings have to be spelled out again here.
pub fn sse_kms_storage_options(config: Option<&BucketConfig>) -> Vec<(String, String)> {
    let Some(config) = config else {
        return Vec::new();
    };
    let Some(key) = config.sse_kms_key() else {
        return Vec::new();
    };

    let mut options = vec![
        (
            "aws_server_side_encryption".to_string(),
            "aws:kms".to_string(),
        ),
        ("aws_sse_kms_key_id".to_string(), key.to_string()),
    ];
    if config.kms_bucket_key {
        options.push(("aws_sse_bucket_key_enabled".to_string(), "true".to_string()));
    }
    options
}

/// Options shared by API-side and execution-side LanceDB connections.
pub fn s3_storage_options(config: Option<&BucketConfig>) -> Vec<(String, String)> {
    let mut options = sse_kms_storage_options(config);
    if let Some(config) = config {
        if let Some(endpoint) = &config.endpoint {
            options.push(("aws_endpoint".into(), endpoint.clone()));
        }
        if config.use_path_style {
            options.push(("aws_virtual_hosted_style_request".into(), "false".into()));
        }
        if config.allow_http {
            options.push(("allow_http".into(), "true".into()));
        }
    }
    options
}

/// R2 rejects conditional multipart copies; it guards copies with this header instead.
pub(crate) const R2_COPY_IF_NOT_EXISTS: (&str, &str) = ("cf-copy-destination-if-none-match", "*");

pub(crate) fn is_r2(config: Option<&BucketConfig>) -> bool {
    config
        .and_then(|config| config.endpoint.as_deref())
        .and_then(|endpoint| url::Url::parse(endpoint).ok())
        .and_then(|endpoint| endpoint.host_str().map(str::to_owned))
        .is_some_and(|host| host.ends_with(".r2.cloudflarestorage.com"))
}

pub(crate) fn s3_copy_if_not_exists(config: Option<&BucketConfig>) -> S3CopyIfNotExists {
    if is_r2(config) {
        let (header, value) = R2_COPY_IF_NOT_EXISTS;
        S3CopyIfNotExists::Header(header.into(), value.into())
    } else {
        S3CopyIfNotExists::Multipart
    }
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
            // The assumed role already carries the `tmp/*` prefixes on the content
            // bucket, so no separate credential is needed.
            StoreType::Tmp => (&self.content_bucket, self.content_config.as_ref()),
        }
    }

    /// Identity of the client built for one bucket: every secret, the session
    /// and its expiry, the region and the bucket's transport options. Rotated
    /// or re-scoped credentials therefore always select another client.
    fn store_cache_key(
        &self,
        bucket_name: &str,
        bucket_config: Option<&BucketConfig>,
    ) -> Option<[u8; 32]> {
        if self
            .expiration
            .is_some_and(|expiry| expiry <= chrono::Utc::now() + STORE_CACHE_EXPIRY_MARGIN)
        {
            return None;
        }
        let mut hasher = blake3::Hasher::new();
        serde_json::to_writer(
            &mut hasher,
            &(
                &self.access_key_id,
                &self.secret_access_key,
                &self.session_token,
                self.expiration,
                &self.region,
                bucket_name,
                bucket_config,
            ),
        )
        .ok()?;
        Some(*hasher.finalize().as_bytes())
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
        let cache_key = self.store_cache_key(bucket_name, bucket_config);
        if let Some(store) = cache_key.and_then(|key| STORES.get(&key)) {
            return Ok(FlowLikeStore::AWS(store));
        }

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
                .with_bucket_name(bucket_name)
                .with_region(&self.region)
                .with_copy_if_not_exists(s3_copy_if_not_exists(bucket_config));

            if let Some(token) = &self.session_token {
                builder = builder.with_token(token);
            }
            if let Some(config) = bucket_config {
                builder = builder.with_allow_http(config.allow_http);
                if config.use_path_style {
                    builder = builder.with_virtual_hosted_style_request(false);
                }
                if let Some(endpoint) = &config.endpoint {
                    builder = builder.with_endpoint(endpoint);
                }
                if config.express {
                    builder = builder.with_s3_express(true);
                }
            }
            apply_sse_kms(builder, bucket_config)?
        };

        let store = Arc::new(
            tokio::task::spawn_blocking(move || builder.build())
                .await
                .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??,
        );
        if let Some(key) = cache_key {
            STORES.insert(key, store.clone());
        }
        Ok(FlowLikeStore::AWS(store))
    }

    #[tracing::instrument(name = "AwsSharedCredentials::to_db", skip(self), level = "debug")]
    #[cfg(feature = "flow-runtime")]
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = self
            .content_path_prefix
            .clone()
            .unwrap_or_else(|| format!("apps/{}", app_id));
        let path = db_path_from_base(&base_path);
        let connection = make_s3_builder(
            &self.content_bucket,
            self.content_config.as_ref(),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
            self.region.clone(),
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    #[cfg(feature = "flow-runtime")]
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = format!("users/{}/apps/{}", sub, app_id);
        let path = db_path_from_base(&base_path);
        let connection = make_s3_builder(
            &self.content_bucket,
            self.content_config.as_ref(),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
            self.region.clone(),
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    #[cfg(feature = "flow-runtime")]
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
            self.logs_config.as_ref(),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token.clone(),
            self.region.clone(),
        );
        Ok(Arc::new(builder))
    }
}

#[cfg(feature = "flow-runtime")]
fn make_s3_builder(
    bucket: &str,
    config: Option<&BucketConfig>,
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    region: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    let bucket = bucket.to_string();
    let storage_options = s3_storage_options(config);
    move |path| {
        let url = format!("s3://{}/{}", bucket, path);
        let mut builder = connect_lance(&url)
            .storage_option("aws_access_key_id".to_string(), access_key.clone())
            .storage_option("aws_secret_access_key".to_string(), secret_key.clone())
            .storage_option("aws_region".to_string(), region.clone());

        if let Some(ref token) = session_token {
            builder = builder.storage_option("aws_session_token".to_string(), token.clone());
        }

        for (key, value) in &storage_options {
            builder = builder.storage_option(key.clone(), value.clone());
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    #[test]
    fn conditional_copy_uses_the_provider_supported_precondition() {
        assert_eq!(s3_copy_if_not_exists(None), S3CopyIfNotExists::Multipart);
        let config = |endpoint: &str| BucketConfig {
            endpoint: Some(endpoint.into()),
            ..Default::default()
        };
        assert_eq!(
            s3_copy_if_not_exists(Some(&config("https://account.r2.cloudflarestorage.com"))),
            S3CopyIfNotExists::Header("cf-copy-destination-if-none-match".into(), "*".into())
        );
        for endpoint in [
            "http://localhost:9000",
            "https://account.r2.cloudflarestorage.com.example.org",
        ] {
            assert_eq!(
                s3_copy_if_not_exists(Some(&config(endpoint))),
                S3CopyIfNotExists::Multipart
            );
        }
    }

    #[flow_like_types::tokio::test]
    async fn static_credentials_build_without_a_session_token() {
        let mut credentials = sample_credentials();
        credentials.session_token = None;
        credentials.meta_config = Some(BucketConfig {
            endpoint: Some("http://127.0.0.1:9000".into()),
            use_path_style: true,
            allow_http: true,
            ..Default::default()
        });
        credentials
            .to_store(true)
            .await
            .expect("static API credentials should build a store");
    }

    fn session_credentials(lifetime: chrono::TimeDelta) -> AwsSharedCredentials {
        AwsSharedCredentials {
            session_token: Some(flow_like_types::create_id()),
            expiration: Some(chrono::Utc::now() + lifetime),
            meta_config: None,
            ..sample_credentials()
        }
    }

    fn s3_client(store: FlowLikeStore) -> Arc<AmazonS3> {
        match store {
            FlowLikeStore::AWS(store) => store,
            other => panic!("expected an S3 store, got {other:?}"),
        }
    }

    #[flow_like_types::tokio::test]
    async fn one_client_serves_each_credential_and_bucket() {
        let credentials = session_credentials(chrono::TimeDelta::hours(1));
        let meta = s3_client(credentials.to_store(true).await.unwrap());

        assert!(Arc::ptr_eq(
            &meta,
            &s3_client(credentials.to_store(true).await.unwrap())
        ));
        assert!(!Arc::ptr_eq(
            &meta,
            &s3_client(credentials.to_store(false).await.unwrap())
        ));
        let rotated = AwsSharedCredentials {
            session_token: Some(flow_like_types::create_id()),
            ..credentials.clone()
        };
        assert!(!Arc::ptr_eq(
            &meta,
            &s3_client(rotated.to_store(true).await.unwrap())
        ));
    }

    #[flow_like_types::tokio::test]
    async fn sessions_near_expiry_get_private_clients() {
        let credentials = session_credentials(chrono::TimeDelta::minutes(1));
        let first = s3_client(credentials.to_store(true).await.unwrap());
        let second = s3_client(credentials.to_store(true).await.unwrap());
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn lance_options_preserve_compatible_s3_transport() {
        let config = BucketConfig {
            endpoint: Some("http://object-gateway:9000".into()),
            use_path_style: true,
            allow_http: true,
            ..Default::default()
        };
        let options = s3_storage_options(Some(&config));
        assert!(options.contains(&("aws_endpoint".into(), "http://object-gateway:9000".into())));
        assert!(options.contains(&("aws_virtual_hosted_style_request".into(), "false".into())));
        assert!(options.contains(&("allow_http".into(), "true".into())));
        for (key, _) in options {
            key.parse::<AmazonS3ConfigKey>()
                .expect("Lance options must be recognized by object_store");
        }
    }

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
                kms_key_arn: None,
                kms_bucket_key: false,
                ..Default::default()
            }),
            content_config: None,
            logs_config: None,
            region: "us-west-2".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    fn kms_bucket(express: bool) -> BucketConfig {
        BucketConfig {
            endpoint: None,
            express,
            kms_key_arn: Some(
                "arn:aws:kms:us-west-2:123456789012:key/1234abcd-12ab-34cd-56ef-1234567890ab"
                    .to_string(),
            ),
            kms_bucket_key: true,
            ..Default::default()
        }
    }

    /// Lance reaches S3 through option strings, so the names have to match
    /// object_store's exactly — a typo is silently ignored and the write goes
    /// out unencrypted.
    #[test]
    fn test_sse_kms_storage_options_use_object_store_names() {
        let config = kms_bucket(false);
        let options = sse_kms_storage_options(Some(&config));

        assert_eq!(
            options
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "aws_server_side_encryption",
                "aws_sse_kms_key_id",
                "aws_sse_bucket_key_enabled",
            ]
        );
        assert_eq!(options[0].1, "aws:kms");
        assert_eq!(options[1].1, config.kms_key_arn.unwrap());
    }

    /// Directory buckets take their key from the bucket and reject per-request
    /// encryption headers.
    #[test]
    fn test_sse_kms_is_skipped_for_express_buckets() {
        assert!(sse_kms_storage_options(Some(&kms_bucket(true))).is_empty());
        assert!(sse_kms_storage_options(None).is_empty());
        assert!(
            sse_kms_storage_options(Some(&BucketConfig::default())).is_empty(),
            "a bucket without a key configures nothing"
        );
    }

    /// Credentials written before SSE-KMS existed must still deserialize.
    #[test]
    fn test_bucket_config_kms_fields_default_when_absent() {
        let config: BucketConfig =
            from_str(r#"{"endpoint":null,"express":false}"#).expect("Failed to deserialize");
        assert_eq!(config.kms_key_arn, None);
        assert!(!config.kms_bucket_key);
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

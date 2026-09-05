//! Shared storage configuration for all backends (Lambda, Kubernetes, Docker Compose, etc.)
//!
//! This module provides a unified way to configure and create FlowLikeStore instances
//! from environment variables across all deployment backends.

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{
    aws::AmazonS3Builder,
    azure::{AzureConfigKey, MicrosoftAzureBuilder},
    gcp::GoogleCloudStorageBuilder,
};
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, sync::Arc};

/// Optional configuration values treat blank Compose interpolation as absent.
pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    normalize_optional(std::env::var(name).ok())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// File-backed secrets take precedence; unreadable and empty files fail startup.
pub(crate) fn secret_env(name: &str) -> Result<Option<String>> {
    if let Some(path) = non_empty_env(&format!("{name}_FILE")) {
        let value = std::fs::read_to_string(&path)
            .map_err(|error| flow_like_types::anyhow!("Cannot read {name}_FILE: {error}"))?;
        let value = value.trim_end_matches(['\r', '\n']).to_owned();
        if value.is_empty() {
            return Err(flow_like_types::anyhow!("{name}_FILE is empty"));
        }
        return Ok(Some(value));
    }
    Ok(std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty()))
}

/// Storage provider type
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageProvider {
    Aws,
    Azure,
    Gcp,
}

impl Display for StorageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageProvider::Aws => write!(f, "aws"),
            StorageProvider::Azure => write!(f, "azure"),
            StorageProvider::Gcp => write!(f, "gcp"),
        }
    }
}

impl std::str::FromStr for StorageProvider {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "aws" | "s3" => Ok(StorageProvider::Aws),
            "azure" | "blob" => Ok(StorageProvider::Azure),
            "gcp" | "gcs" | "google" => Ok(StorageProvider::Gcp),
            _ => Err(format!("Unknown storage provider: {}", s)),
        }
    }
}

/// AWS S3 configuration
///
/// Authentication options:
/// 1. Static credentials (AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY)
/// 2. IAM instance role (on EC2/ECS/EKS - automatic via AWS SDK credential chain)
/// 3. Web Identity / IRSA (on EKS - set AWS_WEB_IDENTITY_TOKEN_FILE + AWS_ROLE_ARN)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub use_path_style: bool,
}

impl S3Config {
    pub fn from_env() -> Result<Self> {
        let access_key_id = secret_env("AWS_ACCESS_KEY_ID")?;
        let secret_access_key = secret_env("AWS_SECRET_ACCESS_KEY")?;
        if access_key_id.is_some() != secret_access_key.is_some() {
            return Err(flow_like_types::anyhow!(
                "Both AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY must be configured"
            ));
        }
        Ok(S3Config {
            // Stores also sign browser/desktop URLs. Resolve the same public
            // hostname inside the deployment instead of rewriting signed URLs.
            endpoint: non_empty_env("S3_PUBLIC_ENDPOINT")
                .or_else(|| non_empty_env("S3_INTERNAL_ENDPOINT"))
                .or_else(|| non_empty_env("AWS_ENDPOINT")),
            region: non_empty_env("AWS_REGION").unwrap_or_else(|| "us-east-1".to_string()),
            access_key_id,
            secret_access_key,
            session_token: secret_env("AWS_SESSION_TOKEN")?,
            use_path_style: std::env::var("AWS_USE_PATH_STYLE")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        })
    }

    pub fn build_store(&self, bucket: &str) -> Result<FlowLikeStore> {
        use flow_like_storage::object_store::aws::AmazonS3ConfigKey;

        let mut builder = AmazonS3Builder::new()
            .with_region(&self.region)
            .with_bucket_name(bucket);

        if let Some(endpoint) = &self.endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }

        // Use static credentials if provided, otherwise rely on AWS credential chain
        // (instance role, web identity, environment variables, etc.)
        if let (Some(access_key), Some(secret_key)) = (&self.access_key_id, &self.secret_access_key)
        {
            builder = builder
                .with_access_key_id(access_key)
                .with_secret_access_key(secret_key);

            // Add session token if present (for assumed role credentials)
            if let Some(token) = &self.session_token {
                builder = builder.with_token(token);
            }
        }

        if self.use_path_style {
            builder = builder.with_config(AmazonS3ConfigKey::VirtualHostedStyleRequest, "false");
        }

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build S3 store: {}", e))?;
        Ok(FlowLikeStore::AWS(Arc::new(store)))
    }
}

/// Azure Blob Storage configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AzureConfig {
    pub account: String,
    pub access_key: Option<String>,
}

impl AzureConfig {
    pub fn from_env() -> Result<Self> {
        Ok(AzureConfig {
            account: std::env::var("AZURE_STORAGE_ACCOUNT_NAME")
                .map_err(|_| flow_like_types::anyhow!("AZURE_STORAGE_ACCOUNT_NAME not set"))?,
            access_key: std::env::var("AZURE_STORAGE_ACCOUNT_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        })
    }

    pub fn build_store(&self, container: &str) -> Result<FlowLikeStore> {
        let mut builder = if self.access_key.is_some() {
            MicrosoftAzureBuilder::new()
        } else {
            MicrosoftAzureBuilder::from_env()
        }
        .with_account(&self.account)
        .with_container_name(container);

        if let Some(access_key) = &self.access_key {
            builder = builder.with_config(AzureConfigKey::AccessKey, access_key);
        }

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build Azure store: {}", e))?;
        Ok(FlowLikeStore::Azure(Arc::new(store)))
    }
}

/// GCP Cloud Storage configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GcpConfig {
    pub project_id: String,
    pub credentials_json: Option<String>,
}

impl GcpConfig {
    pub fn from_env() -> Result<Self> {
        Ok(GcpConfig {
            project_id: std::env::var("GCP_PROJECT_ID")
                .map_err(|_| flow_like_types::anyhow!("GCP_PROJECT_ID not set"))?,
            // Set-but-empty collapses to `None`, matching
            // `credentials::gcp_credentials`. Left as `Some("")` it reached
            // `with_service_account_key("")` below, which object_store rejects as
            // a malformed key — so an optional Terraform variable rendering to an
            // empty env var broke every store on a deployment that has no key by
            // design.
            credentials_json: std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON")
                .ok()
                .filter(|key| !key.trim().is_empty()),
        })
    }

    pub fn build_store(&self, bucket: &str) -> Result<FlowLikeStore> {
        let mut builder = GoogleCloudStorageBuilder::new().with_bucket_name(bucket);

        // No key means Workload Identity: object_store resolves its own chain
        // down to the metadata server. A blank key is never passed through — see
        // `from_env`.
        if let Some(creds) = &self.credentials_json {
            builder = builder.with_service_account_key(creds);
        }

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build GCS store: {}", e))?;
        Ok(FlowLikeStore::Google(Arc::new(store)))
    }
}

/// Unified storage configuration
#[derive(Clone, Debug)]
pub enum StorageConfig {
    Aws(S3Config),
    Azure(AzureConfig),
    Gcp(GcpConfig),
}

impl StorageConfig {
    /// Load storage config from environment based on STORAGE_PROVIDER
    pub fn from_env() -> Result<Self> {
        let provider: StorageProvider = std::env::var("STORAGE_PROVIDER")
            .map_err(|_| flow_like_types::anyhow!("STORAGE_PROVIDER not set"))?
            .parse()
            .map_err(|e| flow_like_types::anyhow!("{}", e))?;

        Self::from_env_with_provider(provider)
    }

    /// Load storage config from environment with explicit provider
    pub fn from_env_with_provider(provider: StorageProvider) -> Result<Self> {
        match provider {
            StorageProvider::Aws => Ok(StorageConfig::Aws(S3Config::from_env()?)),
            StorageProvider::Azure => Ok(StorageConfig::Azure(AzureConfig::from_env()?)),
            StorageProvider::Gcp => Ok(StorageConfig::Gcp(GcpConfig::from_env()?)),
        }
    }

    /// Get the provider type
    pub fn provider(&self) -> StorageProvider {
        match self {
            StorageConfig::Aws(_) => StorageProvider::Aws,
            StorageConfig::Azure(_) => StorageProvider::Azure,
            StorageConfig::Gcp(_) => StorageProvider::Gcp,
        }
    }

    /// Build a FlowLikeStore for the given bucket/container name
    pub fn build_store(&self, bucket: &str) -> Result<FlowLikeStore> {
        match self {
            StorageConfig::Aws(cfg) => cfg.build_store(bucket),
            StorageConfig::Azure(cfg) => cfg.build_store(bucket),
            StorageConfig::Gcp(cfg) => cfg.build_store(bucket),
        }
    }
}

/// Create a FlowLikeStore from the unified storage config
/// This creates a single store - for multi-bucket setups, call build_store multiple times
pub fn create_flow_store(config: &StorageConfig, bucket: &str) -> Result<FlowLikeStore> {
    config.build_store(bucket)
}

/// Load bucket names from environment
#[derive(Clone, Debug)]
pub struct BucketConfig {
    pub meta: String,
    pub content: String,
    pub cdn: String,
    pub logs: String,
}

impl BucketConfig {
    /// Load bucket configuration from environment.
    ///
    /// Uses provider-specific env vars with fallback to generic ones.
    /// `META_BUCKET` falls back to `CONTENT_BUCKET` when unset, enabling
    /// single-bucket deployments.
    pub fn from_env(provider: &StorageProvider) -> Result<Self> {
        let (content, meta, cdn, logs) = match provider {
            StorageProvider::Aws => {
                let content = non_empty_env("AWS_CONTENT_BUCKET")
                    .or_else(|| non_empty_env("CONTENT_BUCKET"))
                    .ok_or_else(|| {
                        flow_like_types::anyhow!("CONTENT_BUCKET or AWS_CONTENT_BUCKET not set")
                    })?;
                let meta = non_empty_env("AWS_META_BUCKET")
                    .or_else(|| non_empty_env("META_BUCKET"))
                    .unwrap_or_else(|| content.clone());
                let cdn = non_empty_env("CDN_BUCKET_NAME")
                    .or_else(|| non_empty_env("AWS_CDN_BUCKET"))
                    .unwrap_or_else(|| content.clone());
                let logs = non_empty_env("AWS_LOG_BUCKET")
                    .or_else(|| non_empty_env("LOG_BUCKET"))
                    .ok_or_else(|| {
                        flow_like_types::anyhow!("LOG_BUCKET or AWS_LOG_BUCKET not set")
                    })?;
                (content, meta, cdn, logs)
            }
            StorageProvider::Azure => {
                let content = std::env::var("AZURE_CONTENT_CONTAINER")
                    .or_else(|_| std::env::var("CONTENT_BUCKET"))
                    .map_err(|_| {
                        flow_like_types::anyhow!(
                            "CONTENT_BUCKET or AZURE_CONTENT_CONTAINER not set"
                        )
                    })?;
                let meta = std::env::var("AZURE_META_CONTAINER")
                    .or_else(|_| std::env::var("META_BUCKET"))
                    .unwrap_or_else(|_| content.clone());
                let cdn = std::env::var("CDN_BUCKET_NAME")
                    .or_else(|_| std::env::var("AZURE_CDN_CONTAINER"))
                    .unwrap_or_else(|_| content.clone());
                let logs = std::env::var("AZURE_LOG_CONTAINER")
                    .or_else(|_| std::env::var("LOG_BUCKET"))
                    .map_err(|_| {
                        flow_like_types::anyhow!("LOG_BUCKET or AZURE_LOG_CONTAINER not set")
                    })?;
                (content, meta, cdn, logs)
            }
            StorageProvider::Gcp => {
                let content = std::env::var("GCP_CONTENT_BUCKET")
                    .or_else(|_| std::env::var("CONTENT_BUCKET"))
                    .map_err(|_| {
                        flow_like_types::anyhow!("CONTENT_BUCKET or GCP_CONTENT_BUCKET not set")
                    })?;
                let meta = std::env::var("GCP_META_BUCKET")
                    .or_else(|_| std::env::var("META_BUCKET"))
                    .unwrap_or_else(|_| content.clone());
                let cdn = std::env::var("CDN_BUCKET_NAME")
                    .or_else(|_| std::env::var("GCP_CDN_BUCKET"))
                    .unwrap_or_else(|_| content.clone());
                let logs = std::env::var("GCP_LOG_BUCKET")
                    .or_else(|_| std::env::var("LOG_BUCKET"))
                    .map_err(|_| {
                        flow_like_types::anyhow!("LOG_BUCKET or GCP_LOG_BUCKET not set")
                    })?;
                (content, meta, cdn, logs)
            }
        };

        Ok(BucketConfig {
            meta,
            content,
            cdn,
            logs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_compose_values_do_not_shadow_fallbacks() {
        for value in [None, Some(String::new()), Some("  ".into())] {
            assert_eq!(
                normalize_optional(value).or_else(|| Some("fallback-bucket".into())),
                Some("fallback-bucket".into())
            );
        }
        assert_eq!(
            normalize_optional(Some("  https://s3.example.com  ".into())),
            Some("https://s3.example.com".into())
        );
    }

    #[test]
    fn test_storage_provider_parse() {
        assert_eq!(
            "aws".parse::<StorageProvider>().unwrap(),
            StorageProvider::Aws
        );
        assert_eq!(
            "s3".parse::<StorageProvider>().unwrap(),
            StorageProvider::Aws
        );
        assert_eq!(
            "azure".parse::<StorageProvider>().unwrap(),
            StorageProvider::Azure
        );
        assert_eq!(
            "blob".parse::<StorageProvider>().unwrap(),
            StorageProvider::Azure
        );
        assert_eq!(
            "gcp".parse::<StorageProvider>().unwrap(),
            StorageProvider::Gcp
        );
        assert_eq!(
            "gcs".parse::<StorageProvider>().unwrap(),
            StorageProvider::Gcp
        );
        assert_eq!(
            "google".parse::<StorageProvider>().unwrap(),
            StorageProvider::Gcp
        );
    }

    #[test]
    fn test_storage_provider_display() {
        assert_eq!(StorageProvider::Aws.to_string(), "aws");
        assert_eq!(StorageProvider::Azure.to_string(), "azure");
        assert_eq!(StorageProvider::Gcp.to_string(), "gcp");
    }
}

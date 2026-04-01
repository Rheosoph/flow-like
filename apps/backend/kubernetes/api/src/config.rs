use serde::Deserialize;
use std::env;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub port: u16,
    pub storage_config: StorageConfig,
}

/// Storage provider configuration - supports AWS, Azure, GCP, and S3-compatible
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum StorageConfig {
    Aws(AwsStorageConfig),
    Azure(AzureStorageConfig),
    Gcp(GcpStorageConfig),
    R2(R2StorageConfig),
    S3(S3StorageConfig),
}

#[derive(Clone, Debug, Deserialize)]
pub struct AwsStorageConfig {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub region: String,
    pub endpoint: Option<String>,
    pub content_bucket: String,
    pub cdn_bucket: String,
    pub meta_bucket: String,
    pub use_path_style: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct R2StorageConfig {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub content_bucket: String,
    pub cdn_bucket: String,
    pub meta_bucket: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AzureStorageConfig {
    pub account_name: String,
    pub account_key: Option<String>,
    pub content_container: String,
    pub cdn_container: String,
    pub meta_container: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GcpStorageConfig {
    pub project_id: String,
    pub service_account_key: Option<String>,
    pub content_bucket: String,
    pub cdn_bucket: String,
    pub meta_bucket: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct S3StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub content_bucket: String,
    pub cdn_bucket: String,
    pub meta_bucket: String,
    pub use_path_style: bool,
}

impl StorageConfig {
    pub fn provider_name(&self) -> &'static str {
        match self {
            StorageConfig::Aws(_) => "aws",
            StorageConfig::Azure(_) => "azure",
            StorageConfig::Gcp(_) => "gcp",
            StorageConfig::R2(_) => "r2",
            StorageConfig::S3(_) => "s3",
        }
    }

    pub fn content_bucket(&self) -> &str {
        match self {
            StorageConfig::Aws(c) => &c.content_bucket,
            StorageConfig::Azure(c) => &c.content_container,
            StorageConfig::Gcp(c) => &c.content_bucket,
            StorageConfig::R2(c) => &c.content_bucket,
            StorageConfig::S3(c) => &c.content_bucket,
        }
    }

    pub fn meta_bucket(&self) -> &str {
        match self {
            StorageConfig::Aws(c) => &c.meta_bucket,
            StorageConfig::Azure(c) => &c.meta_container,
            StorageConfig::Gcp(c) => &c.meta_bucket,
            StorageConfig::R2(c) => &c.meta_bucket,
            StorageConfig::S3(c) => &c.meta_bucket,
        }
    }

    pub fn cdn_bucket(&self) -> &str {
        match self {
            StorageConfig::Aws(c) => &c.cdn_bucket,
            StorageConfig::Azure(c) => &c.cdn_container,
            StorageConfig::Gcp(c) => &c.cdn_bucket,
            StorageConfig::R2(c) => &c.cdn_bucket,
            StorageConfig::S3(c) => &c.cdn_bucket,
        }
    }
}

/// Resolve the meta bucket name.
///
/// Precedence: `META_BUCKET` env → provider-specific env → `CONTENT_BUCKET` →
/// provider-specific content env → `"flow-like-content"` (same default as content).
///
/// This ensures single-bucket setups work: when only `CONTENT_BUCKET` is set,
/// meta automatically falls back to it.
fn meta_bucket_with_fallback(provider_meta_var: &str) -> String {
    env::var("META_BUCKET")
        .or_else(|_| env::var(provider_meta_var))
        .or_else(|_| env::var("CONTENT_BUCKET"))
        .unwrap_or_else(|_| "flow-like-content".to_string())
}

fn cdn_bucket_with_fallback(provider_cdn_var: &str) -> String {
    env::var("CDN_BUCKET_NAME")
        .or_else(|_| env::var(provider_cdn_var))
        .or_else(|_| env::var("CONTENT_BUCKET"))
        .unwrap_or_else(|_| "flow-like-content".to_string())
}

impl Config {
    pub fn storage_provider(&self) -> &'static str {
        self.storage_config.provider_name()
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let storage_config = Self::load_storage_config()?;

        Ok(Config {
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .map_err(|_| ConfigError::InvalidValue("PORT"))?,
            storage_config,
        })
    }

    fn load_storage_config() -> Result<StorageConfig, ConfigError> {
        let provider = env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "s3".to_string());

        match provider.to_lowercase().as_str() {
            "aws" => Ok(StorageConfig::Aws(AwsStorageConfig {
                access_key_id: env::var("AWS_ACCESS_KEY_ID").ok(),
                secret_access_key: env::var("AWS_SECRET_ACCESS_KEY").ok(),
                region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
                endpoint: env::var("AWS_ENDPOINT").ok(),
                content_bucket: env::var("CONTENT_BUCKET")
                    .or_else(|_| env::var("AWS_CONTENT_BUCKET"))
                    .unwrap_or_else(|_| "flow-like-content".to_string()),
                cdn_bucket: cdn_bucket_with_fallback("AWS_CDN_BUCKET"),
                meta_bucket: meta_bucket_with_fallback("AWS_META_BUCKET"),
                use_path_style: env::var("AWS_USE_PATH_STYLE")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false),
            })),

            "r2" => {
                let account_id = env::var("R2_ACCOUNT_ID")
                    .map_err(|_| ConfigError::MissingVar("R2_ACCOUNT_ID"))?;
                Ok(StorageConfig::R2(R2StorageConfig {
                    account_id,
                    access_key_id: env::var("R2_ACCESS_KEY_ID")
                        .map_err(|_| ConfigError::MissingVar("R2_ACCESS_KEY_ID"))?,
                    secret_access_key: env::var("R2_SECRET_ACCESS_KEY")
                        .map_err(|_| ConfigError::MissingVar("R2_SECRET_ACCESS_KEY"))?,
                    content_bucket: env::var("CONTENT_BUCKET")
                        .or_else(|_| env::var("R2_CONTENT_BUCKET"))
                        .unwrap_or_else(|_| "flow-like-content".to_string()),
                    cdn_bucket: cdn_bucket_with_fallback("R2_CDN_BUCKET"),
                    meta_bucket: meta_bucket_with_fallback("R2_META_BUCKET"),
                }))
            }

            "azure" => {
                let account_name = env::var("AZURE_STORAGE_ACCOUNT_NAME")
                    .map_err(|_| ConfigError::MissingVar("AZURE_STORAGE_ACCOUNT_NAME"))?;
                Ok(StorageConfig::Azure(AzureStorageConfig {
                    account_name,
                    account_key: env::var("AZURE_STORAGE_ACCOUNT_KEY").ok(),
                    content_container: env::var("CONTENT_BUCKET")
                        .or_else(|_| env::var("AZURE_CONTENT_CONTAINER"))
                        .unwrap_or_else(|_| "flow-like-content".to_string()),
                    cdn_container: cdn_bucket_with_fallback("AZURE_CDN_CONTAINER"),
                    meta_container: meta_bucket_with_fallback("AZURE_META_CONTAINER"),
                }))
            }

            "gcp" => {
                let project_id = env::var("GCP_PROJECT_ID")
                    .map_err(|_| ConfigError::MissingVar("GCP_PROJECT_ID"))?;
                Ok(StorageConfig::Gcp(GcpStorageConfig {
                    project_id,
                    service_account_key: env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON").ok(),
                    content_bucket: env::var("CONTENT_BUCKET")
                        .or_else(|_| env::var("GCP_CONTENT_BUCKET"))
                        .unwrap_or_else(|_| "flow-like-content".to_string()),
                    cdn_bucket: cdn_bucket_with_fallback("GCP_CDN_BUCKET"),
                    meta_bucket: meta_bucket_with_fallback("GCP_META_BUCKET"),
                }))
            }

            // Default: S3-compatible (MinIO, etc.)
            "s3" | _ => Ok(StorageConfig::S3(S3StorageConfig {
                endpoint: env::var("S3_ENDPOINT")
                    .map_err(|_| ConfigError::MissingVar("S3_ENDPOINT"))?,
                region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
                access_key_id: env::var("S3_ACCESS_KEY_ID")
                    .or_else(|_| env::var("AWS_ACCESS_KEY_ID"))
                    .map_err(|_| ConfigError::MissingVar("S3_ACCESS_KEY_ID"))?,
                secret_access_key: env::var("S3_SECRET_ACCESS_KEY")
                    .or_else(|_| env::var("AWS_SECRET_ACCESS_KEY"))
                    .map_err(|_| ConfigError::MissingVar("S3_SECRET_ACCESS_KEY"))?,
                content_bucket: env::var("CONTENT_BUCKET")
                    .unwrap_or_else(|_| "flow-like-content".to_string()),
                cdn_bucket: cdn_bucket_with_fallback("S3_CDN_BUCKET"),
                meta_bucket: meta_bucket_with_fallback("S3_META_BUCKET"),
                use_path_style: env::var("S3_USE_PATH_STYLE")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(true),
            })),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar(&'static str),
    InvalidValue(&'static str),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingVar(var) => write!(f, "Missing environment variable: {}", var),
            ConfigError::InvalidValue(var) => write!(f, "Invalid value for: {}", var),
        }
    }
}

impl std::error::Error for ConfigError {}

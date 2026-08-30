use flow_like_api::storage_config::{BucketConfig, StorageConfig, StorageProvider};
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub storage_config: StorageConfig,
    pub bucket_config: BucketConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let provider: StorageProvider = env::var("STORAGE_PROVIDER")
            .unwrap_or_else(|_| "aws".to_string())
            .parse()
            .map_err(|e| ConfigError::InvalidValue(format!("STORAGE_PROVIDER: {}", e)))?;

        let storage_config = StorageConfig::from_env_with_provider(provider.clone())
            .map_err(|e| ConfigError::Storage(e.to_string()))?;

        let bucket_config =
            BucketConfig::from_env(&provider).map_err(|e| ConfigError::Storage(e.to_string()))?;

        Ok(Config {
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .map_err(|_| ConfigError::InvalidValue("PORT".to_string()))?,
            storage_config,
            bucket_config,
        })
    }

    pub fn provider(&self) -> StorageProvider {
        self.storage_config.provider()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    #[allow(dead_code)] // retained for config loaders that fail on a missing env var
    MissingVar(&'static str),
    InvalidValue(String),
    Storage(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingVar(var) => write!(f, "Missing environment variable: {}", var),
            ConfigError::InvalidValue(var) => write!(f, "Invalid value for: {}", var),
            ConfigError::Storage(msg) => write!(f, "Storage config error: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

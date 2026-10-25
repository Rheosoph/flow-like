mod env;
mod file;

#[cfg(feature = "aws")]
mod aws;
#[cfg(feature = "azure")]
mod azure;
#[cfg(feature = "gcp")]
mod gcp;

use crate::config::ProviderConfig;
use crate::error::Result;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

pub use env::EnvSecretProvider;
pub use file::FileSecretProvider;

#[cfg(feature = "aws")]
pub use aws::{AwsParameterStoreProvider, AwsSecretsManagerProvider};
#[cfg(feature = "azure")]
pub use azure::AzureKeyVaultProvider;
#[cfg(feature = "gcp")]
pub use gcp::GcpSecretManagerProvider;

#[async_trait]
pub trait SecretProvider: Send + Sync {
    fn kind(&self) -> SecretProviderKind;
    async fn get(&self, reference: &SecretRef) -> Result<SecretValue>;

    /// Batch-fetch all secrets under the configured prefix.
    /// Providers that support bulk listing (e.g. SSM `GetParametersByPath`)
    /// override this to reduce cold-start API calls.
    /// Returns `(bare_key, value)` pairs with the prefix stripped.
    async fn prefetch(&self) -> Result<Vec<(String, SecretValue)>> {
        Ok(Vec::new())
    }
}

/// `negative_cache_ttl` is how long the store trusts a miss; providers that list their
/// secrets trust a listing's misses no longer.
pub(crate) async fn build_provider(
    config: &ProviderConfig,
    negative_cache_ttl: Duration,
) -> Result<Arc<dyn SecretProvider>> {
    match config {
        ProviderConfig::Env(config) => Ok(Arc::new(EnvSecretProvider::new(config.clone()))),
        ProviderConfig::File(config) => Ok(Arc::new(FileSecretProvider::new(config.clone()))),
        ProviderConfig::AwsParameterStore(config) => {
            #[cfg(feature = "aws")]
            {
                Ok(Arc::new(
                    AwsParameterStoreProvider::new(config.clone(), negative_cache_ttl).await?,
                ))
            }
            #[cfg(not(feature = "aws"))]
            {
                let _ = (config, negative_cache_ttl);
                Err(crate::SecretError::ProviderDisabled(
                    SecretProviderKind::AwsParameterStore,
                ))
            }
        }
        ProviderConfig::AwsSecretsManager(config) => {
            #[cfg(feature = "aws")]
            {
                Ok(Arc::new(
                    AwsSecretsManagerProvider::new(config.clone()).await?,
                ))
            }
            #[cfg(not(feature = "aws"))]
            {
                let _ = config;
                Err(crate::SecretError::ProviderDisabled(
                    SecretProviderKind::AwsSecretsManager,
                ))
            }
        }
        ProviderConfig::GcpSecretManager(config) => {
            #[cfg(feature = "gcp")]
            {
                Ok(Arc::new(
                    GcpSecretManagerProvider::new(config.clone()).await?,
                ))
            }
            #[cfg(not(feature = "gcp"))]
            {
                let _ = config;
                Err(crate::SecretError::ProviderDisabled(
                    SecretProviderKind::GcpSecretManager,
                ))
            }
        }
        ProviderConfig::AzureKeyVault(config) => {
            #[cfg(feature = "azure")]
            {
                Ok(Arc::new(AzureKeyVaultProvider::new(config.clone())?))
            }
            #[cfg(not(feature = "azure"))]
            {
                let _ = config;
                Err(crate::SecretError::ProviderDisabled(
                    SecretProviderKind::AzureKeyVault,
                ))
            }
        }
    }
}

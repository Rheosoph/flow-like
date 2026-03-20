mod env;
mod file;

#[cfg(feature = "aws")]
mod aws;
#[cfg(feature = "azure")]
mod azure;
#[cfg(feature = "gcp")]
mod gcp;

use crate::config::ProviderConfig;
use crate::error::{Result, SecretError};
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use std::sync::Arc;

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
}

pub(crate) async fn build_provider(config: &ProviderConfig) -> Result<Arc<dyn SecretProvider>> {
    match config {
        ProviderConfig::Env(config) => Ok(Arc::new(EnvSecretProvider::new(config.clone()))),
        ProviderConfig::File(config) => Ok(Arc::new(FileSecretProvider::new(config.clone()))),
        ProviderConfig::AwsParameterStore(config) => {
            #[cfg(feature = "aws")]
            {
                return Ok(Arc::new(
                    AwsParameterStoreProvider::new(config.clone()).await?,
                ));
            }
            #[cfg(not(feature = "aws"))]
            {
                let _ = config;
                Err(SecretError::ProviderDisabled(
                    SecretProviderKind::AwsParameterStore,
                ))
            }
        }
        ProviderConfig::AwsSecretsManager(config) => {
            #[cfg(feature = "aws")]
            {
                return Ok(Arc::new(
                    AwsSecretsManagerProvider::new(config.clone()).await?,
                ));
            }
            #[cfg(not(feature = "aws"))]
            {
                let _ = config;
                Err(SecretError::ProviderDisabled(
                    SecretProviderKind::AwsSecretsManager,
                ))
            }
        }
        ProviderConfig::GcpSecretManager(config) => {
            #[cfg(feature = "gcp")]
            {
                return Ok(Arc::new(
                    GcpSecretManagerProvider::new(config.clone()).await?,
                ));
            }
            #[cfg(not(feature = "gcp"))]
            {
                let _ = config;
                Err(SecretError::ProviderDisabled(
                    SecretProviderKind::GcpSecretManager,
                ))
            }
        }
        ProviderConfig::AzureKeyVault(config) => {
            #[cfg(feature = "azure")]
            {
                return Ok(Arc::new(AzureKeyVaultProvider::new(config.clone())?));
            }
            #[cfg(not(feature = "azure"))]
            {
                let _ = config;
                Err(SecretError::ProviderDisabled(
                    SecretProviderKind::AzureKeyVault,
                ))
            }
        }
    }
}

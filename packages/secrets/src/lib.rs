#![forbid(unsafe_code)]

mod cache;
mod config;
mod error;
mod manager;
pub mod providers;
mod reference;
mod value;

pub use config::{
    AwsParameterStoreProviderConfig, AwsSecretsManagerProviderConfig,
    AzureClientSecretCredentialConfig, AzureCredentialConfig, AzureKeyVaultProviderConfig,
    AzureManagedIdentityConfig, AzureManagedIdentityId, EnvProviderConfig, FileProviderConfig,
    GcpSecretManagerProviderConfig, ProviderConfig, SecretStoreConfig,
};
pub use error::{Result, SecretError};
pub use manager::SecretStore;
pub use reference::{SecretProviderKind, SecretRef};
pub use secrecy::{ExposeSecret, SecretBox, SecretString};
pub use value::SecretValue;

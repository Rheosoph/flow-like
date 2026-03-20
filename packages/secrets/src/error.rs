use crate::reference::SecretProviderKind;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, SecretError>;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SecretError {
    #[error("invalid secret reference")]
    InvalidReference,
    #[error("no secret providers configured")]
    NoProvidersConfigured,
    #[error("provider `{0}` is not configured")]
    ProviderNotConfigured(SecretProviderKind),
    #[error("provider `{0}` is disabled at compile time")]
    ProviderDisabled(SecretProviderKind),
    #[error("secret not found in provider `{0}`")]
    SecretNotFound(SecretProviderKind),
    #[error("provider `{provider}` request failed: {message}")]
    ProviderFailure {
        provider: SecretProviderKind,
        message: String,
    },
    #[error("requested text secret, but provider returned binary data")]
    SecretValueBinary,
    #[error("requested binary secret, but provider returned text data")]
    SecretValueText,
    #[error("max_cache_entries must be at least 1")]
    InvalidCacheCapacity,
}

impl SecretError {
    pub fn provider_failure(provider: SecretProviderKind, message: impl Into<String>) -> Self {
        Self::ProviderFailure {
            provider,
            message: message.into(),
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::SecretNotFound(_))
    }
}

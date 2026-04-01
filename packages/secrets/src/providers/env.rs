use crate::config::EnvProviderConfig;
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;

pub struct EnvSecretProvider {
    config: EnvProviderConfig,
}

impl EnvSecretProvider {
    pub fn new(config: EnvProviderConfig) -> Self {
        Self { config }
    }

    fn resolve_key(&self, reference: &SecretRef) -> String {
        if let Some(prefix) = &self.config.prefix {
            return format!("{}_{}", prefix.trim_end_matches('_'), reference.key);
        }

        reference.key.clone()
    }
}

#[async_trait]
impl SecretProvider for EnvSecretProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::Env
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let key = self.resolve_key(reference);
        let value = std::env::var(&key).map_err(|_| SecretError::SecretNotFound(self.kind()))?;

        Ok(SecretValue::from_string(value))
    }
}

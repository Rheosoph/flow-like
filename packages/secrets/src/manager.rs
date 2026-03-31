use crate::cache::{CacheKey, SecretCache};
use crate::config::{ProviderConfig, SecretStoreConfig};
use crate::error::{Result, SecretError};
use crate::providers::{self, SecretProvider};
use crate::{SecretProviderKind, SecretRef, SecretString, SecretValue};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);

struct ProviderSlot {
    config: ProviderConfig,
    provider: OnceCell<Arc<dyn SecretProvider>>,
}

impl ProviderSlot {
    fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            provider: OnceCell::new(),
        }
    }

    async fn get_or_init(&self) -> Result<Arc<dyn SecretProvider>> {
        let provider = self
            .provider
            .get_or_try_init(|| async { providers::build_provider(&self.config).await })
            .await?;

        Ok(Arc::clone(provider))
    }
}

pub struct SecretStore {
    cache: SecretCache,
    provider_order: Vec<SecretProviderKind>,
    providers: HashMap<SecretProviderKind, ProviderSlot>,
}

impl SecretStore {
    pub fn new(config: SecretStoreConfig) -> Result<Self> {
        if config.max_cache_entries == 0 {
            return Err(SecretError::InvalidCacheCapacity);
        }

        let global_prefix = config.global_prefix.clone();
        let mut seen = HashSet::new();
        let mut provider_order = Vec::with_capacity(config.providers.len());
        let mut providers = HashMap::with_capacity(config.providers.len());

        for provider_config in config.providers {
            let provider_config = apply_global_prefix(provider_config, global_prefix.as_deref());
            let kind = provider_config.kind();
            if !seen.insert(kind) {
                continue;
            }

            provider_order.push(kind);
            providers.insert(kind, ProviderSlot::new(provider_config));
        }

        Ok(Self {
            cache: SecretCache::new(
                config.cache_ttl,
                config.negative_cache_ttl,
                config.max_cache_entries,
            ),
            provider_order,
            providers,
        })
    }

    pub async fn get_secret(&self, reference: &SecretRef) -> Result<SecretValue> {
        if let Some(value) = env_override(reference) {
            return Ok(value);
        }

        if self.providers.is_empty() {
            return Err(SecretError::NoProvidersConfigured);
        }

        let cache_key = CacheKey::from(reference);
        if let Some(cached) = self.cache.get(&cache_key).await {
            return cached;
        }

        let resolved = match reference.provider {
            Some(kind) => {
                let slot = self.provider_for(kind)?;
                let provider = slot.get_or_init().await?;
                Self::get_with_retry(&provider, reference).await
            }
            None => self.resolve_with_fallback(reference).await,
        };

        match resolved {
            Ok(value) => {
                self.cache.insert_success(cache_key, value.clone()).await;
                Ok(value)
            }
            Err(error) => {
                if error.is_not_found() {
                    self.cache.insert_failure(cache_key, error.clone()).await;
                }
                Err(error)
            }
        }
    }

    pub async fn get_secret_by_ref_str(&self, reference: &str) -> Result<SecretValue> {
        let reference = SecretRef::try_from(reference)?;
        self.get_secret(&reference).await
    }

    pub async fn get_secret_string(&self, reference: &SecretRef) -> Result<Arc<SecretString>> {
        self.get_secret(reference).await?.as_text()
    }

    pub async fn get_secret_string_by_ref_str(&self, reference: &str) -> Result<Arc<SecretString>> {
        self.get_secret_by_ref_str(reference).await?.as_text()
    }

    pub async fn invalidate(&self, reference: &SecretRef) {
        self.cache.invalidate(&CacheKey::from(reference)).await;
    }

    /// Pre-fetch all secrets from providers that support bulk listing
    /// (e.g. AWS SSM `GetParametersByPath`). Populates the cache so that
    /// subsequent [`get_secret`] calls are instant cache hits.
    pub async fn warmup(&self) {
        for kind in &self.provider_order {
            let slot = match self.provider_for(*kind) {
                Ok(slot) => slot,
                Err(_) => continue,
            };
            let provider = match slot.get_or_init().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(provider = %kind, error = %e, "skipping warmup for provider");
                    continue;
                }
            };

            match provider.prefetch().await {
                Ok(entries) => {
                    for (key, value) in entries {
                        let cache_key = CacheKey {
                            provider: None,
                            key,
                            version: None,
                        };
                        self.cache.insert_success(cache_key, value).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(provider = %kind, error = %e, "prefetch failed, falling back to individual gets");
                }
            }
        }
    }

    fn provider_for(&self, kind: SecretProviderKind) -> Result<&ProviderSlot> {
        self.providers
            .get(&kind)
            .ok_or(SecretError::ProviderNotConfigured(kind))
    }

    async fn get_with_retry(
        provider: &Arc<dyn SecretProvider>,
        reference: &SecretRef,
    ) -> Result<SecretValue> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match provider.get(reference).await {
                Ok(value) => return Ok(value),
                Err(error) if error.is_retryable() && attempt + 1 < MAX_RETRIES => {
                    let backoff = INITIAL_BACKOFF * 2u32.saturating_pow(attempt);
                    tracing::warn!(
                        provider = %provider.kind(),
                        key = %reference.key,
                        attempt = attempt + 1,
                        backoff_ms = backoff.as_millis() as u64,
                        error = %error,
                        "transient provider failure, retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or(SecretError::SecretNotFound(provider.kind())))
    }

    async fn resolve_with_fallback(&self, reference: &SecretRef) -> Result<SecretValue> {
        let mut last_error = None;

        for kind in &self.provider_order {
            let provider = self.provider_for(*kind)?;
            let provider = provider.get_or_init().await?;
            match Self::get_with_retry(&provider, reference).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if !error.is_not_found() {
                        tracing::warn!(
                            provider = %kind,
                            key = %reference.key,
                            error = %error,
                            "secret provider failed after retries, falling through to next provider"
                        );
                    }
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or(SecretError::SecretNotFound(SecretProviderKind::Env)))
    }
}

fn env_override(reference: &SecretRef) -> Option<SecretValue> {
    std::env::var(&reference.key)
        .ok()
        .map(SecretValue::from_string)
}

fn apply_global_prefix(config: ProviderConfig, global_prefix: Option<&str>) -> ProviderConfig {
    let Some(global_prefix) = global_prefix.filter(|prefix| !prefix.is_empty()) else {
        return config;
    };

    match config {
        ProviderConfig::AwsParameterStore(mut config) => {
            if config.prefix.is_none() {
                config.prefix = Some(global_prefix.to_string());
            }
            ProviderConfig::AwsParameterStore(config)
        }
        ProviderConfig::AwsSecretsManager(mut config) => {
            if config.prefix.is_none() {
                config.prefix = Some(global_prefix.to_string());
            }
            ProviderConfig::AwsSecretsManager(config)
        }
        ProviderConfig::GcpSecretManager(mut config) => {
            if config.prefix.is_none() {
                config.prefix = Some(global_prefix.to_string());
            }
            ProviderConfig::GcpSecretManager(config)
        }
        ProviderConfig::AzureKeyVault(mut config) => {
            if config.prefix.is_none() {
                config.prefix = Some(global_prefix.to_string());
            }
            ProviderConfig::AzureKeyVault(config)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AwsParameterStoreProviderConfig, AzureKeyVaultProviderConfig, EnvProviderConfig,
        GcpSecretManagerProviderConfig, ProviderConfig, SecretStoreConfig,
    };
    use secrecy::ExposeSecret;

    fn must_ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    #[tokio::test]
    async fn returns_not_configured_for_missing_provider() {
        let store = must_ok(
            SecretStore::new(
                SecretStoreConfig::default()
                    .with_provider(ProviderConfig::Env(EnvProviderConfig::default())),
            ),
            "must create secret store",
        );

        let reference = SecretRef::with_provider(SecretProviderKind::GcpSecretManager, "not-used");

        let err = match store.get_secret(&reference).await {
            Ok(_) => panic!("must fail"),
            Err(error) => error,
        };

        assert_eq!(
            err,
            SecretError::ProviderNotConfigured(SecretProviderKind::GcpSecretManager)
        );
    }

    #[tokio::test]
    async fn env_override_works_without_configured_providers() {
        let expected = must_ok(std::env::var("PATH"), "PATH must be set for test");
        let store = must_ok(
            SecretStore::new(SecretStoreConfig::default()),
            "must create secret store",
        );

        let value = must_ok(
            store.get_secret(&SecretRef::new("PATH")).await,
            "must read PATH",
        );
        let text = must_ok(value.as_text(), "PATH must resolve as text");

        assert_eq!(text.expose_secret(), expected);
    }

    #[test]
    fn applies_global_prefix_to_prefix_capable_providers() {
        let provider = apply_global_prefix(
            ProviderConfig::AwsParameterStore(AwsParameterStoreProviderConfig::default()),
            Some("/flow-like/dev/"),
        );

        match provider {
            ProviderConfig::AwsParameterStore(config) => {
                assert_eq!(config.prefix.as_deref(), Some("/flow-like/dev/"));
            }
            _ => panic!("must remain aws parameter store config"),
        }
    }

    #[test]
    fn preserves_explicit_provider_prefix_over_global_prefix() {
        let provider = apply_global_prefix(
            ProviderConfig::GcpSecretManager(GcpSecretManagerProviderConfig {
                project_id: None,
                endpoint: "https://secretmanager.googleapis.com".to_string(),
                prefix: Some("custom".to_string()),
            }),
            Some("/flow-like/dev/"),
        );

        match provider {
            ProviderConfig::GcpSecretManager(config) => {
                assert_eq!(config.prefix.as_deref(), Some("custom"));
            }
            _ => panic!("must remain gcp secret manager config"),
        }
    }

    #[test]
    fn ignores_global_prefix_for_non_prefix_capable_providers() {
        let provider = apply_global_prefix(
            ProviderConfig::Env(EnvProviderConfig::default()),
            Some("/flow-like/dev/"),
        );

        match provider {
            ProviderConfig::Env(config) => assert_eq!(config.prefix, None),
            _ => panic!("must remain env config"),
        }
    }

    #[test]
    fn applies_global_prefix_to_azure_when_missing() {
        let provider = apply_global_prefix(
            ProviderConfig::AzureKeyVault(AzureKeyVaultProviderConfig::default()),
            Some("/flow-like/dev/"),
        );

        match provider {
            ProviderConfig::AzureKeyVault(config) => {
                assert_eq!(config.prefix.as_deref(), Some("/flow-like/dev/"));
            }
            _ => panic!("must remain azure key vault config"),
        }
    }
}

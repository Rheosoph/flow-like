use crate::config::{AzureCredentialConfig, AzureKeyVaultProviderConfig, AzureManagedIdentityId};
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use azure_core::credentials::{Secret as AzureSecret, TokenCredential};
use azure_core::http::StatusCode;
use azure_identity::{
    ClientSecretCredential, DeveloperToolsCredential, ManagedIdentityCredential,
    ManagedIdentityCredentialOptions, UserAssignedId,
};
use azure_security_keyvault_secrets::{
    SecretClient, SecretClientOptions, models::SecretClientGetSecretOptions,
};
use std::sync::Arc;

pub struct AzureKeyVaultProvider {
    config: AzureKeyVaultProviderConfig,
    client: SecretClient,
}

impl AzureKeyVaultProvider {
    pub fn new(config: AzureKeyVaultProviderConfig) -> Result<Self> {
        let credential = build_credential(&config.credential)?;
        let vault_url = config.vault_url.clone();
        let api_version = config.api_version.clone();
        let verify_challenge_resource = config.verify_challenge_resource;
        let client = SecretClient::new(
            &vault_url,
            credential,
            Some(SecretClientOptions {
                api_version,
                verify_challenge_resource: Some(verify_challenge_resource),
                ..Default::default()
            }),
        )
        .map_err(|error| {
            SecretError::provider_failure(SecretProviderKind::AzureKeyVault, error.to_string())
        })?;

        Ok(Self { config, client })
    }

    fn map_error(&self, error: azure_core::Error) -> SecretError {
        if error.http_status() == Some(StatusCode::NotFound) {
            return SecretError::SecretNotFound(self.kind());
        }

        SecretError::provider_failure(self.kind(), error.to_string())
    }

    fn secret_names(&self, reference: &SecretRef) -> Vec<String> {
        candidate_names(
            &self.config.prefix,
            &reference.key,
            "-",
            self.config.fallback_to_unprefixed,
        )
    }
}

fn build_credential(config: &AzureCredentialConfig) -> Result<Arc<dyn TokenCredential>> {
    match config {
        AzureCredentialConfig::DeveloperTools => DeveloperToolsCredential::new(None)
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| {
                SecretError::provider_failure(SecretProviderKind::AzureKeyVault, error.to_string())
            }),
        AzureCredentialConfig::ManagedIdentity(config) => {
            let user_assigned_id = config.user_assigned_id.as_ref().map(|id| match id {
                AzureManagedIdentityId::ClientId(value) => UserAssignedId::ClientId(value.clone()),
                AzureManagedIdentityId::ObjectId(value) => UserAssignedId::ObjectId(value.clone()),
                AzureManagedIdentityId::ResourceId(value) => {
                    UserAssignedId::ResourceId(value.clone())
                }
            });

            ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
                user_assigned_id,
                ..Default::default()
            }))
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| {
                SecretError::provider_failure(SecretProviderKind::AzureKeyVault, error.to_string())
            })
        }
        AzureCredentialConfig::ClientSecret(config) => {
            let tenant_id = read_env_var(
                SecretProviderKind::AzureKeyVault,
                &config.tenant_id_env,
                "tenant ID",
            )?;
            let client_id = read_env_var(
                SecretProviderKind::AzureKeyVault,
                &config.client_id_env,
                "client ID",
            )?;
            let client_secret = read_env_var(
                SecretProviderKind::AzureKeyVault,
                &config.client_secret_env,
                "client secret",
            )?;

            ClientSecretCredential::new(
                &tenant_id,
                client_id,
                AzureSecret::new(client_secret),
                None,
            )
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| {
                SecretError::provider_failure(SecretProviderKind::AzureKeyVault, error.to_string())
            })
        }
    }
}

fn read_env_var(provider: SecretProviderKind, env_key: &str, label: &str) -> Result<String> {
    std::env::var(env_key).map_err(|_| {
        SecretError::provider_failure(provider, format!("missing {label} env: {env_key}"))
    })
}

#[async_trait]
impl SecretProvider for AzureKeyVaultProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::AzureKeyVault
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let secret_names = self.secret_names(reference);
        let last_index = secret_names.len().saturating_sub(1);

        for (index, secret_name) in secret_names.iter().enumerate() {
            let options = SecretClientGetSecretOptions {
                secret_version: reference.version.clone(),
                ..Default::default()
            };

            let secret = self
                .client
                .get_secret(secret_name, Some(options))
                .await
                .map_err(|error| self.map_error(error));

            match secret {
                Ok(secret) => {
                    let secret = secret.into_model().map_err(|error| self.map_error(error))?;
                    let value = secret.value.ok_or_else(|| {
                        SecretError::provider_failure(self.kind(), "secret response missing value")
                    })?;

                    return Ok(SecretValue::from_string(value));
                }
                Err(error) if error.is_not_found() && index < last_index => continue,
                Err(error) => return Err(error),
            }
        }

        Err(SecretError::SecretNotFound(self.kind()))
    }
}

fn candidate_names(
    prefix: &Option<String>,
    key: &str,
    separator: &str,
    fallback_to_unprefixed: bool,
) -> Vec<String> {
    let normalized_key = normalize_key(key, separator);
    let mut candidates = Vec::with_capacity(2);
    let mut has_prefixed_candidate = false;

    if let Some(prefix) = prefix
        && !prefix.is_empty()
    {
        let prefixed = join_prefix(prefix, &normalized_key, separator);
        if prefixed != normalized_key {
            candidates.push(prefixed);
            has_prefixed_candidate = true;
        }
    }

    if fallback_to_unprefixed || !has_prefixed_candidate {
        candidates.push(normalized_key);
    }
    candidates
}

fn normalize_key(key: &str, separator: &str) -> String {
    if separator != "/" {
        // Key Vault secret names only accept alphanumeric characters and
        // hyphens. Flow-Like uses env-style names such as DATABASE_URL, so
        // normalize both path and underscore separators before lookup.
        return key.trim_matches('/').replace(['/', '_'], separator);
    }

    key.to_string()
}

fn join_prefix(prefix: &str, key: &str, separator: &str) -> String {
    if separator != "/" {
        let normalized_prefix = prefix.trim_matches('/').replace(['/', '_'], separator);
        let normalized_key = key.trim_matches('/').replace(['/', '_'], separator);
        let normalized_prefix = normalized_prefix.trim_end_matches(separator);
        let normalized_key = normalized_key.trim_start_matches(separator);

        if normalized_prefix.is_empty() {
            return normalized_key.to_string();
        }

        if normalized_key.is_empty() {
            return normalized_prefix.to_string();
        }

        return format!("{normalized_prefix}{separator}{normalized_key}");
    }

    format!(
        "{}{}{}",
        prefix.trim_end_matches(separator),
        separator,
        key.trim_start_matches(separator)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_prefix_to_secret_name() {
        assert_eq!(
            candidate_names(&Some("flow-like".to_string()), "db-password", "-", true),
            vec![
                "flow-like-db-password".to_string(),
                "db-password".to_string(),
            ]
        );
    }

    #[test]
    fn avoids_duplicate_separator() {
        assert_eq!(
            candidate_names(&Some("flow-like-".to_string()), "-db-password", "-", true,),
            vec![
                "flow-like-db-password".to_string(),
                "-db-password".to_string(),
            ]
        );
    }

    #[test]
    fn normalizes_path_style_prefix() {
        assert_eq!(
            candidate_names(
                &Some("/flow-like/dev/".to_string()),
                "SECRET_NAME",
                "-",
                true,
            ),
            vec![
                "flow-like-dev-SECRET-NAME".to_string(),
                "SECRET-NAME".to_string(),
            ]
        );
    }

    #[test]
    fn skips_duplicate_candidate_when_prefix_is_empty() {
        assert_eq!(
            candidate_names(&Some(String::new()), "SECRET_NAME", "-", true),
            vec!["SECRET-NAME".to_string()]
        );
    }

    #[test]
    fn normalizes_unprefixed_path_style_keys() {
        assert_eq!(
            candidate_names(&None, "/flow-like/dev/SECRET_NAME", "-", true),
            vec!["flow-like-dev-SECRET-NAME".to_string()]
        );
    }

    #[test]
    fn can_disable_unprefixed_fallback() {
        assert_eq!(
            candidate_names(
                &Some("/flow-like/dev/".to_string()),
                "SECRET_NAME",
                "-",
                false,
            ),
            vec!["flow-like-dev-SECRET-NAME".to_string()]
        );
    }
}

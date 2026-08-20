use crate::config::GcpSecretManagerProviderConfig;
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use google_cloud_secretmanager_v1::client::SecretManagerService;

pub struct GcpSecretManagerProvider {
    config: GcpSecretManagerProviderConfig,
    client: SecretManagerService,
}

impl GcpSecretManagerProvider {
    pub async fn new(config: GcpSecretManagerProviderConfig) -> Result<Self> {
        let client = SecretManagerService::builder()
            .with_endpoint(config.endpoint.trim_end_matches('/'))
            .build()
            .await
            .map_err(|error| {
                SecretError::provider_failure(
                    SecretProviderKind::GcpSecretManager,
                    error.to_string(),
                )
            })?;

        Ok(Self { config, client })
    }

    fn project_id(&self) -> Result<String> {
        if let Some(project_id) = &self.config.project_id
            && !project_id.is_empty()
        {
            return Ok(project_id.clone());
        }

        for env_key in ["GCP_PROJECT_ID", "GOOGLE_CLOUD_PROJECT"] {
            if let Ok(project_id) = std::env::var(env_key)
                && !project_id.is_empty()
            {
                return Ok(project_id);
            }
        }

        Err(SecretError::provider_failure(
            SecretProviderKind::GcpSecretManager,
            "missing project_id (config.project_id, GCP_PROJECT_ID, or GOOGLE_CLOUD_PROJECT)",
        ))
    }

    fn resource_names(&self, reference: &SecretRef) -> Result<Vec<String>> {
        let version = reference.version.as_deref().unwrap_or("latest");

        if reference.key.starts_with("projects/") {
            if reference.key.contains("/versions/") {
                return Ok(vec![reference.key.clone()]);
            }

            return Ok(vec![format!("{}/versions/{version}", reference.key)]);
        }

        let project_id = self.project_id()?;
        Ok(candidate_names(&self.config.prefix, &reference.key, "-")
            .into_iter()
            .map(|secret_name| {
                format!("projects/{project_id}/secrets/{secret_name}/versions/{version}")
            })
            .collect())
    }

    fn map_error(&self, error: google_cloud_secretmanager_v1::Error) -> SecretError {
        if error
            .status()
            .is_some_and(|status| status.code.name() == "NOT_FOUND")
        {
            return SecretError::SecretNotFound(self.kind());
        }

        SecretError::provider_failure(self.kind(), error.to_string())
    }
}

fn candidate_names(prefix: &Option<String>, key: &str, separator: &str) -> Vec<String> {
    let normalized_key = normalize_key(key, separator);
    let mut candidates = Vec::with_capacity(2);

    if let Some(prefix) = prefix
        && !prefix.is_empty()
    {
        let prefixed = join_prefix(prefix, &normalized_key, separator);
        if prefixed != normalized_key {
            candidates.push(prefixed);
        }
    }

    candidates.push(normalized_key);
    candidates
}

fn normalize_key(key: &str, separator: &str) -> String {
    if separator != "/" && key.contains('/') {
        return key.trim_matches('/').replace('/', separator);
    }

    key.to_string()
}

fn join_prefix(prefix: &str, key: &str, separator: &str) -> String {
    if separator != "/" && (prefix.contains('/') || key.contains('/')) {
        let normalized_prefix = prefix.trim_matches('/').replace('/', separator);
        let normalized_key = key.trim_matches('/').replace('/', separator);

        if normalized_prefix.is_empty() {
            return normalized_key;
        }

        if normalized_key.is_empty() {
            return normalized_prefix;
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

#[async_trait]
impl SecretProvider for GcpSecretManagerProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::GcpSecretManager
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let resource_names = self.resource_names(reference)?;
        let last_index = resource_names.len().saturating_sub(1);

        for (index, resource_name) in resource_names.iter().enumerate() {
            let response = self
                .client
                .access_secret_version()
                .set_name(resource_name)
                .send()
                .await
                .map_err(|error| self.map_error(error));

            match response {
                Ok(response) => {
                    let payload = response.payload.ok_or_else(|| {
                        SecretError::provider_failure(
                            self.kind(),
                            "secret version response missing payload",
                        )
                    })?;

                    let bytes = payload.data.to_vec();
                    return match String::from_utf8(bytes) {
                        Ok(value) => Ok(SecretValue::from_string(value)),
                        Err(error) => Ok(SecretValue::from_bytes(error.into_bytes())),
                    };
                }
                Err(error) if error.is_not_found() && index < last_index => continue,
                Err(error) => return Err(error),
            }
        }

        Err(SecretError::SecretNotFound(self.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_prefix_to_relative_secret_ids() {
        assert_eq!(
            candidate_names(&Some("flow-like".to_string()), "openai-api-key", "-"),
            vec![
                "flow-like-openai-api-key".to_string(),
                "openai-api-key".to_string(),
            ]
        );
    }

    #[test]
    fn avoids_duplicate_separator() {
        assert_eq!(
            candidate_names(&Some("flow-like-".to_string()), "-openai-api-key", "-"),
            vec![
                "flow-like-openai-api-key".to_string(),
                "-openai-api-key".to_string(),
            ]
        );
    }

    #[test]
    fn normalizes_path_style_prefix() {
        assert_eq!(
            candidate_names(&Some("/flow-like/dev/".to_string()), "SECRET_NAME", "-"),
            vec![
                "flow-like-dev-SECRET_NAME".to_string(),
                "SECRET_NAME".to_string(),
            ]
        );
    }

    #[test]
    fn skips_duplicate_candidate_when_prefix_is_empty() {
        assert_eq!(
            candidate_names(&Some(String::new()), "SECRET_NAME", "-"),
            vec!["SECRET_NAME".to_string()]
        );
    }

    #[test]
    fn normalizes_unprefixed_path_style_keys() {
        assert_eq!(
            candidate_names(&None, "/flow-like/dev/SECRET_NAME", "-"),
            vec!["flow-like-dev-SECRET_NAME".to_string()]
        );
    }
}

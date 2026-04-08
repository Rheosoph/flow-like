use crate::config::{AwsParameterStoreProviderConfig, AwsSecretsManagerProviderConfig};
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};

type GetParameterSdkError =
    aws_sdk_ssm::error::SdkError<aws_sdk_ssm::operation::get_parameter::GetParameterError>;
type GetParametersByPathSdkError = aws_sdk_ssm::error::SdkError<
    aws_sdk_ssm::operation::get_parameters_by_path::GetParametersByPathError,
>;
type GetSecretValueSdkError = aws_sdk_secretsmanager::error::SdkError<
    aws_sdk_secretsmanager::operation::get_secret_value::GetSecretValueError,
>;

pub struct AwsParameterStoreProvider {
    config: AwsParameterStoreProviderConfig,
    client: aws_sdk_ssm::Client,
}

impl AwsParameterStoreProvider {
    pub async fn new(config: AwsParameterStoreProviderConfig) -> Result<Self> {
        let sdk_config = load_sdk_config(config.region.clone()).await;
        Ok(Self {
            config,
            client: aws_sdk_ssm::Client::new(&sdk_config),
        })
    }

    fn parameter_name(&self, reference: &SecretRef) -> String {
        let base_name = with_optional_prefix(&self.config.prefix, &reference.key);
        match &reference.version {
            Some(version) => format!("{base_name}:{version}"),
            None => base_name,
        }
    }
}

#[async_trait]
impl SecretProvider for AwsParameterStoreProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::AwsParameterStore
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let output = self
            .client
            .get_parameter()
            .name(self.parameter_name(reference))
            .set_with_decryption(Some(self.config.with_decryption))
            .send()
            .await
            .map_err(|error| map_ssm_error(self.kind(), &error))?;

        let value = output
            .parameter
            .and_then(|parameter| parameter.value)
            .ok_or(SecretError::SecretNotFound(self.kind()))?;

        Ok(SecretValue::from_string(value))
    }

    async fn prefetch(&self) -> Result<Vec<(String, SecretValue)>> {
        let Some(prefix) = &self.config.prefix else {
            return Ok(Vec::new());
        };
        let path = format!("/{}/", prefix.trim_matches('/'));
        let mut results = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut request = self
                .client
                .get_parameters_by_path()
                .path(&path)
                .recursive(true)
                .set_with_decryption(Some(self.config.with_decryption))
                .max_results(10);

            if let Some(token) = next_token.take() {
                request = request.next_token(token);
            }

            let output = request
                .send()
                .await
                .map_err(|error| map_ssm_prefetch_error(self.kind(), &path, &error))?;

            if let Some(parameters) = output.parameters {
                for param in parameters {
                    let Some(name) = param.name else { continue };
                    let Some(value) = param.value else { continue };
                    // Strip the prefix path to get the bare key
                    let bare_key = name.strip_prefix(&path).unwrap_or(&name).to_string();
                    results.push((bare_key, SecretValue::from_string(value)));
                }
            }

            next_token = output.next_token;
            if next_token.is_none() {
                break;
            }
        }

        tracing::info!(
            path = %path,
            count = results.len(),
            "SSM GetParametersByPath prefetch complete"
        );
        Ok(results)
    }
}

pub struct AwsSecretsManagerProvider {
    config: AwsSecretsManagerProviderConfig,
    client: aws_sdk_secretsmanager::Client,
}

impl AwsSecretsManagerProvider {
    pub async fn new(config: AwsSecretsManagerProviderConfig) -> Result<Self> {
        let sdk_config = load_sdk_config(config.region.clone()).await;
        Ok(Self {
            config,
            client: aws_sdk_secretsmanager::Client::new(&sdk_config),
        })
    }

    fn secret_id(&self, reference: &SecretRef) -> String {
        with_optional_prefix(&self.config.prefix, &reference.key)
    }
}

#[async_trait]
impl SecretProvider for AwsSecretsManagerProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::AwsSecretsManager
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let mut request = self
            .client
            .get_secret_value()
            .secret_id(self.secret_id(reference));

        if let Some(version) = &reference.version {
            request = request.version_id(version);
        } else if let Some(stage) = &self.config.version_stage {
            request = request.version_stage(stage);
        }

        let output = request
            .send()
            .await
            .map_err(|error| map_secrets_manager_error(self.kind(), &error))?;

        if let Some(value) = output.secret_string {
            return Ok(SecretValue::from_string(value));
        }

        if let Some(value) = output.secret_binary {
            return Ok(SecretValue::from_bytes(value.into_inner().to_vec()));
        }

        Err(SecretError::SecretNotFound(self.kind()))
    }
}

async fn load_sdk_config(region: Option<String>) -> aws_config::SdkConfig {
    let loader = aws_config::defaults(BehaviorVersion::latest());
    let loader = match region {
        Some(region) if !region.trim().is_empty() => loader.region(Region::new(region)),
        _ => loader,
    };

    loader.load().await
}

fn format_provider_operation_failure(
    operation: &str,
    target: Option<&str>,
    code: Option<&str>,
    message: Option<&str>,
    fallback: &impl std::fmt::Display,
) -> String {
    let context = target.filter(|target| !target.is_empty()).map_or_else(
        || operation.to_string(),
        |target| format!("{operation} for {target}"),
    );

    match (
        code.filter(|code| !code.is_empty()),
        message.filter(|message| !message.is_empty()),
    ) {
        (Some(code), Some(message)) => format!("{context} failed with {code}: {message}"),
        (Some(code), None) => format!("{context} failed with {code}"),
        (None, Some(message)) => format!("{context} failed: {message}"),
        (None, None) => format!("{context} failed: {fallback}"),
    }
}

fn map_ssm_prefetch_error(
    kind: SecretProviderKind,
    path: &str,
    error: &GetParametersByPathSdkError,
) -> SecretError {
    let message = match error.as_service_error() {
        Some(service_error) => format_provider_operation_failure(
            "GetParametersByPath",
            Some(path),
            service_error.meta().code(),
            service_error.meta().message(),
            error,
        ),
        None => {
            format_provider_operation_failure("GetParametersByPath", Some(path), None, None, error)
        }
    };

    SecretError::provider_failure(kind, message)
}

fn map_ssm_error(kind: SecretProviderKind, error: &GetParameterSdkError) -> SecretError {
    if let Some(service_error) = error.as_service_error()
        && (service_error.is_parameter_not_found()
            || service_error.is_parameter_version_not_found())
    {
        return SecretError::SecretNotFound(kind);
    }

    let message = match error.as_service_error() {
        Some(service_error) => format_provider_operation_failure(
            "GetParameter",
            None,
            service_error.meta().code(),
            service_error.meta().message(),
            error,
        ),
        None => format_provider_operation_failure("GetParameter", None, None, None, error),
    };

    SecretError::provider_failure(kind, message)
}

fn map_secrets_manager_error(
    kind: SecretProviderKind,
    error: &GetSecretValueSdkError,
) -> SecretError {
    if let Some(service_error) = error.as_service_error()
        && service_error.is_resource_not_found_exception()
    {
        return SecretError::SecretNotFound(kind);
    }

    let message = match error.as_service_error() {
        Some(service_error) => format_provider_operation_failure(
            "GetSecretValue",
            None,
            service_error.meta().code(),
            service_error.meta().message(),
            error,
        ),
        None => format_provider_operation_failure("GetSecretValue", None, None, None, error),
    };

    SecretError::provider_failure(kind, message)
}

fn with_optional_prefix(prefix: &Option<String>, key: &str) -> String {
    match prefix {
        Some(prefix) if !prefix.is_empty() => format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            key.trim_start_matches('/')
        ),
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_operation_failure_with_code_and_message() {
        let message = format_provider_operation_failure(
            "GetParametersByPath",
            Some("/flow-like/dev/"),
            Some("AccessDeniedException"),
            Some("not authorized to perform ssm:GetParametersByPath"),
            &"service error",
        );

        assert_eq!(
            message,
            "GetParametersByPath for /flow-like/dev/ failed with AccessDeniedException: not authorized to perform ssm:GetParametersByPath"
        );
    }

    #[test]
    fn formats_operation_failure_with_fallback_when_metadata_missing() {
        let message = format_provider_operation_failure(
            "GetParameter",
            None,
            None,
            None,
            &"dispatch failure",
        );

        assert_eq!(message, "GetParameter failed: dispatch failure");
    }
}

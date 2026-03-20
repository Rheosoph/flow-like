use crate::config::{AwsParameterStoreProviderConfig, AwsSecretsManagerProviderConfig};
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};

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

fn map_ssm_error(
    kind: SecretProviderKind,
    error: &aws_sdk_ssm::error::SdkError<aws_sdk_ssm::operation::get_parameter::GetParameterError>,
) -> SecretError {
    if let Some(service_error) = error.as_service_error() {
        if service_error.is_parameter_not_found() || service_error.is_parameter_version_not_found()
        {
            return SecretError::SecretNotFound(kind);
        }
    }

    SecretError::provider_failure(kind, error.to_string())
}

fn map_secrets_manager_error(
    kind: SecretProviderKind,
    error: &aws_sdk_secretsmanager::error::SdkError<
        aws_sdk_secretsmanager::operation::get_secret_value::GetSecretValueError,
    >,
) -> SecretError {
    if let Some(service_error) = error.as_service_error() {
        if service_error.is_resource_not_found_exception() {
            return SecretError::SecretNotFound(kind);
        }
    }

    SecretError::provider_failure(kind, error.to_string())
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

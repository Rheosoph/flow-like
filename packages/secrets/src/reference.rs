use crate::error::{Result, SecretError};
use std::fmt;
use std::str::FromStr;

const SECRET_SCHEME_PREFIX: &str = "secret://";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SecretProviderKind {
    Env,
    File,
    AwsParameterStore,
    AwsSecretsManager,
    GcpSecretManager,
    AzureKeyVault,
}

impl SecretProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretProviderKind::Env => "env",
            SecretProviderKind::File => "file",
            SecretProviderKind::AwsParameterStore => "aws-parameter-store",
            SecretProviderKind::AwsSecretsManager => "aws-secrets-manager",
            SecretProviderKind::GcpSecretManager => "gcp-secret-manager",
            SecretProviderKind::AzureKeyVault => "azure-key-vault",
        }
    }
}

impl fmt::Display for SecretProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SecretProviderKind {
    type Err = SecretError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "env" | "environment" => Ok(Self::Env),
            "file" | "filesystem" => Ok(Self::File),
            "aws-parameter-store" | "aws-ssm" | "ssm" => Ok(Self::AwsParameterStore),
            "aws-secrets-manager" | "secrets-manager" => Ok(Self::AwsSecretsManager),
            "gcp-secret-manager" | "gsm" => Ok(Self::GcpSecretManager),
            "azure-key-vault" | "key-vault" => Ok(Self::AzureKeyVault),
            _ => Err(SecretError::InvalidReference),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SecretRef {
    pub provider: Option<SecretProviderKind>,
    pub key: String,
    pub version: Option<String>,
}

impl SecretRef {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            provider: None,
            key: key.into(),
            version: None,
        }
    }

    pub fn with_provider(provider: SecretProviderKind, key: impl Into<String>) -> Self {
        Self {
            provider: Some(provider),
            key: key.into(),
            version: None,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn to_uri(&self) -> String {
        match self.provider {
            None => self.key.clone(),
            Some(provider) => {
                let mut uri = format!("{SECRET_SCHEME_PREFIX}{}/{}", provider.as_str(), self.key);
                if let Some(version) = &self.version {
                    uri.push_str("?version=");
                    uri.push_str(version);
                }
                uri
            }
        }
    }
}

impl TryFrom<&str> for SecretRef {
    type Error = SecretError;

    fn try_from(value: &str) -> Result<Self> {
        parse_secret_ref(value)
    }
}

impl TryFrom<String> for SecretRef {
    type Error = SecretError;

    fn try_from(value: String) -> Result<Self> {
        parse_secret_ref(value.as_str())
    }
}

fn parse_secret_ref(value: &str) -> Result<SecretRef> {
    if !value.starts_with(SECRET_SCHEME_PREFIX) {
        if value.trim().is_empty() {
            return Err(SecretError::InvalidReference);
        }
        return Ok(SecretRef::new(value));
    }

    let raw = &value[SECRET_SCHEME_PREFIX.len()..];
    let (without_query, query) = split_query(raw);

    let (provider_str, key) = without_query
        .split_once('/')
        .ok_or(SecretError::InvalidReference)?;

    if key.is_empty() {
        return Err(SecretError::InvalidReference);
    }

    let provider = SecretProviderKind::from_str(provider_str)?;
    let version = query.and_then(extract_version);

    Ok(SecretRef {
        provider: Some(provider),
        key: key.to_string(),
        version,
    })
}

fn split_query(value: &str) -> (&str, Option<&str>) {
    match value.split_once('?') {
        Some((left, right)) => (left, Some(right)),
        None => (value, None),
    }
}

fn extract_version(query: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "version").then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must_ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    #[test]
    fn parses_unqualified_reference() {
        let parsed = must_ok(SecretRef::try_from("OPENAI_API_KEY"), "must parse");
        assert_eq!(parsed.provider, None);
        assert_eq!(parsed.key, "OPENAI_API_KEY");
        assert_eq!(parsed.version, None);
    }

    #[test]
    fn parses_fully_qualified_reference() {
        let parsed = must_ok(
            SecretRef::try_from("secret://aws-secrets-manager/flow-like/openai?version=1"),
            "must parse",
        );

        assert_eq!(parsed.provider, Some(SecretProviderKind::AwsSecretsManager));
        assert_eq!(parsed.key, "flow-like/openai");
        assert_eq!(parsed.version.as_deref(), Some("1"));
    }

    #[test]
    fn rejects_missing_key() {
        match SecretRef::try_from("secret://env/") {
            Ok(_) => panic!("missing key must fail"),
            Err(error) => assert_eq!(error, SecretError::InvalidReference),
        }
    }
}

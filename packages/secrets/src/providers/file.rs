use crate::config::FileProviderConfig;
use crate::error::{Result, SecretError};
use crate::providers::SecretProvider;
use crate::{SecretProviderKind, SecretRef, SecretValue};
use async_trait::async_trait;
use std::io::ErrorKind;
use std::path::{Component, PathBuf};

pub struct FileSecretProvider {
    config: FileProviderConfig,
}

impl FileSecretProvider {
    pub fn new(config: FileProviderConfig) -> Self {
        Self { config }
    }

    fn resolve_path(&self, key: &str) -> Result<PathBuf> {
        let mut resolved = self.config.root_path.clone();

        for component in PathBuf::from(key).components() {
            match component {
                Component::Normal(normal) => resolved.push(normal),
                _ => return Err(SecretError::InvalidReference),
            }
        }

        Ok(resolved)
    }
}

#[async_trait]
impl SecretProvider for FileSecretProvider {
    fn kind(&self) -> SecretProviderKind {
        SecretProviderKind::File
    }

    async fn get(&self, reference: &SecretRef) -> Result<SecretValue> {
        let path = self.resolve_path(&reference.key)?;
        let mut data = tokio::fs::read(path).await.map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                SecretError::SecretNotFound(self.kind())
            } else {
                SecretError::provider_failure(self.kind(), error.to_string())
            }
        })?;

        if self.config.trim_trailing_newline {
            while matches!(data.last(), Some(b'\n' | b'\r')) {
                data.pop();
            }
        }

        match String::from_utf8(data) {
            Ok(value) => Ok(SecretValue::from_string(value)),
            Err(error) => Ok(SecretValue::from_bytes(error.into_bytes())),
        }
    }
}

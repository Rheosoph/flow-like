//! Secret-safe source selection and bounded I/O, independent of the Hub schema.

use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use std::{ffi::OsString, fmt, io::Read, path::PathBuf};
use tracing::instrument::WithSubscriber;

pub const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;
pub const JSON_ENV: &str = "FLOW_LIKE_CONFIG_JSON";
pub const FILE_ENV: &str = "FLOW_LIKE_CONFIG_FILE";
pub const SECRET_ENV: &str = "FLOW_LIKE_CONFIG_SECRET_REF";

/// These errors deliberately retain no parser, filesystem, provider, or input
/// values. Startup callers may display/debug them without exposing a document.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("API configuration source {0} must be valid UTF-8")]
    Encoding(&'static str),
    #[error("API configuration source {0} must not contain only whitespace")]
    Whitespace(&'static str),
    #[error(
        "Configure only one of FLOW_LIKE_CONFIG_JSON, FLOW_LIKE_CONFIG_FILE, and FLOW_LIKE_CONFIG_SECRET_REF"
    )]
    Conflict,
    #[error("API configuration file setting must be an exact path without surrounding whitespace")]
    FilePath,
    #[error("API configuration secret reference is invalid")]
    SecretReference,
    #[error("API configuration file could not be read as UTF-8")]
    FileRead,
    #[error("API configuration secret could not be resolved as text")]
    SecretRead,
    #[error("API configuration exceeds the 4 MiB document limit")]
    TooLarge,
    #[error("API configuration is not a valid JSON document")]
    Json,
    #[error("API configuration does not match the deployment schema")]
    Schema,
    #[error(
        "API configuration must use client_secret_env references instead of literal OAuth client_secret values"
    )]
    LiteralOAuthSecret,
}

pub enum ConfigSource {
    Embedded,
    Json(String),
    File(PathBuf),
    Secret(SecretRef),
}

impl fmt::Debug for ConfigSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Embedded => "Embedded",
            Self::Json(_) => "Json([redacted])",
            Self::File(_) => "File([redacted])",
            Self::Secret(_) => "Secret([redacted])",
        })
    }
}

impl ConfigSource {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var_os(name))
    }

    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<Self, ConfigError> {
        let mut selected = Vec::new();
        for name in [JSON_ENV, FILE_ENV, SECRET_ENV] {
            let Some(value) = lookup(name) else { continue };
            let value = value
                .into_string()
                .map_err(|_| ConfigError::Encoding(name))?;
            // Compose can interpolate an unset selector to an exactly empty value.
            if value.is_empty() {
                continue;
            }
            if value.trim().is_empty() {
                return Err(ConfigError::Whitespace(name));
            }
            selected.push((name, value));
        }
        if selected.len() > 1 {
            return Err(ConfigError::Conflict);
        }
        let Some((name, value)) = selected.pop() else {
            return Ok(Self::Embedded);
        };
        match name {
            JSON_ENV => Ok(Self::Json(value)),
            FILE_ENV => {
                if value.trim() != value {
                    return Err(ConfigError::FilePath);
                }
                Ok(Self::File(PathBuf::from(value)))
            }
            SECRET_ENV => {
                if value.trim() != value {
                    return Err(ConfigError::SecretReference);
                }
                SecretRef::try_from(value)
                    .map(Self::Secret)
                    .map_err(|_| ConfigError::SecretReference)
            }
            _ => unreachable!("source names come from a fixed allowlist"),
        }
    }

    pub async fn load_with_parser<T>(
        self,
        secrets: &SecretStore,
        embedded: &str,
        parse: impl FnOnce(&str) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let parse = |document: &str| {
            if document.len() > MAX_CONFIG_BYTES {
                return Err(ConfigError::TooLarge);
            }
            parse(document)
        };
        match self {
            Self::Embedded => parse(embedded),
            Self::Json(document) => parse(&document),
            Self::File(path) => {
                let metadata = std::fs::metadata(&path).map_err(|_| ConfigError::FileRead)?;
                if !metadata.is_file() {
                    return Err(ConfigError::FileRead);
                }
                if metadata.len() > MAX_CONFIG_BYTES as u64 {
                    return Err(ConfigError::TooLarge);
                }
                let file = std::fs::File::open(path).map_err(|_| ConfigError::FileRead)?;
                let opened = file.metadata().map_err(|_| ConfigError::FileRead)?;
                if !opened.is_file() {
                    return Err(ConfigError::FileRead);
                }
                if opened.len() > MAX_CONFIG_BYTES as u64 {
                    return Err(ConfigError::TooLarge);
                }
                let mut document = String::new();
                file.take((MAX_CONFIG_BYTES + 1) as u64)
                    .read_to_string(&mut document)
                    .map_err(|_| ConfigError::FileRead)?;
                parse(&document)
            }
            Self::Secret(reference) => {
                let document = secrets
                    .get_secret_string(&reference)
                    // Provider retries can otherwise trace the reference and raw
                    // SDK error. Suppress only this future, never a global guard.
                    .with_subscriber(tracing::subscriber::NoSubscriber::default())
                    .await
                    .map_err(|_| ConfigError::SecretRead)?;
                parse(document.expose_secret())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretStoreConfig};
    use std::sync::{Arc, Mutex};

    fn source(values: &[(&str, &str)]) -> Result<ConfigSource, ConfigError> {
        ConfigSource::from_lookup(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    fn store(root: &std::path::Path) -> SecretStore {
        SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: root.into(),
                    trim_trailing_newline: false,
                })),
        )
        .unwrap()
    }

    async fn load(source: ConfigSource, secrets: &SecretStore) -> Result<String, ConfigError> {
        source
            .load_with_parser(secrets, "embedded-document", |document| {
                Ok(document.to_owned())
            })
            .await
    }

    #[test]
    fn source_conflicts_and_whitespace_fail_closed() {
        assert!(matches!(source(&[]).unwrap(), ConfigSource::Embedded));
        assert!(matches!(
            source(&[(JSON_ENV, ""), (FILE_ENV, ""), (SECRET_ENV, "")]).unwrap(),
            ConfigSource::Embedded
        ));
        for name in [JSON_ENV, FILE_ENV, SECRET_ENV] {
            assert!(matches!(
                source(&[(name, " \n")]),
                Err(ConfigError::Whitespace(_))
            ));
        }
        for values in [
            vec![(JSON_ENV, "a"), (FILE_ENV, "b")],
            vec![(JSON_ENV, "a"), (SECRET_ENV, "b")],
            vec![(FILE_ENV, "a"), (SECRET_ENV, "b")],
            vec![(JSON_ENV, "a"), (FILE_ENV, "b"), (SECRET_ENV, "c")],
        ] {
            assert!(matches!(source(&values), Err(ConfigError::Conflict)));
        }
        assert!(matches!(
            source(&[(FILE_ENV, " path")]),
            Err(ConfigError::FilePath)
        ));
        assert!(matches!(
            source(&[(SECRET_ENV, "key ")]),
            Err(ConfigError::SecretReference)
        ));
        assert!(matches!(
            source(&[(SECRET_ENV, "secret://unknown/key")]),
            Err(ConfigError::SecretReference)
        ));
        assert!(matches!(
            source(&[(JSON_ENV, " {}\n"), (FILE_ENV, "")]),
            Ok(ConfigSource::Json(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_and_debug_output_do_not_expose_input() {
        use std::os::unix::ffi::OsStringExt;
        let result = ConfigSource::from_lookup(|name| {
            (name == JSON_ENV).then(|| OsString::from_vec(vec![0xff]))
        });
        assert!(matches!(result, Err(ConfigError::Encoding(JSON_ENV))));
        for selected in [
            ConfigSource::Json("private-value-marker".into()),
            ConfigSource::File("private-value-marker".into()),
            ConfigSource::Secret(SecretRef::new("private-value-marker")),
        ] {
            assert!(!format!("{selected:?}").contains("private-value-marker"));
        }
    }

    #[tokio::test]
    async fn all_sources_deliver_exact_selected_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        std::fs::write(&path, "mounted-document\n").unwrap();
        let secrets = store(directory.path());
        assert_eq!(
            load(ConfigSource::Embedded, &secrets).await.unwrap(),
            "embedded-document"
        );
        assert_eq!(
            load(ConfigSource::Json("inline-document".into()), &secrets)
                .await
                .unwrap(),
            "inline-document"
        );
        assert_eq!(
            load(ConfigSource::File(path), &secrets).await.unwrap(),
            "mounted-document\n"
        );
        assert_eq!(
            load(
                ConfigSource::Secret(SecretRef::try_from("secret://file/config").unwrap()),
                &secrets
            )
            .await
            .unwrap(),
            "mounted-document\n"
        );
    }

    #[tokio::test]
    async fn parser_failures_never_fall_back_to_embedded() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = store(directory.path());
        for kind in [
            ConfigError::Json,
            ConfigError::Schema,
            ConfigError::LiteralOAuthSecret,
        ] {
            let result: Result<(), _> = ConfigSource::Json("private-document".into())
                .load_with_parser(&secrets, "embedded-document", |_| Err(kind))
                .await;
            let error = result.unwrap_err();
            assert!(!format!("{error:?} {error}").contains("private-document"));
        }
    }

    #[tokio::test]
    async fn file_type_encoding_size_and_missing_sources_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = store(directory.path());
        let path = directory.path().join("private-marker");
        for selected in [
            ConfigSource::File(directory.path().into()),
            ConfigSource::File(path.clone()),
        ] {
            assert!(matches!(
                load(selected, &secrets).await,
                Err(ConfigError::FileRead)
            ));
        }
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(matches!(
            load(ConfigSource::File(path.clone()), &secrets).await,
            Err(ConfigError::FileRead)
        ));
        assert!(matches!(
            load(
                ConfigSource::Secret(SecretRef::new("private-marker")),
                &secrets
            )
            .await,
            Err(ConfigError::SecretRead)
        ));
        let oversized = std::fs::File::create(&path).unwrap();
        oversized.set_len((MAX_CONFIG_BYTES + 1) as u64).unwrap();
        assert!(matches!(
            load(ConfigSource::File(path.clone()), &secrets).await,
            Err(ConfigError::TooLarge)
        ));
        assert!(matches!(
            load(
                ConfigSource::Json("x".repeat(MAX_CONFIG_BYTES + 1)),
                &secrets
            )
            .await,
            Err(ConfigError::TooLarge)
        ));
        std::fs::write(&path, "valid-document").unwrap();
        #[cfg(unix)]
        {
            let link = directory.path().join("projected-config");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert_eq!(
                load(ConfigSource::File(link), &secrets).await.unwrap(),
                "valid-document"
            );
        }
        let missing = load(
            ConfigSource::Secret(SecretRef::new("private-missing-marker")),
            &secrets,
        )
        .await
        .unwrap_err();
        assert!(matches!(missing, ConfigError::SecretRead));
        assert!(!format!("{missing:?} {missing}").contains("private-missing-marker"));
        let unconfigured =
            SecretStore::new(SecretStoreConfig::default().with_allow_env_override(false)).unwrap();
        assert!(matches!(
            load(ConfigSource::Secret(SecretRef::new("key")), &unconfigured).await,
            Err(ConfigError::SecretRead)
        ));
    }

    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn config_secret_lookup_suppresses_nested_provider_error_logs() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = store(directory.path());
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let reference = SecretRef::new("../private-reference-marker");
        // Prove the provider's ordinary fallback path emits the reference.
        let _ = secrets
            .get_secret_string(&reference)
            .with_subscriber(subscriber)
            .await;
        assert!(
            String::from_utf8_lossy(&logs.0.lock().unwrap()).contains("private-reference-marker")
        );
        logs.0.lock().unwrap().clear();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let error = load(ConfigSource::Secret(reference), &secrets)
            .with_subscriber(subscriber)
            .await
            .unwrap_err();
        assert!(matches!(error, ConfigError::SecretRead));
        assert!(logs.0.lock().unwrap().is_empty());
        assert!(!format!("{error:?} {error}").contains("private-reference-marker"));
    }
}

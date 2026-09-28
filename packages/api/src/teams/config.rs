use crate::error::ApiError;
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use tracing::instrument::WithSubscriber;

const PUBLIC_URL: &str = "TEAMS_PUBLIC_BASE_URL";
const MANAGED_ENABLED: &str = "TEAMS_MANAGED_ENABLED";
const MANAGED_BOT_LIMIT: &str = "TEAMS_MANAGED_MAX_BOTS_PER_APP";
const DEFAULT_MANAGED_BOT_LIMIT: i64 = 10;
const MANAGED_KEYS: [&str; 6] = [
    "TEAMS_MANAGED_TENANT_ID",
    "TEAMS_PROVISIONER_CLIENT_ID",
    "TEAMS_PROVISIONER_CLIENT_SECRET",
    "TEAMS_PROVISIONER_OBJECT_ID",
    "TEAMS_AZURE_SUBSCRIPTION_ID",
    "TEAMS_AZURE_RESOURCE_GROUP",
];

#[derive(Debug, thiserror::Error)]
pub(super) enum ConfigError {
    #[error(
        "The server could not read {0} from its secret store. Ask your administrator to check the provider and access permissions."
    )]
    Read(&'static str),
    #[error(
        "Flow-Like-managed bot settings are incomplete. Ask your administrator to configure all six Microsoft provisioning settings."
    )]
    Incomplete,
    #[error("The server has an invalid {0} setting. Ask your administrator to update it.")]
    Invalid(&'static str),
    #[error(
        "Teams requires a public HTTPS API URL. Configure TEAMS_PUBLIC_BASE_URL in the server's secret store."
    )]
    PublicUrl,
}

impl From<ConfigError> for ApiError {
    fn from(error: ConfigError) -> Self {
        ApiError::internal(error.to_string())
    }
}

pub(super) struct ManagedConfig {
    pub tenant: String,
    pub id: String,
    pub secret: String,
    pub owner: String,
    pub subscription: String,
    pub group: String,
}

async fn optional(
    secrets: &SecretStore,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    // Provider errors can contain SDK request details. Suppress this future's
    // nested tracing and retain only the fixed setting name in our error.
    let value = secrets
        .get_secret_string(&SecretRef::new(name))
        .with_subscriber(tracing::subscriber::NoSubscriber::default())
        .await;
    match value {
        Ok(value) if value.expose_secret().trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.expose_secret().to_owned())),
        Err(error) if error.is_not_found() => Ok(None),
        Err(_) => Err(ConfigError::Read(name)),
    }
}

impl ManagedConfig {
    pub async fn load(secrets: &SecretStore) -> Result<Option<Self>, ConfigError> {
        let enabled = match optional(secrets, MANAGED_ENABLED).await?.as_deref() {
            Some(value) => Some(match value.trim() {
                "false" => false,
                "true" => true,
                _ => return Err(ConfigError::Invalid(MANAGED_ENABLED)),
            }),
            None => None,
        };
        // Customer-only deployments intentionally have no IAM access to the
        // provisioning settings. Do not request those secrets when disabled.
        if enabled == Some(false) {
            return Ok(None);
        }
        let values = futures::future::try_join_all(
            MANAGED_KEYS.into_iter().map(|name| optional(secrets, name)),
        )
        .await?;
        if enabled.is_none() && values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let mut values = values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(ConfigError::Incomplete)?
            .into_iter();
        let tenant = values.next().unwrap();
        let id = values.next().unwrap();
        let secret = values.next().unwrap();
        let owner = values.next().unwrap();
        let subscription = values.next().unwrap();
        let group = values.next().unwrap().trim().to_owned();
        let guid = |raw: &str, name| {
            uuid::Uuid::parse_str(raw.trim())
                .map(|value| value.to_string())
                .map_err(|_| ConfigError::Invalid(name))
        };
        if secret.len() > 4096 {
            return Err(ConfigError::Invalid(MANAGED_KEYS[2]));
        }
        if group.len() > 90
            || group.ends_with('.')
            || !group
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.()".contains(&b))
        {
            return Err(ConfigError::Invalid(MANAGED_KEYS[5]));
        }
        Ok(Some(Self {
            tenant: guid(&tenant, MANAGED_KEYS[0])?,
            id: guid(&id, MANAGED_KEYS[1])?,
            secret,
            owner: guid(&owner, MANAGED_KEYS[3])?,
            subscription: guid(&subscription, MANAGED_KEYS[4])?,
            group,
        }))
    }
}

pub(super) async fn managed_bot_limit(secrets: &SecretStore) -> Result<i64, ConfigError> {
    optional(secrets, MANAGED_BOT_LIMIT)
        .await?
        .map_or(Ok(DEFAULT_MANAGED_BOT_LIMIT), |value| {
            value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|limit| (1..=10_000).contains(limit))
                .ok_or(ConfigError::Invalid(MANAGED_BOT_LIMIT))
        })
}

pub(super) async fn public_base(
    secrets: &SecretStore,
    fallback: Option<&str>,
) -> Result<String, ConfigError> {
    let configured = optional(secrets, PUBLIC_URL).await?;
    let raw = configured
        .as_deref()
        .or(fallback)
        .ok_or(ConfigError::PublicUrl)?
        .trim();
    let url = reqwest::Url::parse(raw).map_err(|_| ConfigError::PublicUrl)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ConfigError::PublicUrl);
    }
    let base = raw.trim_end_matches('/');
    Ok(if base.ends_with("/api/v1") {
        base.into()
    } else {
        format!("{base}/api/v1")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretStoreConfig};
    use std::sync::{Arc, Mutex};

    fn store(root: &std::path::Path) -> SecretStore {
        SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: root.into(),
                    trim_trailing_newline: true,
                })),
        )
        .unwrap()
    }

    fn write_managed(root: &std::path::Path) {
        for name in MANAGED_KEYS {
            let value = match name {
                "TEAMS_PROVISIONER_CLIENT_SECRET" => "test-only-private-secret",
                "TEAMS_AZURE_RESOURCE_GROUP" => "flow-like-teams-dev",
                _ => "7304d42a-ece7-4a9d-af88-91022fa6d253",
            };
            std::fs::write(root.join(name), value).unwrap();
        }
    }

    #[tokio::test]
    async fn resolves_managed_settings_through_configured_provider() {
        let directory = tempfile::tempdir().unwrap();
        write_managed(directory.path());
        let settings = ManagedConfig::load(&store(directory.path()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings.tenant, "7304d42a-ece7-4a9d-af88-91022fa6d253");
        assert_eq!(settings.id, settings.tenant);
        assert_eq!(settings.owner, settings.tenant);
        assert_eq!(settings.subscription, settings.tenant);
        assert_eq!(settings.secret, "test-only-private-secret");
        assert_eq!(settings.group, "flow-like-teams-dev");
    }

    #[tokio::test]
    async fn explicitly_disabled_managed_setup_does_not_read_provisioning_secrets() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(MANAGED_ENABLED), "false").unwrap();
        // Each path would fail if SecretStore tried to read it as a text file.
        for name in MANAGED_KEYS {
            std::fs::create_dir(directory.path().join(name)).unwrap();
        }
        assert!(
            ManagedConfig::load(&store(directory.path()))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn explicitly_enabled_managed_setup_requires_all_provisioning_settings() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(MANAGED_ENABLED), "true").unwrap();
        assert!(matches!(
            ManagedConfig::load(&store(directory.path())).await,
            Err(ConfigError::Incomplete)
        ));
        write_managed(directory.path());
        assert!(
            ManagedConfig::load(&store(directory.path()))
                .await
                .unwrap()
                .is_some()
        );
        std::fs::remove_file(directory.path().join(MANAGED_KEYS[2])).unwrap();
        assert!(matches!(
            ManagedConfig::load(&store(directory.path())).await,
            Err(ConfigError::Incomplete)
        ));
    }

    #[tokio::test]
    async fn invalid_managed_enable_flag_fails_without_echoing_its_value() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(MANAGED_ENABLED), "private-marker").unwrap();
        let error = ManagedConfig::load(&store(directory.path()))
            .await
            .err()
            .unwrap();
        assert!(matches!(error, ConfigError::Invalid(MANAGED_ENABLED)));
        assert!(!format!("{error:?} {error}").contains("private-marker"));
    }

    #[tokio::test]
    async fn missing_managed_settings_disable_it_but_partial_settings_fail() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            ManagedConfig::load(&store(directory.path()))
                .await
                .unwrap()
                .is_none()
        );
        std::fs::write(directory.path().join(MANAGED_KEYS[0]), " \n").unwrap();
        assert!(
            ManagedConfig::load(&store(directory.path()))
                .await
                .unwrap()
                .is_none()
        );
        std::fs::write(directory.path().join(MANAGED_KEYS[0]), "private-marker").unwrap();
        let error = ManagedConfig::load(&store(directory.path()))
            .await
            .err()
            .unwrap();
        assert!(matches!(error, ConfigError::Incomplete));
        assert!(!format!("{error:?} {error}").contains("private-marker"));
    }

    #[tokio::test]
    async fn invalid_managed_settings_never_echo_values() {
        let directory = tempfile::tempdir().unwrap();
        for (name, value) in [
            (MANAGED_KEYS[0], "private-marker"),
            (MANAGED_KEYS[1], "private-marker"),
            (MANAGED_KEYS[3], "private-marker"),
            (MANAGED_KEYS[4], "private-marker"),
            (MANAGED_KEYS[5], "private-marker/invalid"),
            (MANAGED_KEYS[5], "private-marker."),
            (MANAGED_KEYS[2], &"private-marker".repeat(400)),
        ] {
            write_managed(directory.path());
            std::fs::write(directory.path().join(name), value).unwrap();
            let error = ManagedConfig::load(&store(directory.path()))
                .await
                .err()
                .unwrap();
            assert!(matches!(error, ConfigError::Invalid(key) if key == name));
            assert!(!format!("{error:?} {error}").contains("private-marker"));
        }
    }

    #[tokio::test]
    async fn managed_bot_limit_defaults_and_rejects_invalid_values() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            managed_bot_limit(&store(directory.path())).await.unwrap(),
            DEFAULT_MANAGED_BOT_LIMIT
        );
        std::fs::write(directory.path().join(MANAGED_BOT_LIMIT), "25\n").unwrap();
        assert_eq!(
            managed_bot_limit(&store(directory.path())).await.unwrap(),
            25
        );
        for value in ["0", "-1", "private-marker", "10001"] {
            std::fs::write(directory.path().join(MANAGED_BOT_LIMIT), value).unwrap();
            let error = managed_bot_limit(&store(directory.path()))
                .await
                .unwrap_err();
            assert!(matches!(error, ConfigError::Invalid(MANAGED_BOT_LIMIT)));
            assert!(!format!("{error:?} {error}").contains("private-marker"));
        }
    }

    #[tokio::test]
    async fn public_url_uses_provider_then_existing_api_url_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = Some("https://internal.example/api/v1/");
        assert_eq!(
            public_base(&store(directory.path()), fallback)
                .await
                .unwrap(),
            "https://internal.example/api/v1"
        );
        for value in ["", " \n"] {
            std::fs::write(directory.path().join(PUBLIC_URL), value).unwrap();
            assert_eq!(
                public_base(&store(directory.path()), fallback)
                    .await
                    .unwrap(),
                "https://internal.example/api/v1"
            );
        }
        std::fs::write(directory.path().join(PUBLIC_URL), "https://public.example/").unwrap();
        assert_eq!(
            public_base(&store(directory.path()), fallback)
                .await
                .unwrap(),
            "https://public.example/api/v1"
        );
    }

    #[tokio::test]
    async fn invalid_public_url_does_not_fall_back_or_echo_input() {
        let directory = tempfile::tempdir().unwrap();
        for value in [
            "http://private-marker.example",
            "https://private-marker@example.com",
            "https://example.com?secret=private-marker",
            "https://example.com#private-marker",
            "private-marker",
        ] {
            std::fs::write(directory.path().join(PUBLIC_URL), value).unwrap();
            let error = public_base(&store(directory.path()), Some("https://valid.example"))
                .await
                .unwrap_err();
            assert!(matches!(error, ConfigError::PublicUrl));
            assert!(!format!("{error:?} {error}").contains("private-marker"));
        }
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
    async fn provider_failures_are_not_missing_and_do_not_log_provider_details() {
        let directory = tempfile::tempdir().unwrap();
        // A directory in place of a text secret makes the file provider fail.
        std::fs::create_dir(directory.path().join(PUBLIC_URL)).unwrap();
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let error = public_base(&store(directory.path()), Some("https://valid.example"))
            .with_subscriber(subscriber)
            .await
            .unwrap_err();
        assert!(matches!(error, ConfigError::Read(PUBLIC_URL)));
        assert!(logs.0.lock().unwrap().is_empty());
        assert!(!format!("{error:?} {error}").contains(directory.path().to_str().unwrap()));
        let unconfigured =
            SecretStore::new(SecretStoreConfig::default().with_allow_env_override(false)).unwrap();
        assert!(matches!(
            ManagedConfig::load(&unconfigured).await,
            Err(ConfigError::Read(_))
        ));
    }
}

//! Resolve the complete API deployment document once, before starting services.

use flow_like::hub::Hub;
use flow_like_secrets::SecretStore;
use serde::Deserialize;
use std::collections::HashMap;

pub(crate) mod mail;
mod source;
use source::MAX_CONFIG_BYTES;
pub(crate) use source::{ConfigError, ConfigSource};

const EMBEDDED_CONFIG: &str = include_str!("../../../flow-like.config.json");

pub(crate) fn configure_mail_capability(hub: &mut Hub, enabled: bool) {
    hub.supported_sinks
        .get_or_insert_with(Default::default)
        .inbound_email = enabled;
}

impl ConfigSource {
    pub(crate) async fn load(self, secrets: &SecretStore) -> Result<EffectiveConfig, ConfigError> {
        self.load_with_parser(secrets, EMBEDDED_CONFIG, EffectiveConfig::parse)
            .await
    }
}

/// Private API settings retain fields the public Hub schema does not expose.
/// No raw document or resolved OAuth credential is retained in this structure.
pub(crate) struct EffectiveConfig {
    pub(crate) hub: Hub,
    pub(crate) oauth_providers: HashMap<String, OAuthProviderConfig>,
    pub(crate) openid: OpenIdValidationOverrides,
    pub(crate) mail_automation: Option<mail::MailAutomationConfig>,
}

impl EffectiveConfig {
    pub(crate) fn parse(document: &str) -> Result<Self, ConfigError> {
        if document.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let document: serde_json::Value =
            serde_json::from_str(document).map_err(|_| ConfigError::Json)?;
        let hub = Hub::deserialize(&document).map_err(|_| ConfigError::Schema)?;
        if hub
            .oauth_providers
            .values()
            .any(|provider| provider.client_secret.is_some())
        {
            return Err(ConfigError::LiteralOAuthSecret);
        }
        let oauth_providers = match document.get("oauth_providers") {
            Some(value) => HashMap::deserialize(value).map_err(|_| ConfigError::Schema)?,
            None => HashMap::new(),
        };
        let openid = match document.pointer("/authentication/openid") {
            Some(value) if !value.is_null() => {
                OpenIdValidationOverrides::deserialize(value).map_err(|_| ConfigError::Schema)?
            }
            _ => OpenIdValidationOverrides::default(),
        };
        let mail_automation = match document.get("mail_automation") {
            Some(value) if !value.is_null() => Some(
                mail::MailAutomationConfig::parse(&value.to_string())
                    .map_err(|_| ConfigError::Schema)?,
            ),
            _ => None,
        };
        Ok(Self {
            hub,
            oauth_providers,
            openid,
            mail_automation,
        })
    }
}

#[derive(Clone, Deserialize)]
pub(crate) struct OAuthProviderConfig {
    #[serde(default)]
    pub(crate) client_id: Option<String>,
    pub(crate) client_secret_env: Option<String>,
    pub(crate) token_url: String,
    pub(crate) revoke_url: Option<String>,
    pub(crate) userinfo_url: Option<String>,
    pub(crate) device_auth_url: Option<String>,
    pub(crate) auth_method: Option<String>,
}

/// API-only OpenID fields are parsed from the same selected document as Hub.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenIdValidationOverrides {
    #[serde(default = "default_openid_leeway_seconds")]
    pub(crate) leeway_seconds: u64,
    #[serde(default)]
    pub(crate) additional_client_ids: Vec<String>,
}

impl Default for OpenIdValidationOverrides {
    fn default() -> Self {
        Self {
            leeway_seconds: default_openid_leeway_seconds(),
            additional_client_ids: Vec::new(),
        }
    }
}

fn default_openid_leeway_seconds() -> u64 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretRef, SecretStoreConfig};
    use source::{FILE_ENV, JSON_ENV, SECRET_ENV};
    use std::ffi::OsString;

    fn source(values: &[(&str, &str)]) -> Result<ConfigSource, ConfigError> {
        ConfigSource::from_lookup(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    fn store() -> SecretStore {
        SecretStore::new(SecretStoreConfig::default().with_allow_env_override(false)).unwrap()
    }

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../apps/backend/kubernetes/flow-like.config.example.json"
        ))
        .unwrap()
    }

    #[test]
    fn resolved_mail_configuration_controls_advertised_inbound_capability() {
        let mut hub = EffectiveConfig::parse(EMBEDDED_CONFIG).unwrap().hub;
        let http = hub.supported_sinks.as_ref().map(|sinks| sinks.http);
        configure_mail_capability(&mut hub, false);
        assert!(!hub.supported_sinks.as_ref().unwrap().inbound_email);
        assert_eq!(hub.supported_sinks.as_ref().map(|sinks| sinks.http), http);
        configure_mail_capability(&mut hub, true);
        assert!(hub.supported_sinks.as_ref().unwrap().inbound_email);
        hub.supported_sinks = None;
        configure_mail_capability(&mut hub, true);
        assert!(hub.supported_sinks.as_ref().unwrap().inbound_email);
    }

    fn error<T>(result: Result<T, ConfigError>) -> ConfigError {
        match result {
            Ok(_) => panic!("invalid configuration unexpectedly accepted"),
            Err(error) => error,
        }
    }

    #[test]
    fn source_selection_has_no_precedence_or_implicit_fallback() {
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
        for pair in [
            [JSON_ENV, FILE_ENV],
            [JSON_ENV, SECRET_ENV],
            [FILE_ENV, SECRET_ENV],
        ] {
            assert!(matches!(
                source(&[(pair[0], "one"), (pair[1], "two")]),
                Err(ConfigError::Conflict)
            ));
        }
        assert!(matches!(
            source(&[(FILE_ENV, " /mounted/config.json")]),
            Err(ConfigError::FilePath)
        ));
        assert!(matches!(
            source(&[(SECRET_ENV, "secret://not-a-provider/key")]),
            Err(ConfigError::SecretReference)
        ));
        assert!(matches!(
            source(&[(JSON_ENV, "{}"), (FILE_ENV, "")]).unwrap(),
            ConfigSource::Json(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_selector_fails_without_printing_input() {
        use std::os::unix::ffi::OsStringExt;
        let result = ConfigSource::from_lookup(|name| {
            (name == JSON_ENV).then(|| OsString::from_vec(vec![0xff]))
        });
        assert!(matches!(result, Err(ConfigError::Encoding(JSON_ENV))));
    }

    #[tokio::test]
    async fn absent_source_uses_embedded_config() {
        let selected = source(&[]).unwrap().load(&store()).await.unwrap();
        let embedded = EffectiveConfig::parse(EMBEDDED_CONFIG).unwrap();
        assert_eq!(selected.hub.domain, embedded.hub.domain);
        assert_eq!(
            selected.oauth_providers.len(),
            embedded.oauth_providers.len()
        );
    }

    #[tokio::test]
    async fn runtime_document_replaces_hub_oauth_and_openid_together() {
        let mut document = fixture();
        document["domain"] = serde_json::json!("runtime.example.test");
        document["oauth_providers"] = serde_json::json!({"runtime-provider": {
            "name":"Runtime provider", "client_id":"runtime-client", "client_secret_env":"RUNTIME_OAUTH_REF",
            "auth_url":"https://provider.example.test/authorize", "token_url":"https://provider.example.test/token",
            "auth_method":"basic_json"
        }});
        document["authentication"]["openid"]["additional_client_ids"] =
            serde_json::json!(["second-client"]);
        document["authentication"]["openid"]["leeway_seconds"] = serde_json::json!(7);
        let loaded = ConfigSource::Json(document.to_string())
            .load(&store())
            .await
            .unwrap();
        assert_eq!(loaded.hub.domain, "runtime.example.test");
        assert_eq!(loaded.hub.oauth_providers.len(), 1);
        assert_eq!(loaded.oauth_providers.len(), 1);
        assert_eq!(
            loaded.oauth_providers["runtime-provider"]
                .auth_method
                .as_deref(),
            Some("basic_json")
        );
        assert_eq!(loaded.openid.additional_client_ids, ["second-client"]);
        assert_eq!(loaded.openid.leeway_seconds, 7);
        assert!(
            loaded.hub.oauth_providers["runtime-provider"]
                .client_secret
                .is_none()
        );
    }

    #[test]
    fn malformed_json_schema_and_openid_overrides_fail_closed() {
        for invalid in [
            "",
            " ",
            "{",
            "null",
            "[]",
            "{}",
            r#"{"name":"private-marker"}"#,
        ] {
            let error = error(EffectiveConfig::parse(invalid));
            assert!(!format!("{error:?} {error}").contains("private-marker"));
        }
        for (key, value) in [
            ("leeway_seconds", serde_json::json!("bad")),
            ("leeway_seconds", serde_json::json!(-1)),
            ("additional_client_ids", serde_json::json!([false])),
        ] {
            let mut document = fixture();
            document["authentication"]["openid"][key] = value;
            assert!(matches!(
                EffectiveConfig::parse(&document.to_string()),
                Err(ConfigError::Schema)
            ));
        }
        assert!(matches!(
            EffectiveConfig::parse(&"x".repeat(MAX_CONFIG_BYTES + 1)),
            Err(ConfigError::TooLarge)
        ));
    }

    #[test]
    fn literal_oauth_secrets_are_rejected_and_never_debugged() {
        let mut document = fixture();
        document["oauth_providers"] = serde_json::json!({"provider": {
            "name":"Provider", "auth_url":"https://provider.example.test/auth", "token_url":"https://provider.example.test/token",
            "client_secret":"private-literal-marker"
        }});
        let error = error(EffectiveConfig::parse(&document.to_string()));
        assert!(matches!(error, ConfigError::LiteralOAuthSecret));
        assert!(!format!("{error:?} {error}").contains("private-literal-marker"));
        for selected in [
            ConfigSource::Json("private-value-marker".into()),
            ConfigSource::File("private-value-marker".into()),
            ConfigSource::Secret(SecretRef::new("private-value-marker")),
        ] {
            assert!(!format!("{selected:?}").contains("private-value-marker"));
        }
    }

    #[tokio::test]
    async fn file_and_secret_sources_use_selected_content_and_fail_without_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.json");
        let mut document = fixture();
        document["domain"] = serde_json::json!("mounted.example.test");
        std::fs::write(&path, document.to_string()).unwrap();
        let files = SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: directory.path().into(),
                    trim_trailing_newline: false,
                })),
        )
        .unwrap();
        let from_file = ConfigSource::File(path.clone()).load(&files).await.unwrap();
        let from_secret =
            ConfigSource::Secret(SecretRef::try_from("secret://file/document.json").unwrap())
                .load(&files)
                .await
                .unwrap();
        assert_eq!(from_file.hub.domain, "mounted.example.test");
        assert_eq!(from_secret.hub.domain, from_file.hub.domain);
        std::fs::write(&path, b"private-invalid-json-marker").unwrap();
        let invalid = error(ConfigSource::File(path).load(&files).await);
        assert!(!format!("{invalid:?} {invalid}").contains("private-invalid-json-marker"));
        let missing_file = error(
            ConfigSource::File(directory.path().join("private-path-marker"))
                .load(&files)
                .await,
        );
        assert!(matches!(missing_file, ConfigError::FileRead));
        assert!(!format!("{missing_file:?} {missing_file}").contains("private-path-marker"));
        let missing_secret = error(
            ConfigSource::Secret(SecretRef::new("private-reference-marker"))
                .load(&files)
                .await,
        );
        assert!(matches!(missing_secret, ConfigError::SecretRead));
        assert!(
            !format!("{missing_secret:?} {missing_secret}").contains("private-reference-marker")
        );
        assert!(matches!(
            ConfigSource::Secret(SecretRef::new("missing"))
                .load(&store())
                .await,
            Err(ConfigError::SecretRead)
        ));
    }

    #[tokio::test]
    async fn invalid_files_are_bounded_and_never_echoed() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            ConfigSource::File(directory.path().into())
                .load(&store())
                .await,
            Err(ConfigError::FileRead)
        ));
        let path = directory.path().join("private-path-marker");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let invalid_utf8 = error(ConfigSource::File(path.clone()).load(&store()).await);
        assert!(matches!(invalid_utf8, ConfigError::FileRead));
        assert!(!format!("{invalid_utf8:?} {invalid_utf8}").contains("private-path-marker"));
        let oversized = std::fs::File::create(&path).unwrap();
        oversized.set_len((MAX_CONFIG_BYTES + 1) as u64).unwrap();
        assert!(matches!(
            ConfigSource::File(path.clone()).load(&store()).await,
            Err(ConfigError::TooLarge)
        ));
        std::fs::write(&path, fixture().to_string()).unwrap();
        #[cfg(unix)]
        {
            let link = directory.path().join("projected-config");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(ConfigSource::File(link).load(&store()).await.is_ok());
        }
    }
}

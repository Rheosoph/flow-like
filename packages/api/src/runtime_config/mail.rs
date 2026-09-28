use flow_like_secrets::{ExposeSecret, SecretError, SecretRef, SecretStore};
use serde::Deserialize;
use std::ffi::OsString;
use tracing::instrument::WithSubscriber;

const MAX_MAIL_CONFIG_BYTES: usize = 64 * 1024;
pub(crate) const MAX_SUPPORTED_MAIL_BYTES: usize = 40 * 1024 * 1024;
/// A day must admit at least one message with the maximum recipient count.
const DAILY_RECIPIENT_LIMITS: std::ops::RangeInclusive<u32> = 20..=1_000_000;

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MailAutomationConfig {
    pub(crate) enabled: bool,
    pub(crate) domain: Option<String>,
    pub(crate) bucket: Option<String>,
    pub(crate) prefix: String,
    pub(crate) ttl_seconds: u64,
    pub(crate) max_bytes: usize,
    pub(crate) sending_enabled: bool,
    pub(crate) min_send_interval_seconds: u64,
    pub(crate) daily_app_recipient_limit: u32,
    pub(crate) daily_principal_recipient_limit: u32,
    pub(crate) outbound: Option<AutomationOutboundConfig>,
}

impl Default for MailAutomationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            domain: None,
            bucket: None,
            prefix: "raw/".into(),
            ttl_seconds: 3600,
            max_bytes: 10 * 1024 * 1024,
            sending_enabled: true,
            min_send_interval_seconds: 5,
            daily_app_recipient_limit: 500,
            daily_principal_recipient_limit: 2000,
            outbound: None,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AutomationOutboundConfig {
    Smtp {
        host: String,
        #[serde(default)]
        port: Option<u16>,
        username: String,
        password_secret_ref: String,
        #[serde(default)]
        tls: SmtpTls,
        #[serde(default)]
        from_name: Option<String>,
        #[serde(default)]
        bounce_address: Option<String>,
    },
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SmtpTls {
    #[default]
    Starttls,
    Tls,
}

/// Errors retain no configuration values, secret references or provider details.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum MailConfigError {
    #[error("MAIL_CONFIG could not be read from the configured secret store")]
    Read,
    #[error("Mail automation configuration must be a JSON object of at most 64 KiB")]
    Document,
    #[error("Mail automation configuration contains invalid fields")]
    Schema,
    #[error("Mail automation requires an exact DNS domain")]
    Domain,
    #[error("Mail automation receipt bucket or object prefix is invalid")]
    ReceiptLocation,
    #[error("Mail automation limits are outside the supported range")]
    Limits,
    #[error("A mail automation environment override is invalid")]
    Environment,
    #[error("Mail automation SMTP settings or password reference are invalid")]
    Smtp,
    #[error("Mail automation provider or credentials could not be initialized")]
    Provider,
    #[error(
        "Mail automation needs an outbound SMTP configuration when transactional mail uses Azure Communication Services"
    )]
    DynamicSender,
}

impl MailAutomationConfig {
    pub(crate) fn parse(document: &str) -> Result<Self, MailConfigError> {
        if document.len() > MAX_MAIL_CONFIG_BYTES {
            return Err(MailConfigError::Document);
        }
        let value: serde_json::Value =
            serde_json::from_str(document).map_err(|_| MailConfigError::Document)?;
        if !value.is_object() {
            return Err(MailConfigError::Document);
        }
        serde_json::from_value(value).map_err(|_| MailConfigError::Schema)
    }

    pub(crate) async fn load(
        fallback: Option<Self>,
        secrets: &SecretStore,
    ) -> Result<Self, MailConfigError> {
        Self::load_with_lookup(fallback, secrets, |name| std::env::var_os(name)).await
    }

    async fn load_with_lookup(
        fallback: Option<Self>,
        secrets: &SecretStore,
        lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<Self, MailConfigError> {
        let selected = secrets
            .get_secret_string(&SecretRef::new("MAIL_CONFIG"))
            .with_subscriber(tracing::subscriber::NoSubscriber::default())
            .await;
        let selected = match selected {
            Ok(document) => Some(Self::parse(document.expose_secret())?),
            Err(error) => {
                optional_secret_error(&error)?;
                None
            }
        };
        let mut config = selected.or(fallback).unwrap_or_default();
        config.apply_environment(lookup)?;
        config.validate()?;
        Ok(config)
    }

    fn apply_environment(
        &mut self,
        mut lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<(), MailConfigError> {
        for key in [
            "INBOUND_MAIL_DOMAIN",
            "INBOUND_MAIL_BUCKET",
            "INBOUND_MAIL_PREFIX",
            "INBOUND_MAIL_TTL_SECONDS",
            "INBOUND_MAIL_MAX_BYTES",
            "MAIL_AUTOMATION_ENABLED",
            "MAIL_AUTOMATION_MIN_INTERVAL_SECONDS",
        ] {
            let Some(value) = lookup(key) else { continue };
            let value = value
                .into_string()
                .map_err(|_| MailConfigError::Environment)?;
            if value.is_empty() {
                continue;
            }
            if value.trim() != value {
                return Err(MailConfigError::Environment);
            }
            match key {
                "INBOUND_MAIL_DOMAIN" => {
                    // A legacy domain setting provisioned inbound mail before MAIL_CONFIG
                    // existed; sending stays off unless it is enabled explicitly.
                    if !self.enabled {
                        self.sending_enabled = false;
                    }
                    self.domain = Some(value);
                    self.enabled = true;
                }
                "INBOUND_MAIL_BUCKET" => self.bucket = Some(value),
                "INBOUND_MAIL_PREFIX" => self.prefix = value,
                "INBOUND_MAIL_TTL_SECONDS" => {
                    self.ttl_seconds = value.parse().map_err(|_| MailConfigError::Environment)?
                }
                "INBOUND_MAIL_MAX_BYTES" => {
                    self.max_bytes = value.parse().map_err(|_| MailConfigError::Environment)?
                }
                "MAIL_AUTOMATION_ENABLED" => {
                    self.sending_enabled = match value.to_ascii_lowercase().as_str() {
                        "true" | "1" => true,
                        "false" | "0" => false,
                        _ => return Err(MailConfigError::Environment),
                    }
                }
                "MAIL_AUTOMATION_MIN_INTERVAL_SECONDS" => {
                    self.min_send_interval_seconds =
                        value.parse().map_err(|_| MailConfigError::Environment)?
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    fn validate(&mut self) -> Result<(), MailConfigError> {
        if let Some(domain) = &mut self.domain {
            *domain = domain.to_ascii_lowercase();
            if !valid_domain(domain) {
                return Err(MailConfigError::Domain);
            }
        }
        if self.enabled && self.domain.is_none() {
            return Err(MailConfigError::Domain);
        }
        if self.bucket.as_ref().is_some_and(|bucket| {
            !(3..=63).contains(&bucket.len())
                || !bucket
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
                || !bucket.as_bytes()[0].is_ascii_alphanumeric()
                || !bucket.as_bytes()[bucket.len() - 1].is_ascii_alphanumeric()
                || bucket.contains("..")
                || bucket.parse::<std::net::IpAddr>().is_ok()
        }) || self.prefix.len() > 512
            || (!self.prefix.is_empty()
                && (!self.prefix.ends_with('/')
                    || self
                        .prefix
                        .strip_suffix('/')
                        .unwrap_or_default()
                        .split('/')
                        .any(|part| {
                            part.is_empty()
                                || matches!(part, "." | "..")
                                || !part
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                        })))
        {
            return Err(MailConfigError::ReceiptLocation);
        }
        if !(300..=86400).contains(&self.ttl_seconds)
            || !(1024..=MAX_SUPPORTED_MAIL_BYTES).contains(&self.max_bytes)
            || !(1..=3600).contains(&self.min_send_interval_seconds)
            || !DAILY_RECIPIENT_LIMITS.contains(&self.daily_app_recipient_limit)
            || !DAILY_RECIPIENT_LIMITS.contains(&self.daily_principal_recipient_limit)
        {
            return Err(MailConfigError::Limits);
        }
        if let Some(AutomationOutboundConfig::Smtp {
            host,
            port,
            username,
            password_secret_ref,
            from_name,
            bounce_address,
            ..
        }) = &self.outbound
        {
            if host.is_empty()
                || host.len() > 253
                || host
                    .bytes()
                    .any(|b| !(b.is_ascii_alphanumeric() || b".-:".contains(&b)))
                || port == &Some(0)
                || username.is_empty()
                || username.len() > 1024
                || username.chars().any(char::is_control)
                || password_secret_ref.trim() != password_secret_ref
                || SecretRef::try_from(password_secret_ref.as_str()).is_err()
                || from_name
                    .as_ref()
                    .is_some_and(|name| name.len() > 256 || name.chars().any(char::is_control))
                || bounce_address
                    .as_deref()
                    .is_some_and(|address| !valid_mailbox(address))
            {
                return Err(MailConfigError::Smtp);
            }
        }
        Ok(())
    }

    /// Loads the configuration; an invalid one is logged and leaves mail automation disabled.
    pub(crate) async fn load_or_disabled(fallback: Option<Self>, secrets: &SecretStore) -> Self {
        Self::load(fallback, secrets).await.unwrap_or_else(|error| {
            tracing::error!(%error, "Mail automation is disabled: its configuration is invalid");
            Self::default()
        })
    }

    /// Creates the automation sender; a failure is logged and disables sending only.
    pub(crate) async fn create_client_or_disable(
        &mut self,
        secrets: &SecretStore,
        transactional_config: Option<&flow_like::hub::MailConfig>,
        transactional_client: Option<crate::mail::DynMailClient>,
    ) -> Option<crate::mail::DynMailClient> {
        match self
            .create_client(secrets, transactional_config, transactional_client)
            .await
        {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(%error, "Mail automation sending is disabled: the outbound provider could not be initialized");
                self.sending_enabled = false;
                None
            }
        }
    }

    pub(crate) fn domain(&self) -> Option<&str> {
        self.enabled.then_some(self.domain.as_deref()).flatten()
    }

    pub(crate) fn can_send(&self) -> bool {
        self.enabled && self.sending_enabled
    }

    pub(crate) async fn create_client(
        &self,
        secrets: &SecretStore,
        transactional_config: Option<&flow_like::hub::MailConfig>,
        transactional_client: Option<crate::mail::DynMailClient>,
    ) -> Result<Option<crate::mail::DynMailClient>, MailConfigError> {
        if !self.can_send() {
            return Ok(None);
        }
        match &self.outbound {
            None => {
                let config = transactional_config.ok_or(MailConfigError::Provider)?;
                let provider = crate::mail::runtime_provider_override()
                    .map_err(|_| MailConfigError::Provider)?
                    .unwrap_or(config.provider);
                if provider == flow_like::hub::MailProviderType::AzureCommunicationServices {
                    return Err(MailConfigError::DynamicSender);
                }
                transactional_client
                    .map(Some)
                    .ok_or(MailConfigError::Provider)
            }
            Some(AutomationOutboundConfig::Smtp {
                host,
                port,
                username,
                password_secret_ref,
                tls,
                from_name,
                bounce_address,
            }) => {
                #[cfg(feature = "smtp")]
                {
                    let reference = SecretRef::try_from(password_secret_ref.as_str())
                        .map_err(|_| MailConfigError::Smtp)?;
                    let password = secrets
                        .get_secret_string(&reference)
                        .with_subscriber(tracing::subscriber::NoSubscriber::default())
                        .await
                        .map_err(|_| MailConfigError::Provider)?;
                    if password.expose_secret().is_empty() || password.expose_secret().len() > 4096
                    {
                        return Err(MailConfigError::Provider);
                    }
                    let config = flow_like::hub::MailConfig {
                        provider: flow_like::hub::MailProviderType::Smtp,
                        from_email: format!(
                            "no-reply@{}",
                            self.domain().ok_or(MailConfigError::Domain)?
                        ),
                        from_name: from_name
                            .clone()
                            .or_else(|| transactional_config.map(|config| config.from_name.clone()))
                            .unwrap_or_else(|| "Flow-Like".into()),
                        smtp: None,
                        sendgrid: None,
                    };
                    let port = port.unwrap_or(if *tls == SmtpTls::Starttls { 587 } else { 465 });
                    let mut client = crate::mail::SmtpMailClient::from_settings(
                        &config,
                        host,
                        port,
                        username.clone(),
                        password.expose_secret().to_owned(),
                        *tls == SmtpTls::Starttls,
                    )
                    .map_err(|_| MailConfigError::Provider)?;
                    if let Some(address) = bounce_address {
                        client = client.with_envelope_sender(address.clone());
                    }
                    Ok(Some(std::sync::Arc::new(client)))
                }
                #[cfg(not(feature = "smtp"))]
                {
                    let _ = (
                        secrets,
                        host,
                        port,
                        username,
                        password_secret_ref,
                        tls,
                        from_name,
                        bounce_address,
                    );
                    Err(MailConfigError::Provider)
                }
            }
        }
    }
}

fn optional_secret_error(error: &SecretError) -> Result<(), MailConfigError> {
    if error.is_not_found() {
        Ok(())
    } else {
        Err(MailConfigError::Read)
    }
}

fn valid_mailbox(address: &str) -> bool {
    address.len() <= 254
        && address.split_once('@').is_some_and(|(local, domain)| {
            (1..=64).contains(&local.len())
                && local
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
                && valid_domain(domain)
        })
}

fn valid_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::{
        FileProviderConfig, ProviderConfig, SecretProviderKind, SecretStoreConfig,
    };

    fn parsed(document: &str) -> MailAutomationConfig {
        let mut config = MailAutomationConfig::parse(document).unwrap();
        config.validate().unwrap();
        config
    }

    fn file_store(path: &std::path::Path) -> SecretStore {
        SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: path.to_owned(),
                    trim_trailing_newline: true,
                })),
        )
        .unwrap()
    }

    #[test]
    fn minimal_provisioned_config_has_bounded_defaults_and_absence_is_disabled() {
        let absent = MailAutomationConfig::default();
        assert!(absent.domain().is_none());
        assert!(!absent.can_send());
        let configured =
            parsed(r#"{"enabled":true,"domain":"Mail.Example.com","bucket":"receipts-bucket"}"#);
        assert_eq!(configured.domain(), Some("mail.example.com"));
        assert!(configured.can_send());
        assert_eq!(configured.prefix, "raw/");
        assert_eq!(configured.ttl_seconds, 3600);
        assert_eq!(configured.max_bytes, 10 * 1024 * 1024);
        assert_eq!(configured.min_send_interval_seconds, 5);
        assert_eq!(configured.daily_app_recipient_limit, 500);
        assert_eq!(configured.daily_principal_recipient_limit, 2000);
        let disabled =
            parsed(r#"{"enabled":false,"domain":"mail.example.com","sending_enabled":true}"#);
        assert!(disabled.domain().is_none());
        assert!(!disabled.can_send());
    }

    #[test]
    fn configured_domain_receipt_location_and_limits_are_exact() {
        for document in [
            r#"{"enabled":true}"#,
            r#"{"domain":"*.example.com"}"#,
            r#"{"domain":"https://mail.example.com"}"#,
            r#"{"domain":" mail.example.com"}"#,
            r#"{"bucket":""}"#,
            r#"{"bucket":"192.0.2.1"}"#,
            r#"{"prefix":"../raw/"}"#,
            r#"{"prefix":"/raw/"}"#,
            r#"{"prefix":"raw//"}"#,
            r#"{"prefix":"raw"}"#,
            r#"{"ttl_seconds":299}"#,
            r#"{"ttl_seconds":86401}"#,
            r#"{"max_bytes":41943041}"#,
            r#"{"min_send_interval_seconds":0}"#,
            r#"{"daily_app_recipient_limit":19}"#,
            r#"{"daily_principal_recipient_limit":1000001}"#,
        ] {
            let mut config = MailAutomationConfig::parse(document).unwrap();
            assert!(config.validate().is_err(), "accepted {document}");
        }
        assert_eq!(parsed(r#"{"prefix":""}"#).prefix, "");
    }

    fn environment(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl FnMut(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn legacy_domain_enables_inbound_mail_without_enabling_sending() {
        let mut config = MailAutomationConfig::default();
        config
            .apply_environment(environment(&[("INBOUND_MAIL_DOMAIN", "mail.example.com")]))
            .unwrap();
        config.validate().unwrap();
        assert_eq!(config.domain(), Some("mail.example.com"));
        assert!(!config.can_send());

        let mut config = MailAutomationConfig::default();
        config
            .apply_environment(environment(&[
                ("INBOUND_MAIL_DOMAIN", "mail.example.com"),
                ("MAIL_AUTOMATION_ENABLED", "true"),
            ]))
            .unwrap();
        assert!(config.can_send());

        let mut config = parsed(r#"{"enabled":true,"domain":"configured.example.com"}"#);
        config
            .apply_environment(environment(&[("INBOUND_MAIL_DOMAIN", "mail.example.com")]))
            .unwrap();
        assert!(
            config.can_send(),
            "an explicit document keeps its sending choice"
        );
    }

    #[tokio::test]
    async fn invalid_configuration_disables_mail_instead_of_failing_startup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("MAIL_CONFIG"),
            r#"{"enabled":true,"domain":"mail.example.com","ttl_seconds":1}"#,
        )
        .unwrap();
        let config = MailAutomationConfig::load_or_disabled(None, &file_store(dir.path())).await;
        assert!(config.domain().is_none());
        assert!(!config.can_send());

        let mut config = parsed(r#"{"enabled":true,"domain":"mail.example.com"}"#);
        let store = SecretStore::new(SecretStoreConfig::default()).unwrap();
        assert!(
            config
                .create_client_or_disable(&store, None, None)
                .await
                .is_none()
        );
        assert!(!config.can_send());
        assert_eq!(config.domain(), Some("mail.example.com"));
    }

    #[test]
    fn smtp_bounce_address_must_be_a_plain_mailbox() {
        let document = |bounce: &str| {
            format!(
                r#"{{"outbound":{{"provider":"smtp","host":"relay.example.com","username":"user","password_secret_ref":"PASSWORD","bounce_address":"{bounce}"}}}}"#
            )
        };
        assert!(
            MailAutomationConfig::parse(&document("bounces@example.com"))
                .unwrap()
                .validate()
                .is_ok()
        );
        for bounce in [
            "Bounces <bounces@example.com>",
            "bounces",
            "a@b@example.com",
        ] {
            let mut config = MailAutomationConfig::parse(&document(bounce)).unwrap();
            assert!(config.validate().is_err(), "accepted {bounce}");
        }
    }

    #[test]
    fn legacy_overrides_are_resolved_once_and_validated() {
        let mut config = MailAutomationConfig::default();
        let overrides = [
            ("INBOUND_MAIL_DOMAIN", "mail.example.com"),
            ("INBOUND_MAIL_BUCKET", "receipt-bucket"),
            ("INBOUND_MAIL_TTL_SECONDS", "900"),
            ("MAIL_AUTOMATION_ENABLED", "false"),
            ("MAIL_AUTOMATION_MIN_INTERVAL_SECONDS", "10"),
        ];
        config
            .apply_environment(|name| {
                overrides
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            })
            .unwrap();
        config.validate().unwrap();
        assert!(config.enabled);
        assert!(!config.can_send());
        assert_eq!(config.ttl_seconds, 900);
        assert_eq!(config.min_send_interval_seconds, 10);
        assert!(
            config
                .apply_environment(|name| (name == "INBOUND_MAIL_MAX_BYTES")
                    .then(|| OsString::from("not-a-number")))
                .is_err()
        );
    }

    #[test]
    fn provider_failures_are_not_treated_as_an_absent_optional_document() {
        assert!(
            optional_secret_error(&SecretError::SecretNotFound(SecretProviderKind::File)).is_ok()
        );
        for error in [
            SecretError::provider_failure(
                SecretProviderKind::AwsParameterStore,
                "access denied private-marker",
            ),
            SecretError::NoProvidersConfigured,
            SecretError::SecretValueBinary,
        ] {
            let error = optional_secret_error(&error).unwrap_err();
            assert_eq!(error, MailConfigError::Read);
            assert!(!format!("{error:?} {error}").contains("private-marker"));
        }
    }

    #[tokio::test]
    async fn secret_document_replaces_private_fallback_and_only_missing_can_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let fallback = parsed(r#"{"enabled":true,"domain":"fallback.example.com"}"#);
        let config = MailAutomationConfig::load_with_lookup(
            Some(fallback.clone()),
            &file_store(dir.path()),
            |_| None,
        )
        .await
        .unwrap();
        assert_eq!(config.domain(), Some("fallback.example.com"));
        std::fs::write(
            dir.path().join("MAIL_CONFIG"),
            r#"{"enabled":true,"domain":"configured.example.com"}"#,
        )
        .unwrap();
        let config = MailAutomationConfig::load_with_lookup(
            Some(fallback.clone()),
            &file_store(dir.path()),
            |_| None,
        )
        .await
        .unwrap();
        assert_eq!(config.domain(), Some("configured.example.com"));
        std::fs::write(
            dir.path().join("MAIL_CONFIG"),
            r#"{"private-marker":"secret"}"#,
        )
        .unwrap();
        let result =
            MailAutomationConfig::load_with_lookup(Some(fallback), &file_store(dir.path()), |_| {
                None
            })
            .await;
        assert!(matches!(result, Err(MailConfigError::Schema)));
    }

    #[test]
    fn smtp_override_uses_secret_references_and_explicit_tls_modes() {
        let config = parsed(
            r#"{"enabled":true,"domain":"mail.example.com","outbound":{"provider":"smtp","host":"relay.example.com","username":"apikey","password_secret_ref":"MAIL_SMTP_PASSWORD"}}"#,
        );
        assert!(matches!(
            config.outbound,
            Some(AutomationOutboundConfig::Smtp {
                tls: SmtpTls::Starttls,
                port: None,
                ..
            })
        ));
        for document in [
            r#"{"outbound":{"provider":"smtp","host":"smtp://relay.example.com","username":"user","password_secret_ref":"PASSWORD"}}"#,
            r#"{"outbound":{"provider":"smtp","host":"relay.example.com","username":"user","password_secret_ref":"PASSWORD","port":0}}"#,
            r#"{"outbound":{"provider":"smtp","host":"relay.example.com","username":"user","password_secret_ref":""}}"#,
        ] {
            let mut config = MailAutomationConfig::parse(document).unwrap();
            assert!(config.validate().is_err());
        }
        for document in [
            r#"{"outbound":{"provider":"smtp","host":"relay.example.com","username":"user","password":"literal"}}"#,
            r#"{"outbound":{"provider":"smtp","host":"relay.example.com","username":"user","password_secret_ref":"PASSWORD","tls":"none"}}"#,
        ] {
            assert!(MailAutomationConfig::parse(document).is_err());
        }
    }

    #[tokio::test]
    async fn disabled_automation_does_not_require_an_outbound_client() {
        let store = SecretStore::new(SecretStoreConfig::default()).unwrap();
        assert!(
            MailAutomationConfig::default()
                .create_client(&store, None, None)
                .await
                .unwrap()
                .is_none()
        );
    }
}

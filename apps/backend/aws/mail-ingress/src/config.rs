use flow_like_mail_ingress::{Result, invalid};
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore,
    SecretStoreConfig,
};
use serde::Deserialize;
use std::env;
use tracing::instrument::WithSubscriber;

// This document contains routing settings. The registered token is kept in a
// separate SecureString so its access and rotation remain independent.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub api_base_url: String,
    pub bucket: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    pub topic_arn: String,
    pub dispatch_rule_arn: String,
}

fn default_prefix() -> String {
    "raw/".into()
}

fn required(name: &str) -> Result<String> {
    let value = env::var(name).map_err(|_| invalid(format!("{name} is required")))?;
    if value.trim().is_empty() {
        return Err(invalid(format!("{name} is empty")));
    }
    Ok(value)
}

impl Config {
    fn parse(document: &str) -> Result<Self> {
        if document.len() > 8192 {
            return Err(invalid("MAIL_INGRESS_CONFIG exceeds 8 KiB"));
        }
        let config: Self = serde_json::from_str(document)
            .map_err(|_| invalid("MAIL_INGRESS_CONFIG is invalid"))?;
        config.validate(false)?;
        Ok(config)
    }

    fn validate(&self, allow_insecure: bool) -> Result<()> {
        if !self.api_base_url.starts_with("https://")
            && !(allow_insecure && self.api_base_url.starts_with("http://"))
        {
            return Err(invalid("Mail ingress API must use HTTPS"));
        }
        if self.bucket.len() < 3
            || self.bucket.len() > 63
            || !self
                .bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            || self.prefix.is_empty()
            || self.prefix.len() > 128
            || !self.prefix.ends_with('/')
            || self.prefix.starts_with('/')
            || self
                .prefix
                .split('/')
                .any(|part| part == "." || part == "..")
            || !self
                .prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
        {
            return Err(invalid("Mail ingress bucket or prefix is invalid"));
        }
        let topic: Vec<_> = self.topic_arn.splitn(6, ':').collect();
        let rule: Vec<_> = self.dispatch_rule_arn.splitn(6, ':').collect();
        if topic.len() != 6
            || rule.len() != 6
            || topic[0] != "arn"
            || rule[0] != "arn"
            || topic[1].is_empty()
            || topic[1] != rule[1]
            || topic[2] != "sns"
            || rule[2] != "events"
            || topic[3].is_empty()
            || topic[3] != rule[3]
            || topic[4].len() != 12
            || !topic[4].bytes().all(|b| b.is_ascii_digit())
            || topic[4] != rule[4]
            || topic[5].is_empty()
            || !rule[5].starts_with("rule/")
            || rule[5].len() <= 5
        {
            return Err(invalid("Mail ingress source ARNs are invalid"));
        }
        Ok(())
    }

    async fn from_store(store: &SecretStore) -> Result<(Self, String)> {
        let document = store
            .get_secret_string(&SecretRef::new("MAIL_INGRESS_CONFIG"))
            .with_subscriber(tracing::subscriber::NoSubscriber::default())
            .await
            .map_err(|_| invalid("MAIL_INGRESS_CONFIG could not be loaded"))?;
        let config = Self::parse(document.expose_secret())?;
        let token = store
            .get_secret_string(&SecretRef::new("SINK_TRIGGER_JWT"))
            .with_subscriber(tracing::subscriber::NoSubscriber::default())
            .await
            .map_err(|_| invalid("Mail ingress token could not be loaded"))?;
        let token = token.expose_secret().trim();
        if token.is_empty() || token.len() > 16384 {
            return Err(invalid("Mail ingress token is invalid"));
        }
        Ok((config, token.to_owned()))
    }

    pub async fn load() -> Result<(Self, String)> {
        match env::var("SECRET_PREFIX") {
            Ok(prefix) => {
                if prefix.trim().is_empty() || prefix.trim() != prefix {
                    return Err(invalid("SECRET_PREFIX is invalid"));
                }
                let store = SecretStore::new(
                    SecretStoreConfig::default()
                        .with_allow_env_override(false)
                        .with_provider(ProviderConfig::AwsParameterStore(
                            AwsParameterStoreProviderConfig {
                                prefix: Some(prefix),
                                with_decryption: true,
                                ..Default::default()
                            },
                        )),
                )
                .map_err(|_| invalid("Mail ingress parameter store could not be initialized"))?;
                Self::from_store(&store).await
            }
            Err(env::VarError::NotUnicode(_)) => Err(invalid("SECRET_PREFIX is invalid")),
            // Keep the existing SAM environment contract usable independently.
            Err(env::VarError::NotPresent) => {
                let config = Self {
                    api_base_url: required("API_BASE_URL")?,
                    bucket: required("INBOUND_MAIL_BUCKET")?,
                    prefix: env::var("INBOUND_MAIL_PREFIX").unwrap_or_else(|_| default_prefix()),
                    topic_arn: required("MAIL_SNS_TOPIC_ARN")?,
                    dispatch_rule_arn: required("MAIL_DISPATCH_RULE_ARN")?,
                };
                config.validate(env::var("ALLOW_INSECURE_API_BASE_URL").as_deref() == Ok("1"))?;
                Ok((config, required("SINK_TRIGGER_JWT")?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_secrets::FileProviderConfig;

    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "api_base_url": "https://api.example.test",
            "bucket": "mail-bucket",
            "topic_arn": "arn:aws:sns:eu-west-1:123456789012:mail",
            "dispatch_rule_arn": "arn:aws:events:eu-west-1:123456789012:rule/mail"
        })
    }

    #[test]
    fn managed_config_requires_https_and_exact_source_scope() {
        assert_eq!(
            Config::parse(&fixture().to_string()).unwrap().prefix,
            "raw/"
        );
        for (key, value) in [
            ("api_base_url", "http://api.example.test"),
            ("bucket", ""),
            ("prefix", "raw/../other/"),
            ("topic_arn", "arn:aws:sns:eu-west-1:999999999999:mail"),
            (
                "dispatch_rule_arn",
                "arn:aws:events:eu-west-2:123456789012:rule/mail",
            ),
            ("unexpected", "private-marker"),
        ] {
            let mut document = fixture();
            document[key] = value.into();
            let error = Config::parse(&document.to_string()).err().unwrap();
            assert!(!error.to_string().contains("private-marker"));
        }
        assert!(Config::parse(&"x".repeat(8193)).is_err());
    }

    #[tokio::test]
    async fn managed_config_and_token_must_both_exist() {
        let directory = tempfile::tempdir().unwrap();
        let store = SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: directory.path().into(),
                    trim_trailing_newline: true,
                })),
        )
        .unwrap();
        std::fs::write(
            directory.path().join("MAIL_INGRESS_CONFIG"),
            fixture().to_string(),
        )
        .unwrap();
        assert!(Config::from_store(&store).await.is_err());
        std::fs::write(directory.path().join("SINK_TRIGGER_JWT"), "scoped-token\n").unwrap();
        store.invalidate(&SecretRef::new("SINK_TRIGGER_JWT")).await;
        let (config, token) = Config::from_store(&store).await.unwrap();
        assert_eq!(config.bucket, "mail-bucket");
        assert_eq!(token, "scoped-token");
    }
}

//! AWS IoT Core transport: STS-minted, per-channel session credentials for both sides and the
//! `SendDirectMessage` forwarder for pushes that arrive on the HTTP fallback.

use std::sync::Arc;

use aws_config::SdkConfig;
use aws_sdk_sts::error::DisplayErrorContext;
use flow_like_channels::ChannelForwarder;
use flow_like_channels::aws::{
    AwsIotForwarder, client_session_policy, executor_client_id, executor_session_policy, topic_for,
    validate_channel_id, validate_topic,
};
use flow_like_types::channel::{
    AwsTemporaryCredentials, ChannelClientDescriptor, ChannelExecutorGrant,
};
use flow_like_types::{Result, anyhow, bail};

use super::issuer::{MintedDescriptor, MintedExecutor};

pub const ENDPOINT_ENV: &str = "CHANNEL_IOT_ENDPOINT";
pub const ROLE_ARN_ENV: &str = "CHANNEL_IOT_ROLE_ARN";
pub const ACCOUNT_ID_ENV: &str = "CHANNEL_IOT_ACCOUNT_ID";
pub const TOPIC_PREFIX_ENV: &str = "CHANNEL_IOT_TOPIC_PREFIX";
pub const DEFAULT_TOPIC_PREFIX: &str = "runs";
/// STS bounds for a chained `AssumeRole` (the API itself runs on a role).
const STS_MIN_DURATION_SECS: i64 = 900;
const STS_MAX_DURATION_SECS: i64 = 3600;
const SESSION_NAME_MAX: usize = 64;

#[derive(Clone, Debug)]
pub struct AwsChannelConfig {
    /// IoT data-plane host (`xxxx-ats.iot.{region}.amazonaws.com`), scheme stripped.
    pub endpoint: String,
    pub role_arn: String,
    pub account_id: Option<String>,
    pub topic_prefix: String,
    pub region: Option<String>,
}

impl AwsChannelConfig {
    pub fn from_env() -> Result<Self> {
        let endpoint = required(ENDPOINT_ENV)?;
        let endpoint = endpoint
            .split_once("://")
            .map_or(endpoint.as_str(), |(_, rest)| rest)
            .trim_end_matches('/')
            .to_string();
        if endpoint.contains('/') {
            bail!("{ENDPOINT_ENV} must be the IoT data endpoint host only, got '{endpoint}'");
        }
        let role_arn = required(ROLE_ARN_ENV)?;
        if !role_arn.starts_with("arn:") {
            bail!("{ROLE_ARN_ENV} must be an IAM role ARN, got '{role_arn}'");
        }
        Ok(Self {
            endpoint,
            role_arn,
            account_id: optional(ACCOUNT_ID_ENV),
            topic_prefix: optional(TOPIC_PREFIX_ENV)
                .unwrap_or_else(|| DEFAULT_TOPIC_PREFIX.to_string()),
            region: optional("AWS_REGION").or_else(|| optional("AWS_DEFAULT_REGION")),
        })
    }
}

fn required(name: &str) -> Result<String> {
    optional(name).ok_or_else(|| anyhow!("{name} is required for the aws_mqtt channel transport"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `RoleSessionName` accepts `[\w+=,.@-]{2,64}`.
pub fn session_name(channel_id: &str) -> String {
    let mut name: String = channel_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '=' | ',' | '.' | '@' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(SESSION_NAME_MAX)
        .collect();
    while name.len() < 2 {
        name.push('-');
    }
    name
}

pub struct AwsChannelRuntime {
    config: AwsChannelConfig,
    sts: aws_sdk_sts::Client,
    account_id: String,
    region: String,
    forwarder: Arc<AwsIotForwarder>,
}

impl AwsChannelRuntime {
    pub async fn from_env(sdk: Arc<SdkConfig>) -> Result<Self> {
        let config = AwsChannelConfig::from_env()?;
        let region = config
            .region
            .clone()
            .or_else(|| sdk.region().map(|region| region.to_string()))
            .ok_or_else(|| anyhow!("AWS_REGION is required for the aws_mqtt channel transport"))?;
        let sts = aws_sdk_sts::Client::new(&sdk);
        let account_id = match config.account_id.clone() {
            Some(account_id) => account_id,
            None => sts
                .get_caller_identity()
                .send()
                .await
                .map_err(|e| {
                    anyhow!(
                        "sts:GetCallerIdentity failed while resolving {ACCOUNT_ID_ENV}: {}",
                        DisplayErrorContext(&e)
                    )
                })?
                .account()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("sts:GetCallerIdentity returned no account id"))?,
        };
        let iot = aws_sdk_iotdataplane::Client::from_conf(
            aws_sdk_iotdataplane::config::Builder::from(sdk.as_ref())
                .endpoint_url(format!("https://{}", config.endpoint))
                .build(),
        );
        let forwarder = Arc::new(AwsIotForwarder::new(iot, config.topic_prefix.clone()));
        Ok(Self {
            config,
            sts,
            account_id,
            region,
            forwarder,
        })
    }

    pub fn forwarder(&self) -> Arc<dyn ChannelForwarder> {
        self.forwarder.clone()
    }

    fn names(&self, channel_id: &str) -> Result<(String, String)> {
        validate_channel_id(channel_id)?;
        let topic = topic_for(&self.config.topic_prefix, channel_id);
        validate_topic(&topic)?;
        Ok((executor_client_id(channel_id), topic))
    }

    async fn assume(
        &self,
        channel_id: &str,
        policy: String,
        ttl_secs: i64,
    ) -> Result<AwsTemporaryCredentials> {
        let duration = ttl_secs.clamp(STS_MIN_DURATION_SECS, STS_MAX_DURATION_SECS) as i32;
        let output = self
            .sts
            .assume_role()
            .role_arn(&self.config.role_arn)
            .role_session_name(session_name(channel_id))
            .policy(policy)
            .duration_seconds(duration)
            .send()
            .await
            .map_err(|e| {
                anyhow!(
                    "sts:AssumeRole on {} for channel '{channel_id}' failed: {}",
                    self.config.role_arn,
                    DisplayErrorContext(&e)
                )
            })?;
        let credentials = output
            .credentials()
            .ok_or_else(|| anyhow!("sts:AssumeRole returned no credentials"))?;
        Ok(AwsTemporaryCredentials {
            access_key_id: credentials.access_key_id().to_string(),
            secret_access_key: credentials.secret_access_key().to_string(),
            session_token: credentials.session_token().to_string(),
            expiration: credentials.expiration().secs(),
        })
    }

    /// Connect / subscribe / receive on exactly this channel's client id and inbox topic.
    pub async fn executor(&self, channel_id: &str, ttl_secs: i64) -> Result<MintedExecutor> {
        let (client_id, topic) = self.names(channel_id)?;
        let policy = executor_session_policy(&self.region, &self.account_id, &client_id, &topic);
        let credentials = self.assume(channel_id, policy, ttl_secs).await?;
        Ok(MintedExecutor {
            expires_at: credentials.expiration,
            grant: ChannelExecutorGrant::AwsMqtt {
                endpoint: self.config.endpoint.clone(),
                region: self.region.clone(),
                client_id,
                inbox_topic: topic,
                credentials,
            },
        })
    }

    /// `iot:SendDirectMessage` to exactly this channel's waiter on exactly its inbox topic.
    pub async fn client(
        &self,
        channel_id: &str,
        _sub: &str,
        ttl_secs: i64,
    ) -> Result<MintedDescriptor> {
        let (client_id, topic) = self.names(channel_id)?;
        let policy = client_session_policy(&self.region, &self.account_id, &client_id, &topic);
        let credentials = self.assume(channel_id, policy, ttl_secs).await?;
        Ok(MintedDescriptor {
            expires_at: credentials.expiration,
            descriptor: ChannelClientDescriptor::AwsMqtt {
                endpoint: self.config.endpoint.clone(),
                region: self.region.clone(),
                target_client_id: client_id,
                topic,
                credentials,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_names_fit_sts() {
        assert_eq!(session_name("run-1"), "run-1");
        assert_eq!(session_name("a"), "a-");
        assert_eq!(session_name("run 1/x"), "run-1-x");
        assert_eq!(session_name(&"x".repeat(100)).len(), SESSION_NAME_MAX);
    }
}

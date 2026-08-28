//! API side: relay a push that arrived on the HTTP fallback to the waiter's IoT client via
//! `SendDirectMessage`.

use aws_sdk_iotdataplane::Client;
use aws_sdk_iotdataplane::error::{DisplayErrorContext, SdkError};
use aws_sdk_iotdataplane::operation::send_direct_message::SendDirectMessageError;
use aws_sdk_iotdataplane::primitives::Blob;
use aws_smithy_runtime_api::client::orchestrator::HttpResponse;
use flow_like_types::async_trait;
use flow_like_types::channel::{CHANNEL_TRANSPORT_AWS_MQTT, ChannelPush};
use flow_like_types::{Result, anyhow};

use super::policy::{executor_client_id, topic_for, validate_channel_id, validate_topic};
use crate::ChannelForwarder;

/// Seconds AWS IoT waits for the waiter's PUBACK before answering 504 (allowed range 1..=15).
pub const DIRECT_MESSAGE_TIMEOUT_SECS: i32 = 10;

/// Built by the API from its `SdkConfig` with `endpoint_url("https://{iot data endpoint}")`.
pub struct AwsIotForwarder {
    client: Client,
    topic_prefix: String,
}

impl AwsIotForwarder {
    pub fn new(client: Client, topic_prefix: impl Into<String>) -> Self {
        Self {
            client,
            topic_prefix: topic_prefix.into(),
        }
    }

    pub fn topic_prefix(&self) -> &str {
        &self.topic_prefix
    }
}

#[async_trait]
impl ChannelForwarder for AwsIotForwarder {
    fn transport(&self) -> &'static str {
        CHANNEL_TRANSPORT_AWS_MQTT
    }

    async fn forward(&self, push: &ChannelPush) -> Result<()> {
        validate_channel_id(&push.channel_id)?;
        let topic = topic_for(&self.topic_prefix, &push.channel_id);
        validate_topic(&topic)?;
        let client_id = executor_client_id(&push.channel_id);
        let payload = serde_json::to_vec(push)?;
        self.client
            .send_direct_message()
            .client_id(client_id.as_str())
            .topic(topic)
            .confirmation(true)
            .timeout(DIRECT_MESSAGE_TIMEOUT_SECS)
            .payload(Blob::new(payload))
            .send()
            .await
            .map(drop)
            .map_err(|error| describe_error(&push.channel_id, &client_id, error))
    }
}

fn describe_error(
    channel_id: &str,
    client_id: &str,
    error: SdkError<SendDirectMessageError, HttpResponse>,
) -> flow_like_types::Error {
    if let SdkError::ServiceError(context) = &error {
        let status = context.raw().status().as_u16();
        let service_error = context.err();
        if status == 404 || service_error.is_resource_not_found_exception() {
            return anyhow!(
                "channel '{channel_id}' has no waiter connected as '{client_id}' (AWS IoT direct message returned 404)"
            );
        }
        if status == 504 || service_error.is_gateway_timeout_exception() {
            return anyhow!(
                "channel '{channel_id}': waiter '{client_id}' did not acknowledge the push within {DIRECT_MESSAGE_TIMEOUT_SECS}s"
            );
        }
    }
    anyhow!(
        "channel '{channel_id}': SendDirectMessage to '{client_id}' failed: {}",
        DisplayErrorContext(&error)
    )
}

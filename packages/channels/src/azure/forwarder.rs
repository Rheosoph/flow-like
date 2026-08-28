//! API-side push fallback: deliver a [`ChannelPush`] into the channel's group with the
//! data-plane REST `:send` call, signed per request with the hub access key.

use flow_like_types::async_trait;
use flow_like_types::channel::{CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB, ChannelPush};
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;

use super::token::{group_for, normalize_endpoint, rest_token, send_to_group_url};
use crate::ChannelForwarder;

const REST_TOKEN_TTL_SECS: i64 = 60;
const ERROR_BODY_PREVIEW: usize = 512;

pub struct AzureWebPubSubForwarder {
    endpoint: String,
    hub: String,
    access_key: String,
    client: reqwest::Client,
}

impl AzureWebPubSubForwarder {
    pub fn new(
        endpoint: impl Into<String>,
        hub: impl Into<String>,
        access_key: impl Into<String>,
    ) -> Self {
        Self::with_client(endpoint, hub, access_key, reqwest::Client::new())
    }

    pub fn with_client(
        endpoint: impl Into<String>,
        hub: impl Into<String>,
        access_key: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            endpoint: normalize_endpoint(&endpoint.into()).to_string(),
            hub: hub.into(),
            access_key: access_key.into(),
            client,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn hub(&self) -> &str {
        &self.hub
    }

    pub fn send_url(&self, group: &str) -> String {
        send_to_group_url(&self.endpoint, &self.hub, group)
    }
}

#[async_trait]
impl ChannelForwarder for AzureWebPubSubForwarder {
    fn transport(&self) -> &'static str {
        CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB
    }

    async fn forward(&self, push: &ChannelPush) -> flow_like_types::Result<()> {
        let group = group_for(&push.channel_id)?;
        let url = self.send_url(&group);
        let token = rest_token(&self.access_key, &url, REST_TOKEN_TTL_SECS)?;
        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header(CONTENT_TYPE, "application/json")
            .json(push)
            .send()
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "channel {}: Azure Web PubSub send to group {group} failed: {e}",
                    push.channel_id
                )
            })?;
        let status = response.status();
        if status == StatusCode::ACCEPTED {
            return Ok(());
        }
        let body: String = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(ERROR_BODY_PREVIEW)
            .collect();
        flow_like_types::bail!(
            "channel {}: Azure Web PubSub send to group {group} returned {status}: {body}",
            push.channel_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarder_targets_encoded_group_send_url() {
        let forwarder =
            AzureWebPubSubForwarder::new("https://demo.webpubsub.azure.com/", "hub", "key");
        assert_eq!(forwarder.endpoint(), "https://demo.webpubsub.azure.com");
        assert_eq!(forwarder.transport(), "azure_web_pubsub");
        assert_eq!(
            forwarder.send_url(&group_for("abc").unwrap()),
            "https://demo.webpubsub.azure.com/api/hubs/hub/groups/run%3Aabc/:send?api-version=2024-12-01"
        );
    }
}

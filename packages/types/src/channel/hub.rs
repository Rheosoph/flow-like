//! [`ChannelStore`] over the API's `/api/v1/channels` surface, for executors that have no
//! database. Every call is a short request; the executor never holds a connection open.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::polling::{ChannelPoll, ChannelStore};
use crate::Value;
use crate::async_trait;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterMessageRequest {
    pub request_id: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollMessageResponse {
    Pending,
    Responded { value: Value },
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrainInboundResponse {
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatusResponse {
    pub cancelled: bool,
}

/// Every hub call is a short control-plane request. Without a ceiling a hung API leaves the run
/// blocked inside `open` or `poll`, where the ticket's own deadline cannot reach it.
const HUB_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HUB_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct HubChannelStore {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl HubChannelStore {
    pub fn new(hub_url: &str, token: impl Into<String>) -> Self {
        Self {
            base_url: format!("{}/api/v1/channels", hub_url.trim_end_matches('/')),
            token: token.into(),
            client: reqwest::Client::builder()
                .connect_timeout(HUB_CONNECT_TIMEOUT)
                .timeout(HUB_REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn url(&self, channel_id: &str, suffix: &str) -> String {
        format!("{}/{}{}", self.base_url, channel_id, suffix)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> crate::Result<reqwest::Response> {
        let response = request.bearer_auth(&self.token).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(crate::anyhow!(
                "channel hub request failed with {status}: {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        Ok(response)
    }
}

#[async_trait]
impl ChannelStore for HubChannelStore {
    async fn register(
        &self,
        channel_id: &str,
        request_id: &str,
        expires_at: i64,
    ) -> crate::Result<()> {
        self.send(self.client.post(self.url(channel_id, "/messages")).json(
            &RegisterMessageRequest {
                request_id: request_id.to_string(),
                expires_at,
            },
        ))
        .await?;
        Ok(())
    }

    async fn poll(&self, channel_id: &str, request_id: &str) -> crate::Result<ChannelPoll> {
        let response = self
            .send(
                self.client
                    .get(self.url(channel_id, &format!("/messages/{request_id}"))),
            )
            .await?;
        Ok(match response.json::<PollMessageResponse>().await? {
            PollMessageResponse::Pending => ChannelPoll::Pending,
            PollMessageResponse::Responded { value } => ChannelPoll::Responded(value),
            PollMessageResponse::Missing => ChannelPoll::Missing,
        })
    }

    async fn remove(&self, channel_id: &str, request_id: &str) -> crate::Result<()> {
        self.send(
            self.client
                .delete(self.url(channel_id, &format!("/messages/{request_id}"))),
        )
        .await?;
        Ok(())
    }

    async fn drain_inbound(&self, channel_id: &str) -> crate::Result<Vec<Value>> {
        let response = self
            .send(self.client.post(self.url(channel_id, "/inbound/drain")))
            .await?;
        Ok(response.json::<DrainInboundResponse>().await?.messages)
    }

    async fn is_cancelled(&self, channel_id: &str) -> crate::Result<bool> {
        let response = self
            .send(self.client.get(self.url(channel_id, "/status")))
            .await?;
        Ok(response.json::<ChannelStatusResponse>().await?.cancelled)
    }

    async fn close(&self, channel_id: &str) -> crate::Result<()> {
        self.send(self.client.delete(self.url(channel_id, "")))
            .await?;
        Ok(())
    }
}

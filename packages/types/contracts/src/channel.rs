//! Wire contracts for Channels: the run ⇄ client reply primitive.
//!
//! A waiter (executor, API chat loop, desktop run) sends a request to the client over the run's
//! existing event stream and blocks until the client pushes a reply through the channel's
//! transport. Every request carries a [`ChannelHandle`] telling the client how to reply; the API
//! mints the transport credentials at dispatch and ships them to the executor as a
//! [`ChannelGrant`] inside the execution payload.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire tag of the default transport: an authenticated POST to the API's push endpoint.
pub const CHANNEL_TRANSPORT_HTTP: &str = "http";
pub const CHANNEL_TRANSPORT_IN_PROCESS: &str = "in_process";
pub const CHANNEL_TRANSPORT_AWS_MQTT: &str = "aws_mqtt";
pub const CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB: &str = "azure_web_pubsub";
pub const CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB: &str = "gcp_firebase_rtdb";

/// Temporary AWS credentials scoped to one channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct AwsTemporaryCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    /// Unix seconds.
    pub expiration: i64,
}

/// How a client delivers a [`ChannelPush`] for a channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelClientDescriptor {
    /// `POST {push_url}` with `Authorization: Bearer {token}` and a [`ChannelPush`] body.
    Http { push_url: String, token: String },
    /// Same process as the waiter (desktop): delivered through the host's `channel_push` command.
    InProcess {},
    /// AWS IoT Core Direct Messaging: `SendDirectMessage` to `target_client_id` on `topic`,
    /// SigV4-signed with `credentials` (scoped to exactly that client id and topic).
    AwsMqtt {
        /// IoT data-plane host, e.g. `xxxx-ats.iot.eu-central-1.amazonaws.com`.
        endpoint: String,
        region: String,
        target_client_id: String,
        topic: String,
        credentials: AwsTemporaryCredentials,
    },
    /// Azure Web PubSub: connect to `url` (client access token embedded) with the
    /// `json.webpubsub.azure.v1` subprotocol and `sendToGroup(group)`.
    AzureWebPubSub {
        url: String,
        group: String,
        /// Unix seconds after which the token no longer opens new connections.
        expires_at: i64,
    },
    /// Firebase Realtime Database: `signInWithCustomToken(custom_token)` on an app initialised
    /// with `api_key`/`database_url`, then create `{inbox_path}/{request_id}` (replies) or
    /// push under `inbound_path` (unsolicited messages). Rules allow exactly one `payload` field.
    GcpFirebaseRtdb {
        database_url: String,
        api_key: String,
        project_id: String,
        custom_token: String,
        inbox_path: String,
        inbound_path: String,
        expires_at: i64,
    },
}

impl ChannelClientDescriptor {
    pub fn transport(&self) -> &'static str {
        match self {
            Self::Http { .. } => CHANNEL_TRANSPORT_HTTP,
            Self::InProcess {} => CHANNEL_TRANSPORT_IN_PROCESS,
            Self::AwsMqtt { .. } => CHANNEL_TRANSPORT_AWS_MQTT,
            Self::AzureWebPubSub { .. } => CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
            Self::GcpFirebaseRtdb { .. } => CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
        }
    }
}

/// Everything a client needs to answer one request (or, with `request_id: None`, to push an
/// unsolicited message such as cancel/steer into the channel). Embedded verbatim in every
/// request the waiter streams to the client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ChannelHandle {
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Unix seconds; the waiter stops listening after this.
    pub expires_at: i64,
    pub transport: ChannelClientDescriptor,
    /// Used when `transport` cannot be reached; always the API push endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ChannelClientDescriptor>,
}

impl ChannelHandle {
    pub fn for_request(&self, request_id: &str, expires_at: i64) -> Self {
        Self {
            channel_id: self.channel_id.clone(),
            request_id: Some(request_id.to_string()),
            expires_at,
            transport: self.transport.clone(),
            fallback: self.fallback.clone(),
        }
    }
}

/// Meaning of a client-to-waiter message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelPushKind {
    /// Answer to `request_id`.
    #[default]
    Reply,
    /// Unsolicited message the waiter drains at its next boundary (e.g. steering text).
    Inbound,
    /// Stop the run; idempotent.
    Cancel,
}

/// The one message shape every transport carries from the client to the waiter. Identical to
/// the HTTP push body so the API can forward it verbatim onto any transport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ChannelPush {
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub kind: ChannelPushKind,
    #[serde(default)]
    pub value: Value,
}

/// Waiter-side credentials for one channel, minted by the API at dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelExecutorGrant {
    /// Register/poll rows through the API (`profile.hub` + the run's token).
    Http {},
    /// Hold one MQTT-over-WebSocket connection as `client_id` and receive on `inbox_topic`.
    AwsMqtt {
        endpoint: String,
        region: String,
        client_id: String,
        inbox_topic: String,
        credentials: AwsTemporaryCredentials,
    },
    /// Join `group` on a `json.webpubsub.azure.v1` connection and receive group messages.
    AzureWebPubSub { url: String, group: String },
    /// Sign in with `custom_token` and stream `inbox_path` / `inbound_path` over REST SSE.
    GcpFirebaseRtdb {
        database_url: String,
        api_key: String,
        custom_token: String,
        inbox_path: String,
        inbound_path: String,
    },
}

/// Rides `DispatchPayload.channel` / `ExecutionRequest.channel`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ChannelGrant {
    pub channel_id: String,
    /// Unix seconds; credentials in both grants are valid until at least this time.
    pub expires_at: i64,
    pub executor: ChannelExecutorGrant,
    /// Channel-level client handle (`request_id: None`), forwarded inside every request.
    pub client: ChannelHandle,
}

impl ChannelGrant {
    pub fn transport(&self) -> &'static str {
        match &self.executor {
            ChannelExecutorGrant::Http {} => CHANNEL_TRANSPORT_HTTP,
            ChannelExecutorGrant::AwsMqtt { .. } => CHANNEL_TRANSPORT_AWS_MQTT,
            ChannelExecutorGrant::AzureWebPubSub { .. } => CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
            ChannelExecutorGrant::GcpFirebaseRtdb { .. } => CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_defaults_to_reply() {
        let push: ChannelPush =
            serde_json::from_str(r#"{"channel_id":"c","request_id":"r","value":1}"#).unwrap();
        assert_eq!(push.kind, ChannelPushKind::Reply);
        assert_eq!(push.value, Value::from(1));
    }

    #[test]
    fn descriptor_is_type_tagged() {
        let handle = ChannelHandle {
            channel_id: "run".into(),
            request_id: None,
            expires_at: 1,
            transport: ChannelClientDescriptor::Http {
                push_url: "https://api/api/v1/channels/run/push".into(),
                token: "t".into(),
            },
            fallback: None,
        };
        let json = serde_json::to_value(&handle).unwrap();
        assert_eq!(json["transport"]["type"], "http");
        assert!(json.get("request_id").is_none());
        let back: ChannelHandle = serde_json::from_value(json).unwrap();
        assert_eq!(back, handle);
    }
}

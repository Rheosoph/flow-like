//! AWS IoT Core transport.
//!
//! Waiter side: [`AwsIotChannel`] holds one MQTT-over-WebSocket connection to the account's IoT
//! data endpoint as `run-{channel_id}` and receives [`ChannelPush`](flow_like_types::channel::ChannelPush)
//! payloads on the channel's inbox topic, either through its subscription or through IoT Core
//! Direct Messaging, which delivers a PUBLISH to a connected client id without any subscription.
//!
//! API side: [`AwsIotForwarder`] relays pushes that arrived on the HTTP fallback with
//! `SendDirectMessage`, and the policy builders produce the inline session policies for the two
//! STS grants. The API mints both with its own `aws-sdk-sts` client:
//!
//! ```text
//! sts.assume_role()
//!     .role_arn(CHANNEL_IOT_ROLE_ARN)
//!     .role_session_name(channel_id)
//!     .policy(client_session_policy(..) /* or executor_session_policy(..) */)
//!     .duration_seconds(3600)
//! ```

mod channel;
mod forwarder;
mod policy;
mod presign;
mod router;

pub use channel::AwsIotChannel;
pub use forwarder::{AwsIotForwarder, DIRECT_MESSAGE_TIMEOUT_SECS};
pub use policy::{
    EXECUTOR_CLIENT_ID_PREFIX, client_session_policy, executor_client_id, executor_session_policy,
    topic_for, validate_channel_id, validate_topic,
};
pub use presign::{IOT_SIGNING_SERVICE, PRESIGN_EXPIRES_IN, mqtt_wss_url, presign_wss_url};

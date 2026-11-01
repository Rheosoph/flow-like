//! NATS run inboxes with expiring, per-channel credentials and receiver acknowledgements.

mod channel;
mod forwarder;
mod proxy;
mod router;
mod token;

pub use channel::NatsChannel;
pub use forwarder::{NatsForwarder, publish_confirmed};
pub use token::{
    Role, mint_realtime_publisher_token, mint_realtime_subscriber_token, mint_user_token,
    mint_user_token_until, realtime_subject, subject_for, validate_account_seed, validate_url,
};

use flow_like_types::channel::ChannelPush;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const ACK_TIMEOUT: Duration = Duration::from_secs(3);
/// About a minute of async-nats reconnect backoff. Unbounded, a client whose credential has
/// expired would retry forever, even after it was dropped.
const MAX_RECONNECTS: usize = 30;
/// API-side clients retry only briefly; their owner reconnects with a fresh credential.
const API_MAX_RECONNECTS: usize = 3;

#[derive(Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub push: ChannelPush,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Acknowledgement {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Acknowledgement {
    fn accepted() -> Self {
        Self {
            accepted: true,
            error: None,
        }
    }

    fn rejected(error: &str) -> Self {
        Self {
            accepted: false,
            error: Some(error.to_owned()),
        }
    }
}

pub fn valid_delivery_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

pub async fn connect(url: &str, token: &str) -> flow_like_types::Result<async_nats::Client> {
    let token = token.to_owned();
    connect_with(url, MAX_RECONNECTS, CONNECT_TIMEOUT, move || {
        Ok(token.clone())
    })
    .await
}

/// `credential` runs on every (re)connect, so a reconnect can present a freshly minted JWT.
pub(crate) async fn connect_with(
    url: &str,
    max_reconnects: usize,
    timeout: Duration,
    credential: impl Fn() -> flow_like_types::Result<String> + Send + Sync + 'static,
) -> flow_like_types::Result<async_nats::Client> {
    validate_url(url)?;
    let options = async_nats::ConnectOptions::with_auth_callback(move |_| {
        let jwt =
            credential().map_err(|_| async_nats::AuthError::new("NATS credential unavailable"));
        async move {
            let mut auth = async_nats::Auth::new();
            auth.jwt = Some(jwt?);
            Ok(auth)
        }
    })
    .name("flow-like-channel")
    .tls_client_config(crate::tls::client_config()?.as_ref().clone())
    .ignore_discovered_servers()
    .ping_interval(Duration::from_secs(10))
    .max_reconnects(max_reconnects)
    .connection_timeout(timeout);
    tokio::time::timeout(timeout, options.connect(url))
        .await
        .map_err(|_| flow_like_types::anyhow!("NATS channel connection timed out"))?
        .map_err(|_| flow_like_types::anyhow!("NATS channel connection failed"))
}

#[cfg(test)]
mod tests;

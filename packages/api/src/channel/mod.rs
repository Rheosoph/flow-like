//! API side of Channels (`todo/channels.md`): transport selection, credential minting, the
//! Postgres row store behind the HTTP transport, push forwarding and row sweeping.
//!
//! `CHANNEL_TRANSPORT` selects the transport for every run this API dispatches:
//!
//! | value | feature | API-to-waiter path |
//! |---|---|---|
//! | `http` (default) | none | Database row polling |
//! | `aws_mqtt` | `channel-aws` | IoT Core `SendDirectMessage` |
//! | `azure_web_pubsub` | `channel-azure` | Web PubSub `sendToGroup` |
//! | `gcp_firebase_rtdb` | `channel-gcp` | Realtime Database write |
//! | `nats` | `channel-nats` | NATS publish with executor acknowledgement |
//!
//! Remote clients post to `POST /api/v1/channels/{cid}/push`. The responder token binds each
//! push to its channel and waiter transport. The API forwards cloud-channel input through its
//! configured provider. Existing direct provider handles can still use their HTTP fallback;
//! refreshing one returns an HTTP handle with the original capability and deadline.
//!
//! Every dispatched run gets a grant, whoever triggered it — which nodes a board runs is not
//! something dispatch can know in advance. What the trigger decides is how much that grant is
//! worth minting: an attended run (`DispatchTrigger::User` — a page, a quick event, a board run
//! from the editor) gets the transport above, an unattended one (a schedule, a webhook, a bot, an
//! inbound REST/MCP call) gets the HTTP grant, which mints locally instead of spending a
//! round-trip on credentials for a client that will never connect. The executor opens the
//! connection only when a node actually asks its client something.
//!
//! `CHANNEL_TTL_SECONDS` (default 3600) caps a channel's life, and with it how long a run whose
//! trigger left no client behind can sit waiting for a reply; the executor JWT's own expiry caps
//! it further.

#[cfg(feature = "channel-aws")]
pub mod aws;
#[cfg(feature = "channel-azure")]
pub mod azure;
pub mod forward;
#[cfg(feature = "channel-gcp")]
pub mod gcp;
pub mod issuer;
#[cfg(feature = "channel-nats")]
pub mod nats;
pub mod store;
pub mod sweep;

pub use forward::{ForwardOutcome, forward_push, forward_push_with_id};
pub use issuer::ChannelIssuer;
pub use store::DbChannelStore;
pub use sweep::{ChannelSweeperConfig, spawn_channel_sweeper, sweep_expired};

use flow_like_types::channel::{
    CHANNEL_TRANSPORT_AWS_MQTT, CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
    CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB, CHANNEL_TRANSPORT_HTTP, CHANNEL_TRANSPORT_NATS,
};

pub const CHANNEL_TRANSPORT_ENV: &str = "CHANNEL_TRANSPORT";

/// Transport this API mints grants for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChannelBackend {
    #[default]
    Http,
    #[cfg(feature = "channel-aws")]
    AwsMqtt,
    #[cfg(feature = "channel-azure")]
    AzureWebPubSub,
    #[cfg(feature = "channel-gcp")]
    GcpFirebaseRtdb,
    #[cfg(feature = "channel-nats")]
    Nats,
}

impl ChannelBackend {
    /// Reads `CHANNEL_TRANSPORT`; an unusable value logs and falls back to HTTP. Cloud API
    /// binaries validate the variable at boot (see their `config.rs`) so they never get here
    /// with a value this build cannot serve.
    pub fn from_env() -> Self {
        let raw = std::env::var(CHANNEL_TRANSPORT_ENV).ok();
        match Self::parse(raw.as_deref()) {
            Ok(backend) => backend,
            Err(error) => {
                tracing::error!(%error, "falling back to the HTTP channel transport");
                Self::Http
            }
        }
    }

    /// Strict parse: `Err` names a transport this build does not carry or an unknown value.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let value = value.unwrap_or_default().trim().to_ascii_lowercase();
        match value.as_str() {
            "" | "http" | "default" => Ok(Self::Http),
            #[cfg(feature = "channel-aws")]
            "aws_mqtt" | "aws" | "iot" => Ok(Self::AwsMqtt),
            #[cfg(feature = "channel-azure")]
            "azure_web_pubsub" | "azure" | "webpubsub" => Ok(Self::AzureWebPubSub),
            #[cfg(feature = "channel-gcp")]
            "gcp_firebase_rtdb" | "gcp" | "firebase" => Ok(Self::GcpFirebaseRtdb),
            #[cfg(feature = "channel-nats")]
            "nats" => Ok(Self::Nats),
            other if CLOUD_TRANSPORT_ALIASES.contains(&other) => Err(format!(
                "{CHANNEL_TRANSPORT_ENV}={other} names a channel transport that is not compiled into this binary"
            )),
            other => Err(format!(
                "{CHANNEL_TRANSPORT_ENV}={other} is not a channel transport (expected http, aws_mqtt, azure_web_pubsub, gcp_firebase_rtdb or nats)"
            )),
        }
    }

    pub fn transport(&self) -> &'static str {
        match self {
            Self::Http => CHANNEL_TRANSPORT_HTTP,
            #[cfg(feature = "channel-aws")]
            Self::AwsMqtt => CHANNEL_TRANSPORT_AWS_MQTT,
            #[cfg(feature = "channel-azure")]
            Self::AzureWebPubSub => CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
            #[cfg(feature = "channel-gcp")]
            Self::GcpFirebaseRtdb => CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
            #[cfg(feature = "channel-nats")]
            Self::Nats => CHANNEL_TRANSPORT_NATS,
        }
    }

    pub fn is_http(&self) -> bool {
        matches!(self, Self::Http)
    }
}

/// Every spelling `parse` accepts for a cloud transport, so a build without that transport can
/// tell "not compiled" from "not a transport".
const CLOUD_TRANSPORT_ALIASES: &[&str] = &[
    "aws_mqtt",
    "aws",
    "iot",
    "azure_web_pubsub",
    "azure",
    "webpubsub",
    "gcp_firebase_rtdb",
    "gcp",
    "firebase",
    "nats",
];

/// The name every known transport is spelled with on the wire, whether or not this build
/// carries it. Lets the strict parse distinguish "not compiled" from "not a transport".
pub const KNOWN_TRANSPORTS: &[&str] = &[
    CHANNEL_TRANSPORT_HTTP,
    CHANNEL_TRANSPORT_AWS_MQTT,
    CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
    CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
    CHANNEL_TRANSPORT_NATS,
];

pub(crate) fn api_base_url() -> String {
    std::env::var("PUBLIC_API_URL")
        .or_else(|_| std::env::var("API_BASE_URL"))
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "http://localhost:8080".to_string())
}

pub(crate) fn push_url(api_base_url: &str, channel_id: &str) -> String {
    format!(
        "{}/api/v1/channels/{channel_id}/push",
        api_base_url.trim_end_matches('/')
    )
}

/// Provider errors can embed request URLs whose query or user info carries a credential
/// (a Firebase `access_token`, reqwest's `for url (…)` suffix). Scheme, host and path remain.
pub(crate) fn redact_provider_error(error: &flow_like_types::Error) -> String {
    let message = format!("{error:#}");
    let mut redacted = String::with_capacity(message.len());
    let mut rest = message.as_str();
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + 3);
        redacted.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\'' | '<' | '>'))
            .unwrap_or(tail.len());
        let (url, after) = tail.split_at(end);
        let location = url.split(['?', '#']).next().unwrap_or_default();
        let (authority, path) = location.split_at(location.find('/').unwrap_or(location.len()));
        redacted.push_str(
            authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host),
        );
        redacted.push_str(path);
        if location.len() < url.len() {
            redacted.push_str("?<redacted>");
        }
        rest = after;
    }
    redacted.push_str(rest);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_is_the_default_and_accepts_aliases() {
        assert_eq!(ChannelBackend::parse(None).unwrap(), ChannelBackend::Http);
        assert_eq!(
            ChannelBackend::parse(Some("")).unwrap(),
            ChannelBackend::Http
        );
        assert_eq!(
            ChannelBackend::parse(Some(" HTTP ")).unwrap(),
            ChannelBackend::Http
        );
        assert_eq!(
            ChannelBackend::parse(Some("default")).unwrap(),
            ChannelBackend::Http
        );
        assert_eq!(ChannelBackend::Http.transport(), CHANNEL_TRANSPORT_HTTP);
        assert!(ChannelBackend::Http.is_http());
    }

    #[test]
    fn unknown_values_are_rejected_with_the_variable_name() {
        let error = ChannelBackend::parse(Some("carrier-pigeon")).unwrap_err();
        assert!(error.contains(CHANNEL_TRANSPORT_ENV), "{error}");
        assert!(error.contains("carrier-pigeon"), "{error}");
    }

    #[test]
    fn known_transports_parse_only_when_compiled() {
        for name in KNOWN_TRANSPORTS {
            let parsed = ChannelBackend::parse(Some(name));
            match *name {
                CHANNEL_TRANSPORT_HTTP => assert!(parsed.is_ok()),
                CHANNEL_TRANSPORT_AWS_MQTT => {
                    assert_eq!(parsed.is_ok(), cfg!(feature = "channel-aws"), "{name}")
                }
                CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB => {
                    assert_eq!(parsed.is_ok(), cfg!(feature = "channel-azure"), "{name}")
                }
                CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB => {
                    assert_eq!(parsed.is_ok(), cfg!(feature = "channel-gcp"), "{name}")
                }
                CHANNEL_TRANSPORT_NATS => {
                    assert_eq!(parsed.is_ok(), cfg!(feature = "channel-nats"), "{name}")
                }
                _ => unreachable!(),
            }
            if let Ok(backend) = parsed {
                assert_eq!(backend.transport(), *name);
            } else {
                let error = parsed.unwrap_err();
                assert!(error.contains("not compiled"), "{error}");
            }
        }
    }

    #[test]
    fn push_url_shape() {
        assert_eq!(
            push_url("https://api.test/", "run-1"),
            "https://api.test/api/v1/channels/run-1/push"
        );
    }

    #[test]
    fn provider_error_urls_lose_their_credentials() {
        let reqwest_style = flow_like_types::anyhow!(
            "firebase POST channels/run-1/inbound failed: error sending request for url (https://db.example.test/channels/run-1/inbound.json?access_token=ya29.secret&print=silent)"
        );
        assert_eq!(
            redact_provider_error(&reqwest_style),
            "firebase POST channels/run-1/inbound failed: error sending request for url (https://db.example.test/channels/run-1/inbound.json?<redacted>)"
        );
        let user_info = flow_like_types::anyhow!("connect wss://user:secret@nats.example.test/ws");
        assert_eq!(
            redact_provider_error(&user_info),
            "connect wss://nats.example.test/ws"
        );
        let chained = flow_like_types::anyhow!("401 from https://h.test/p#sig=secret end")
            .context("realtime publish failed");
        assert_eq!(
            redact_provider_error(&chained),
            "realtime publish failed: 401 from https://h.test/p?<redacted> end"
        );
        let plain = flow_like_types::anyhow!("NATS channel has no connected waiter");
        assert_eq!(
            redact_provider_error(&plain),
            "NATS channel has no connected waiter"
        );
    }
}

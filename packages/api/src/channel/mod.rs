//! API side of Channels (`todo/channels.md`): transport selection, credential minting, the
//! Postgres row store behind the HTTP transport, push forwarding and row sweeping.
//!
//! `CHANNEL_TRANSPORT` selects the transport for every run this API dispatches:
//!
//! | value | feature | client reply path |
//! |---|---|---|
//! | `http` (default) | — | `POST /api/v1/channels/{cid}/push` |
//! | `aws_mqtt` | `channel-aws` | IoT Core `SendDirectMessage` |
//! | `azure_web_pubsub` | `channel-azure` | Web PubSub `sendToGroup` |
//! | `gcp_firebase_rtdb` | `channel-gcp` | Realtime Database write |
//!
//! Every non-HTTP handle carries the HTTP push endpoint as its fallback; a push arriving there
//! for a cloud-transport channel is forwarded onto that transport.
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
pub mod store;
pub mod sweep;

pub use forward::{ForwardOutcome, forward_push};
pub use issuer::ChannelIssuer;
pub use store::DbChannelStore;
pub use sweep::{ChannelSweeperConfig, spawn_channel_sweeper, sweep_expired};

use flow_like_types::channel::{
    CHANNEL_TRANSPORT_AWS_MQTT, CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
    CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB, CHANNEL_TRANSPORT_HTTP,
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
            other if CLOUD_TRANSPORT_ALIASES.contains(&other) => Err(format!(
                "{CHANNEL_TRANSPORT_ENV}={other} names a channel transport that is not compiled into this binary"
            )),
            other => Err(format!(
                "{CHANNEL_TRANSPORT_ENV}={other} is not a channel transport (expected http, aws_mqtt, azure_web_pubsub or gcp_firebase_rtdb)"
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
];

/// The name every known transport is spelled with on the wire, whether or not this build
/// carries it. Lets the strict parse distinguish "not compiled" from "not a transport".
pub const KNOWN_TRANSPORTS: &[&str] = &[
    CHANNEL_TRANSPORT_HTTP,
    CHANNEL_TRANSPORT_AWS_MQTT,
    CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB,
    CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
];

pub(crate) fn api_base_url() -> String {
    std::env::var("API_BASE_URL")
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
}

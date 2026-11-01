//! Account-scoped notification snapshots and independently replayable run subscriptions.
//! Payloads are opaque to connection owners. Authentication and stream authorization belong
//! to the API; a local subscription only controls which tab or window receives a response.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

use crate::channel::AwsTemporaryCredentials;

pub const REALTIME_PROTOCOL_VERSION: u8 = 1;
pub const MAX_REALTIME_RUNS: usize = 16;
pub const MAX_REALTIME_EVENTS: usize = 100;
pub const MAX_REALTIME_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REALTIME_TIMEOUT_MS: u64 = 20_000;
pub const MAX_REALTIME_WAKEUP_BYTES: usize = 4096;
pub const MAX_REALTIME_POLL_TOKEN_BYTES: usize = 8192;

/// Transient replay failure. Owners keep the subscription and retry with backoff.
pub const REALTIME_ERROR_UNAVAILABLE: &str = "unavailable";
/// The run does not exist or this account cannot read it. Owners drop the subscription.
pub const REALTIME_ERROR_NOT_FOUND: &str = "not_found";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeTransportPreference {
    #[default]
    Configured,
    Http,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeConnectRequest {
    #[serde(default)]
    pub transport: RealtimeTransportPreference,
}

/// Receive-only credentials for one account's invalidations. Event data and
/// authorization still come from the authenticated replay endpoint.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeTransport {
    Http {},
    Nats {
        url: String,
        subject: String,
        token: String,
    },
    AwsMqtt {
        endpoint: String,
        region: String,
        client_id: String,
        topic: String,
        credentials: AwsTemporaryCredentials,
    },
    #[serde(rename = "azure_web_pubsub")]
    AzureWebPubSub {
        url: String,
        group: String,
    },
    GcpFirebaseRtdb {
        database_url: String,
        api_key: String,
        project_id: String,
        custom_token: String,
        path: String,
    },
}

impl fmt::Debug for RealtimeTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Http {} => "Http",
            Self::Nats { .. } => "Nats",
            Self::AwsMqtt { .. } => "AwsMqtt",
            Self::AzureWebPubSub { .. } => "AzureWebPubSub",
            Self::GcpFirebaseRtdb { .. } => "GcpFirebaseRtdb",
        };
        f.debug_struct(name).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeConnectResponse {
    pub protocol_version: u8,
    /// Unix seconds. Connection owners renew before this deadline.
    pub expires_at: i64,
    /// Seconds until `expires_at` by the server's clock. Owners schedule renewal from when
    /// they receive the response, so a skewed local clock cannot expire a fresh grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    /// Longest `timeout_ms` the replay endpoint holds open. Zero means it answers at once,
    /// so owners without a live provider poll on an interval instead of back to back.
    #[serde(default = "default_timeout")]
    pub max_poll_wait_ms: u64,
    pub transport: RealtimeTransport,
}

/// A wakeup asks clients to replay their subscriptions. It carries no message
/// payload, account identity, execution identity, or authority to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeWakeup {
    pub protocol_version: u8,
    pub kind: String,
    pub nonce: String,
}

impl RealtimeWakeup {
    pub fn is_valid(&self) -> bool {
        self.protocol_version == REALTIME_PROTOCOL_VERSION
            && self.kind == "invalidate"
            && !self.nonce.is_empty()
            && self.nonce.len() <= 128
            && !self.nonce.chars().any(char::is_control)
    }
}

fn initial_sequence() -> i32 {
    -1
}

fn default_timeout() -> u64 {
    MAX_REALTIME_TIMEOUT_MS
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeNotificationSubscription {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeRunSubscription {
    pub run_id: String,
    /// Invocation-issued capability for this run. The API also checks the signed-in account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_token: Option<String>,
    /// Exclusive cursor, advanced only after the consumer accepts the returned batch.
    #[serde(default = "initial_sequence")]
    pub after_sequence: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl fmt::Debug for RealtimeRunSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimeRunSubscription")
            .field("run_id", &self.run_id)
            .field(
                "poll_token",
                &self.poll_token.as_ref().map(|_| "[redacted]"),
            )
            .field("after_sequence", &self.after_sequence)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimePollRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<RealtimeNotificationSubscription>,
    #[serde(default)]
    pub runs: Vec<RealtimeRunSubscription>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

impl Default for RealtimePollRequest {
    fn default() -> Self {
        Self {
            notifications: None,
            runs: Vec::new(),
            timeout_ms: default_timeout(),
        }
    }
}

impl RealtimePollRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.notifications.is_none() && self.runs.is_empty() {
            return Err("At least one realtime subscription is required");
        }
        if self.runs.len() > MAX_REALTIME_RUNS {
            return Err("Too many realtime run subscriptions");
        }
        if self.timeout_ms > MAX_REALTIME_TIMEOUT_MS {
            return Err("Realtime timeout exceeds the supported limit");
        }
        let valid_revision = |revision: &Option<String>| {
            revision.as_ref().is_none_or(|value| {
                !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
            })
        };
        if self
            .notifications
            .as_ref()
            .is_some_and(|subscription| !valid_revision(&subscription.revision))
        {
            return Err("Invalid notification revision");
        }
        let mut seen = HashSet::new();
        for run in &self.runs {
            if run.run_id.is_empty()
                || run.run_id.len() > 256
                || run.run_id.chars().any(char::is_control)
            {
                return Err("Invalid realtime run identifier");
            }
            if !seen.insert(&run.run_id) {
                return Err("Duplicate realtime run subscription");
            }
            if run.poll_token.as_ref().is_some_and(|token| {
                token.is_empty()
                    || token.len() > MAX_REALTIME_POLL_TOKEN_BYTES
                    || token.chars().any(char::is_control)
            }) {
                return Err("Invalid realtime run capability");
            }
            if run.after_sequence < -1 || run.after_sequence == i32::MAX {
                return Err("Invalid realtime event cursor");
            }
            if !valid_revision(&run.revision) {
                return Err("Invalid run revision");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeNotificationOverview {
    pub invites_count: u64,
    pub notifications_count: u64,
    pub unread_count: u64,
}

/// A revision of a bounded snapshot, used to refresh the user's notification queries.
/// This revision is not a durable change-log cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeNotificationUpdate {
    pub revision: String,
    pub overview: RealtimeNotificationOverview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeRunEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub sequence: i32,
    pub event_type: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeRunUpdate {
    pub run_id: String,
    pub revision: String,
    pub status: String,
    pub progress: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub events: Vec<RealtimeRunEvent>,
    pub after_sequence: i32,
    pub has_more: bool,
}

/// A failed run subscription does not interrupt the account inbox or other runs.
/// `code` is [`REALTIME_ERROR_UNAVAILABLE`] or [`REALTIME_ERROR_NOT_FOUND`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeSubscriptionError {
    pub run_id: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimePollResponse {
    pub protocol_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<RealtimeNotificationUpdate>,
    #[serde(default)]
    pub runs: Vec<RealtimeRunUpdate>,
    #[serde(default)]
    pub errors: Vec<RealtimeSubscriptionError>,
}

impl Default for RealtimePollResponse {
    fn default() -> Self {
        Self {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            notifications: None,
            runs: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_selection_cannot_supply_a_topic_or_another_account() {
        let default: RealtimeConnectRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(default.transport, RealtimeTransportPreference::Configured);
        let http: RealtimeConnectRequest =
            serde_json::from_value(json!({"transport": "http"})).unwrap();
        assert_eq!(http.transport, RealtimeTransportPreference::Http);
        for value in [
            json!({"transport": "nats"}),
            json!({"user_id": "another-user"}),
            json!({"topic": "another-inbox"}),
        ] {
            assert!(serde_json::from_value::<RealtimeConnectRequest>(value).is_err());
        }
    }

    #[test]
    fn wakeups_have_no_payload_or_execution_authority() {
        let wakeup = RealtimeWakeup {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            kind: "invalidate".into(),
            nonce: "notice-1".into(),
        };
        assert!(wakeup.is_valid());
        for invalid in [
            RealtimeWakeup {
                protocol_version: 2,
                ..wakeup.clone()
            },
            RealtimeWakeup {
                kind: "execute".into(),
                ..wakeup.clone()
            },
            RealtimeWakeup {
                nonce: String::new(),
                ..wakeup.clone()
            },
            RealtimeWakeup {
                nonce: "x".repeat(129),
                ..wakeup.clone()
            },
            RealtimeWakeup {
                nonce: "x\n".into(),
                ..wakeup.clone()
            },
        ] {
            assert!(!invalid.is_valid());
        }
        assert!(
            serde_json::from_value::<RealtimeWakeup>(json!({
                "protocol_version": 1, "kind": "invalidate", "nonce": "notice-1",
                "payload": {"run_id": "another-run"}
            }))
            .is_err()
        );
    }

    #[test]
    fn provider_grants_do_not_expose_credentials_in_debug_output() {
        let grant = RealtimeConnectResponse {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            expires_at: 100,
            expires_in: Some(60),
            max_poll_wait_ms: 0,
            transport: RealtimeTransport::AzureWebPubSub {
                url: "wss://example.test?access_token=private-token".into(),
                group: "private-account".into(),
            },
        };
        let debug = format!("{grant:?}");
        assert!(!debug.contains("private-token"));
        assert!(!debug.contains("private-account"));
        assert_eq!(
            serde_json::to_value(&grant).unwrap()["transport"]["type"],
            "azure_web_pubsub"
        );
    }

    #[test]
    fn grants_without_poll_limits_keep_a_full_hold_and_no_relative_expiry() {
        let grant: RealtimeConnectResponse = serde_json::from_value(json!({
            "protocol_version": 1, "expires_at": 100, "transport": {"type": "http"}
        }))
        .unwrap();
        assert_eq!(grant.expires_in, None);
        assert_eq!(grant.max_poll_wait_ms, MAX_REALTIME_TIMEOUT_MS);
        let encoded = serde_json::to_value(&grant).unwrap();
        assert!(encoded.get("expires_in").is_none());
        assert_eq!(encoded["max_poll_wait_ms"], MAX_REALTIME_TIMEOUT_MS);

        let lambda: RealtimeConnectResponse = serde_json::from_value(json!({
            "protocol_version": 1, "expires_at": 100, "expires_in": 300,
            "max_poll_wait_ms": 0, "transport": {"type": "http"}
        }))
        .unwrap();
        assert_eq!(lambda.expires_in, Some(300));
        assert_eq!(lambda.max_poll_wait_ms, 0);
    }

    #[test]
    fn subscription_error_codes_are_part_of_the_protocol() {
        assert_eq!(REALTIME_ERROR_UNAVAILABLE, "unavailable");
        assert_eq!(REALTIME_ERROR_NOT_FOUND, "not_found");
        let error: RealtimeSubscriptionError =
            serde_json::from_value(json!({"run_id": "run-a", "code": "not_found"})).unwrap();
        assert_eq!(error.code, REALTIME_ERROR_NOT_FOUND);
    }

    #[test]
    fn new_subscribers_start_before_the_first_event() {
        let request: RealtimePollRequest =
            serde_json::from_value(json!({"runs": [{"run_id": "run-a"}]})).unwrap();
        assert_eq!(request.runs[0].after_sequence, -1);
        assert_eq!(request.timeout_ms, MAX_REALTIME_TIMEOUT_MS);
        assert!(request.validate().is_ok());
    }

    #[test]
    fn run_capabilities_are_optional_bounded_and_redacted() {
        let request: RealtimePollRequest = serde_json::from_value(json!({
            "runs": [{"run_id": "run-a", "poll_token": "private-run-capability"}]
        }))
        .unwrap();
        assert!(request.validate().is_ok());
        assert!(!format!("{request:?}").contains("private-run-capability"));
        assert_eq!(
            serde_json::to_value(&request).unwrap()["runs"][0]["poll_token"],
            "private-run-capability"
        );
        for token in [
            String::new(),
            "line\nbreak".into(),
            "x".repeat(MAX_REALTIME_POLL_TOKEN_BYTES + 1),
        ] {
            let mut invalid = request.clone();
            invalid.runs[0].poll_token = Some(token);
            assert!(invalid.validate().is_err());
        }
        let mut legacy = request;
        legacy.runs[0].poll_token = None;
        assert!(
            serde_json::to_value(legacy).unwrap()["runs"][0]
                .get("poll_token")
                .is_none()
        );
    }

    #[test]
    fn invalid_and_ambiguous_subscriptions_are_rejected() {
        for value in [
            json!({}),
            json!({"runs": [{"run_id": ""}]}),
            json!({"runs": [{"run_id": "a"}, {"run_id": "a"}]}),
            json!({"runs": [{"run_id": "a", "after_sequence": -2}]}),
            json!({"runs": [{"run_id": "a", "after_sequence": i32::MAX}]}),
            json!({"notifications": {"revision": ""}}),
            json!({"notifications": {}, "timeout_ms": MAX_REALTIME_TIMEOUT_MS + 1}),
            json!({"runs": (0..=MAX_REALTIME_RUNS).map(|id| json!({"run_id": id.to_string()})).collect::<Vec<_>>()}),
        ] {
            let request: RealtimePollRequest = serde_json::from_value(value).unwrap();
            assert!(request.validate().is_err());
        }
    }

    #[test]
    fn request_cannot_supply_another_account_identity() {
        assert!(
            serde_json::from_value::<RealtimePollRequest>(json!({
                "notifications": {}, "user_id": "another-user"
            }))
            .is_err()
        );
    }

    #[test]
    fn empty_responses_keep_a_protocol_version_and_no_subscription_changes() {
        assert_eq!(
            serde_json::to_value(RealtimePollResponse::default()).unwrap(),
            json!({"protocol_version": 1, "runs": [], "errors": []})
        );
    }
}

//! Account-scoped notification snapshots and independently replayable run subscriptions.
//! Payloads are opaque to connection owners. Authentication and stream authorization belong
//! to the API; a local subscription only controls which tab or window receives a response.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const REALTIME_PROTOCOL_VERSION: u8 = 1;
pub const MAX_REALTIME_RUNS: usize = 16;
pub const MAX_REALTIME_EVENTS: usize = 100;
pub const MAX_REALTIME_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_REALTIME_TIMEOUT_MS: u64 = 20_000;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeRunSubscription {
    pub run_id: String,
    /// Exclusive cursor, advanced only after the consumer accepts the returned batch.
    #[serde(default = "initial_sequence")]
    pub after_sequence: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
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
    fn new_subscribers_start_before_the_first_event() {
        let request: RealtimePollRequest =
            serde_json::from_value(json!({"runs": [{"run_id": "run-a"}]})).unwrap();
        assert_eq!(request.runs[0].after_sequence, -1);
        assert_eq!(request.timeout_ms, MAX_REALTIME_TIMEOUT_MS);
        assert!(request.validate().is_ok());
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
        assert!(serde_json::from_value::<RealtimePollRequest>(json!({
            "notifications": {}, "user_id": "another-user"
        })).is_err());
    }

    #[test]
    fn empty_responses_keep_a_protocol_version_and_no_subscription_changes() {
        assert_eq!(serde_json::to_value(RealtimePollResponse::default()).unwrap(),
            json!({"protocol_version": 1, "runs": [], "errors": []}));
    }
}

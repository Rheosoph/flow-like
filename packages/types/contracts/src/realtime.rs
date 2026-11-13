//! Account-scoped notification snapshots and independently replayable run subscriptions.
//! Payloads are opaque to connection owners. Authentication and stream authorization belong
//! to the API; a local subscription only controls which tab or window receives a response.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

use crate::channel::AwsTemporaryCredentials;
use crate::inbox::EntryCursor;

pub const REALTIME_PROTOCOL_VERSION: u8 = 1;
pub const MAX_REALTIME_RUNS: usize = 16;
pub const MAX_REALTIME_EVENTS: usize = 100;
pub const MAX_REALTIME_CONVERSATIONS: usize = 8;
pub const MAX_REALTIME_CONVERSATION_ENTRIES: usize = 50;
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

/// Receive-only credentials for one account's wakeups and the replies written for it
/// ([`RealtimeReply`]). Stored data and authorization still come from the authenticated replay
/// endpoint.
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
        /// Sibling of `path` that carries [`RealtimeReply`] messages, so they never overwrite a
        /// wakeup. Absent when the server publishes none there.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_path: Option<String>,
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

pub const REALTIME_KIND_REPLY: &str = "reply";

/// How far a reply that is still being written has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeReplyState {
    /// More text follows.
    Writing,
    /// The bot is busy without writing, for example while a tool runs.
    Working,
    /// The text is complete; the stored message follows through the replay.
    Done,
    /// The reply is not posted; what a device has not shown or spoken of it is discarded.
    Dropped,
}

impl RealtimeReplyState {
    /// Whether no further message of the reply follows.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Dropped)
    }
}

/// A bot's reply while its host writes it, for the devices of the people in the conversation.
/// It shortens the wait and decides nothing: the stored message still arrives through the
/// authenticated replay and replaces it, and a device that misses these messages loses only the
/// head start. Unlike a [`RealtimeWakeup`] it carries conversation content, so a host sends it
/// only to accounts that also receive the stored message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeReply {
    pub protocol_version: u8,
    /// Always [`REALTIME_KIND_REPLY`].
    pub kind: String,
    pub conversation_id: String,
    /// The id the stored message will have.
    pub message_id: String,
    /// The message this reply answers.
    pub reply_to: String,
    /// The bot that writes the reply.
    pub sender_id: String,
    /// Where `text` starts in the reply, in UTF-16 code units.
    pub offset: u32,
    /// The reply from `offset` to its current end. Normally the whole reply so far, so that any
    /// one message makes up for the ones lost before it.
    pub text: String,
    pub state: RealtimeReplyState,
    /// Counts how often the bot began its reply again because the conversation moved on while
    /// it wrote. A later draft replaces what a device holds of an earlier one from its start,
    /// and messages of an earlier draft are ignored once a later one arrived.
    #[serde(default, skip_serializing_if = "is_first_draft")]
    pub draft: u32,
}

fn is_first_draft(draft: &u32) -> bool {
    *draft == 0
}

/// Bytes of reply text a message is first tried with; escaping can still push it over the limit.
const REPLY_TEXT_BYTES: usize = 3_072;

impl RealtimeReply {
    pub fn is_valid(&self) -> bool {
        let identifies = |value: &String| {
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        };
        self.protocol_version == REALTIME_PROTOCOL_VERSION
            && self.kind == REALTIME_KIND_REPLY
            && [
                &self.conversation_id,
                &self.message_id,
                &self.reply_to,
                &self.sender_id,
            ]
            .into_iter()
            .all(identifies)
    }

    /// Encodes the reply with as much of the end of `text` as [`MAX_REALTIME_WAKEUP_BYTES`]
    /// allows: all of it while it fits, otherwise a tail that starts on a character boundary.
    /// `None` when the identifiers alone are too large.
    pub fn encode_fitting(mut self, text: &str) -> Option<Vec<u8>> {
        let mut budget = REPLY_TEXT_BYTES;
        loop {
            let start = tail_start(text, budget);
            self.offset = u32::try_from(text[..start].encode_utf16().count()).ok()?;
            self.text = text[start..].to_owned();
            let encoded = serde_json::to_vec(&self).ok()?;
            if encoded.len() <= MAX_REALTIME_WAKEUP_BYTES {
                return Some(encoded);
            }
            if budget == 0 {
                return None;
            }
            budget /= 2;
        }
    }
}

fn tail_start(text: &str, budget: usize) -> usize {
    let mut start = text.len().saturating_sub(budget);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    start
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

/// An open inbox conversation whose new entries ride on the poll a wakeup causes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimeConversationSubscription {
    pub conversation_id: String,
    /// Cursor of the newest entry the client holds (the `cursor` of an inbox entry); only
    /// later entries come back, oldest first. Absent: the newest
    /// [`MAX_REALTIME_CONVERSATION_ENTRIES`] entries come back once, for a client that holds
    /// none (an empty conversation); it then continues from the last cursor it received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Read as one of the caller's bots, like `?as=` on the inbox routes.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_principal: Option<String>,
}

impl RealtimeConversationSubscription {
    pub fn key(&self) -> (&str, Option<&str>) {
        (&self.conversation_id, self.as_principal.as_deref())
    }

    pub fn after_cursor(&self) -> Option<EntryCursor> {
        self.after
            .as_deref()
            .and_then(|after| EntryCursor::parse(after).ok())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealtimePollRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<RealtimeNotificationSubscription>,
    #[serde(default)]
    pub runs: Vec<RealtimeRunSubscription>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversations: Vec<RealtimeConversationSubscription>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

impl Default for RealtimePollRequest {
    fn default() -> Self {
        Self {
            notifications: None,
            runs: Vec::new(),
            conversations: Vec::new(),
            timeout_ms: default_timeout(),
        }
    }
}

impl RealtimePollRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.notifications.is_none() && self.runs.is_empty() && self.conversations.is_empty() {
            return Err("At least one realtime subscription is required");
        }
        if self.runs.len() > MAX_REALTIME_RUNS {
            return Err("Too many realtime run subscriptions");
        }
        if self.conversations.len() > MAX_REALTIME_CONVERSATIONS {
            return Err("Too many realtime conversation subscriptions");
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
        let mut seen_conversations = HashSet::new();
        for conversation in &self.conversations {
            let id = &conversation.conversation_id;
            if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                return Err("Invalid realtime conversation identifier");
            }
            if conversation.as_principal.as_ref().is_some_and(|principal| {
                principal.is_empty()
                    || principal.len() > 256
                    || principal.chars().any(char::is_control)
            }) {
                return Err("Invalid realtime conversation reader");
            }
            if conversation
                .after
                .as_deref()
                .is_some_and(|after| EntryCursor::parse(after).is_err())
            {
                return Err("Invalid realtime conversation cursor");
            }
            if !seen_conversations.insert(conversation.key()) {
                return Err("Duplicate realtime conversation subscription");
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

/// New entries of one subscribed conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeConversationUpdate {
    pub conversation_id: String,
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_principal: Option<String>,
    /// The first entries after the subscription's `after`, oldest first, each exactly as
    /// `GET /inbox/conversations/{id}/entries` returns it.
    pub entries: Vec<Value>,
    /// More entries follow the last one returned; poll again with its cursor.
    pub has_more: bool,
}

/// A failed conversation subscription. `code` is [`REALTIME_ERROR_UNAVAILABLE`] or
/// [`REALTIME_ERROR_NOT_FOUND`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RealtimeConversationError {
    pub conversation_id: String,
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_principal: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversations: Vec<RealtimeConversationUpdate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversation_errors: Vec<RealtimeConversationError>,
}

impl Default for RealtimePollResponse {
    fn default() -> Self {
        Self {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            notifications: None,
            runs: Vec::new(),
            errors: Vec::new(),
            conversations: Vec::new(),
            conversation_errors: Vec::new(),
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

    fn reply(state: RealtimeReplyState) -> RealtimeReply {
        RealtimeReply {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            kind: REALTIME_KIND_REPLY.into(),
            conversation_id: "c-a".into(),
            message_id: "re-1".into(),
            reply_to: "m-1".into(),
            sender_id: "bot-1".into(),
            offset: 0,
            text: String::new(),
            state,
            draft: 0,
        }
    }

    #[test]
    fn only_a_rewritten_reply_names_its_draft() {
        let first: serde_json::Value = serde_json::from_slice(
            &reply(RealtimeReplyState::Writing)
                .encode_fitting("Sure")
                .unwrap(),
        )
        .unwrap();
        assert!(first.get("draft").is_none());
        assert_eq!(
            serde_json::from_value::<RealtimeReply>(first)
                .unwrap()
                .draft,
            0
        );
        let rewrite = RealtimeReply {
            draft: 2,
            ..reply(RealtimeReplyState::Writing)
        };
        let encoded: serde_json::Value =
            serde_json::from_slice(&rewrite.clone().encode_fitting("Anna said it").unwrap())
                .unwrap();
        assert_eq!(encoded["draft"], 2);
        assert_eq!(
            serde_json::from_value::<RealtimeReply>(encoded).unwrap(),
            RealtimeReply {
                text: "Anna said it".into(),
                ..rewrite
            }
        );
    }

    #[test]
    fn replies_name_their_conversation_message_and_writer() {
        let writing = reply(RealtimeReplyState::Writing);
        assert!(writing.is_valid());
        for invalid in [
            RealtimeReply {
                protocol_version: 2,
                ..writing.clone()
            },
            RealtimeReply {
                kind: "invalidate".into(),
                ..writing.clone()
            },
            RealtimeReply {
                conversation_id: String::new(),
                ..writing.clone()
            },
            RealtimeReply {
                message_id: "x".repeat(257),
                ..writing.clone()
            },
            RealtimeReply {
                reply_to: "m\n1".into(),
                ..writing.clone()
            },
            RealtimeReply {
                sender_id: String::new(),
                ..writing.clone()
            },
        ] {
            assert!(!invalid.is_valid(), "{invalid:?}");
        }
    }

    #[test]
    fn replies_and_wakeups_are_never_mistaken_for_each_other() {
        let encoded = reply(RealtimeReplyState::Working)
            .encode_fitting("Let me check.")
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value["kind"], "reply");
        assert_eq!(value["state"], "working");
        assert_eq!(value["offset"], 0);
        assert_eq!(value["text"], "Let me check.");
        assert!(serde_json::from_slice::<RealtimeWakeup>(&encoded).is_err());
        assert!(
            serde_json::from_value::<RealtimeReply>(json!({
                "protocol_version": 1, "kind": "invalidate", "nonce": "n"
            }))
            .is_err()
        );
        let mut later = value.clone();
        later["added_later"] = json!(true);
        assert!(
            serde_json::from_value::<RealtimeReply>(later)
                .unwrap()
                .is_valid()
        );
        let mut unknown = value;
        unknown["state"] = json!("paused");
        assert!(serde_json::from_value::<RealtimeReply>(unknown).is_err());
    }

    #[test]
    fn a_reply_carries_its_whole_text_while_that_fits_and_its_tail_after() {
        let short = "Grüße 👋 an alle.";
        let encoded = reply(RealtimeReplyState::Done)
            .encode_fitting(short)
            .unwrap();
        let decoded: RealtimeReply = serde_json::from_slice(&encoded).unwrap();
        assert_eq!((decoded.offset, decoded.text.as_str()), (0, short));

        let long = "Ein längerer Satz mit Umlauten und 👋. ".repeat(200);
        let encoded = reply(RealtimeReplyState::Writing)
            .encode_fitting(&long)
            .unwrap();
        assert!(encoded.len() <= MAX_REALTIME_WAKEUP_BYTES);
        let decoded: RealtimeReply = serde_json::from_slice(&encoded).unwrap();
        assert!(decoded.offset > 0 && decoded.text.len() > 2_000);
        let units: Vec<u16> = long.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[decoded.offset as usize..]).unwrap(),
            decoded.text
        );
    }

    #[test]
    fn text_that_grows_when_escaped_still_fits_one_message() {
        let quotes = "\"\n".repeat(4_000);
        let encoded = reply(RealtimeReplyState::Writing)
            .encode_fitting(&quotes)
            .unwrap();
        assert!(encoded.len() <= MAX_REALTIME_WAKEUP_BYTES);
        let decoded: RealtimeReply = serde_json::from_slice(&encoded).unwrap();
        assert!(!decoded.text.is_empty() && quotes.ends_with(&decoded.text));

        let oversized = RealtimeReply {
            conversation_id: "c".repeat(MAX_REALTIME_WAKEUP_BYTES),
            ..reply(RealtimeReplyState::Writing)
        };
        assert!(oversized.encode_fitting("text").is_none());
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

    fn conversation(id: &str) -> serde_json::Value {
        json!({"conversation_id": id})
    }

    #[test]
    fn conversation_subscriptions_count_as_a_subscription_and_are_bounded() {
        let request: RealtimePollRequest =
            serde_json::from_value(json!({"conversations": [conversation("c-a")]})).unwrap();
        assert!(request.validate().is_ok());
        let too_many: RealtimePollRequest = serde_json::from_value(json!({
            "conversations": (0..=MAX_REALTIME_CONVERSATIONS)
                .map(|id| conversation(&id.to_string())).collect::<Vec<_>>()
        }))
        .unwrap();
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn invalid_conversation_subscriptions_are_rejected() {
        let long = "x".repeat(257);
        for subscriptions in [
            json!([conversation("")]),
            json!([conversation(&long)]),
            json!([conversation("a\nb")]),
            json!([conversation("a"), conversation("a")]),
            json!([{"conversation_id": "a", "as": ""}]),
            json!([{"conversation_id": "a", "after": "not-a-cursor"}]),
            json!([{"conversation_id": "a", "after": "12:"}]),
        ] {
            let request: RealtimePollRequest =
                serde_json::from_value(json!({"conversations": subscriptions})).unwrap();
            assert!(request.validate().is_err(), "{request:?}");
        }
        assert!(
            serde_json::from_value::<RealtimePollRequest>(json!({
                "conversations": [{"conversation_id": "a", "user_id": "b"}]
            }))
            .is_err()
        );
    }

    #[test]
    fn one_conversation_may_be_read_as_the_caller_and_as_each_bot() {
        let request: RealtimePollRequest = serde_json::from_value(json!({
            "conversations": [
                {"conversation_id": "a", "after": "1800000000123000:e1"},
                {"conversation_id": "a", "as": "bot-1"},
                {"conversation_id": "a", "as": "bot-2"},
            ]
        }))
        .unwrap();
        assert!(request.validate().is_ok());
        assert_eq!(
            request.conversations[0].after_cursor(),
            Some(EntryCursor::new(1_800_000_000_123_000, "e1"))
        );
        assert_eq!(request.conversations[1].key(), ("a", Some("bot-1")));
        assert_eq!(request.conversations[1].after_cursor(), None);
    }

    #[test]
    fn conversation_sections_are_omitted_when_empty() {
        let request = RealtimePollRequest {
            notifications: Some(RealtimeNotificationSubscription::default()),
            ..Default::default()
        };
        assert!(
            serde_json::to_value(&request)
                .unwrap()
                .get("conversations")
                .is_none()
        );
        let response = serde_json::to_value(RealtimePollResponse::default()).unwrap();
        assert!(response.get("conversations").is_none());
        assert!(response.get("conversation_errors").is_none());

        let response = RealtimePollResponse {
            conversations: vec![RealtimeConversationUpdate {
                conversation_id: "c-a".into(),
                as_principal: Some("bot-1".into()),
                entries: vec![json!({"cursor": "1:e"})],
                has_more: true,
            }],
            conversation_errors: vec![RealtimeConversationError {
                conversation_id: "c-b".into(),
                as_principal: None,
                code: REALTIME_ERROR_NOT_FOUND.into(),
            }],
            ..Default::default()
        };
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["conversations"][0]["as"], "bot-1");
        assert_eq!(encoded["conversations"][0]["has_more"], true);
        assert!(encoded["conversation_errors"][0].get("as").is_none());
        let decoded: RealtimePollResponse = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn empty_responses_keep_a_protocol_version_and_no_subscription_changes() {
        assert_eq!(
            serde_json::to_value(RealtimePollResponse::default()).unwrap(),
            json!({"protocol_version": 1, "runs": [], "errors": []})
        );
    }
}

use crate::entity::{conversation_member, inbox_entry, inbox_key};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    Value,
    inbox::{ConversationEncryption, ConversationKind, EntryCursor, InboxEntryKind},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

/// A principal in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ConversationMember {
    /// A user sub or a bot id.
    pub principal_id: String,
    /// `user` or `bot`.
    #[schema(example = "user")]
    pub kind: String,
    /// Who added the member; null for the person who started the conversation.
    pub added_by: Option<String>,
    pub joined_at: DateTime<FixedOffset>,
    /// A bot's room limits; only its owner sees them.
    #[schema(value_type = Option<Object>)]
    pub limits: Option<Value>,
}

impl From<conversation_member::Model> for ConversationMember {
    fn from(row: conversation_member::Model) -> Self {
        Self {
            principal_id: row.principal_id,
            kind: row.principal_kind.to_ascii_lowercase(),
            added_by: row.added_by,
            joined_at: row.joined_at,
            limits: row.limits,
        }
    }
}

/// Someone invited to a group who has not joined: they read and receive nothing yet.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct PendingMember {
    pub principal_id: String,
    /// `user` or `bot`.
    #[schema(example = "user")]
    pub kind: String,
    pub invited_by: Option<String>,
    pub invited_at: Option<DateTime<FixedOffset>>,
    /// `pending`, or `declined` until the inviter withdraws it.
    #[schema(example = "pending")]
    pub status: String,
}

impl From<conversation_member::Model> for PendingMember {
    fn from(row: conversation_member::Model) -> Self {
        Self {
            principal_id: row.principal_id,
            kind: row.principal_kind.to_ascii_lowercase(),
            invited_by: row.invited_by,
            invited_at: row.invited_at,
            status: row.status.to_ascii_lowercase(),
        }
    }
}

/// One principal's copy of one event. Every copy of the same event shares `message_id` and
/// `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct InboxEntry {
    pub id: String,
    /// Chosen by the sender and the same in every member's copy.
    pub message_id: String,
    pub conversation_id: String,
    pub sender_id: String,
    /// `message`, `reaction`, `edit`, `delete`, `topic` (a "New topic" marker) or `member` (a
    /// join or leave the server wrote).
    #[schema(example = "message")]
    pub kind: String,
    /// Envelope v1: `{ v: 1, enc: "none", body }`, or sealed to this inbox's key. A deleted
    /// message keeps its entry with the body `{ "deleted": true }`.
    #[schema(value_type = Object, example = json!({ "v": 1, "enc": "none", "body": { "text": "Hello" } }))]
    pub envelope: Value,
    pub created_at: DateTime<FixedOffset>,
    /// The event's position in the conversation, `#seq` in a bot's history: the same in every
    /// member's copy and never reused. Null on entries from before positions existed.
    pub seq: Option<i64>,
    /// Opaque position; pass it back as `before` or `after`.
    pub cursor: String,
}

impl From<inbox_entry::Model> for InboxEntry {
    fn from(row: inbox_entry::Model) -> Self {
        Self {
            cursor: EntryCursor::new(row.created_at.timestamp_micros(), row.id.as_str())
                .to_string(),
            kind: row.kind.to_ascii_lowercase(),
            id: row.id,
            message_id: row.message_id,
            conversation_id: row.conversation_id,
            sender_id: row.sender_id,
            envelope: row.envelope,
            created_at: row.created_at,
            seq: row.seq,
        }
    }
}

/// A conversation as one of its members sees it.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Conversation {
    pub id: String,
    /// `direct` (two principals, one conversation per pair) or `group`.
    #[schema(example = "group")]
    pub kind: String,
    pub title: Option<String>,
    /// `plain`, or `e2e` where every member receives an envelope sealed to their key.
    #[schema(example = "plain")]
    pub encryption: String,
    pub created_by: String,
    pub created_at: DateTime<FixedOffset>,
    /// When the newest message was sent, or when the conversation started.
    pub last_activity_at: DateTime<FixedOffset>,
    /// Active members only, in the order they joined.
    pub members: Vec<ConversationMember>,
    /// Invitations that haven't been accepted, oldest first.
    pub pending: Vec<PendingMember>,
    /// Messages from others the reader has not read yet.
    pub unread_count: u64,
    /// The newest message in the reader's inbox.
    pub last_entry: Option<InboxEntry>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationList {
    /// Newest activity first, at most 200.
    pub conversations: Vec<Conversation>,
}

/// A new conversation. The caller is always a member.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateConversationRequest {
    /// `direct` or `group`.
    #[schema(value_type = String, example = "group")]
    pub kind: ConversationKind,
    /// Groups only; at most 120 characters.
    pub title: Option<String>,
    /// Everyone except you: your own bots and people who share an app with you. A direct
    /// conversation takes exactly one; a group at most 49.
    pub members: Vec<String>,
    /// `plain` (default) or `e2e`.
    #[schema(value_type = Option<String>, example = "plain")]
    pub encryption: Option<ConversationEncryption>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RenameConversationRequest {
    /// At most 120 characters; empty or null removes the title.
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddMembersRequest {
    /// Your own bots and people who share an app with you. Current members are skipped.
    pub members: Vec<String>,
}

/// An event to send. Plain conversations take `envelope`; end-to-end encrypted ones take
/// `envelopes`, one sealed envelope per active member keyed by principal id.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SendEntryRequest {
    /// 1 to 64 characters of `[A-Za-z0-9_-]`, chosen by you. Sending the same id again
    /// returns the first result instead of sending twice.
    pub message_id: String,
    /// `message`, `reaction`, `edit`, `delete` or `topic` ("New topic", body `{ title? }`).
    #[schema(value_type = String, example = "message")]
    pub kind: InboxEntryKind,
    /// `{ v: 1, enc: "none", body }`, at most 64 KiB. A message's body is `{ text, reply_to?,
    /// mentions?, parts?, error?, hops?, ask? }`: markdown text, parts numbered `t1`… (tool
    /// calls), `ui1`… (UI blocks) and `a1`… (attachments), each too large for the envelope kept
    /// in the sending bot's workspace as `handle { app_id, path, bytes, sha256, mime }`. An
    /// edit's body is a message body with its `target`.
    #[schema(value_type = Option<Object>, example = json!({ "v": 1, "enc": "none", "body": { "text": "Hello" } }))]
    pub envelope: Option<Value>,
    /// Principal id to `{ v: 1, enc, key_id, enc_key, ct }`, each at most 64 KiB.
    #[schema(value_type = Option<Object>)]
    pub envelopes: Option<BTreeMap<String, Value>>,
    /// Sending as a bot (`?as=`): the turn its host claimed and answers with this message.
    pub turn: Option<TurnClaim>,
}

/// A bot host's claim on the turn a message answers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct TurnClaim {
    /// The turn's id.
    pub id: String,
    /// The host that claimed it.
    pub host_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InboxEntriesPage {
    /// Newest first; by position (`from_seq`, `to_seq`, `around_seq`) oldest first.
    pub entries: Vec<InboxEntry>,
    /// Pass as `before` for the next older page. Set when more entries lie between the
    /// oldest entry here and `after` (or the start of the conversation).
    pub next_before: Option<String>,
    /// Reading by position: more entries follow; pass this as `from_seq` for them.
    pub next_from_seq: Option<i64>,
    /// Reading by position: more entries precede; pass this as `to_seq` for them.
    pub next_to_seq: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct MarkReadRequest {
    /// The cursor of the newest entry you have seen; omitted marks everything read.
    pub up_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MarkReadResponse {
    pub last_read_at: DateTime<FixedOffset>,
}

/// A public key others seal envelopes to.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct InboxKey {
    pub principal_id: String,
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub created_at: DateTime<FixedOffset>,
}

impl From<inbox_key::Model> for InboxKey {
    fn from(row: inbox_key::Model) -> Self {
        Self {
            principal_id: row.principal_id,
            key_id: row.key_id,
            algorithm: row.algorithm,
            public_key: row.public_key,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InboxKeyList {
    pub keys: Vec<InboxKey>,
}

/// Registers a public key. Keys never change: register a new `key_id` to rotate.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RegisterInboxKeyRequest {
    /// Omit for your own key; the id of a bot you own registers that bot's key.
    pub principal_id: Option<String>,
    /// 1 to 128 characters of `[A-Za-z0-9._:-]`.
    pub key_id: String,
    /// 1 to 64 characters of `[A-Za-z0-9._-]`, e.g. `hpke-x25519-sha256-chacha20poly1305`.
    pub algorithm: String,
    /// The encoded public key, at most 4,096 characters.
    pub public_key: String,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReaderQuery {
    /// Read the inbox of one of your bots instead of your own: every conversation it is in.
    #[serde(rename = "as")]
    pub as_bot: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SendQuery {
    /// Send as one of your desktop bots, answering the `turn` its host claimed.
    #[serde(rename = "as")]
    pub as_bot: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntriesQuery {
    /// Read the inbox of one of your bots instead of your own: every conversation it is in.
    #[serde(rename = "as")]
    pub as_bot: Option<String>,
    /// Only entries older than this cursor.
    pub before: Option<String>,
    /// Only entries newer than this cursor.
    pub after: Option<String>,
    /// 1 to 100 entries; 50 by default.
    pub limit: Option<u64>,
    /// By position: entries from this `seq` on, oldest first (with `to_seq`, up to it).
    pub from_seq: Option<i64>,
    /// By position: entries up to this `seq`; alone, the newest `limit` of them.
    pub to_seq: Option<i64>,
    /// By position: the entry at this `seq` with entries before and after it.
    pub around_seq: Option<i64>,
    /// With `around_seq`: 0 to 50 entries before it; 10 by default.
    pub before_count: Option<u64>,
    /// With `around_seq`: 0 to 50 entries after it; 10 by default.
    pub after_count: Option<u64>,
}

/// A summary of part of a conversation's history, written by a bot's host: level 0 summarises
/// the entries `seq_from..=seq_to`, a higher level merges older summaries and keeps their range.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ConversationSegment {
    /// Chosen by its writer; unique per conversation and author.
    pub id: String,
    /// The bot whose host wrote it.
    pub author_id: String,
    pub level: i32,
    pub seq_from: i64,
    pub seq_to: i64,
    /// When the oldest and the newest summarised entry were sent.
    pub t_from: DateTime<FixedOffset>,
    pub t_to: DateTime<FixedOffset>,
    pub title: Option<String>,
    pub summary: String,
    /// The higher-level segment that merged this one.
    pub parent_id: Option<String>,
    /// 1 for the first summary of this range by this author; each changed summary is the next.
    pub version: i32,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationSegmentList {
    /// The newest version of each range, oldest range first.
    pub segments: Vec<ConversationSegment>,
    /// Pass as `after` for the next page.
    pub next_after: Option<String>,
}

/// A summary a bot's host wrote. It is written once: a changed summary of a range goes under a
/// new id and becomes the range's next version.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WriteSegmentRequest {
    /// 1 to 64 characters of `[A-Za-z0-9_.-]`, unique per conversation and bot. Writing the
    /// same id again returns the first write.
    #[schema(example = "s0-4800-5120")]
    pub id: String,
    /// 0 summarises entries; a higher level, at most 9, merges summaries.
    pub level: i32,
    /// The first position summarised, from 1.
    pub seq_from: i64,
    /// The last position summarised, at most the conversation's newest.
    pub seq_to: i64,
    /// At most 200 characters.
    pub title: Option<String>,
    /// At most 32 KiB.
    pub summary: String,
    /// The higher-level segment of the same bot that merged this one.
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SegmentsQuery {
    /// Read as one of your bots: the conversations it is in, and its own summaries by default.
    #[serde(rename = "as")]
    pub as_bot: Option<String>,
    /// The bot whose summaries to list; by default the bot you read as, or the bot of a direct
    /// conversation, else every bot's.
    pub author: Option<String>,
    /// Only this level.
    pub level: Option<i32>,
    /// Only segments whose range holds this position.
    pub covering: Option<i64>,
    /// The `next_after` of the previous page.
    pub after: Option<String>,
    /// 1 to 100 segments; 50 by default.
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WriteSegmentQuery {
    /// The bot whose host writes the summary: one of your bots in this conversation.
    #[serde(rename = "as")]
    pub as_bot: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KeyOwnerQuery {
    /// The id of one of your bots to revoke its key; omit to revoke your own.
    pub principal_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KeysQuery {
    /// Up to 50 comma-separated user subs or bot ids.
    pub principals: Option<String>,
}

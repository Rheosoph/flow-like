//! The wire shapes: the API's `Bot`, `BotApp`, `BotSecretInfo`, `BotPermission`,
//! `Conversation`, `ConversationSegment`, `InboxEntry`, their requests and pages, plus
//! `local: true` on bots and conversations and the bundle a bot moves to an account with.

use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot::{BotCapability, BotLocation, BotModelPolicy, BotSystemKind},
    bot_restrictions::BotRestrictions,
    data_classification::DataSensitivity,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind, PrincipalKind},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};
use zeroize::Zeroizing;

/// A bot on this device, shaped like the API's `Bot`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bot {
    pub id: String,
    /// Always [`crate::LOCAL_OWNER_ID`].
    pub owner_id: String,
    /// `general` (Assistant) or `flux` on a built-in's row; null on bots the person made.
    pub system: Option<BotSystemKind>,
    pub preset_id: Option<String>,
    /// Null on a built-in the person never renamed: the client shows the localized default.
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub appearance: Value,
    pub model: BotModelPolicy,
    /// Bit id of the embedding model its memory uses; null lets the host choose.
    pub embedding_model: Option<String>,
    /// Bit ids of the speech-to-text, text-to-speech, image generation and vision models it
    /// pins; null lets the host choose.
    pub stt_model: Option<String>,
    pub tts_model: Option<String>,
    pub image_model: Option<String>,
    pub vision_model: Option<String>,
    pub classification: DataSensitivity,
    pub capabilities: Vec<BotCapability>,
    pub profile_id: Option<String>,
    /// Always `desktop`.
    pub location: BotLocation,
    /// Always null.
    pub device_id: Option<String>,
    /// The local app that holds the bot's files, databases and memory. Not on the wire: an
    /// account marks the workspace on the app (`App.botId`) instead.
    #[serde(skip)]
    pub workspace_app_id: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BotApp {
    pub app_id: String,
    pub added_at: DateTime<FixedOffset>,
    /// Always `owner`: the signed-out person owns everything on this device.
    pub owner_role: Option<String>,
    pub restrictions: BotRestrictions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BotSecretEncryption {
    /// Sealed under a key kept in this device's keychain.
    Device,
}

/// A secret's metadata; never its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BotSecretInfo {
    pub name: String,
    pub encryption: BotSecretEncryption,
    pub description: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
    pub last_used_at: Option<DateTime<FixedOffset>>,
}

/// The API's `CreateBotRequest`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateBotRequest {
    pub name: String,
    pub preset_id: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub appearance: Value,
    pub model: Option<BotModelPolicy>,
    pub embedding_model: Option<String>,
    pub stt_model: Option<String>,
    pub tts_model: Option<String>,
    pub image_model: Option<String>,
    pub vision_model: Option<String>,
    pub classification: Option<DataSensitivity>,
    pub capabilities: Option<Vec<String>>,
    pub profile_id: Option<String>,
    /// Omitted means `desktop`, the only location a bot without an account has.
    pub location: Option<BotLocation>,
    pub device_id: Option<String>,
}

/// The API's `UpdateBotRequest`: omitted fields stay, `null` clears a nullable field.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateBotRequest {
    #[serde(default, deserialize_with = "nullable")]
    pub name: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub instructions: Option<Option<String>>,
    pub appearance: Option<Value>,
    pub model: Option<BotModelPolicy>,
    #[serde(default, deserialize_with = "nullable")]
    pub embedding_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub stt_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub tts_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub image_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub vision_model: Option<Option<String>>,
    pub classification: Option<DataSensitivity>,
    pub capabilities: Option<Vec<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub profile_id: Option<Option<String>>,
    pub location: Option<BotLocation>,
    #[serde(default, deserialize_with = "nullable")]
    pub device_id: Option<Option<String>>,
}

/// Tells an explicit `null` (`Some(None)`) from an omitted field (`None`).
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationMember {
    pub principal_id: String,
    pub kind: PrincipalKind,
    pub added_by: Option<String>,
    pub joined_at: DateTime<FixedOffset>,
}

/// One inbox event. The local inbox has one reader, so each event is stored once.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InboxEntry {
    pub id: String,
    pub message_id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    pub envelope: Value,
    pub created_at: DateTime<FixedOffset>,
    /// The event's position in its conversation (1, 2, …), never reused; null only on entries
    /// from before positions existed.
    pub seq: Option<i64>,
    /// `<createdAt epoch micros>:<entry id>`, as the API writes it.
    pub cursor: String,
}

/// A conversation as the signed-out person sees it, shaped like the API's `Conversation`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conversation {
    pub id: String,
    pub kind: ConversationKind,
    pub title: Option<String>,
    /// Always `plain`.
    pub encryption: ConversationEncryption,
    pub created_by: String,
    pub created_at: DateTime<FixedOffset>,
    /// When the newest message was written, or when the conversation started.
    pub last_activity_at: DateTime<FixedOffset>,
    /// Active members, in the order they joined.
    pub members: Vec<ConversationMember>,
    /// Messages from others after the person's read marker.
    pub unread_count: u64,
    /// The newest message.
    pub last_entry: Option<InboxEntry>,
    /// When the person pinned the conversation to the top of their list.
    pub pinned_at: Option<DateTime<FixedOffset>>,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationPinState {
    pub conversation_id: String,
    pub pinned_at: Option<DateTime<FixedOffset>>,
}

/// A message pinned for the conversation; `entry` and `latest_edit` are the local copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PinnedMessage {
    pub message_id: String,
    pub pinned_by: String,
    pub pinned_at: DateTime<FixedOffset>,
    pub entry: Option<InboxEntry>,
    pub latest_edit: Option<InboxEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarredMessage {
    pub conversation_id: String,
    pub message_id: String,
    pub starred_at: DateTime<FixedOffset>,
    pub entry: InboxEntry,
    pub latest_edit: Option<InboxEntry>,
    /// Opaque; pass back as `before`.
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarredMessagesPage {
    pub stars: Vec<StarredMessage>,
    pub next_before: Option<String>,
}

/// One bot's summary of a conversation's history, shaped like the API's `ConversationSegment`:
/// the events `seq_from..=seq_to` (level 0), or at level 1 a summary of its older segments that
/// keeps their range. Written once; a changed summary is a new `version`. Two bots in one
/// conversation each keep their own; deleting the bot deletes its summaries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationSegment {
    pub id: String,
    pub conversation_id: String,
    /// The bot whose host wrote the summary.
    pub author_id: String,
    pub level: i32,
    pub seq_from: i64,
    pub seq_to: i64,
    pub t_from: DateTime<FixedOffset>,
    pub t_to: DateTime<FixedOffset>,
    pub title: Option<String>,
    pub summary: String,
    /// The level-1 segment that merged this one.
    pub parent_id: Option<String>,
    pub version: i32,
    pub created_at: DateTime<FixedOffset>,
}

/// A page of [`ConversationSegment`]s, the newest version of each range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationSegmentList {
    /// Oldest range first.
    pub segments: Vec<ConversationSegment>,
    /// Pass as `after` for the next page.
    pub next_after: Option<String>,
}

/// A summary a bot's host writes, as in the API: written once, so a changed summary of a range
/// goes under a new id and becomes the range's next version.
#[derive(Debug, Clone, Deserialize)]
pub struct WriteSegmentRequest {
    /// 1 to 64 characters of `[A-Za-z0-9_.-]`, unique per conversation and bot.
    pub id: String,
    /// 0 summarises entries; a higher level, at most 9, merges summaries.
    pub level: i32,
    pub seq_from: i64,
    /// At most the conversation's newest position.
    pub seq_to: i64,
    pub title: Option<String>,
    pub summary: String,
    /// The higher-level segment of the same bot that merged this one.
    pub parent_id: Option<String>,
}

/// The API's segments query: `author` is the bot whose summaries to list, by default the bot of
/// a direct conversation, else every bot's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SegmentsQuery {
    pub author: Option<String>,
    pub level: Option<i32>,
    /// Only segments whose range holds this position.
    pub covering: Option<i64>,
    /// The `next_after` of the previous page.
    pub after: Option<String>,
    /// 1 to 100 segments; 50 by default.
    pub limit: Option<u64>,
}

/// A standing grant, shaped like the API's `BotPermission`: `from_bot_id` may ask `to_bot_id`
/// without asking the person again. Deleting either bot deletes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BotPermission {
    pub id: String,
    /// Always [`crate::LOCAL_OWNER_ID`].
    pub owner_id: String,
    pub from_bot_id: String,
    pub to_bot_id: String,
    pub granted_at: DateTime<FixedOffset>,
    pub granted_by: String,
}

/// The API's `CreateConversationRequest`; `members` are everyone except the person.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateConversationRequest {
    pub kind: ConversationKind,
    pub title: Option<String>,
    pub members: Vec<String>,
    pub encryption: Option<ConversationEncryption>,
}

/// The API's `SendEntryRequest`. Local conversations take one plain `envelope`.
#[derive(Debug, Clone, Deserialize)]
pub struct SendEntryRequest {
    pub message_id: String,
    pub kind: InboxEntryKind,
    pub envelope: Option<Value>,
    pub envelopes: Option<BTreeMap<String, Value>>,
}

/// The API's entries query without `as`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntriesQuery {
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u64>,
}

/// The API's entries query by position: `from_seq` and `to_seq` bound a range, `around_seq` with
/// `before_count` and `after_count` (0 to 50, 10 by default) reads around one entry. Positions
/// count from 1 and never mix with the cursors of [`EntriesQuery`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PositionQuery {
    pub from_seq: Option<i64>,
    pub to_seq: Option<i64>,
    pub around_seq: Option<i64>,
    pub before_count: Option<u64>,
    pub after_count: Option<u64>,
    /// 1 to 100 entries of a range; 50 by default.
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InboxEntriesPage {
    /// Newest first; by position oldest first.
    pub entries: Vec<InboxEntry>,
    /// Set when more entries lie between the oldest entry here and `after` (or the start).
    pub next_before: Option<String>,
    /// Reading by position: more entries follow; pass this as `from_seq` for them.
    pub next_from_seq: Option<i64>,
    /// Reading by position: more entries precede; pass this as `to_seq` for them.
    pub next_to_seq: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MarkReadResponse {
    pub last_read_at: DateTime<FixedOffset>,
}

/// Everything a bot takes along when it moves to an account. Secret values stay in this
/// process: [`crate::LocalBotStore::open_secrets`] hands them to the host's upload.
#[derive(Debug, Clone, Serialize)]
pub struct LocalBotBundle {
    pub bot: Bot,
    pub app_ids: Vec<String>,
    pub secrets: Vec<ExportedSecret>,
    /// Every conversation the bot and the person are both active members of.
    pub conversations: Vec<ExportedConversation>,
}

/// A secret the bot takes along, without its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportedSecret {
    pub name: String,
    pub description: Option<String>,
    /// Whether this device's key opens it; one that doesn't can't move.
    pub readable: bool,
}

/// A secret's value for the upload to the account. Never serialized; Debug hides the value.
pub struct SecretValue {
    pub name: String,
    pub value: Zeroizing<String>,
    pub description: Option<String>,
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretValue")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("description", &self.description)
            .finish()
    }
}

/// A bot's secrets opened with this device's key, by name.
#[derive(Debug, Default)]
pub struct OpenedSecrets {
    pub readable: Vec<SecretValue>,
    /// Secrets the key doesn't open (or no key exists for).
    pub unreadable: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportedConversation {
    pub conversation: Conversation,
    /// Every event, member events included, oldest first.
    pub entries: Vec<InboxEntry>,
    /// Every version of the summaries the exported bot wrote of this conversation, parents
    /// before the segments they merged.
    pub segments: Vec<ConversationSegment>,
}

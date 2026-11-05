//! Inboxes: the conversation, member and entry kinds, envelope v1, entry cursors and the
//! direct-conversation pair key every Flow-Like process shares. The lowercase ids are the wire
//! format; the database stores them uppercase.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;

/// At most this many active members, so one fan-out stays far below DSQL's 3,000-row cap.
pub const MAX_CONVERSATION_MEMBERS: usize = 50;
/// The largest envelope, serialized, the server accepts.
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
/// The envelope format version this server reads and writes.
pub const ENVELOPE_VERSION: u64 = 1;
/// The `enc` of an envelope whose `body` the server can read.
pub const PLAIN_ENCRYPTION: &str = "none";
/// Message ids are 1 to 64 characters of `[A-Za-z0-9_-]`.
pub const MESSAGE_ID_MAX_CHARS: usize = 64;
/// Entry ids in a cursor are 1 to 64 characters of `[A-Za-z0-9_-]`.
pub const ENTRY_ID_MAX_CHARS: usize = 64;

const PLAIN_FIELDS: [&str; 3] = ["v", "enc", "body"];
const SEALED_FIELDS: [&str; 5] = ["v", "enc", "key_id", "enc_key", "ct"];

macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident => $id:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $id),+
                }
            }

            /// The value named `value`, ignoring case and surrounding whitespace, so stored
            /// uppercase values read back.
            pub fn parse(value: &str) -> Option<Self> {
                let value = value.trim();
                Self::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate.as_str().eq_ignore_ascii_case(value))
            }
        }
    };
}

wire_enum!(
    /// A conversation of two principals or a titled group.
    ConversationKind {
        /// Exactly two principals, one conversation per pair.
        Direct => "direct",
        /// A titled conversation of up to [`MAX_CONVERSATION_MEMBERS`] principals.
        Group => "group",
    }
);

wire_enum!(
    /// Whether the server can read what members send.
    ConversationEncryption {
        /// One `enc: "none"` envelope per event.
        Plain => "plain",
        /// One sealed envelope per active member; the server never sees content.
        E2e => "e2e",
    }
);

wire_enum!(
    /// Who a principal id belongs to.
    PrincipalKind {
        User => "user",
        Bot => "bot",
    }
);

wire_enum!(
    /// What one inbox entry records.
    InboxEntryKind {
        Message => "message",
        Reaction => "reaction",
        Edit => "edit",
        Delete => "delete",
        /// A join or leave the server writes; clients never send it.
        Member => "member",
        /// "New topic": the history's next segment starts after this marker.
        Topic => "topic",
        /// Work that wakes a bot: a schedule firing, a webhook, an email or an event. Hosts write
        /// it as the bot's owner into the bot's conversation; clients never send it.
        Trigger => "trigger",
        /// A line about something a bot did elsewhere, e.g. one bot asking another. Hosts write
        /// it, clients never send it, and it wakes no one.
        Activity => "activity",
        /// A call started or ended in the conversation. It wakes no bot and counts as unread
        /// for no one.
        Call => "call",
        /// A line a person spoke in a call, transcribed on their device. It wakes only the
        /// bots it mentions and counts as unread for no one.
        Transcript => "transcript",
        /// A group was renamed or its picture changed. The server writes it; clients never send
        /// it, it wakes no one and counts as unread for no one.
        Room => "room",
    }
);

impl InboxEntryKind {
    pub fn client_sendable(self) -> bool {
        !matches!(
            self,
            Self::Member | Self::Trigger | Self::Activity | Self::Room
        )
    }

    /// Entries that ask a bot to act, as a message from its owner does.
    pub fn wakes_bot(self) -> bool {
        matches!(self, Self::Message | Self::Trigger)
    }

    /// Entries whose plain body names the message they act on in `target`.
    pub fn targets_message(self) -> bool {
        matches!(self, Self::Reaction | Self::Edit | Self::Delete)
    }

    /// Edits and deletions act only on messages their sender wrote.
    pub fn requires_own_target(self) -> bool {
        matches!(self, Self::Edit | Self::Delete)
    }
}

/// What an envelope's `enc` says about its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeKind {
    Plain,
    Sealed,
}

/// Why an envelope was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    NotAnObject,
    UnsupportedVersion,
    MissingEncryption,
    BodyNotAnObject,
    MissingField(&'static str),
    UnexpectedField(String),
    TooLarge(usize),
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => f.write_str("an envelope must be a JSON object"),
            Self::UnsupportedVersion => {
                write!(f, "an envelope needs \"v\": {ENVELOPE_VERSION}")
            }
            Self::MissingEncryption => f.write_str("an envelope needs a non-empty string \"enc\""),
            Self::BodyNotAnObject => {
                f.write_str("a plain envelope (\"enc\": \"none\") needs an object \"body\"")
            }
            Self::MissingField(field) => {
                write!(f, "a sealed envelope needs a non-empty string \"{field}\"")
            }
            Self::UnexpectedField(field) => {
                write!(f, "envelope field \"{field}\" is not part of envelope v1")
            }
            Self::TooLarge(bytes) => write!(
                f,
                "the envelope has {bytes} bytes; at most {MAX_ENVELOPE_BYTES} are allowed"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// Checks an envelope v1 without reading its content: `v` is 1, a plain envelope carries an
/// object `body`, a sealed one carries string `key_id`, `enc_key` and `ct`, nothing else is
/// present (a sealed envelope can't smuggle a readable body) and it serializes to at most
/// [`MAX_ENVELOPE_BYTES`].
pub fn check_envelope(envelope: &Value) -> Result<EnvelopeKind, EnvelopeError> {
    let fields = envelope.as_object().ok_or(EnvelopeError::NotAnObject)?;
    if fields.get("v").and_then(Value::as_u64) != Some(ENVELOPE_VERSION) {
        return Err(EnvelopeError::UnsupportedVersion);
    }
    let enc = fields
        .get("enc")
        .and_then(Value::as_str)
        .filter(|enc| !enc.is_empty())
        .ok_or(EnvelopeError::MissingEncryption)?;
    let kind = if enc == PLAIN_ENCRYPTION {
        check_plain_fields(fields)?
    } else {
        check_sealed_fields(fields)?
    };
    let bytes = serialized_len(envelope);
    if bytes > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError::TooLarge(bytes));
    }
    Ok(kind)
}

type Fields = serde_json::Map<String, Value>;

fn only_fields(fields: &Fields, allowed: &[&str]) -> Result<(), EnvelopeError> {
    match fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(field) => Err(EnvelopeError::UnexpectedField(field.clone())),
        None => Ok(()),
    }
}

fn check_plain_fields(fields: &Fields) -> Result<EnvelopeKind, EnvelopeError> {
    only_fields(fields, &PLAIN_FIELDS)?;
    if !fields.get("body").is_some_and(Value::is_object) {
        return Err(EnvelopeError::BodyNotAnObject);
    }
    Ok(EnvelopeKind::Plain)
}

fn check_sealed_fields(fields: &Fields) -> Result<EnvelopeKind, EnvelopeError> {
    only_fields(fields, &SEALED_FIELDS)?;
    let missing = ["key_id", "enc_key", "ct"].into_iter().find(|field| {
        !fields
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    });
    match missing {
        Some(field) => Err(EnvelopeError::MissingField(field)),
        None => Ok(EnvelopeKind::Sealed),
    }
}

/// The plain envelope v1 around `body`.
pub fn plain_envelope(body: Value) -> Value {
    json!({ "v": ENVELOPE_VERSION, "enc": PLAIN_ENCRYPTION, "body": body })
}

/// The message a plain reaction, edit or deletion acts on: its body's string `target`.
pub fn body_target(envelope: &Value) -> Option<&str> {
    envelope
        .get("body")?
        .get("target")?
        .as_str()
        .filter(|target| !target.is_empty())
}

/// A plain reaction entry: `sender_id` added or removed `emoji` on the message `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionEvent {
    pub message_id: String,
    pub sender_id: String,
    pub target: String,
    pub emoji: String,
    pub add: bool,
}

impl ReactionEvent {
    /// The reaction in a plain envelope's body `{ target, emoji, op? }`; `op` defaults to `add`.
    pub fn parse(message_id: &str, sender_id: &str, envelope: &Value) -> Option<Self> {
        let body = envelope.get("body")?;
        let emoji = body
            .get("emoji")?
            .as_str()
            .filter(|emoji| !emoji.is_empty())?;
        let add = match body.get("op").and_then(Value::as_str) {
            None | Some("add") => true,
            Some("remove") => false,
            Some(_) => return None,
        };
        Some(Self {
            message_id: message_id.to_string(),
            sender_id: sender_id.to_string(),
            target: body_target(envelope)?.to_string(),
            emoji: emoji.to_string(),
            add,
        })
    }
}

/// The reactions still in place: the additions among `events` (oldest first) that no later
/// removal by the same sender of the same emoji on the same message undid.
pub fn net_reactions(events: &[ReactionEvent]) -> Vec<ReactionEvent> {
    events
        .iter()
        .enumerate()
        .filter(|(index, event)| {
            event.add
                && !events[index + 1..].iter().any(|later| {
                    !later.add
                        && later.sender_id == event.sender_id
                        && later.target == event.target
                        && later.emoji == event.emoji
                })
        })
        .map(|(_, event)| event.clone())
        .collect()
}

/// The body field only tombstones carry.
pub const TOMBSTONE_FIELD: &str = "deleted";

/// What every copy of a deleted plain message, and of its edits, holds from then on: the entry,
/// its position, sender and time stay; the content is gone.
pub fn tombstone_envelope() -> Value {
    plain_envelope(json!({ TOMBSTONE_FIELD: true }))
}

/// Whether a plain envelope is a deleted message's tombstone.
pub fn is_tombstone(envelope: &Value) -> bool {
    envelope.get("enc").and_then(Value::as_str) == Some(PLAIN_ENCRYPTION)
        && envelope
            .get("body")
            .and_then(|body| body.get(TOMBSTONE_FIELD))
            .and_then(Value::as_bool)
            == Some(true)
}

/// A "New topic" marker's title is at most this many characters.
pub const TOPIC_TITLE_MAX_CHARS: usize = 120;

/// The body of a `topic` entry: where the history's next segment starts, optionally named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TopicBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl TopicBody {
    /// The topic marker in `body`, its title at most [`TOPIC_TITLE_MAX_CHARS`] characters.
    pub fn parse(body: &Value) -> Result<Self, String> {
        let topic: Self = serde_json::from_value(body.clone())
            .map_err(|error| format!("a topic body is {{ title? }}: {error}"))?;
        let chars = topic
            .title
            .as_deref()
            .map_or(0, |title| title.chars().count());
        if chars > TOPIC_TITLE_MAX_CHARS {
            return Err(format!(
                "a topic title has at most {TOPIC_TITLE_MAX_CHARS} characters; this one has {chars}"
            ));
        }
        Ok(topic)
    }
}

/// A call lasts at most this many milliseconds.
pub const CALL_MAX_DURATION_MS: u64 = 86_400_000;
/// A transcript line's text is at most this many characters.
pub const TRANSCRIPT_TEXT_MAX_CHARS: usize = 4_000;
/// One transcribed utterance lasts at most this many milliseconds.
pub const TRANSCRIPT_MAX_DURATION_MS: u64 = 15 * 60 * 1000;
/// A transcript names at most this many principals.
pub const TRANSCRIPT_MAX_MENTIONS: usize = MAX_CONVERSATION_MEMBERS;

/// What bots read in front of a transcribed line.
pub const SPOKEN_IN_CALL: &str = "(spoken in call)";

/// The body of a `call` entry. A call's id is the `message_id` of its `started` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CallBody {
    /// The call begins; the entry's `message_id` is the call's id.
    Started,
    /// The last person left call `call` after `duration_ms`.
    Ended { call: String, duration_ms: u64 },
}

impl CallBody {
    pub fn parse(body: &Value) -> Result<Self, String> {
        let call: Self = serde_json::from_value(body.clone()).map_err(|error| {
            format!(
                "a call body is {{ op: \"started\" }} or {{ op: \"ended\", call, duration_ms }}: {error}"
            )
        })?;
        if let Self::Ended {
            call: id,
            duration_ms,
        } = &call
        {
            check_call_id(id)?;
            if *duration_ms > CALL_MAX_DURATION_MS {
                return Err(format!(
                    "a call lasts at most {CALL_MAX_DURATION_MS} ms; body.duration_ms is {duration_ms}"
                ));
            }
        }
        Ok(call)
    }
}

fn check_call_id(call: &str) -> Result<(), String> {
    if is_valid_message_id(call) {
        return Ok(());
    }
    Err(format!(
        "body.call '{}' must be the call's message id: 1 to {MESSAGE_ID_MAX_CHARS} characters of A-Z, a-z, 0-9, '_' and '-'",
        call.chars().take(80).collect::<String>()
    ))
}

/// The body of a `transcript` entry: one utterance of a person in a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptBody {
    pub call: String,
    pub text: String,
    /// When the utterance began, in milliseconds after the call started.
    pub offset_ms: u64,
    pub duration_ms: u64,
    #[serde(default)]
    pub mentions: Vec<String>,
}

impl TranscriptBody {
    /// The line a bot reads: the text, marked as spoken in a call.
    pub fn render(&self) -> String {
        format!("{SPOKEN_IN_CALL} {}", self.text)
    }

    pub fn parse(body: &Value) -> Result<Self, String> {
        let transcript: Self = serde_json::from_value(body.clone()).map_err(|error| {
            format!(
                "a transcript body is {{ call, text, offset_ms, duration_ms, mentions? }}: {error}"
            )
        })?;
        check_call_id(&transcript.call)?;
        let chars = transcript.text.chars().count();
        if transcript.text.trim().is_empty() || chars > TRANSCRIPT_TEXT_MAX_CHARS {
            return Err(format!(
                "a transcript text has 1 to {TRANSCRIPT_TEXT_MAX_CHARS} characters; this one has {chars}"
            ));
        }
        if transcript.duration_ms > TRANSCRIPT_MAX_DURATION_MS {
            return Err(format!(
                "a transcript lasts at most {TRANSCRIPT_MAX_DURATION_MS} ms; body.duration_ms is {}",
                transcript.duration_ms
            ));
        }
        if transcript.mentions.len() > TRANSCRIPT_MAX_MENTIONS {
            return Err(format!(
                "body.mentions names at most {TRANSCRIPT_MAX_MENTIONS} principals; this one names {}",
                transcript.mentions.len()
            ));
        }
        if let Some(mention) = transcript.mentions.iter().find(|mention| {
            mention.is_empty()
                || mention.chars().count() > crate::bot_message::MAX_NAME_CHARS
                || mention
                    .chars()
                    .any(|char| char.is_whitespace() || char.is_control())
        }) {
            return Err(format!(
                "body.mentions '{}' must be 1 to {} characters without spaces",
                mention.chars().take(80).collect::<String>(),
                crate::bot_message::MAX_NAME_CHARS
            ));
        }
        Ok(transcript)
    }
}

/// What wakes a bot through a `trigger` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    Schedule,
    Webhook,
    Email,
    Event,
}

impl TriggerSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::Email => "email",
            Self::Event => "event",
        }
    }
}

/// A trigger's title is at most this many characters.
pub const TRIGGER_TITLE_MAX_CHARS: usize = 200;
/// A trigger's text is at most this many characters.
pub const TRIGGER_TEXT_MAX_CHARS: usize = 8_000;
/// An inline trigger payload serializes to at most this many bytes; larger ones go through
/// `payload_handle`.
pub const TRIGGER_PAYLOAD_MAX_BYTES: usize = 32 * 1024;

/// The body of a `trigger` entry. A host writes it in a plain envelope as the bot's owner, into
/// the bot's conversation with them; the bot answers it like an owner message with 0 hops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TriggerBody {
    pub source: TriggerSource,
    /// The task, webhook, mail or event that fired; with the time it fires it names the entry.
    pub ref_id: String,
    pub title: String,
    /// What the bot is asked to do.
    pub text: String,
    /// Structured data of the trigger, at most [`TRIGGER_PAYLOAD_MAX_BYTES`] serialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// A payload too large to inline, kept in the bot's workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_handle: Option<crate::bot_message::WorkspaceHandle>,
    /// The bot to wake when the conversation is a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
}

impl TriggerBody {
    pub fn parse(body: &Value) -> Result<Self, String> {
        let trigger: Self = serde_json::from_value(body.clone()).map_err(|error| {
            format!("a trigger body is {{ source, ref_id, title, text }}: {error}")
        })?;
        if trigger.ref_id.trim().is_empty() {
            return Err("a trigger needs a non-empty body.ref_id".to_string());
        }
        let title = trigger.title.chars().count();
        if title > TRIGGER_TITLE_MAX_CHARS {
            return Err(format!(
                "a trigger title has at most {TRIGGER_TITLE_MAX_CHARS} characters; this one has {title}"
            ));
        }
        let text = trigger.text.chars().count();
        if text > TRIGGER_TEXT_MAX_CHARS {
            return Err(format!(
                "a trigger text has at most {TRIGGER_TEXT_MAX_CHARS} characters; this one has {text}"
            ));
        }
        if let Some(payload) = &trigger.payload {
            let bytes = serialized_len(payload);
            if bytes > TRIGGER_PAYLOAD_MAX_BYTES {
                return Err(format!(
                    "a trigger payload has at most {TRIGGER_PAYLOAD_MAX_BYTES} bytes; this one has {bytes}; put it in the workspace and send payload_handle"
                ));
            }
        }
        Ok(trigger)
    }

    pub fn envelope(&self) -> Value {
        plain_envelope(serde_json::to_value(self).unwrap_or(Value::Null))
    }

    /// What the model reads for this trigger: its text (the title when there is none), then the
    /// data that came with it, marked with where it came from.
    pub fn render(&self) -> String {
        let mut text = if self.text.trim().is_empty() {
            self.title.clone()
        } else {
            self.text.clone()
        };
        let source = self.source.as_str();
        if let Some(payload) = &self.payload {
            text.push_str(&format!("\n\nPayload ({source}): {payload}"));
        }
        if let Some(handle) = &self.payload_handle {
            text.push_str(&format!(
                "\n\nPayload ({source}) saved as file: {}",
                handle.path
            ));
        }
        text
    }
}

/// An activity's question is at most this many characters.
pub const ACTIVITY_QUESTION_MAX_CHARS: usize = 300;

/// The body of an `activity` entry, written in a plain envelope by a host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityBody {
    /// `asker_id` asked `callee_id`; question and answer live in the exchange `exchange_id`.
    BotAsk {
        ask_id: String,
        asker_id: String,
        callee_id: String,
        exchange_id: String,
        question: String,
    },
}

impl ActivityBody {
    /// A `bot_ask` whose question is clipped to [`ACTIVITY_QUESTION_MAX_CHARS`].
    pub fn bot_ask(
        ask_id: impl Into<String>,
        asker_id: impl Into<String>,
        callee_id: impl Into<String>,
        exchange_id: impl Into<String>,
        question: &str,
    ) -> Self {
        Self::BotAsk {
            ask_id: ask_id.into(),
            asker_id: asker_id.into(),
            callee_id: callee_id.into(),
            exchange_id: exchange_id.into(),
            question: question.chars().take(ACTIVITY_QUESTION_MAX_CHARS).collect(),
        }
    }

    pub fn parse(body: &Value) -> Result<Self, String> {
        let activity: Self = serde_json::from_value(body.clone()).map_err(|error| {
            format!(
                "an activity body is {{ type: \"bot_ask\", ask_id, asker_id, callee_id, exchange_id, question }}: {error}"
            )
        })?;
        let Self::BotAsk {
            ask_id,
            asker_id,
            callee_id,
            exchange_id,
            question,
        } = &activity;
        for (name, value) in [
            ("ask_id", ask_id),
            ("asker_id", asker_id),
            ("callee_id", callee_id),
            ("exchange_id", exchange_id),
        ] {
            if value.trim().is_empty() {
                return Err(format!("an activity needs a non-empty body.{name}"));
            }
        }
        let chars = question.chars().count();
        if chars > ACTIVITY_QUESTION_MAX_CHARS {
            return Err(format!(
                "an activity question has at most {ACTIVITY_QUESTION_MAX_CHARS} characters; this one has {chars}"
            ));
        }
        Ok(activity)
    }

    pub fn envelope(&self) -> Value {
        plain_envelope(serde_json::to_value(self).unwrap_or(Value::Null))
    }
}

struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_len(value: &Value) -> usize {
    let mut count = ByteCount(0);
    serde_json::to_writer(&mut count, value).map_or(usize::MAX, |()| count.0)
}

fn is_id_text(value: &str, max_chars: usize) -> bool {
    (1..=max_chars).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A sender-chosen message id: 1 to 64 characters of `[A-Za-z0-9_-]`.
pub fn is_valid_message_id(message_id: &str) -> bool {
    is_id_text(message_id, MESSAGE_ID_MAX_CHARS)
}

/// The position of an entry in an inbox: `<createdAt epoch micros>:<entry id>`. Cursors order
/// by the micros numerically, then by the entry id bytewise, which the derived `Ord` does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryCursor {
    pub created_at_micros: i64,
    pub entry_id: String,
}

/// Why a cursor string was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorError(String);

impl fmt::Display for CursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cursor '{}' is not <createdAt epoch micros>:<entry id>",
            self.0
        )
    }
}

impl std::error::Error for CursorError {}

impl EntryCursor {
    pub fn new(created_at_micros: i64, entry_id: impl Into<String>) -> Self {
        Self {
            created_at_micros,
            entry_id: entry_id.into(),
        }
    }

    pub fn parse(value: &str) -> Result<Self, CursorError> {
        let refuse = || CursorError(value.chars().take(160).collect());
        let (micros, entry_id) = value.split_once(':').ok_or_else(refuse)?;
        if micros.is_empty() || !micros.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(refuse());
        }
        let created_at_micros = micros.parse::<i64>().map_err(|_| refuse())?;
        if !is_id_text(entry_id, ENTRY_ID_MAX_CHARS) {
            return Err(refuse());
        }
        Ok(Self::new(created_at_micros, entry_id))
    }
}

impl fmt::Display for EntryCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.created_at_micros, self.entry_id)
    }
}

/// The key of the direct conversation between `a` and `b`: the same for either order and
/// stable across releases, so one pair always finds its one conversation.
pub fn direct_pair_key(a: &str, b: &str) -> String {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.inbox.direct/v1\0");
    for principal in [first, second] {
        hasher.update(&(principal.len() as u64).to_be_bytes());
        hasher.update(principal.as_bytes());
    }
    format!("v1:{}", hasher.finalize().to_hex())
}

/// Whether a member joined or left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberOp {
    Join,
    Leave,
}

/// The body of a `member` entry. The server writes it in a plain envelope, also in end-to-end
/// encrypted conversations: membership is metadata the server knows anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberEventBody {
    pub op: MemberOp,
    /// Who joined or left.
    pub principal_id: String,
    /// Who added or removed them; the principal itself when they left on their own.
    pub by: String,
}

impl MemberEventBody {
    pub fn envelope(&self) -> Value {
        plain_envelope(json!({
            "op": self.op,
            "principal_id": self.principal_id,
            "by": self.by,
        }))
    }
}

/// What a `room` entry says changed about a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomChange {
    Title,
    Image,
}

/// The body of a `room` entry: a member renamed a group or changed its picture. Plain, also in
/// end-to-end encrypted conversations, like `member` entries: the server holds both anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomEventBody {
    pub change: RoomChange,
    /// The new name, on a `title` change that set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The name or picture was removed rather than set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// Who changed it.
    pub by: String,
}

impl RoomEventBody {
    /// A rename to `title`, or with `None` the removal of the name.
    pub fn title(title: Option<String>, by: &str) -> Self {
        Self {
            change: RoomChange::Title,
            removed: title.is_none(),
            title,
            by: by.to_string(),
        }
    }

    /// A new picture, or with `removed` the removal of the picture.
    pub fn image(removed: bool, by: &str) -> Self {
        Self {
            change: RoomChange::Image,
            title: None,
            removed,
            by: by.to_string(),
        }
    }

    pub fn envelope(&self) -> Value {
        plain_envelope(serde_json::to_value(self).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed() -> Value {
        json!({
            "v": 1,
            "enc": "hpke-x25519-sha256-chacha20poly1305",
            "key_id": "key-1",
            "enc_key": "ZW5j",
            "ct": "Y3Q=",
        })
    }

    #[test]
    fn kinds_use_lowercase_ids_and_read_stored_uppercase_values() {
        for kind in InboxEntryKind::ALL {
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, Value::String(kind.as_str().to_string()));
            assert_eq!(
                InboxEntryKind::parse(&kind.as_str().to_ascii_uppercase()),
                Some(*kind)
            );
        }
        assert_eq!(
            ConversationKind::ALL
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>(),
            ["direct", "group"]
        );
        assert_eq!(
            serde_json::to_value(ConversationEncryption::E2e).unwrap(),
            "e2e"
        );
        assert_eq!(
            ConversationEncryption::parse("PLAIN"),
            Some(ConversationEncryption::Plain)
        );
        assert_eq!(PrincipalKind::parse(" BOT "), Some(PrincipalKind::Bot));
        assert_eq!(InboxEntryKind::parse("typing"), None);
        assert!(serde_json::from_value::<ConversationKind>(json!("channel")).is_err());
    }

    #[test]
    fn only_the_server_writes_member_entries_and_only_edits_need_their_own_target() {
        let sendable: Vec<_> = InboxEntryKind::ALL
            .iter()
            .filter(|kind| kind.client_sendable())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(
            sendable,
            [
                "message",
                "reaction",
                "edit",
                "delete",
                "topic",
                "call",
                "transcript"
            ]
        );
        assert!(!InboxEntryKind::Call.wakes_bot());
        assert!(!InboxEntryKind::Transcript.wakes_bot());
        assert!(!InboxEntryKind::Call.targets_message());
        assert!(!InboxEntryKind::Transcript.targets_message());
        assert!(InboxEntryKind::Trigger.wakes_bot());
        assert!(InboxEntryKind::Message.wakes_bot());
        assert!(!InboxEntryKind::Topic.wakes_bot());
        assert!(!InboxEntryKind::Activity.wakes_bot());
        assert!(!InboxEntryKind::Activity.client_sendable());
        assert!(InboxEntryKind::Reaction.targets_message());
        assert!(!InboxEntryKind::Reaction.requires_own_target());
        assert!(InboxEntryKind::Edit.requires_own_target());
        assert!(InboxEntryKind::Delete.requires_own_target());
        assert!(!InboxEntryKind::Message.targets_message());
        assert!(!InboxEntryKind::Topic.targets_message());
    }

    #[test]
    fn a_call_body_is_started_or_ended_with_a_bounded_duration() {
        assert_eq!(
            CallBody::parse(&json!({ "op": "started" })),
            Ok(CallBody::Started)
        );
        assert_eq!(
            CallBody::parse(&json!({ "op": "ended", "call": "c_1", "duration_ms": 1500 })),
            Ok(CallBody::Ended {
                call: "c_1".into(),
                duration_ms: 1500
            })
        );
        let longest = json!({ "op": "ended", "call": "c_1", "duration_ms": CALL_MAX_DURATION_MS });
        assert!(CallBody::parse(&longest).is_ok());
        let too_long =
            json!({ "op": "ended", "call": "c_1", "duration_ms": CALL_MAX_DURATION_MS + 1 });
        assert!(
            CallBody::parse(&too_long)
                .unwrap_err()
                .contains("body.duration_ms")
        );
        let long_id = "c".repeat(MESSAGE_ID_MAX_CHARS + 1);
        let error = CallBody::parse(&json!({ "op": "ended", "call": long_id, "duration_ms": 1 }))
            .unwrap_err();
        assert!(error.contains("body.call"));
        assert!(CallBody::parse(&json!({ "op": "ended", "duration_ms": 1 })).is_err());
        assert!(CallBody::parse(&json!({ "op": "ringing" })).is_err());
        assert!(CallBody::parse(&json!({})).is_err());
    }

    #[test]
    fn a_transcript_body_enforces_its_text_duration_and_mention_limits() {
        let line = |text: &str| json!({ "call": "c_1", "text": text, "offset_ms": 2000, "duration_ms": 3000, "mentions": ["bot_1"] });
        let parsed = TranscriptBody::parse(&line("hello")).unwrap();
        assert_eq!(parsed.mentions, ["bot_1"]);
        assert_eq!((parsed.offset_ms, parsed.duration_ms), (2000, 3000));
        let no_mentions = json!({ "call": "c_1", "text": "hi", "offset_ms": 0, "duration_ms": 10 });
        assert!(
            TranscriptBody::parse(&no_mentions)
                .unwrap()
                .mentions
                .is_empty()
        );
        assert_eq!(parsed.render(), "(spoken in call) hello");
        assert!(TranscriptBody::parse(&line(&"a".repeat(TRANSCRIPT_TEXT_MAX_CHARS))).is_ok());
        let long = TranscriptBody::parse(&line(&"a".repeat(TRANSCRIPT_TEXT_MAX_CHARS + 1)));
        assert!(long.unwrap_err().contains("4000"));
        assert!(TranscriptBody::parse(&line("   ")).is_err());
        let slow = json!({ "call": "c_1", "text": "hi", "offset_ms": 0, "duration_ms": TRANSCRIPT_MAX_DURATION_MS + 1 });
        assert!(
            TranscriptBody::parse(&slow)
                .unwrap_err()
                .contains("body.duration_ms")
        );
        let crowd: Vec<String> = (0..=TRANSCRIPT_MAX_MENTIONS)
            .map(|n| format!("p{n}"))
            .collect();
        let crowded = json!({ "call": "c_1", "text": "hi", "offset_ms": 0, "duration_ms": 1, "mentions": crowd });
        assert!(
            TranscriptBody::parse(&crowded)
                .unwrap_err()
                .contains("body.mentions")
        );
        let spaced = json!({ "call": "c_1", "text": "hi", "offset_ms": 0, "duration_ms": 1, "mentions": ["has space"] });
        assert!(TranscriptBody::parse(&spaced).is_err());
        let bad_call =
            json!({ "call": "has space", "text": "hi", "offset_ms": 0, "duration_ms": 1 });
        assert!(
            TranscriptBody::parse(&bad_call)
                .unwrap_err()
                .contains("body.call")
        );
        assert!(TranscriptBody::parse(&json!({ "call": "c_1", "text": "hi" })).is_err());
    }

    #[test]
    fn a_trigger_body_round_trips_and_enforces_its_limits() {
        let body = TriggerBody {
            source: TriggerSource::Schedule,
            ref_id: "task-1".into(),
            title: "Report".into(),
            text: "Check the report".into(),
            payload: Some(json!({ "n": 1 })),
            payload_handle: None,
            bot_id: None,
        };
        let envelope = body.envelope();
        assert_eq!(check_envelope(&envelope), Ok(EnvelopeKind::Plain));
        assert_eq!(TriggerBody::parse(&envelope["body"]), Ok(body.clone()));
        assert_eq!(
            body.render(),
            "Check the report\n\nPayload (schedule): {\"n\":1}"
        );
        let mut big = body.clone();
        big.payload = Some(json!("x".repeat(TRIGGER_PAYLOAD_MAX_BYTES)));
        assert!(TriggerBody::parse(&big.envelope()["body"]).is_err());
        let mut blank = body;
        blank.ref_id = " ".into();
        assert!(TriggerBody::parse(&blank.envelope()["body"]).is_err());
        assert!(TriggerBody::parse(&json!({ "source": "carrier-pigeon" })).is_err());
    }

    #[test]
    fn an_activity_body_round_trips_clips_and_rejects() {
        let body = ActivityBody::bot_ask("a1", "scout", "pebble", "ex1", &"q".repeat(500));
        let ActivityBody::BotAsk { question, .. } = &body;
        assert_eq!(question.chars().count(), ACTIVITY_QUESTION_MAX_CHARS);
        let envelope = body.envelope();
        assert_eq!(check_envelope(&envelope), Ok(EnvelopeKind::Plain));
        assert_eq!(ActivityBody::parse(&envelope["body"]), Ok(body.clone()));
        assert_eq!(envelope["body"]["type"], "bot_ask");
        let blank = ActivityBody::bot_ask("a1", " ", "pebble", "ex1", "hi");
        assert!(ActivityBody::parse(&blank.envelope()["body"]).is_err());
        let long = json!({ "type": "bot_ask", "ask_id": "a", "asker_id": "b", "callee_id": "c",
            "exchange_id": "e", "question": "x".repeat(ACTIVITY_QUESTION_MAX_CHARS + 1) });
        assert!(ActivityBody::parse(&long).is_err());
        assert!(ActivityBody::parse(&json!({ "type": "nope" })).is_err());
    }

    #[test]
    fn a_tombstone_is_a_plain_envelope_without_content() {
        let tombstone = tombstone_envelope();
        assert_eq!(
            tombstone,
            json!({ "v": 1, "enc": "none", "body": { "deleted": true } })
        );
        assert_eq!(check_envelope(&tombstone), Ok(EnvelopeKind::Plain));
        assert!(is_tombstone(&tombstone));
        assert!(!is_tombstone(&plain_envelope(json!({ "text": "deleted" }))));
        assert!(!is_tombstone(&plain_envelope(json!({ "deleted": "yes" }))));
        assert!(!is_tombstone(&sealed()));
    }

    #[test]
    fn a_topic_marker_has_an_optional_short_title() {
        assert_eq!(TopicBody::parse(&json!({})), Ok(TopicBody::default()));
        let named = TopicBody::parse(&json!({ "title": "Launch" })).unwrap();
        assert_eq!(named.title.as_deref(), Some("Launch"));
        let longest = "x".repeat(TOPIC_TITLE_MAX_CHARS);
        assert!(TopicBody::parse(&json!({ "title": longest })).is_ok());
        let long = "x".repeat(TOPIC_TITLE_MAX_CHARS + 1);
        let error = TopicBody::parse(&json!({ "title": long })).unwrap_err();
        assert!(error.contains("121"), "{error}");
        assert!(TopicBody::parse(&json!({ "title": 5 })).is_err());
    }

    #[test]
    fn plain_and_sealed_envelopes_pass_and_name_their_kind() {
        let plain = plain_envelope(json!({ "text": "Hi", "reply_to": "msg_1" }));
        assert_eq!(check_envelope(&plain), Ok(EnvelopeKind::Plain));
        assert_eq!(check_envelope(&sealed()), Ok(EnvelopeKind::Sealed));
        assert_eq!(body_target(&plain), None);
        let reaction = plain_envelope(json!({ "target": "msg_1", "emoji": "👍", "op": "add" }));
        assert_eq!(body_target(&reaction), Some("msg_1"));
    }

    fn malformed_plain_envelopes() -> Vec<(Value, EnvelopeError)> {
        vec![
            (json!("text"), EnvelopeError::NotAnObject),
            (
                json!({ "v": 2, "enc": "none", "body": {} }),
                EnvelopeError::UnsupportedVersion,
            ),
            (
                json!({ "v": "1", "enc": "none", "body": {} }),
                EnvelopeError::UnsupportedVersion,
            ),
            (
                json!({ "v": 1.5, "enc": "none", "body": {} }),
                EnvelopeError::UnsupportedVersion,
            ),
            (
                json!({ "v": 1, "body": {} }),
                EnvelopeError::MissingEncryption,
            ),
            (
                json!({ "v": 1, "enc": "", "body": {} }),
                EnvelopeError::MissingEncryption,
            ),
            (
                json!({ "v": 1, "enc": "none", "body": "Hi" }),
                EnvelopeError::BodyNotAnObject,
            ),
            (
                json!({ "v": 1, "enc": "none" }),
                EnvelopeError::BodyNotAnObject,
            ),
            (
                json!({ "v": 1, "enc": "none", "body": {}, "ct": "x" }),
                EnvelopeError::UnexpectedField("ct".into()),
            ),
        ]
    }

    #[test]
    fn malformed_envelopes_are_refused_with_the_reason() {
        let mut sealed_with_body = sealed();
        sealed_with_body["body"] = json!({ "text": "plaintext" });
        let mut no_ct = sealed();
        no_ct["ct"] = json!("");
        let mut cases = malformed_plain_envelopes();
        cases.push((
            sealed_with_body,
            EnvelopeError::UnexpectedField("body".into()),
        ));
        cases.push((no_ct, EnvelopeError::MissingField("ct")));
        for (envelope, expected) in cases {
            assert_eq!(check_envelope(&envelope), Err(expected), "{envelope}");
        }
        assert!(
            EnvelopeError::MissingField("ct")
                .to_string()
                .contains("\"ct\"")
        );
    }

    #[test]
    fn envelopes_are_limited_to_64_kib_serialized() {
        let overhead = serialized_len(&plain_envelope(json!({ "text": "" })));
        let fits = plain_envelope(json!({ "text": "x".repeat(MAX_ENVELOPE_BYTES - overhead) }));
        assert_eq!(serialized_len(&fits), MAX_ENVELOPE_BYTES);
        assert_eq!(check_envelope(&fits), Ok(EnvelopeKind::Plain));

        let over = plain_envelope(json!({ "text": "x".repeat(MAX_ENVELOPE_BYTES - overhead + 1) }));
        assert_eq!(
            check_envelope(&over),
            Err(EnvelopeError::TooLarge(MAX_ENVELOPE_BYTES + 1))
        );
    }

    #[test]
    fn message_ids_are_short_url_safe_tokens() {
        for id in ["msg_0a1b", "a", "A-Z_09", &"x".repeat(MESSAGE_ID_MAX_CHARS)] {
            assert!(is_valid_message_id(id), "{id}");
        }
        for id in [
            "",
            "with space",
            "dot.ted",
            "member:abc",
            "ümlaut",
            &"x".repeat(MESSAGE_ID_MAX_CHARS + 1),
        ] {
            assert!(!is_valid_message_id(id), "{id}");
        }
    }

    #[test]
    fn cursors_round_trip_and_order_by_time_then_id_bytes() {
        let cursor = EntryCursor::new(1_800_000_000_123_000, "ckabc123");
        assert_eq!(cursor.to_string(), "1800000000123000:ckabc123");
        assert_eq!(EntryCursor::parse(&cursor.to_string()), Ok(cursor.clone()));

        let mut cursors = vec![
            EntryCursor::parse("10:a").unwrap(),
            EntryCursor::parse("9:z").unwrap(),
            EntryCursor::parse("10:B").unwrap(),
            EntryCursor::parse("10:ab").unwrap(),
        ];
        cursors.sort();
        let ordered: Vec<String> = cursors.iter().map(ToString::to_string).collect();
        assert_eq!(ordered, ["9:z", "10:B", "10:a", "10:ab"]);
    }

    #[test]
    fn malformed_cursors_are_refused() {
        for value in [
            "",
            "123",
            ":abc",
            "123:",
            "-5:abc",
            "+5:abc",
            "1e3:abc",
            "99999999999999999999:abc",
            "123:a b",
            "123:a:b",
            &format!("1:{}", "x".repeat(ENTRY_ID_MAX_CHARS + 1)),
        ] {
            let error = EntryCursor::parse(value).unwrap_err();
            assert!(error.to_string().contains("is not <createdAt"), "{value}");
        }
    }

    #[test]
    fn the_pair_key_ignores_order_and_separates_every_pair() {
        let key = direct_pair_key("user-a", "bot_b");
        assert_eq!(key, direct_pair_key("bot_b", "user-a"));
        assert!(key.starts_with("v1:"));
        assert_eq!(key.len(), 3 + 64);
        assert_ne!(key, direct_pair_key("user-a", "bot_c"));
        assert_ne!(direct_pair_key("ab", "c"), direct_pair_key("a", "bc"));
        assert_eq!(
            key,
            "v1:9e498726bca879afa9a8bbe9a1bfc4a678c229502c21834e1866c0d4e88994bc"
        );
    }

    #[test]
    fn member_entries_are_plain_envelopes_with_the_planned_body() {
        let body = MemberEventBody {
            op: MemberOp::Leave,
            principal_id: "bot_1".into(),
            by: "user-1".into(),
        };
        let envelope = body.envelope();
        assert_eq!(
            envelope,
            json!({
                "v": 1,
                "enc": "none",
                "body": { "op": "leave", "principal_id": "bot_1", "by": "user-1" },
            })
        );
        assert_eq!(check_envelope(&envelope), Ok(EnvelopeKind::Plain));
        assert_eq!(
            serde_json::from_value::<MemberEventBody>(envelope["body"].clone()).unwrap(),
            body
        );
    }

    #[test]
    fn room_entries_are_server_written_plain_envelopes() {
        assert!(!InboxEntryKind::Room.client_sendable());
        assert!(!InboxEntryKind::Room.wakes_bot());
        let renamed = RoomEventBody::title(Some("Launch crew".into()), "user-1");
        assert_eq!(
            renamed.envelope(),
            json!({
                "v": 1,
                "enc": "none",
                "body": { "change": "title", "title": "Launch crew", "by": "user-1" },
            })
        );
        let cleared = RoomEventBody::image(true, "user-2");
        assert_eq!(
            cleared.envelope()["body"],
            json!({ "change": "image", "removed": true, "by": "user-2" })
        );
        assert_eq!(check_envelope(&cleared.envelope()), Ok(EnvelopeKind::Plain));
        assert_eq!(
            serde_json::from_value::<RoomEventBody>(
                json!({ "change": "title", "removed": true, "by": "u" })
            )
            .unwrap(),
            RoomEventBody::title(None, "u")
        );
    }

    #[test]
    fn a_removed_reaction_is_no_longer_in_place_but_a_later_one_is() {
        let react = |id: &str, sender: &str, emoji: &str, op: &str| {
            let envelope = serde_json::json!({
                "body": { "target": "m1", "emoji": emoji, "op": op }
            });
            ReactionEvent::parse(id, sender, &envelope).unwrap()
        };
        let events = [
            react("r1", "ann", "👍", "add"),
            react("r2", "ann", "👍", "remove"),
            react("r3", "ann", "❓", "add"),
            react("r4", "bob", "👍", "add"),
            react("r5", "bob", "👍", "remove"),
            react("r6", "bob", "👍", "add"),
        ];
        let kept: Vec<_> = net_reactions(&events)
            .into_iter()
            .map(|event| event.message_id)
            .collect();
        assert_eq!(kept, ["r3", "r6"]);
        let no_op = serde_json::json!({ "body": { "target": "m1", "emoji": "👍" } });
        assert!(ReactionEvent::parse("r", "ann", &no_op).unwrap().add);
        assert!(
            ReactionEvent::parse(
                "r",
                "ann",
                &serde_json::json!({ "body": { "target": "m1" } })
            )
            .is_none()
        );
    }
}

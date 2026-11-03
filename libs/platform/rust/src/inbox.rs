//! Inboxes: conversations between people and bots. Every member gets its own copy of each event
//! (an [`InboxEntry`]); copies of one event share `message_id`. These routes accept people only
//! (personal access tokens), not API keys or running flows.

use crate::{Client, Error, Method, RequestOptions, Result, Value, json};
use futures_util::{
    StreamExt, TryStreamExt,
    stream::{self, BoxStream},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, hash_map::RandomState},
    hash::{BuildHasher, Hasher},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// A string enum of the API whose unknown values deserialize into `Other`, so a newer server
/// never breaks an older client.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
            /// A value this version of the client does not know.
            Other(String),
        }

        impl $name {
            /// The id on the wire.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Other(value) => value,
                }
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $($wire => Self::$variant,)+
                    other => Self::Other(other.to_owned()),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let value = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::from(value.as_str()))
            }
        }
    };
}
pub(crate) use wire_enum;

wire_enum!(
    /// `Direct` holds two principals, one conversation per pair; `Group` up to 50.
    ConversationKind {
        Direct => "direct",
        Group => "group",
    }
);

wire_enum!(
    /// Whether the server can read what members send.
    ConversationEncryption {
        Plain => "plain",
        /// Every member receives an envelope sealed to their key.
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
    }
);

/// Whether a reaction is added or taken back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReactionOp {
    Add,
    Remove,
}

/// A principal in a conversation: a user sub or a bot id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMember {
    pub principal_id: String,
    pub kind: PrincipalKind,
    /// Who added the member; `None` for the person who started the conversation.
    pub added_by: Option<String>,
    pub joined_at: String,
}

/// One principal's copy of one event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxEntry {
    pub id: String,
    /// Chosen by the sender and the same in every member's copy.
    pub message_id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    /// Envelope v1: `{ v: 1, enc: "none", body }`, or sealed to this inbox's key.
    pub envelope: Value,
    pub created_at: String,
    /// Opaque position; pass it back as `before`, `after` or `up_to`.
    pub cursor: String,
}

impl InboxEntry {
    /// The body of a plain envelope; `None` when the envelope is sealed.
    pub fn body(&self) -> Option<&Value> {
        if self.envelope.get("enc").and_then(Value::as_str) != Some("none") {
            return None;
        }
        self.envelope.get("body").filter(|body| body.is_object())
    }

    /// The text of a plain message or edit.
    pub fn text(&self) -> Option<&str> {
        self.body()?.get("text")?.as_str()
    }
}

/// A conversation as the reader sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub kind: ConversationKind,
    pub title: Option<String>,
    pub encryption: ConversationEncryption,
    pub created_by: String,
    pub created_at: String,
    pub last_activity_at: String,
    /// Active members, in the order they joined.
    #[serde(default)]
    pub members: Vec<ConversationMember>,
    /// Messages from others the reader has not read yet.
    #[serde(default)]
    pub unread_count: u64,
    /// The newest message in the reader's inbox.
    pub last_entry: Option<InboxEntry>,
}

/// A conversation to start; the caller is always a member.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CreateConversation {
    pub kind: ConversationKind,
    /// Groups only; at most 120 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Everyone except the caller: their own bots and people who share an app with them.
    pub members: Vec<String>,
    /// `Plain` when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<ConversationEncryption>,
}

impl CreateConversation {
    /// A direct conversation; the one you already have with `member` is returned instead of a new one.
    pub fn direct(member: impl Into<String>) -> Self {
        Self {
            kind: ConversationKind::Direct,
            title: None,
            members: vec![member.into()],
            encryption: None,
        }
    }

    /// A titled group of up to 49 members besides you.
    pub fn group(
        title: impl Into<String>,
        members: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            kind: ConversationKind::Group,
            title: Some(title.into()),
            members: members.into_iter().map(Into::into).collect(),
            encryption: None,
        }
    }
}

/// Which entries of a conversation to read; everything is optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InboxEntriesQuery {
    /// Read the copy of one of your bots instead of your own (read-only).
    pub as_bot: Option<String>,
    /// Only entries older than this cursor.
    pub before: Option<String>,
    /// Only entries newer than this cursor.
    pub after: Option<String>,
    /// 1 to 100 entries per page; 50 by default.
    pub limit: Option<u32>,
}

impl InboxEntriesQuery {
    fn options(&self) -> RequestOptions {
        let limit = self.limit.map(|limit| limit.to_string());
        query(&[
            ("as", self.as_bot.as_deref()),
            ("before", self.before.as_deref()),
            ("after", self.after.as_deref()),
            ("limit", limit.as_deref()),
        ])
    }
}

/// Entries newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxEntriesPage {
    pub entries: Vec<InboxEntry>,
    /// Pass as `before` for the next older page. With `after`, set when more entries lie between
    /// the oldest entry here and `after`: reload instead of keeping a gap.
    pub next_before: Option<String>,
}

/// An event to send. Plain conversations take `envelope`, end-to-end encrypted ones `envelopes`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SendInboxEntry {
    /// 1 to 64 characters of `[A-Za-z0-9_-]`; generated when `None`. Sending the same id again
    /// returns the first result instead of sending twice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub kind: InboxEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Value>,
    /// One sealed envelope per active member, keyed by principal id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelopes: Option<BTreeMap<String, Value>>,
}

impl SendInboxEntry {
    /// A plain event with `body` in envelope v1.
    pub fn plain(kind: InboxEntryKind, body: Value) -> Self {
        Self {
            message_id: None,
            kind,
            envelope: Some(plain_envelope(body)),
            envelopes: None,
        }
    }

    /// An end-to-end encrypted event: one sealed envelope per active member.
    pub fn sealed(kind: InboxEntryKind, envelopes: BTreeMap<String, Value>) -> Self {
        Self {
            message_id: None,
            kind,
            envelope: None,
            envelopes: Some(envelopes),
        }
    }

    pub fn message(text: impl Into<String>) -> Self {
        Self::plain(InboxEntryKind::Message, json!({ "text": text.into() }))
    }

    /// A message answering the message `reply_to` (its `message_id`).
    pub fn reply(text: impl Into<String>, reply_to: impl Into<String>) -> Self {
        Self::plain(
            InboxEntryKind::Message,
            json!({ "text": text.into(), "reply_to": reply_to.into() }),
        )
    }

    pub fn reaction(target: impl Into<String>, emoji: impl Into<String>, op: ReactionOp) -> Self {
        Self::plain(
            InboxEntryKind::Reaction,
            json!({ "target": target.into(), "emoji": emoji.into(), "op": op }),
        )
    }

    /// Replaces the text of a message you sent.
    pub fn edit(target: impl Into<String>, text: impl Into<String>) -> Self {
        Self::plain(
            InboxEntryKind::Edit,
            json!({ "target": target.into(), "text": text.into() }),
        )
    }

    /// Deletes a message you sent.
    pub fn delete(target: impl Into<String>) -> Self {
        Self::plain(InboxEntryKind::Delete, json!({ "target": target.into() }))
    }

    /// Fixes the message id, e.g. to retry a send without sending twice.
    pub fn with_message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }
}

/// Where the reader's read marker stands; it never moves back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxReadMarker {
    pub last_read_at: String,
}

/// A public key others seal envelopes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxKey {
    pub principal_id: String,
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub created_at: String,
}

/// A public key to register. Keys never change: register a new `key_id` to rotate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegisterInboxKey {
    /// `None` for your own key; the id of a bot you own registers that bot's key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// 1 to 128 characters of `[A-Za-z0-9._:-]`.
    pub key_id: String,
    /// e.g. `hpke-x25519-sha256-chacha20poly1305`.
    pub algorithm: String,
    pub public_key: String,
}

impl RegisterInboxKey {
    pub fn new(
        key_id: impl Into<String>,
        algorithm: impl Into<String>,
        public_key: impl Into<String>,
    ) -> Self {
        Self {
            principal_id: None,
            key_id: key_id.into(),
            algorithm: algorithm.into(),
            public_key: public_key.into(),
        }
    }

    /// Registers the key of one of your bots instead of your own.
    pub fn for_bot(mut self, bot_id: impl Into<String>) -> Self {
        self.principal_id = Some(bot_id.into());
        self
    }
}

/// A fresh client-chosen message id: `msg_` and 32 hex digits.
pub fn new_inbox_message_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let half = |lane: u8| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u8(lane);
        hasher.write_u64(sequence);
        hasher.write_u128(nanos);
        hasher.write_u32(std::process::id());
        hasher.finish()
    };
    format!("msg_{:016x}{:016x}", half(0), half(1))
}

/// The plain envelope v1 around `body`.
pub fn plain_envelope(body: Value) -> Value {
    json!({ "v": 1, "enc": "none", "body": body })
}

fn query(pairs: &[(&str, Option<&str>)]) -> RequestOptions {
    pairs
        .iter()
        .fold(RequestOptions::new(), |options, (key, value)| match value {
            Some(value) => options.query(*key, value),
            None => options,
        })
}

fn reader(as_bot: Option<&str>) -> RequestOptions {
    query(&[("as", as_bot)])
}

#[derive(Deserialize)]
struct Conversations {
    conversations: Vec<Conversation>,
}

#[derive(Deserialize)]
struct Keys {
    keys: Vec<InboxKey>,
}

type HistoryPage = Option<(Vec<InboxEntry>, Option<InboxEntriesQuery>)>;

async fn history_page(
    client: Client,
    conversation_id: String,
    query: Option<InboxEntriesQuery>,
) -> Result<HistoryPage> {
    let Some(mut query) = query else {
        return Ok(None);
    };
    let page = client.list_inbox_entries(&conversation_id, &query).await?;
    let next = page
        .next_before
        .filter(|before| !before.is_empty())
        .map(|before| {
            query.before = Some(before);
            query
        });
    Ok(Some((page.entries, next)))
}

impl Client {
    /// Your conversations, newest activity first (at most 200); `as_bot` reads one of your
    /// bots' inboxes instead, limited to the conversations you are in too.
    pub async fn list_conversations(&self, as_bot: Option<&str>) -> Result<Vec<Conversation>> {
        let list: Conversations = self
            .request(Method::GET, &["inbox", "conversations"], &reader(as_bot))
            .await?;
        Ok(list.conversations)
    }

    pub async fn get_conversation(
        &self,
        conversation_id: &str,
        as_bot: Option<&str>,
    ) -> Result<Conversation> {
        self.request(
            Method::GET,
            &["inbox", "conversations", conversation_id],
            &reader(as_bot),
        )
        .await
    }

    pub async fn create_conversation(&self, request: &CreateConversation) -> Result<Conversation> {
        self.request(
            Method::POST,
            &["inbox", "conversations"],
            &RequestOptions::new().json(serde_json::to_value(request)?),
        )
        .await
    }

    /// Renames a group; `None` or an empty title removes it.
    pub async fn rename_conversation(
        &self,
        conversation_id: &str,
        title: Option<&str>,
    ) -> Result<Conversation> {
        self.request(
            Method::PATCH,
            &["inbox", "conversations", conversation_id],
            &RequestOptions::new().json(json!({ "title": title })),
        )
        .await
    }

    /// Adds your own bots or people who share an app with you; current members are skipped.
    pub async fn add_conversation_members(
        &self,
        conversation_id: &str,
        members: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Conversation> {
        let members: Vec<String> = members.into_iter().map(Into::into).collect();
        self.request(
            Method::POST,
            &["inbox", "conversations", conversation_id, "members"],
            &RequestOptions::new().json(json!({ "members": members })),
        )
        .await
    }

    /// Leaves (your own id), removes one of your bots, or anyone from a group you started.
    pub async fn remove_conversation_member(
        &self,
        conversation_id: &str,
        principal_id: &str,
    ) -> Result<()> {
        self.request(
            Method::DELETE,
            &[
                "inbox",
                "conversations",
                conversation_id,
                "members",
                principal_id,
            ],
            &RequestOptions::new(),
        )
        .await
    }

    /// One page of the reader's copy of a conversation, newest first.
    pub async fn list_inbox_entries(
        &self,
        conversation_id: &str,
        query: &InboxEntriesQuery,
    ) -> Result<InboxEntriesPage> {
        self.request(
            Method::GET,
            &["inbox", "conversations", conversation_id, "entries"],
            &query.options(),
        )
        .await
    }

    /// Every entry from `query.before` (or the newest) down to `query.after` (or the start),
    /// newest first, fetched page by page as the stream is polled.
    pub fn stream_inbox_entries(
        &self,
        conversation_id: &str,
        query: InboxEntriesQuery,
    ) -> BoxStream<'static, Result<InboxEntry>> {
        let client = self.clone();
        let conversation_id = conversation_id.to_owned();
        stream::try_unfold(Some(query), move |query| {
            history_page(client.clone(), conversation_id.clone(), query)
        })
        .map_ok(|entries| stream::iter(entries.into_iter().map(Ok::<_, Error>)))
        .try_flatten()
        .boxed()
    }

    /// Sends any event and returns your copy, generating `message_id` when it is `None`.
    pub async fn send_inbox_entry(
        &self,
        conversation_id: &str,
        request: &SendInboxEntry,
    ) -> Result<InboxEntry> {
        let mut body = serde_json::to_value(request)?;
        if request.message_id.is_none() {
            body["message_id"] = Value::String(new_inbox_message_id());
        }
        self.request(
            Method::POST,
            &["inbox", "conversations", conversation_id, "entries"],
            &RequestOptions::new().json(body),
        )
        .await
    }

    pub async fn send_inbox_message(
        &self,
        conversation_id: &str,
        text: &str,
    ) -> Result<InboxEntry> {
        self.send_inbox_entry(conversation_id, &SendInboxEntry::message(text))
            .await
    }

    /// Reacts to the message `target` (its `message_id`).
    pub async fn react_to_inbox_message(
        &self,
        conversation_id: &str,
        target: &str,
        emoji: &str,
        op: ReactionOp,
    ) -> Result<InboxEntry> {
        self.send_inbox_entry(
            conversation_id,
            &SendInboxEntry::reaction(target, emoji, op),
        )
        .await
    }

    /// Replaces the text of a message you sent.
    pub async fn edit_inbox_message(
        &self,
        conversation_id: &str,
        target: &str,
        text: &str,
    ) -> Result<InboxEntry> {
        self.send_inbox_entry(conversation_id, &SendInboxEntry::edit(target, text))
            .await
    }

    /// Deletes a message you sent.
    pub async fn delete_inbox_message(
        &self,
        conversation_id: &str,
        target: &str,
    ) -> Result<InboxEntry> {
        self.send_inbox_entry(conversation_id, &SendInboxEntry::delete(target))
            .await
    }

    /// Moves your read marker to the entry with cursor `up_to`, or to now.
    pub async fn mark_conversation_read(
        &self,
        conversation_id: &str,
        up_to: Option<&str>,
    ) -> Result<InboxReadMarker> {
        let body = match up_to {
            Some(cursor) => json!({ "up_to": cursor }),
            None => json!({}),
        };
        self.request(
            Method::POST,
            &["inbox", "conversations", conversation_id, "read"],
            &RequestOptions::new().json(body),
        )
        .await
    }

    /// The active public keys of up to 50 people and bots.
    pub async fn get_inbox_keys(
        &self,
        principal_ids: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Vec<InboxKey>> {
        let principals = principal_ids
            .into_iter()
            .map(|id| id.as_ref().to_owned())
            .collect::<Vec<_>>()
            .join(",");
        let list: Keys = self
            .request(
                Method::GET,
                &["inbox", "keys"],
                &RequestOptions::new().query("principals", principals),
            )
            .await?;
        Ok(list.keys)
    }

    pub async fn register_inbox_key(&self, request: &RegisterInboxKey) -> Result<InboxKey> {
        self.request(
            Method::PUT,
            &["inbox", "keys"],
            &RequestOptions::new().json(serde_json::to_value(request)?),
        )
        .await
    }

    /// Revokes your key, or with `principal_id` one of your bots' keys.
    pub async fn revoke_inbox_key(&self, key_id: &str, principal_id: Option<&str>) -> Result<()> {
        self.request(
            Method::DELETE,
            &["inbox", "keys", key_id],
            &query(&[("principal_id", principal_id)]),
        )
        .await
    }
}

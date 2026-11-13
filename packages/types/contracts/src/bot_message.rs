//! The body of a plain `message` entry, which people, bots, hosts and SDKs all write: markdown
//! text (```` ```a2ui ```` fences render as UI), addressable parts (`t1`… tool calls with their
//! results, `ui1`… UI blocks, `a1`… attachments), the message it replies to, the principals it
//! mentions, a failed turn's error, how many bot turns led to it and, reserved for bots asking each
//! other across hosts, the question it carries. A part too large for the 64 KiB envelope lives in
//! the sending bot's workspace and the body keeps a [`WorkspaceHandle`] to it. Every field is
//! optional, so the `{ text, reply_to }` bodies sent before stay valid; unknown fields are stored
//! and ignored. An edit's body is a message body plus the `target` it replaces.

use crate::inbox::{MAX_CONVERSATION_MEMBERS, TOMBSTONE_FIELD, is_valid_message_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// At most this many parts in one message.
pub const MAX_PARTS: usize = 32;
/// Usage summaries leave room for message text, attachments and encryption overhead.
pub const MAX_USAGE_STATS_BYTES: usize = 8 * 1024;
pub const MAX_USAGE_STATS_RECORDS: usize = 4;
/// Principal ids, call ids and tool names have at most this many characters.
pub const MAX_NAME_CHARS: usize = 128;
/// A part's one-line summary has at most this many characters.
pub const MAX_SUMMARY_CHARS: usize = 500;
/// An attachment's file name has at most this many characters.
pub const MAX_FILE_NAME_CHARS: usize = 255;
/// A workspace path has at most this many characters.
pub const MAX_PATH_CHARS: usize = 1_024;
/// An attachment's link has at most this many bytes; signed links run to about 2,000.
pub const MAX_LINK_CHARS: usize = 4_096;
/// An error code or a MIME type has at most this many characters.
pub const MAX_CODE_CHARS: usize = 64;
/// At most this many bots wait on one question's chain.
pub const MAX_ASK_CHAIN: usize = 8;
/// A question's `reply_by` time has at most this many characters.
const MAX_TIME_CHARS: usize = 64;
/// A file a person or bot puts into a conversation has at most this many bytes, the size the 1:1
/// chat's temporary uploads allow.
pub const MAX_CONVERSATION_FILE_BYTES: u64 = 35 * 1024 * 1024;
/// A conversation file's handle names the conversation as `conversation_<id>` in `app_id`: the
/// file lives in platform storage under the conversation, not in an app.
pub const CONVERSATION_FILE_APP_PREFIX: &str = "conversation_";

fn is_false(value: &bool) -> bool {
    !*value
}

/// The body of a `message` entry, and of an `edit` beside its `target`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageBody {
    /// Markdown; ```` ```a2ui ```` fences render as UI.
    #[serde(default)]
    pub text: String,
    /// The `message_id` this message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Principal ids the message addresses: in a group a bot answers when it is named here or
    /// replied to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// Tool calls, UI blocks and attachments, numbered within their kind in order: `t1`, `t2` …,
    /// `ui1` …, `a1` …; `#<seq>.t1` points at one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
    /// Set when a bot's turn failed; `text` says what happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<MessageError>,
    /// How many bot turns led to this message; 0 from a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<u32>,
    /// Reserved: the question one bot asks another across hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<BotAsk>,
    /// A bot's turn is parked until its owner answers this card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRequest>,
    /// The owner's answer to an approval card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_response: Option<ApprovalResponse>,
    /// Sent from an interactive chat that answers it itself: hosts leave the message alone while
    /// that chat holds the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<InteractiveMark>,
    /// The bot note this reply was written with: its title and status show on the message, its
    /// body only to the bot's owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<crate::bot_notes::NoteRef>,
    /// Numeric model usage and bounded call/compaction detail, grouped by step name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage_stats: Vec<Value>,
}

/// Names the interactive chat run that answers a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InteractiveMark {
    pub client_run_id: String,
}

/// The approval card a bot posts when a headless turn needs its owner to allow a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    /// Equals the card's `message_id`; the answer names it.
    pub id: String,
    /// The parked turn.
    pub turn_id: String,
    pub tool: String,
    /// What the call would do, in one line.
    pub summary: String,
    /// The grant scopes an "always" answer stores, comma separated (`app:<id>`, `host:<h>` …).
    pub scope: String,
    /// Hash of the tool and its arguments; a one-time answer is good for this call only.
    pub args_hash: String,
    /// Whose message woke the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_name: Option<String>,
    /// The title of the room the turn runs in; none in the direct conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// Set on a `secret_request` card: the owner stores the value in the bot's secret store, then
    /// answers `once` (or `deny`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretAsk>,
    /// Set on a `room_invite` card: who the bot wants to bring into the room, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<RoomInviteAsk>,
    /// The full proposed settings patch, shown before approving a bot configuration change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_changes: Option<Value>,
    /// The calls the card asks about. Empty on a card for one plain call, which the top-level
    /// fields describe; a batch card's top-level fields summarize its items for older clients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ApprovalItem>,
}

/// At most this many items on one approval card.
pub const MAX_APPROVAL_ITEMS: usize = 32;

/// The `tool` of a card that asks about several calls at once.
pub const APPROVAL_BATCH_TOOL: &str = "approval_batch";

fn one_call() -> u32 {
    1
}

fn is_one_call(calls: &u32) -> bool {
    *calls == 1
}

/// One call on an approval card, or several identical ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalItem {
    /// Hash of the tool and its arguments; the answer names the item by it.
    pub args_hash: String,
    pub tool: String,
    /// What the call would do, in one line.
    pub summary: String,
    /// The grant scopes an "always" answer stores, comma separated; empty when the call is asked
    /// every time.
    pub scope: String,
    /// How many identical calls the item covers.
    #[serde(default = "one_call", skip_serializing_if = "is_one_call")]
    pub calls: u32,
    /// The turn read web content before this call, so the owner's standing grants for it were set
    /// aside and it asks again; "always" is not offered.
    #[serde(default, skip_serializing_if = "is_false")]
    pub after_web_content: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretAsk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<RoomInviteAsk>,
}

impl ApprovalItem {
    fn check(&self, field: &str) -> Result<(), BodyError> {
        check_name(
            &format!("{field}.args_hash"),
            &self.args_hash,
            MAX_NAME_CHARS,
        )?;
        check_name(&format!("{field}.tool"), &self.tool, MAX_NAME_CHARS)?;
        check_line(
            &format!("{field}.summary"),
            Some(&self.summary),
            MAX_SUMMARY_CHARS,
        )?;
        check_line(
            &format!("{field}.scope"),
            Some(&self.scope),
            crate::tool_grant::MAX_KEY_CHARS * 4,
        )?;
        if self.calls == 0 {
            return refuse(format!("{field}.calls must be at least 1"));
        }
        self.secret.as_ref().map_or(Ok(()), SecretAsk::check)?;
        self.invite.as_ref().map_or(Ok(()), RoomInviteAsk::check)
    }
}

/// The owner's answer to one item of a batch card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ItemDecision {
    pub args_hash: String,
    pub decision: crate::tool_grant::ApprovalDecision,
}

/// A secret name has at most this many characters.
pub const MAX_SECRET_NAME_CHARS: usize = 64;

/// The tool a bot asks its owner for a secret with.
pub const SECRET_REQUEST_TOOL: &str = "secret_request";

/// `A-Z` first, then up to 63 of `A-Z`, `0-9` or `_`: the rule every secret store keeps.
pub fn is_secret_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_SECRET_NAME_CHARS
        && bytes.next().is_some_and(|first| first.is_ascii_uppercase())
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The secret a bot asks its owner for. The value never travels in a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretAsk {
    pub name: String,
    /// What the bot needs it for, in its own words.
    pub description: String,
    /// The stored value was rejected; the owner enters a new one.
    #[serde(default, skip_serializing_if = "is_false")]
    pub replace: bool,
}

impl SecretAsk {
    fn check(&self) -> Result<(), BodyError> {
        if !is_secret_name(&self.name) {
            return refuse(format!(
                "body.approval.secret.name '{}' must start with A-Z and continue with up to 63 of A-Z, 0-9 or _",
                clipped(&self.name)
            ));
        }
        check_line(
            "body.approval.secret.description",
            Some(&self.description),
            MAX_SUMMARY_CHARS,
        )
    }
}

/// The tool a bot invites a person into its group chat with.
pub const ROOM_INVITE_TOOL: &str = "room_invite";

/// An invited person's username or email address has at most this many characters.
pub const MAX_INVITE_PERSON_CHARS: usize = 254;

/// The person a bot wants to invite into the room its turn runs in, as the bot named them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoomInviteAsk {
    /// The exact username or email address the bot was given.
    pub person: String,
    /// Why the bot wants them in the room, in its own words.
    pub reason: String,
}

impl RoomInviteAsk {
    fn check(&self) -> Result<(), BodyError> {
        check_line(
            "body.approval.invite.person",
            Some(&self.person),
            MAX_INVITE_PERSON_CHARS,
        )?;
        check_line(
            "body.approval.invite.reason",
            Some(&self.reason),
            MAX_SUMMARY_CHARS,
        )
    }
}

/// The owner's answer to an [`ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalResponse {
    pub approval_id: String,
    /// The answer to every item not listed in `items`.
    pub decision: crate::tool_grant::ApprovalDecision,
    /// The items answered differently from `decision`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemDecision>,
}

impl ApprovalResponse {
    /// The answer entry for a card's resolved items: their common decision, or `once` with the
    /// items that differ from it when they do not agree. A card for one call answers like before.
    pub fn for_decisions(
        approval_id: impl Into<String>,
        decisions: &[(ApprovalItem, crate::tool_grant::ApprovalDecision)],
    ) -> Self {
        let first = decisions.first().map(|(_, decision)| *decision);
        let decision = first
            .filter(|first| decisions.iter().all(|(_, decision)| decision == first))
            .unwrap_or(crate::tool_grant::ApprovalDecision::Once);
        Self {
            approval_id: approval_id.into(),
            decision,
            items: decisions
                .iter()
                .filter(|(_, item_decision)| *item_decision != decision)
                .map(|(item, item_decision)| ItemDecision {
                    args_hash: item.args_hash.clone(),
                    decision: *item_decision,
                })
                .collect(),
        }
    }
}

impl ApprovalRequest {
    /// The calls this card asks about: its items, or the card itself for a card written for one
    /// call (and every card written before batches).
    pub fn batch(&self) -> Vec<ApprovalItem> {
        if !self.items.is_empty() {
            return self.items.clone();
        }
        vec![ApprovalItem {
            args_hash: self.args_hash.clone(),
            tool: self.tool.clone(),
            summary: self.summary.clone(),
            scope: self.scope.clone(),
            calls: 1,
            after_web_content: false,
            secret: self.secret.clone(),
            invite: self.invite.clone(),
        }]
    }

    /// The tools the card asks about, each once, in the order of its items.
    pub fn tool_names(&self) -> Vec<String> {
        let mut tools: Vec<String> = Vec::new();
        for item in self.batch() {
            if !tools.contains(&item.tool) {
                tools.push(item.tool);
            }
        }
        tools
    }

    /// Every item with its answer: a listed item takes its own decision, every other item
    /// `decision`. Refuses a hash that is not on the card or is listed twice.
    pub fn decisions(
        &self,
        decision: crate::tool_grant::ApprovalDecision,
        answers: &[ItemDecision],
    ) -> Result<Vec<(ApprovalItem, crate::tool_grant::ApprovalDecision)>, String> {
        let batch = self.batch();
        let mut listed = std::collections::HashMap::new();
        for answer in answers {
            if !batch.iter().any(|item| item.args_hash == answer.args_hash) {
                return Err(format!(
                    "Approval '{}' has no item '{}'",
                    self.id,
                    clipped(&answer.args_hash)
                ));
            }
            if listed
                .insert(answer.args_hash.as_str(), answer.decision)
                .is_some()
            {
                return Err(format!(
                    "Approval '{}' answers item '{}' twice",
                    self.id,
                    clipped(&answer.args_hash)
                ));
            }
        }
        Ok(batch
            .into_iter()
            .map(|item| {
                let answer = listed
                    .get(item.args_hash.as_str())
                    .copied()
                    .unwrap_or(decision);
                (item, answer)
            })
            .collect())
    }

    fn check(&self) -> Result<(), BodyError> {
        check_name("body.approval.id", &self.id, MAX_NAME_CHARS)?;
        check_name("body.approval.turn_id", &self.turn_id, MAX_NAME_CHARS)?;
        check_name("body.approval.tool", &self.tool, MAX_NAME_CHARS)?;
        check_name("body.approval.args_hash", &self.args_hash, MAX_NAME_CHARS)?;
        check_line(
            "body.approval.summary",
            Some(&self.summary),
            MAX_SUMMARY_CHARS,
        )?;
        check_line(
            "body.approval.scope",
            Some(&self.scope),
            crate::tool_grant::MAX_KEY_CHARS * 4,
        )?;
        check_line(
            "body.approval.requester_name",
            self.requester_name.as_deref(),
            MAX_NAME_CHARS,
        )?;
        check_line("body.approval.room", self.room.as_deref(), MAX_NAME_CHARS)?;
        if self
            .configuration_changes
            .as_ref()
            .is_some_and(|changes| !changes.is_object())
        {
            return refuse("body.approval.configuration_changes must be an object".to_string());
        }
        self.secret.as_ref().map_or(Ok(()), SecretAsk::check)?;
        self.invite.as_ref().map_or(Ok(()), RoomInviteAsk::check)?;
        check_item_hashes(
            "body.approval.items",
            self.items.len(),
            self.items.iter().map(|item| item.args_hash.as_str()),
        )?;
        for (index, item) in self.items.iter().enumerate() {
            item.check(&format!("body.approval.items[{index}]"))?;
        }
        Ok(())
    }
}

impl ApprovalResponse {
    fn check(&self) -> Result<(), BodyError> {
        check_name(
            "body.approval_response.approval_id",
            &self.approval_id,
            MAX_NAME_CHARS,
        )?;
        check_item_hashes(
            "body.approval_response.items",
            self.items.len(),
            self.items.iter().map(|item| item.args_hash.as_str()),
        )?;
        for (index, item) in self.items.iter().enumerate() {
            check_name(
                &format!("body.approval_response.items[{index}].args_hash"),
                &item.args_hash,
                MAX_NAME_CHARS,
            )?;
        }
        Ok(())
    }
}

fn check_item_hashes<'a>(
    field: &str,
    count: usize,
    mut hashes: impl Iterator<Item = &'a str>,
) -> Result<(), BodyError> {
    if count > MAX_APPROVAL_ITEMS {
        return refuse(format!(
            "{field} holds at most {MAX_APPROVAL_ITEMS} items; this one holds {count}"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(repeated) = hashes.find(|hash| !seen.insert(*hash)) {
        return refuse(format!("{field} names item '{}' twice", clipped(repeated)));
    }
    Ok(())
}

/// One addressable part of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessagePart {
    /// A tool call, with its result once known: `t<n>`.
    Tool(ToolPart),
    /// A rendered UI block: `ui<n>`.
    Ui(UiPart),
    /// A file: `a<n>`.
    Attachment(AttachmentPart),
}

/// A tool call and its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolPart {
    /// `t1`, `t2` …
    pub id: String,
    /// Shared by the call and a result that arrives in a later message.
    pub call_id: String,
    /// The tool's name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    /// The result, when it fits the envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The result in the sending bot's workspace, when it doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<WorkspaceHandle>,
    /// What the result says, in one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub failed: bool,
}

/// A UI block the message rendered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UiPart {
    /// `ui1`, `ui2` …
    pub id: String,
    /// What the block shows, in one line: all a model sees of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The A2UI document, when it fits the envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2ui: Option<Value>,
    /// The document in the sending bot's workspace, when it doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<WorkspaceHandle>,
}

/// A file from the sending bot's workspace, a conversation file any member sent, or a link to a
/// file stored elsewhere. Exactly one of `handle` and `url`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentPart {
    /// `a1`, `a2` …
    pub id: String,
    /// The file name shown.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<WorkspaceHandle>,
    /// An http(s) link to the file, such as a signed download link; it works only until it
    /// expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The linked file's `type/subtype`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

impl AttachmentPart {
    pub fn stored(id: String, name: String, handle: WorkspaceHandle) -> Self {
        Self {
            id,
            name,
            handle: Some(handle),
            url: None,
            mime: None,
        }
    }

    pub fn linked(id: String, name: String, url: String, mime: Option<String>) -> Self {
        Self {
            id,
            name,
            handle: None,
            url: Some(url),
            mime,
        }
    }

    /// The line a model reads for the file. A conversation file carries its whole handle, which
    /// the file tools take as the `file` argument.
    pub fn note(&self) -> String {
        let Some(handle) = &self.handle else {
            let url = self.url.as_deref().unwrap_or_default();
            return format!("[file: {} (link: {url})]", self.name);
        };
        if !handle.is_conversation_file() {
            return format!("[file: {}]", self.name);
        }
        let json = serde_json::to_string(handle).unwrap_or_default();
        format!(
            "[file: {} ({}, {} bytes); read it with the file tools, file = {json}]",
            self.name, handle.mime, handle.bytes
        )
    }

    fn check(&self, field: &str) -> Result<(), BodyError> {
        if !(1..=MAX_FILE_NAME_CHARS).contains(&chars(&self.name))
            || self.name.chars().any(char::is_control)
        {
            return refuse(format!(
                "{field}.name must be 1 to {MAX_FILE_NAME_CHARS} characters"
            ));
        }
        match (&self.handle, &self.url) {
            (Some(_), None) => {
                if self.mime.is_some() {
                    return refuse(format!(
                        "{field}.mime belongs to a linked file; a stored file keeps its type in handle.mime"
                    ));
                }
                Ok(())
            }
            (None, Some(url)) => {
                if !is_file_link(url) {
                    return refuse(format!(
                        "{field}.url must be an http(s) address with a host and no user name or password, at most {MAX_LINK_CHARS} characters"
                    ));
                }
                match &self.mime {
                    Some(mime) if !is_mime(mime) => refuse(format!(
                        "{field}.mime '{}' is not type/subtype",
                        clipped(mime)
                    )),
                    _ => Ok(()),
                }
            }
            _ => refuse(format!(
                "{field} needs exactly one of handle (a stored file) and url (a link)"
            )),
        }
    }
}

/// Whether `url` can be an attachment's link: http(s), a host, no credentials, no whitespace.
pub fn is_file_link(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= MAX_LINK_CHARS
        && !authority.is_empty()
        && !authority.starts_with(':')
        && !authority.contains('@')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A file in a bot's workspace app, for content too large for the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceHandle {
    /// The sending bot's workspace app; the server refuses any other. A conversation file
    /// (`conversation_<conversation id>`) instead names the conversation it was uploaded to.
    pub app_id: String,
    /// Relative to the app's storage, `/`-separated, without `.` or `..` segments.
    pub path: String,
    pub bytes: u64,
    /// The file's SHA-256 in lowercase hex.
    pub sha256: String,
    /// `type/subtype`.
    pub mime: String,
}

/// Why a bot's turn failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageError {
    /// Stable and machine-readable, e.g. `BOT_TURN_TIMEOUT`: 1 to 64 of `[A-Za-z0-9_.-]`.
    pub code: String,
}

/// A question one bot asks another; the answer's `reply_to` is `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotAsk {
    pub id: String,
    /// The conversation the asking bot waits in.
    pub origin_conversation: String,
    /// Bot turns that led to the question.
    pub hops: u32,
    /// RFC 3339; unanswered by then, the question lapses.
    pub reply_by: String,
    /// Ids of the bots waiting on this question's chain, the first asker first, the asker last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<String>,
    /// How long the asker waits for the answer before it moves on; a later answer reaches it as
    /// a new turn in `origin_conversation`. 0 or absent: it doesn't wait.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wait_secs: u32,
    /// Which of the asker's questions to this callee for the same trigger this is, from 1. Its
    /// answer wakes the asker at the asker's own hops, so follow-ups don't add up to the hop
    /// limit; the asker's round limit ends them instead. 0 or absent (hosts that predate rounds):
    /// the answer counts its hops as before.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub round: u32,
}

impl BotAsk {
    /// The hops the asker's turn gets when this question's answer wakes it: the asker's own, at
    /// least 1 because a bot woke it. `None` for a question without a round.
    pub fn answer_wakes_at(&self) -> Option<u32> {
        (self.round > 0).then(|| self.hops.saturating_sub(1).max(1))
    }
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Why a body was refused; the text names the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyError(String);

impl fmt::Display for BodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BodyError {}

fn refuse<T>(message: impl Into<String>) -> Result<T, BodyError> {
    Err(BodyError(message.into()))
}

fn clipped(value: &str) -> String {
    value.chars().take(80).collect()
}

fn chars(value: &str) -> usize {
    value.chars().count()
}

/// 1 to `max` characters, none of them whitespace or control characters.
pub(crate) fn is_name(value: &str, max: usize) -> bool {
    (1..=max).contains(&chars(value))
        && !value
            .chars()
            .any(|char| char.is_whitespace() || char.is_control())
}

fn check_name(field: &str, value: &str, max: usize) -> Result<(), BodyError> {
    if is_name(value, max) {
        return Ok(());
    }
    refuse(format!(
        "{field} '{}' must be 1 to {max} characters without spaces",
        clipped(value)
    ))
}

fn check_line(field: &str, value: Option<&str>, max: usize) -> Result<(), BodyError> {
    let count = value.map_or(0, chars);
    if count <= max && !value.is_some_and(|text| text.chars().any(char::is_control)) {
        return Ok(());
    }
    refuse(format!(
        "{field} is one line of at most {max} characters; this one has {count} characters or a line break"
    ))
}

/// A copy of a plain envelope whose tool parts keep their name, arguments and a one-line summary
/// but lose the raw result and workspace handle, for bot replies in rooms; `None` when nothing
/// needs cutting. The owner reads the full details in the bot's own inbox copy (`?as=`).
pub fn without_tool_details(envelope: &Value) -> Option<Value> {
    let mut copy = envelope.clone();
    let parts = copy.get_mut("body")?.get_mut("parts")?.as_array_mut()?;
    let mut changed = false;
    for part in parts
        .iter_mut()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("tool"))
    {
        let Some(tool) = part.as_object_mut() else {
            continue;
        };
        for field in ["result", "handle"] {
            changed |= tool.remove(field).is_some();
        }
        if !tool.get("summary").is_some_and(Value::is_string) {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or("a tool");
            let summary = format!("Used {name}");
            tool.insert("summary".into(), Value::String(summary));
            changed = true;
        }
    }
    changed.then_some(copy)
}

impl MessageBody {
    /// The body of a plain message or edit, checked against the contract.
    pub fn parse(body: &Value) -> Result<Self, BodyError> {
        if body.get(TOMBSTONE_FIELD).is_some() {
            return refuse(format!(
                "body.{TOMBSTONE_FIELD} is set by the server on deleted messages; send a delete instead"
            ));
        }
        let parsed = Self::deserialize(body)
            .map_err(|error| BodyError(format!("Invalid message body: {error}")))?;
        parsed.check()?;
        Ok(parsed)
    }

    pub fn check(&self) -> Result<(), BodyError> {
        if let Some(reply_to) = &self.reply_to
            && !is_valid_message_id(reply_to)
        {
            return refuse(format!(
                "body.reply_to '{}' is not a message id: 1 to 64 of A-Z, a-z, 0-9, '_' and '-'",
                clipped(reply_to)
            ));
        }
        self.check_mentions()?;
        check_parts(&self.parts)?;
        if self.usage_stats.len() > MAX_USAGE_STATS_RECORDS
            || serde_json::to_vec(&self.usage_stats).map_or(usize::MAX, |bytes| bytes.len())
                > MAX_USAGE_STATS_BYTES
            || self.usage_stats.iter().any(|record| {
                !record.get("step_name").is_some_and(Value::is_string)
                    || !record.get("stats").is_some_and(Value::is_object)
            })
        {
            return refuse(
                "body.usage_stats exceeds the usage summary limits or has an invalid record",
            );
        }
        if let Some(error) = &self.error {
            error.check()?;
        }
        if let Some(ask) = &self.ask {
            ask.check()?;
        }
        if let Some(approval) = &self.approval {
            approval.check()?;
        }
        if let Some(response) = &self.approval_response {
            response.check()?;
        }
        if let Some(mark) = &self.interactive {
            check_name(
                "body.interactive.client_run_id",
                &mark.client_run_id,
                MAX_NAME_CHARS,
            )?;
        }
        if let Some(note) = &self.note {
            note.check()
                .map_err(|error| BodyError(format!("body.note: {error}")))?;
        }
        Ok(())
    }

    fn check_mentions(&self) -> Result<(), BodyError> {
        if self.mentions.len() > MAX_CONVERSATION_MEMBERS {
            return refuse(format!(
                "body.mentions names at most {MAX_CONVERSATION_MEMBERS} principals; this one names {}",
                self.mentions.len()
            ));
        }
        self.mentions
            .iter()
            .try_for_each(|mention| check_name("body.mentions", mention, MAX_NAME_CHARS))
    }

    /// The part with this id, such as `t1` or `ui2`.
    pub fn part(&self, id: &str) -> Option<&MessagePart> {
        self.parts.iter().find(|part| part.id() == id)
    }

    /// Every file the parts point to, workspace and conversation files alike.
    pub fn handles(&self) -> impl Iterator<Item = &WorkspaceHandle> {
        self.parts.iter().filter_map(MessagePart::handle)
    }

    /// The attachments that carry a file uploaded to `conversation_id`.
    pub fn conversation_attachments<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> impl Iterator<Item = &'a AttachmentPart> + 'a {
        self.parts.iter().filter_map(move |part| match part {
            MessagePart::Attachment(file)
                if file
                    .handle
                    .as_ref()
                    .and_then(WorkspaceHandle::conversation_id)
                    == Some(conversation_id) =>
            {
                Some(file)
            }
            _ => None,
        })
    }

    /// The conversation files the parts point to.
    pub fn conversation_files(&self) -> impl Iterator<Item = ConversationFileRef<'_>> {
        self.handles().filter_map(|handle| {
            Some(ConversationFileRef {
                conversation_id: handle.conversation_id()?,
                file_id: handle.file_id()?,
                handle,
            })
        })
    }
}

const PART_PREFIXES: [&str; 3] = ["t", "ui", "a"];

impl MessagePart {
    pub fn id(&self) -> &str {
        match self {
            Self::Tool(part) => &part.id,
            Self::Ui(part) => &part.id,
            Self::Attachment(part) => &part.id,
        }
    }

    /// The id prefix of the part's kind: `t`, `ui` or `a`.
    pub fn prefix(&self) -> &'static str {
        PART_PREFIXES[self.kind_index()]
    }

    fn kind_index(&self) -> usize {
        match self {
            Self::Tool(_) => 0,
            Self::Ui(_) => 1,
            Self::Attachment(_) => 2,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::Tool(_) => "tool",
            Self::Ui(_) => "ui",
            Self::Attachment(_) => "attachment",
        }
    }

    pub fn handle(&self) -> Option<&WorkspaceHandle> {
        match self {
            Self::Tool(part) => part.handle.as_ref(),
            Self::Ui(part) => part.handle.as_ref(),
            Self::Attachment(part) => part.handle.as_ref(),
        }
    }

    fn check(&self, field: &str) -> Result<(), BodyError> {
        match self {
            Self::Tool(part) => {
                check_name(&format!("{field}.call_id"), &part.call_id, MAX_NAME_CHARS)?;
                check_name(&format!("{field}.name"), &part.name, MAX_NAME_CHARS)?;
                check_line(
                    &format!("{field}.summary"),
                    part.summary.as_deref(),
                    MAX_SUMMARY_CHARS,
                )?;
                if part.result.is_some() && part.handle.is_some() {
                    return refuse(format!(
                        "{field} keeps its result inline or in handle, not both"
                    ));
                }
            }
            Self::Ui(part) => {
                check_line(
                    &format!("{field}.summary"),
                    part.summary.as_deref(),
                    MAX_SUMMARY_CHARS,
                )?;
                if part.a2ui.is_some() && part.handle.is_some() {
                    return refuse(format!(
                        "{field} keeps its document inline in a2ui or in handle, not both"
                    ));
                }
            }
            Self::Attachment(part) => part.check(field)?,
        }
        match self.handle() {
            Some(handle) => {
                if handle.is_conversation_file() && !matches!(self, Self::Attachment(_)) {
                    return refuse(format!(
                        "{field}.handle is a conversation file; only attachments point at one"
                    ));
                }
                handle.check(&format!("{field}.handle"))
            }
            None => Ok(()),
        }
    }
}

/// Parts are numbered within their kind in the order they appear: `t1`, `ui1`, `t2`, `a1` ….
fn check_parts(parts: &[MessagePart]) -> Result<(), BodyError> {
    if parts.len() > MAX_PARTS {
        return refuse(format!(
            "body.parts holds at most {MAX_PARTS} parts; this one holds {}",
            parts.len()
        ));
    }
    let mut numbered = [0usize; PART_PREFIXES.len()];
    for (index, part) in parts.iter().enumerate() {
        let field = format!("body.parts[{index}]");
        let count = &mut numbered[part.kind_index()];
        *count += 1;
        let expected = format!("{}{count}", part.prefix());
        if part.id() != expected {
            return refuse(format!(
                "{field} is {} part '{}'; parts are numbered within their kind in order, so this one is '{expected}'",
                part.kind_name(),
                clipped(part.id())
            ));
        }
        part.check(&field)?;
    }
    Ok(())
}

fn is_mime_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&byte))
}

/// What the asker's turn reads when the answer to a question it stopped waiting for arrives:
/// `latest` is the last thing in its conversation with the owner.
pub fn ask_result_prompt(
    callee: &str,
    question: &str,
    answer: &str,
    failed: bool,
    latest: Option<&str>,
) -> String {
    let outcome = if failed {
        format!("{callee} could not answer:\n{answer}")
    } else {
        format!("{callee} answered:\n{answer}")
    };
    let latest = latest
        .map(|text| format!("\n\nThe last thing in your conversation with your owner: {text}"))
        .unwrap_or_default();
    format!(
        "Earlier you asked {callee}: \"{question}\" and stopped waiting for the answer. It has arrived.\n\n{outcome}{latest}\n\nContinue from here: tell your owner what you learned, ask {callee} a follow-up when its answer leaves something open, or finish what you were doing with it."
    )
}

/// `type/subtype` without parameters.
pub fn is_mime(value: &str) -> bool {
    value.len() <= MAX_CODE_CHARS * 2
        && value
            .split_once('/')
            .is_some_and(|(kind, subtype)| is_mime_token(kind) && is_mime_token(subtype))
}

fn conversation_of_app(app_id: &str) -> Option<&str> {
    app_id
        .strip_prefix(CONVERSATION_FILE_APP_PREFIX)
        .filter(|id| is_valid_message_id(id))
}

/// A relative, `/`-separated path without empty, `.` or `..` segments.
pub fn is_workspace_path(path: &str) -> bool {
    (1..=MAX_PATH_CHARS).contains(&chars(path))
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// A conversation file a message points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationFileRef<'a> {
    pub conversation_id: &'a str,
    pub file_id: &'a str,
    pub handle: &'a WorkspaceHandle,
}

/// `<file id>` or `<file id>.<ext>`: the one path segment of a conversation file.
pub fn is_conversation_file_path(path: &str) -> bool {
    let (id, extension) = match path.split_once('.') {
        Some((id, extension)) => (id, Some(extension)),
        None => (path, None),
    };
    is_valid_message_id(id)
        && extension.is_none_or(|extension| {
            (1..=16).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

impl WorkspaceHandle {
    /// The handle of a file uploaded to `conversation_id`, stored as `file_name` (`<id>.<ext>`).
    pub fn conversation_file(
        conversation_id: &str,
        file_name: &str,
        bytes: u64,
        sha256: &str,
        mime: &str,
    ) -> Self {
        Self {
            app_id: format!("{CONVERSATION_FILE_APP_PREFIX}{conversation_id}"),
            path: file_name.to_string(),
            bytes,
            sha256: sha256.to_string(),
            mime: mime.to_string(),
        }
    }

    /// The conversation a conversation file was uploaded to; `None` for a workspace file.
    pub fn conversation_id(&self) -> Option<&str> {
        conversation_of_app(&self.app_id)
    }

    pub fn is_conversation_file(&self) -> bool {
        self.app_id.starts_with(CONVERSATION_FILE_APP_PREFIX)
    }

    /// The file's id: its path without the extension.
    pub fn file_id(&self) -> Option<&str> {
        self.conversation_id()?;
        Some(
            self.path
                .split_once('.')
                .map_or(self.path.as_str(), |(id, _)| id),
        )
    }

    fn check(&self, field: &str) -> Result<(), BodyError> {
        if self.is_conversation_file() {
            return self.check_conversation_file(field);
        }
        if !is_valid_message_id(&self.app_id) {
            return refuse(format!(
                "{field}.app_id '{}' is not an app id",
                clipped(&self.app_id)
            ));
        }
        self.check_file(field)
    }

    fn check_conversation_file(&self, field: &str) -> Result<(), BodyError> {
        if self.conversation_id().is_none() {
            return refuse(format!(
                "{field}.app_id '{}' must be '{CONVERSATION_FILE_APP_PREFIX}' followed by a conversation id",
                clipped(&self.app_id)
            ));
        }
        if !is_conversation_file_path(&self.path) {
            return refuse(format!(
                "{field}.path '{}' must be a conversation file id with an optional extension",
                clipped(&self.path)
            ));
        }
        if self.bytes > MAX_CONVERSATION_FILE_BYTES {
            return refuse(format!(
                "{field}.bytes is {}; a conversation file has at most {MAX_CONVERSATION_FILE_BYTES} bytes",
                self.bytes
            ));
        }
        self.check_file(field)
    }

    fn check_file(&self, field: &str) -> Result<(), BodyError> {
        if !is_workspace_path(&self.path) {
            return refuse(format!(
                "{field}.path '{}' must be a relative path of 1 to {MAX_PATH_CHARS} characters without '.', '..' or empty segments",
                clipped(&self.path)
            ));
        }
        let hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
        if self.sha256.len() != 64 || !self.sha256.bytes().all(hex) {
            return refuse(format!("{field}.sha256 must be 64 lowercase hex digits"));
        }
        if !is_mime(&self.mime) {
            return refuse(format!(
                "{field}.mime '{}' must be a type/subtype such as application/json",
                clipped(&self.mime)
            ));
        }
        Ok(())
    }
}

impl MessageError {
    fn check(&self) -> Result<(), BodyError> {
        let valid = (1..=MAX_CODE_CHARS).contains(&self.code.len())
            && self.code.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-'
            });
        if valid {
            return Ok(());
        }
        refuse(format!(
            "body.error.code '{}' must be 1 to {MAX_CODE_CHARS} of A-Z, a-z, 0-9, '_', '.' and '-'",
            clipped(&self.code)
        ))
    }
}

impl BotAsk {
    fn check(&self) -> Result<(), BodyError> {
        if !is_valid_message_id(&self.id) {
            return refuse(format!(
                "body.ask.id '{}' is not a message id",
                clipped(&self.id)
            ));
        }
        if !is_valid_message_id(&self.origin_conversation) {
            return refuse(format!(
                "body.ask.origin_conversation '{}' is not a conversation id",
                clipped(&self.origin_conversation)
            ));
        }
        if !is_name(&self.reply_by, MAX_TIME_CHARS) {
            return refuse("body.ask.reply_by must be an RFC 3339 time");
        }
        if self.chain.len() > MAX_ASK_CHAIN {
            return refuse(format!("body.ask.chain has at most {MAX_ASK_CHAIN} bots"));
        }
        for bot_id in &self.chain {
            check_name("body.ask.chain", bot_id, MAX_NAME_CHARS)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tool_detail_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_parts_lose_raw_results_and_keep_a_summary() {
        let envelope = json!({ "v": 1, "enc": "none", "body": { "text": "done", "parts": [
            { "kind": "tool", "id": "t1", "call_id": "c", "name": "database_tool",
              "arguments": { "sql": "select secret" }, "result": { "rows": [1] }, "summary": "Read 1 row" },
            { "kind": "tool", "id": "t2", "call_id": "d", "name": "file_read", "result": "x" },
            { "kind": "ui", "id": "ui1", "summary": "a chart" }
        ] } });
        let stripped = without_tool_details(&envelope).unwrap();
        let parts = stripped["body"]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["summary"], "Read 1 row");
        assert_eq!(parts[1]["summary"], "Used file_read");
        for part in &parts[..2] {
            assert!(part.get("result").is_none());
        }
        assert_eq!(parts[0]["arguments"]["sql"], "select secret");
        assert_eq!(parts[2], envelope["body"]["parts"][2]);
        assert_eq!(stripped["body"]["text"], "done");
    }

    #[test]
    fn nothing_to_cut_returns_none() {
        let envelope = json!({ "v": 1, "enc": "none", "body": { "text": "hi" } });
        assert_eq!(without_tool_details(&envelope), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_summaries_round_trip_and_reject_oversized_or_malformed_records() {
        let record = serde_json::json!({"step_name": "Assistant", "stats": {
            "usage": {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110},
            "context": {"estimated_tokens": 200, "context_window": 32_000, "reserved_output_tokens": 2_000}
        }});
        let body = serde_json::json!({"text": "done", "usage_stats": [record.clone()]});
        let parsed = MessageBody::parse(&body).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["usage_stats"],
            body["usage_stats"]
        );
        assert!(MessageBody::parse(&serde_json::json!({"usage_stats": [record.clone(), record.clone(), record.clone(), record.clone(), record]})).is_err());
        assert!(MessageBody::parse(&serde_json::json!({"usage_stats": [{"step_name": "Assistant", "stats": {"data": "x".repeat(MAX_USAGE_STATS_BYTES)}}]})).is_err());
        assert!(
            MessageBody::parse(
                &serde_json::json!({"usage_stats": [{"step_name": 2, "stats": {}}]})
            )
            .is_err()
        );
    }
    use serde_json::json;

    fn handle(app_id: &str, path: &str) -> Value {
        json!({
            "app_id": app_id,
            "path": path,
            "bytes": 120_000,
            "sha256": "a".repeat(64),
            "mime": "application/json",
        })
    }

    fn rich() -> Value {
        json!({
            "text": "Here is the forecast.\n```a2ui\n{\"id\":\"root\",\"type\":\"Text\"}\n```",
            "reply_to": "msg_1",
            "mentions": ["bot_ledger"],
            "parts": [
                { "kind": "tool", "id": "t1", "call_id": "c1", "name": "weather",
                  "arguments": { "city": "Berlin" }, "result": { "temp": 14 }, "summary": "14°C" },
                { "kind": "ui", "id": "ui1", "summary": "Weather card", "a2ui": { "id": "root" } },
                { "kind": "tool", "id": "t2", "call_id": "c2", "name": "search",
                  "handle": handle("app_ws", "tool-results/c2.json"), "failed": true },
                { "kind": "attachment", "id": "a1", "name": "report.pdf",
                  "handle": handle("app_ws", "files/report.pdf") },
            ],
            "error": { "code": "BOT_TURN_TIMEOUT" },
            "hops": 1,
            "ask": { "id": "ask_1", "origin_conversation": "conv_a", "hops": 1,
                     "reply_by": "2026-11-09T12:00:00Z" },
        })
    }

    #[test]
    fn the_bodies_sent_today_stay_valid() {
        for body in [
            json!({ "text": "Hi" }),
            json!({ "text": "Hi", "reply_to": "msg_1" }),
            json!({ "target": "msg_1", "text": "edited" }),
            json!({}),
            json!({ "text": "Hi", "client_hint": { "draft": 2 } }),
        ] {
            let parsed =
                MessageBody::parse(&body).unwrap_or_else(|error| panic!("{body}: {error}"));
            assert_eq!(parsed.text, body["text"].as_str().unwrap_or_default());
        }
    }

    #[test]
    fn a_rich_body_parses_and_round_trips() {
        let body = MessageBody::parse(&rich()).unwrap();
        assert_eq!(body.mentions, ["bot_ledger"]);
        assert_eq!(body.parts.len(), 4);
        assert_eq!(
            body.parts.iter().map(MessagePart::id).collect::<Vec<_>>(),
            ["t1", "ui1", "t2", "a1"]
        );
        let MessagePart::Tool(tool) = body.part("t2").unwrap() else {
            panic!("t2 is a tool part");
        };
        assert!(tool.failed && tool.result.is_none());
        assert_eq!(
            body.handles()
                .map(|handle| handle.path.as_str())
                .collect::<Vec<_>>(),
            ["tool-results/c2.json", "files/report.pdf"]
        );
        assert_eq!(body.error.as_ref().unwrap().code, "BOT_TURN_TIMEOUT");
        assert_eq!(body.ask.as_ref().unwrap().origin_conversation, "conv_a");
        let written = serde_json::to_value(&body).unwrap();
        assert_eq!(MessageBody::parse(&written).unwrap(), body);
        assert_eq!(written["parts"][0].get("failed"), None);
        assert_eq!(written["parts"][2]["failed"], json!(true));
    }

    #[test]
    fn parts_are_numbered_within_their_kind_in_order() {
        let mut body = rich();
        body["parts"][2]["id"] = json!("t3");
        let error = MessageBody::parse(&body).unwrap_err().to_string();
        assert!(
            error.contains("body.parts[2]") && error.contains("'t2'"),
            "{error}"
        );
        let mut body = rich();
        body["parts"][1]["id"] = json!("t1");
        let error = MessageBody::parse(&body).unwrap_err().to_string();
        assert!(error.contains("'ui1'"), "{error}");
        let many: Vec<Value> = (1..=MAX_PARTS + 1)
            .map(|n| json!({ "kind": "ui", "id": format!("ui{n}") }))
            .collect();
        let error = MessageBody::parse(&json!({ "parts": many })).unwrap_err();
        assert!(error.to_string().contains("at most 32"), "{error}");
        let unknown = json!({ "parts": [{ "kind": "video", "id": "v1" }] });
        assert!(MessageBody::parse(&unknown).is_err());
    }

    #[test]
    fn handles_point_at_a_plain_file_of_an_app() {
        for (field, value) in [
            ("app_id", json!("app ws")),
            ("path", json!("/etc/passwd")),
            ("path", json!("files/../secrets")),
            ("path", json!("files//report.pdf")),
            ("path", json!("files\\report.pdf")),
            ("path", json!("")),
            ("sha256", json!("A".repeat(64))),
            ("sha256", json!("a".repeat(63))),
            ("mime", json!("json")),
            ("mime", json!("text/plain; charset=utf-8")),
        ] {
            let mut body = rich();
            body["parts"][3]["handle"][field] = value.clone();
            let error = MessageBody::parse(&body).unwrap_err().to_string();
            assert!(
                error.contains(&format!("body.parts[3].handle.{field}")),
                "{field} {value}: {error}"
            );
        }
        assert!(is_workspace_path("a/b.c/d-e_f.json"));
        assert!(is_mime("application/vnd.api+json"));
    }

    #[test]
    fn a_part_keeps_its_content_inline_or_in_the_workspace_not_both() {
        let mut body = rich();
        body["parts"][0]["handle"] = handle("app_ws", "tool-results/c1.json");
        let error = MessageBody::parse(&body).unwrap_err().to_string();
        assert!(error.contains("not both"), "{error}");
        let mut body = rich();
        body["parts"][1]["handle"] = handle("app_ws", "ui/card.json");
        assert!(MessageBody::parse(&body).is_err());
        let mut body = rich();
        body["parts"][3].as_object_mut().unwrap().remove("handle");
        assert!(
            MessageBody::parse(&body).is_err(),
            "an attachment is a file"
        );
    }

    #[test]
    fn the_other_fields_name_what_they_refuse() {
        let cases = [
            (json!({ "deleted": true }), "body.deleted"),
            (json!({ "reply_to": "not an id" }), "body.reply_to"),
            (json!({ "mentions": ["has space"] }), "body.mentions"),
            (
                json!({ "mentions": (0..=MAX_CONVERSATION_MEMBERS).map(|n| format!("p{n}")).collect::<Vec<_>>() }),
                "at most 50",
            ),
            (
                json!({ "error": { "code": "two words" } }),
                "body.error.code",
            ),
            (json!({ "text": 5 }), "Invalid message body"),
            (json!({ "hops": -1 }), "Invalid message body"),
            (
                json!({ "ask": { "id": "a b", "origin_conversation": "c", "hops": 0, "reply_by": "x" } }),
                "body.ask.id",
            ),
            (
                json!({ "ask": { "id": "a", "origin_conversation": "c", "hops": 0, "reply_by": "" } }),
                "body.ask.reply_by",
            ),
            (
                json!({ "parts": [{ "kind": "tool", "id": "t1", "call_id": "c1", "name": "x",
                                     "summary": "line\nbreak" }] }),
                "body.parts[0].summary",
            ),
        ];
        for (body, expected) in cases {
            let error = MessageBody::parse(&body).unwrap_err().to_string();
            assert!(error.contains(expected), "{body}: {error}");
        }
    }

    #[test]
    fn the_late_answer_prompt_names_the_callee_the_outcome_and_the_last_message() {
        let answered = ask_result_prompt("Ledger", "Total?", "42", false, Some("Hi"));
        assert!(answered.contains("Earlier you asked Ledger: \"Total?\""));
        assert!(answered.contains("Ledger answered:\n42"));
        assert!(answered.contains("The last thing in your conversation with your owner: Hi"));
        let failed = ask_result_prompt("Ledger", "Total?", "offline", true, None);
        assert!(failed.contains("Ledger could not answer:\noffline"));
        assert!(!failed.contains("The last thing"));
        assert!(answered.contains("ask Ledger a follow-up"));
    }

    #[test]
    fn an_answer_wakes_the_asker_at_its_own_hops_only_for_questions_with_a_round() {
        let ask = |hops, round| BotAsk {
            id: "ask_1".into(),
            origin_conversation: "conv_a".into(),
            hops,
            reply_by: "2026-11-11T00:00:00Z".into(),
            chain: Vec::new(),
            wait_secs: 90,
            round,
        };
        assert_eq!(ask(1, 1).answer_wakes_at(), Some(1));
        assert_eq!(ask(2, 3).answer_wakes_at(), Some(1));
        assert_eq!(ask(3, 1).answer_wakes_at(), Some(2));
        assert_eq!(ask(2, 0).answer_wakes_at(), None);
        let legacy: BotAsk = serde_json::from_value(serde_json::json!({
            "id": "ask_1", "origin_conversation": "conv_a", "hops": 1, "reply_by": "x"
        }))
        .unwrap();
        assert_eq!(legacy.round, 0);
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("round")
                .is_none()
        );
    }
}

#[cfg(test)]
mod conversation_file_tests {
    use super::*;
    use serde_json::json;

    fn file_body(handle: Value) -> Value {
        json!({ "text": "see file", "parts": [
            { "kind": "attachment", "id": "a1", "name": "Übersicht (2)#1.pdf", "handle": handle }
        ] })
    }

    fn handle() -> WorkspaceHandle {
        WorkspaceHandle::conversation_file(
            "conv9",
            "file_1.pdf",
            2048,
            &"b".repeat(64),
            "application/pdf",
        )
    }

    #[test]
    fn a_conversation_file_is_an_attachment_with_a_conversation_handle() {
        let body = MessageBody::parse(&file_body(serde_json::to_value(handle()).unwrap())).unwrap();
        let files: Vec<_> = body.conversation_files().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].conversation_id, "conv9");
        assert_eq!(files[0].file_id, "file_1");
        assert!(files[0].handle.is_conversation_file());
    }

    #[test]
    fn workspace_handles_are_not_conversation_files() {
        let workspace = json!({ "app_id": "app_ws", "path": "files/a.pdf", "bytes": 1,
            "sha256": "a".repeat(64), "mime": "application/pdf" });
        let body = MessageBody::parse(&file_body(workspace)).unwrap();
        assert_eq!(body.conversation_files().count(), 0);
    }

    #[test]
    fn conversation_handles_are_checked_for_shape_and_size() {
        for (field, value) in [
            ("app_id", json!("conversation_")),
            ("app_id", json!("conversation_a b")),
            ("path", json!("files/file_1.pdf")),
            ("path", json!("file_1.pdf.exe")),
            ("path", json!("..")),
            ("bytes", json!(MAX_CONVERSATION_FILE_BYTES + 1)),
        ] {
            let mut raw = serde_json::to_value(handle()).unwrap();
            raw[field] = value.clone();
            let error = MessageBody::parse(&file_body(raw)).unwrap_err().to_string();
            assert!(error.contains(field), "{field}={value}: {error}");
        }
    }

    #[test]
    fn only_attachments_point_at_conversation_files() {
        let raw = serde_json::to_value(handle()).unwrap();
        let tool = json!({ "parts": [{ "kind": "tool", "id": "t1", "call_id": "c", "name": "x", "handle": raw }] });
        let error = MessageBody::parse(&tool).unwrap_err().to_string();
        assert!(error.contains("only attachments"), "{error}");
    }

    fn secret_card(secret: Value) -> Value {
        json!({ "approval": {
            "id": "ap-1", "turn_id": "bt_1", "tool": SECRET_REQUEST_TOOL,
            "summary": "Wants GITHUB_TOKEN", "scope": "", "args_hash": "h", "secret": secret,
        } })
    }

    #[test]
    fn configuration_approvals_keep_the_full_patch_beyond_the_summary_limit() {
        let changes = json!({ "instructions": "Read carefully.\n".repeat(100), "name": "Atlas" });
        let raw = json!({ "approval": {
            "id": "ap-1", "turn_id": "bt_1", "tool": "bot_update_configuration",
            "summary": "Change my settings", "scope": "", "args_hash": "h",
            "configuration_changes": changes,
        } });
        let body = MessageBody::parse(&raw).unwrap();
        assert_eq!(body.approval.unwrap().configuration_changes, Some(changes));
        let mut invalid = raw;
        invalid["approval"]["configuration_changes"] = json!(["name"]);
        assert!(MessageBody::parse(&invalid).is_err());
    }

    #[test]
    fn a_secret_card_names_the_secret_and_never_carries_a_value() {
        let body = MessageBody::parse(&secret_card(
            json!({ "name": "GITHUB_TOKEN", "description": "Open PRs" }),
        ))
        .unwrap();
        let secret = body.approval.unwrap().secret.unwrap();
        assert_eq!(secret.name, "GITHUB_TOKEN");
        assert!(!secret.replace);
        let written = serde_json::to_value(&secret).unwrap();
        assert!(written.get("replace").is_none());
        for name in ["github_token", "1KEY", "A B", "", &"A".repeat(65)] {
            let error =
                MessageBody::parse(&secret_card(json!({ "name": name, "description": "x" })))
                    .unwrap_err()
                    .to_string();
            assert!(error.contains("secret.name"), "{name}: {error}");
        }
        let long = MessageBody::parse(&secret_card(
            json!({ "name": "KEY", "description": "d".repeat(MAX_SUMMARY_CHARS + 1) }),
        ));
        assert!(long.is_err());
    }

    #[test]
    fn an_invite_card_carries_the_person_and_one_line_of_reason() {
        let card = |invite: Value| {
            json!({ "approval": {
                "id": "ap-1", "turn_id": "bt_1", "tool": ROOM_INVITE_TOOL,
                "summary": "Invite ana", "scope": "", "args_hash": "h", "invite": invite,
            } })
        };
        let body = MessageBody::parse(&card(
            json!({ "person": "ana.lima", "reason": "She owns the budget sheet." }),
        ))
        .unwrap();
        let approval = body.approval.unwrap();
        assert_eq!(
            approval.invite,
            Some(RoomInviteAsk {
                person: "ana.lima".into(),
                reason: "She owns the budget sheet.".into(),
            })
        );
        assert_eq!(approval.batch()[0].invite, approval.invite);
        for invite in [
            json!({ "person": "ana", "reason": "line\nbreak" }),
            json!({ "person": "a".repeat(MAX_INVITE_PERSON_CHARS + 1), "reason": "x" }),
            json!({ "person": "ana", "reason": "r".repeat(MAX_SUMMARY_CHARS + 1) }),
        ] {
            let error = MessageBody::parse(&card(invite)).unwrap_err().to_string();
            assert!(error.contains("approval.invite"), "{error}");
        }
    }

    fn plain_card() -> ApprovalRequest {
        ApprovalRequest {
            id: "ap-1".into(),
            turn_id: "bt_1".into(),
            tool: "web_request".into(),
            summary: "The bot wants to send a GET request to cisa.gov/feeds.".into(),
            scope: "web:cisa.gov:read".into(),
            args_hash: "h1".into(),
            requester_id: None,
            requester_name: None,
            room: None,
            secret: None,
            invite: None,
            configuration_changes: None,
            items: Vec::new(),
        }
    }

    fn item(hash: &str) -> ApprovalItem {
        ApprovalItem {
            args_hash: hash.into(),
            tool: "web_request".into(),
            summary: format!("The bot wants to fetch {hash}."),
            scope: "web:cisa.gov:read".into(),
            calls: 1,
            after_web_content: false,
            secret: None,
            invite: None,
        }
    }

    fn batch_card(hashes: &[&str]) -> ApprovalRequest {
        ApprovalRequest {
            tool: APPROVAL_BATCH_TOOL.into(),
            items: hashes.iter().map(|hash| item(hash)).collect(),
            ..plain_card()
        }
    }

    #[test]
    fn a_batch_of_one_serializes_like_todays_card() {
        use crate::tool_grant::{ApprovalAnswer, ApprovalDecision};
        let golden = r#"{"id":"ap-1","turn_id":"bt_1","tool":"web_request","summary":"The bot wants to send a GET request to cisa.gov/feeds.","scope":"web:cisa.gov:read","args_hash":"h1"}"#;
        assert_eq!(serde_json::to_string(&plain_card()).unwrap(), golden);
        assert_eq!(
            serde_json::from_str::<ApprovalRequest>(golden).unwrap(),
            plain_card()
        );
        let decisions = plain_card()
            .decisions(ApprovalDecision::Always, &[])
            .unwrap();
        let response = ApprovalResponse::for_decisions("ap-1", &decisions);
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"approval_id":"ap-1","decision":"always"}"#
        );
        let answer = ApprovalAnswer {
            approval_id: "ap-1".into(),
            decision: ApprovalDecision::Once,
            items: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&answer).unwrap(),
            r#"{"approval_id":"ap-1","decision":"once"}"#
        );
        assert_eq!(
            serde_json::to_value(item("h2")).unwrap(),
            json!({"args_hash":"h2","tool":"web_request","summary":"The bot wants to fetch h2.","scope":"web:cisa.gov:read"})
        );
    }

    #[test]
    fn an_old_card_reads_as_a_batch_of_one() {
        let mut card = plain_card();
        card.secret = Some(SecretAsk {
            name: "CISA_KEY".into(),
            description: "Read the feed".into(),
            replace: false,
        });
        let raw = serde_json::to_value(&card).unwrap();
        assert!(raw.get("items").is_none());
        let parsed = MessageBody::parse(&json!({ "approval": raw })).unwrap();
        let batch = parsed.approval.unwrap().batch();
        assert_eq!(batch.len(), 1);
        assert_eq!(
            batch[0],
            ApprovalItem {
                args_hash: "h1".into(),
                tool: "web_request".into(),
                summary: card.summary.clone(),
                scope: card.scope.clone(),
                calls: 1,
                after_web_content: false,
                secret: card.secret.clone(),
                invite: None,
            }
        );
        let parsed: ApprovalResponse =
            serde_json::from_value(json!({"approval_id": "ap-1", "decision": "deny"})).unwrap();
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn a_card_refuses_33_items_and_repeated_hashes() {
        let body = |card: &ApprovalRequest| MessageBody {
            approval: Some(card.clone()),
            ..Default::default()
        };
        let hashes: Vec<String> = (0..MAX_APPROVAL_ITEMS).map(|n| format!("h{n}")).collect();
        let full = batch_card(&hashes.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(body(&full).check().is_ok());
        let mut over = full.clone();
        over.items.push(item("h99"));
        let error = body(&over).check().unwrap_err().to_string();
        assert!(error.contains("at most 32 items"), "{error}");
        let repeated = batch_card(&["h1", "h2", "h1"]);
        let error = body(&repeated).check().unwrap_err().to_string();
        assert!(error.contains("names item 'h1' twice"), "{error}");
        let mut no_calls = batch_card(&["h1", "h2"]);
        no_calls.items[1].calls = 0;
        let error = body(&no_calls).check().unwrap_err().to_string();
        assert!(error.contains("items[1].calls"), "{error}");
        let mut spaced = batch_card(&["h1", "h2"]);
        spaced.items[0].tool = "web request".into();
        let error = body(&spaced).check().unwrap_err().to_string();
        assert!(error.contains("items[0].tool"), "{error}");
        let response = MessageBody {
            approval_response: Some(ApprovalResponse {
                approval_id: "ap-1".into(),
                decision: crate::tool_grant::ApprovalDecision::Once,
                items: ["h1", "h1"]
                    .map(|hash| ItemDecision {
                        args_hash: hash.into(),
                        decision: crate::tool_grant::ApprovalDecision::Deny,
                    })
                    .to_vec(),
            }),
            ..Default::default()
        };
        assert!(response.check().is_err());
    }

    #[test]
    fn an_answer_defaults_unlisted_items_to_its_decision() {
        use crate::tool_grant::ApprovalDecision::{Always, Deny, Once};
        let card = batch_card(&["h1", "h2", "h3"]);
        let answer = |hash: &str, decision| ItemDecision {
            args_hash: hash.into(),
            decision,
        };
        let decided = |decisions: Vec<(ApprovalItem, crate::tool_grant::ApprovalDecision)>| {
            decisions
                .into_iter()
                .map(|(item, decision)| (item.args_hash, decision))
                .collect::<Vec<_>>()
        };
        let mixed = card.decisions(Deny, &[answer("h2", Once)]).unwrap();
        assert_eq!(
            decided(mixed.clone()),
            vec![
                ("h1".to_string(), Deny),
                ("h2".to_string(), Once),
                ("h3".to_string(), Deny)
            ]
        );
        let response = ApprovalResponse::for_decisions(card.id.clone(), &mixed);
        assert_eq!(response.decision, Once);
        assert_eq!(response.items, vec![answer("h1", Deny), answer("h3", Deny)]);
        let uniform = card.decisions(Always, &[answer("h3", Always)]).unwrap();
        assert_eq!(
            ApprovalResponse::for_decisions(card.id.clone(), &uniform),
            ApprovalResponse {
                approval_id: card.id.clone(),
                decision: Always,
                items: Vec::new(),
            }
        );
        let unknown = card.decisions(Once, &[answer("h9", Deny)]).unwrap_err();
        assert!(unknown.contains("no item 'h9'"), "{unknown}");
        let twice = card
            .decisions(Once, &[answer("h1", Deny), answer("h1", Once)])
            .unwrap_err();
        assert!(twice.contains("twice"), "{twice}");
    }

    #[test]
    fn secret_names_follow_the_store_rule() {
        assert!(is_secret_name("A"));
        assert!(is_secret_name("API_KEY_2"));
        assert!(is_secret_name(&format!("A{}", "B".repeat(63))));
        for bad in ["", "a", "_A", "A-B", "A.B", "Ä"] {
            assert!(!is_secret_name(bad), "{bad}");
        }
        assert!(!is_secret_name(&"A".repeat(65)));
    }
}

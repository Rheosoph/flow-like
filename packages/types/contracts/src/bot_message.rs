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
/// Principal ids, call ids and tool names have at most this many characters.
pub const MAX_NAME_CHARS: usize = 128;
/// A part's one-line summary has at most this many characters.
pub const MAX_SUMMARY_CHARS: usize = 500;
/// An attachment's file name has at most this many characters.
pub const MAX_FILE_NAME_CHARS: usize = 255;
/// A workspace path has at most this many characters.
pub const MAX_PATH_CHARS: usize = 1_024;
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
    /// The full proposed settings patch, shown before approving a bot configuration change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_changes: Option<Value>,
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

/// The owner's answer to an [`ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalResponse {
    pub approval_id: String,
    pub decision: crate::tool_grant::ApprovalDecision,
}

impl ApprovalRequest {
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
        self.secret.as_ref().map_or(Ok(()), SecretAsk::check)
    }
}

impl ApprovalResponse {
    fn check(&self) -> Result<(), BodyError> {
        check_name(
            "body.approval_response.approval_id",
            &self.approval_id,
            MAX_NAME_CHARS,
        )
    }
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

/// A file from the sending bot's workspace, or a conversation file any member sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentPart {
    /// `a1`, `a2` …
    pub id: String,
    /// The file name shown.
    pub name: String,
    pub handle: WorkspaceHandle,
}

impl AttachmentPart {
    /// The line a model reads for the file. A conversation file carries its whole handle, which
    /// the file tools take as the `file` argument.
    pub fn note(&self) -> String {
        if !self.handle.is_conversation_file() {
            return format!("[file: {}]", self.name);
        }
        let handle = serde_json::to_string(&self.handle).unwrap_or_default();
        format!(
            "[file: {} ({}, {} bytes); read it with the file tools, file = {handle}]",
            self.name, self.handle.mime, self.handle.bytes
        )
    }
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
fn is_name(value: &str, max: usize) -> bool {
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
            Self::Attachment(part) => Some(&part.handle),
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
            Self::Attachment(part) => {
                if !(1..=MAX_FILE_NAME_CHARS).contains(&chars(&part.name))
                    || part.name.chars().any(char::is_control)
                {
                    return refuse(format!(
                        "{field}.name must be 1 to {MAX_FILE_NAME_CHARS} characters"
                    ));
                }
            }
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
        Some(self.path.split_once('.').map_or(self.path.as_str(), |(id, _)| id))
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
        WorkspaceHandle::conversation_file("conv9", "file_1.pdf", 2048, &"b".repeat(64), "application/pdf")
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

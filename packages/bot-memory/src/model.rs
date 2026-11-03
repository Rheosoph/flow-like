//! History items as the engine sees them: one inbox message each, with its `seq`, role, times,
//! text projection, tool parts with their sizes, UI blocks and flags.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::pointer::{PartRef, Pointer};
use crate::text::estimate_tokens;

/// A per-conversation position: never reused, the same in every member's copy.
pub type Seq = u64;

/// An inclusive range of positions: a segment, an evicted span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeqSpan {
    pub from: Seq,
    pub to: Seq,
}

impl SeqSpan {
    pub fn new(from: Seq, to: Seq) -> Self {
        Self {
            from: from.min(to),
            to: from.max(to),
        }
    }

    pub fn contains(self, seq: Seq) -> bool {
        (self.from..=self.to).contains(&seq)
    }

    pub fn overlaps(self, other: SeqSpan) -> bool {
        self.from <= other.to && other.from <= self.to
    }

    pub fn pointer(self) -> Pointer {
        Pointer::range(self.from, self.to)
    }
}

impl std::fmt::Display for SeqSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.pointer().fmt(f)
    }
}

/// Who wrote an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The bot's owner.
    User,
    /// The bot.
    Assistant,
    /// A tool result delivered as its own item.
    Tool,
    /// Notices the host writes: wake-ups, job and approval updates.
    System,
    /// Content from outside the conversation: webhooks, mail, other bots.
    External,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::User,
        Role::Assistant,
        Role::Tool,
        Role::System,
        Role::External,
    ];

    /// The label the model sees.
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "bot",
            Self::Tool => "tool",
            Self::System => "system",
            Self::External => "external",
        }
    }

    /// `user`, `bot`/`assistant`, `tool`, `system` or `external`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "assistant" => Some(Self::Assistant),
            other => Self::ALL.into_iter().find(|role| role.label() == other),
        }
    }

    /// The most a fact can claim when this role's text is its evidence.
    pub fn trust(self) -> FactSource {
        match self {
            Self::User => FactSource::UserStated,
            Self::Assistant | Self::System => FactSource::BotInferred,
            Self::Tool | Self::External => FactSource::ToolUntrusted,
        }
    }
}

/// Where a fact comes from, ordered by trust: the user said it, the bot concluded it, or it came
/// from tool, web or outside content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    ToolUntrusted,
    BotInferred,
    UserStated,
}

impl FactSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserStated => "user_stated",
            Self::BotInferred => "bot_inferred",
            Self::ToolUntrusted => "tool_untrusted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::UserStated, Self::BotInferred, Self::ToolUntrusted]
            .into_iter()
            .find(|source| source.as_str().eq_ignore_ascii_case(value.trim()))
    }

    /// The one-word label in recalled fact lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::UserStated => "user",
            Self::BotInferred => "bot",
            Self::ToolUntrusted => "tool",
        }
    }
}

/// The result of a tool call: its size and, once the model no longer sees it, one line about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tokens: u32,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub gist: Option<String>,
    /// The output's first and last characters (a few hundred each), shown when a large failed
    /// result is capped: errors are evidence, often at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
}

/// A tool call with its result. A result delivered later arrives in a later item as a part with
/// the same `call_id`; the first part with a call id is the call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPart {
    /// Position among the message's tool parts, from 1: `#seq.t<index>`.
    pub index: u16,
    pub call_id: String,
    pub name: String,
    /// Tokens of the call (name and arguments).
    #[serde(default)]
    pub call_tokens: u32,
    /// `None` while the call is unresolved.
    #[serde(default)]
    pub result: Option<ToolResult>,
}

impl ToolPart {
    pub fn call(
        index: u16,
        call_id: impl Into<String>,
        name: impl Into<String>,
        call_tokens: u32,
    ) -> Self {
        Self {
            index,
            call_id: call_id.into(),
            name: name.into(),
            call_tokens,
            result: None,
        }
    }

    pub fn with_result(mut self, tokens: u32, failed: bool, gist: Option<&str>) -> Self {
        self.result = Some(ToolResult {
            tokens,
            failed,
            gist: gist.map(str::to_string),
            head: None,
            tail: None,
        });
        self
    }

    /// The result's first and last characters; see [`ToolResult::head`].
    pub fn with_excerpt(mut self, head: &str, tail: &str) -> Self {
        if let Some(result) = self.result.as_mut() {
            result.head = Some(head.to_string());
            result.tail = Some(tail.to_string());
        }
        self
    }

    pub fn part(&self) -> PartRef {
        PartRef::Tool(self.index)
    }
}

/// A rendered UI block; the model only ever sees its one-line projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPart {
    /// Position among the message's UI blocks, from 1: `#seq.ui<index>`.
    pub index: u16,
    pub kind: String,
    pub projection: String,
    pub tokens: u32,
}

impl UiPart {
    pub fn new(index: u16, kind: impl Into<String>, projection: impl Into<String>) -> Self {
        let projection = projection.into();
        Self {
            index,
            kind: kind.into(),
            tokens: estimate_tokens(&projection),
            projection,
        }
    }

    pub fn part(&self) -> PartRef {
        PartRef::Ui(self.index)
    }
}

/// What an entry of the history is; the inbox's entry kinds map onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// A message: the user's, the bot's, a tool result or a notice.
    #[default]
    Message,
    /// A "New topic" marker (the inbox's `topic` entry): a new segment starts here.
    Topic,
}

/// One entry of the bot's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub seq: Seq,
    pub message_id: String,
    #[serde(default)]
    pub kind: ItemKind,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub edited_at: Option<DateTime<Utc>>,
    /// The message's text projection.
    pub text: String,
    pub text_tokens: u32,
    #[serde(default)]
    pub tools: Vec<ToolPart>,
    #[serde(default)]
    pub ui: Vec<UiPart>,
    #[serde(default)]
    pub pinned: bool,
    /// The turn failed; its error stays verbatim.
    #[serde(default)]
    pub failed: bool,
    /// A tombstone left by a deletion.
    #[serde(default)]
    pub deleted: bool,
    /// "New topic" on a message: a new segment starts here.
    #[serde(default)]
    pub new_topic: bool,
}

impl HistoryItem {
    pub fn new(
        seq: Seq,
        message_id: impl Into<String>,
        role: Role,
        created_at: DateTime<Utc>,
        text: impl Into<String>,
    ) -> Self {
        let text = text.into();
        Self {
            seq,
            message_id: message_id.into(),
            kind: ItemKind::Message,
            role,
            created_at,
            edited_at: None,
            text_tokens: estimate_tokens(&text),
            text,
            tools: Vec::new(),
            ui: Vec::new(),
            pinned: false,
            failed: false,
            deleted: false,
            new_topic: false,
        }
    }

    /// A "New topic" marker with its own position; `title` may be empty.
    pub fn topic(
        seq: Seq,
        message_id: impl Into<String>,
        created_at: DateTime<Utc>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            kind: ItemKind::Topic,
            ..Self::new(seq, message_id, Role::System, created_at, title)
        }
    }

    pub fn with_tool(mut self, part: ToolPart) -> Self {
        self.tools.push(part);
        self
    }

    pub fn with_ui(mut self, part: UiPart) -> Self {
        self.ui.push(part);
        self
    }

    pub fn with_pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn with_failed(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }

    pub fn with_new_topic(mut self, new_topic: bool) -> Self {
        self.new_topic = new_topic;
        self
    }

    pub fn pointer(&self) -> Pointer {
        Pointer::Message(self.seq)
    }

    /// A user message opens a turn.
    pub fn starts_turn(&self) -> bool {
        self.role == Role::User && !self.deleted && self.kind == ItemKind::Message
    }

    /// A new segment starts here: a topic marker, or an entry flagged "New topic".
    pub fn starts_topic(&self) -> bool {
        !self.deleted && (self.kind == ItemKind::Topic || self.new_topic)
    }

    /// Where a cut or a segment may begin: a turn or a topic.
    pub fn opens_span(&self) -> bool {
        self.starts_turn() || self.starts_topic()
    }

    /// The turn failed or one of its tool calls did.
    pub fn has_failure(&self) -> bool {
        self.failed
            || self
                .tools
                .iter()
                .any(|part| part.result.as_ref().is_some_and(|result| result.failed))
    }

    pub fn tool(&self, index: u16) -> Option<&ToolPart> {
        self.tools.iter().find(|part| part.index == index)
    }

    pub fn ui_part(&self, index: u16) -> Option<&UiPart> {
        self.ui.iter().find(|part| part.index == index)
    }

    /// Tokens with every tool result shown in full.
    pub fn full_tokens(&self) -> u32 {
        let tools: u32 = self
            .tools
            .iter()
            .map(|part| part.call_tokens + part.result.as_ref().map_or(0, |result| result.tokens))
            .sum();
        let ui: u32 = self.ui.iter().map(|part| part.tokens).sum();
        self.text_tokens + tools + ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_carry_their_trust_and_labels() {
        assert_eq!(Role::User.trust(), FactSource::UserStated);
        assert_eq!(Role::Assistant.trust(), FactSource::BotInferred);
        assert_eq!(Role::Tool.trust(), FactSource::ToolUntrusted);
        assert_eq!(Role::External.trust(), FactSource::ToolUntrusted);
        assert!(FactSource::UserStated > FactSource::BotInferred);
        assert!(FactSource::BotInferred > FactSource::ToolUntrusted);
        assert_eq!(Role::parse("BOT"), Some(Role::Assistant));
        assert_eq!(Role::parse("assistant"), Some(Role::Assistant));
        assert_eq!(Role::parse("nobody"), None);
        assert_eq!(
            FactSource::parse("Tool_Untrusted"),
            Some(FactSource::ToolUntrusted)
        );
        assert_eq!(
            serde_json::to_value(FactSource::UserStated).unwrap(),
            "user_stated"
        );
    }

    #[test]
    fn items_count_their_parts_and_failures() {
        let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let item = HistoryItem::new(7, "m7", Role::Assistant, at, "checking the weather")
            .with_tool(ToolPart::call(1, "c1", "weather_api", 20).with_result(2_000, false, None))
            .with_tool(ToolPart::call(2, "c2", "geo", 10).with_result(50, true, Some("timeout")))
            .with_ui(UiPart::new(1, "card", "Weather card: 14°C"));
        assert_eq!(
            item.full_tokens(),
            item.text_tokens + 20 + 2_000 + 10 + 50 + item.ui[0].tokens
        );
        assert!(item.has_failure());
        assert!(!item.starts_turn());
        assert_eq!(item.tool(2).unwrap().name, "geo");
        assert_eq!(item.ui_part(1).unwrap().part(), PartRef::Ui(1));
        let round_trip: HistoryItem =
            serde_json::from_value(serde_json::to_value(&item).unwrap()).unwrap();
        assert_eq!(round_trip, item);
    }
}

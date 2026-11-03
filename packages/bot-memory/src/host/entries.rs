//! A bot's inbox copy as the engine's history: entries map to [`HistoryItem`]s (messages, topic
//! markers, tombstones), edits replace their message's text, and [`InboxReader`] serves the
//! engine's [`HistoryReader`] from any store that can read entries by position.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flow_like_types::Result;
use flow_like_types::bot_message::{
    MessageBody, MessagePart, ToolPart as BodyTool, WorkspaceHandle,
};
use flow_like_types::inbox::{
    InboxEntryKind, PLAIN_ENCRYPTION, TopicBody, TriggerBody, TriggerSource, body_target,
    is_tombstone,
};
use serde_json::Value;

use crate::model::{HistoryItem, Role, Seq, SeqSpan, ToolPart, UiPart};
use crate::pointer::PartRef;
use crate::text::estimate_tokens;
use crate::tools::{Deleted, HistoryReader};

const PAGE: usize = 100;
const MAX_PAGES: usize = 20;
const EDIT_SCAN: usize = 500;

/// One entry of the bot's copy of a conversation, as a store hands it out.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEntry {
    pub seq: Seq,
    pub message_id: String,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    pub envelope: Value,
    pub created_at: DateTime<Utc>,
}

/// Reads the bot's copy of its conversation by position. Pages are oldest first.
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait EntrySource: Send + Sync {
    async fn at(&self, seq: Seq) -> Result<Option<StoredEntry>>;
    /// Up to `limit` entries right before `seq`, oldest first.
    async fn before(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>>;
    /// Up to `limit` entries right after `seq`, oldest first.
    async fn after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>>;
    /// Up to `limit` entries of `from..=to`, oldest first.
    async fn range(&self, from: Seq, to: Seq, limit: usize) -> Result<Vec<StoredEntry>>;
    /// Up to `limit` edit entries after `seq`, oldest first.
    async fn edits_after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>>;
    /// The newest position of the copy.
    async fn latest(&self) -> Result<Option<Seq>>;
    /// The positions of messages whose deletion is recorded after `seq` (at most `limit`
    /// deletions), and the position of the last deletion read.
    async fn deleted_after(&self, seq: Seq, limit: usize) -> Result<Deleted>;
}

/// Reads a part stored in the bot's workspace.
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait HandleReader: Send + Sync {
    async fn read(&self, handle: &WorkspaceHandle) -> Result<Option<String>>;
}

fn plain_body(envelope: &Value) -> Option<&Value> {
    (envelope.get("enc").and_then(Value::as_str) == Some(PLAIN_ENCRYPTION))
        .then(|| envelope.get("body"))
        .flatten()
}

fn part_index(id: &str) -> u16 {
    id.trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .unwrap_or(1)
}

fn tool_part(part: &BodyTool) -> ToolPart {
    let arguments = part
        .arguments
        .as_ref()
        .map_or(0, |arguments| estimate_tokens(&arguments.to_string()));
    let call = ToolPart::call(
        part_index(&part.id),
        part.call_id.clone(),
        part.name.clone(),
        arguments + estimate_tokens(&part.name),
    );
    let size = match (&part.result, &part.handle) {
        (Some(result), _) => Some(estimate_tokens(&result.to_string())),
        (None, Some(handle)) => Some(u32::try_from(handle.bytes / 4).unwrap_or(u32::MAX)),
        (None, None) => part
            .summary
            .as_ref()
            .map(|summary| estimate_tokens(summary)),
    };
    match size {
        Some(tokens) => call.with_result(tokens, part.failed, part.summary.as_deref()),
        None => call,
    }
}

fn message_text(body: &MessageBody) -> String {
    let mut text = body.text.clone();
    for part in &body.parts {
        if let MessagePart::Attachment(file) = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("[file: {}]", file.name));
        }
    }
    text
}

fn apply_body(item: &mut HistoryItem, body: &MessageBody) {
    item.text = message_text(body);
    item.text_tokens = estimate_tokens(&item.text);
    item.tools = body
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Tool(tool) => Some(tool_part(tool)),
            _ => None,
        })
        .collect();
    item.ui = body
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Ui(ui) => Some(UiPart::new(
                part_index(&ui.id),
                "ui",
                ui.summary.clone().unwrap_or_else(|| "UI block".into()),
            )),
            _ => None,
        })
        .collect();
    item.failed = body.error.is_some();
}

fn role_of(sender: &str, bot_id: &str, body: Option<&MessageBody>) -> Role {
    if sender == bot_id {
        Role::Assistant
    } else if body.is_some_and(|body| body.hops.unwrap_or(0) > 0) {
        Role::External
    } else {
        Role::User
    }
}

/// The history item an entry stands for: a message, a "New topic" marker or a tombstone.
/// Edits, reactions, member events and sealed entries are not items.
pub fn item_of(entry: &StoredEntry, bot_id: &str) -> Option<HistoryItem> {
    let body = plain_body(&entry.envelope)?;
    match entry.kind {
        InboxEntryKind::Message => {
            if is_tombstone(&entry.envelope) {
                let mut item = HistoryItem::new(
                    entry.seq,
                    entry.message_id.clone(),
                    role_of(&entry.sender_id, bot_id, None),
                    entry.created_at,
                    "",
                );
                item.deleted = true;
                return Some(item);
            }
            let parsed: MessageBody = serde_json::from_value(body.clone()).ok()?;
            let mut item = HistoryItem::new(
                entry.seq,
                entry.message_id.clone(),
                role_of(&entry.sender_id, bot_id, Some(&parsed)),
                entry.created_at,
                "",
            );
            apply_body(&mut item, &parsed);
            Some(item)
        }
        InboxEntryKind::Trigger => {
            let trigger = TriggerBody::parse(body).ok()?;
            let role = if trigger.source == TriggerSource::Email {
                Role::External
            } else {
                role_of(&entry.sender_id, bot_id, None)
            };
            let parsed = MessageBody {
                text: trigger.render(),
                ..MessageBody::default()
            };
            let mut item = HistoryItem::new(
                entry.seq,
                entry.message_id.clone(),
                role,
                entry.created_at,
                "",
            );
            apply_body(&mut item, &parsed);
            Some(item)
        }
        InboxEntryKind::Topic => {
            let title = serde_json::from_value::<TopicBody>(body.clone())
                .ok()
                .and_then(|topic| topic.title)
                .unwrap_or_default();
            Some(HistoryItem::topic(
                entry.seq,
                entry.message_id.clone(),
                entry.created_at,
                title,
            ))
        }
        _ => None,
    }
}

/// Lets the newest edit of a message by its sender replace the message's text and parts.
pub fn apply_edits(
    items: &mut [HistoryItem],
    edits: &[StoredEntry],
    senders: &HashMap<String, String>,
) {
    for edit in edits {
        if is_tombstone(&edit.envelope) {
            continue;
        }
        let (Some(target), Some(body)) = (body_target(&edit.envelope), plain_body(&edit.envelope))
        else {
            continue;
        };
        let Ok(parsed) = serde_json::from_value::<MessageBody>(body.clone()) else {
            continue;
        };
        if let Some(item) = items.iter_mut().find(|item| {
            item.message_id == target
                && !item.deleted
                && senders
                    .get(target)
                    .is_some_and(|sender| *sender == edit.sender_id)
        }) {
            apply_body(item, &parsed);
            item.edited_at = Some(edit.created_at);
        }
    }
}

/// The engine's [`HistoryReader`] over the bot's copy of its conversation.
pub struct InboxReader<S> {
    source: S,
    bot_id: String,
    handles: Option<Arc<dyn HandleReader>>,
}

impl<S: EntrySource> InboxReader<S> {
    pub fn new(source: S, bot_id: impl Into<String>) -> Self {
        Self {
            source,
            bot_id: bot_id.into(),
            handles: None,
        }
    }

    pub fn with_handles(mut self, handles: Arc<dyn HandleReader>) -> Self {
        self.handles = Some(handles);
        self
    }

    async fn items(&self, entries: &[StoredEntry]) -> Result<Vec<HistoryItem>> {
        let mut items: Vec<HistoryItem> = entries
            .iter()
            .filter_map(|entry| item_of(entry, &self.bot_id))
            .collect();
        let Some(first) = items.first().map(|item| item.seq) else {
            return Ok(items);
        };
        if items.iter().all(|item| item.deleted) {
            return Ok(items);
        }
        let edits = self.source.edits_after(first, EDIT_SCAN).await?;
        if !edits.is_empty() {
            let senders = entries
                .iter()
                .map(|entry| (entry.message_id.clone(), entry.sender_id.clone()))
                .collect();
            apply_edits(&mut items, &edits, &senders);
        }
        Ok(items)
    }
}

#[async_trait]
impl<S: EntrySource> HistoryReader for InboxReader<S> {
    async fn message(&self, seq: Seq) -> Result<Option<HistoryItem>> {
        let Some(entry) = self.source.at(seq).await? else {
            return Ok(None);
        };
        Ok(self.items(&[entry]).await?.into_iter().next())
    }

    async fn before(&self, seq: Seq, limit: usize) -> Result<Vec<HistoryItem>> {
        let mut found: Vec<HistoryItem> = Vec::new();
        let mut edge = seq;
        for _ in 0..MAX_PAGES {
            let want = limit.saturating_sub(found.len());
            if want == 0 || edge <= 1 {
                break;
            }
            let entries = self
                .source
                .before(edge, (want * 2).max(8).min(PAGE))
                .await?;
            let Some(oldest) = entries.first().map(|entry| entry.seq) else {
                break;
            };
            let mut page = self.items(&entries).await?;
            page.append(&mut found);
            found = page;
            if found.len() > limit {
                found.drain(..found.len() - limit);
            }
            edge = oldest;
        }
        Ok(found)
    }

    async fn after(&self, seq: Seq, limit: usize) -> Result<Vec<HistoryItem>> {
        let mut found = Vec::new();
        let mut edge = seq;
        for _ in 0..MAX_PAGES {
            let want = limit.saturating_sub(found.len());
            if want == 0 {
                break;
            }
            let entries = self.source.after(edge, (want * 2).max(8).min(PAGE)).await?;
            let Some(newest) = entries.last().map(|entry| entry.seq) else {
                break;
            };
            found.extend(self.items(&entries).await?);
            found.truncate(limit);
            edge = newest;
        }
        Ok(found)
    }

    async fn range(&self, span: SeqSpan, limit: usize) -> Result<Vec<HistoryItem>> {
        let mut found: Vec<HistoryItem> = Vec::new();
        let mut from = span.from;
        while from <= span.to && found.len() < limit {
            let entries = self.source.range(from, span.to, PAGE).await?;
            let Some(newest) = entries.last().map(|entry| entry.seq) else {
                break;
            };
            found.extend(self.items(&entries).await?);
            if entries.len() < PAGE {
                break;
            }
            from = newest + 1;
        }
        found.truncate(limit);
        Ok(found)
    }

    async fn deleted_since(&self, seq: Seq, limit: usize) -> Result<Deleted> {
        self.source.deleted_after(seq, limit).await
    }

    async fn latest(&self) -> Result<Option<Seq>> {
        self.source.latest().await
    }

    async fn part(&self, seq: Seq, part: PartRef) -> Result<Option<String>> {
        let Some(entry) = self.source.at(seq).await? else {
            return Ok(None);
        };
        if is_tombstone(&entry.envelope) {
            return Ok(None);
        }
        let Some(body) = plain_body(&entry.envelope)
            .and_then(|body| serde_json::from_value::<MessageBody>(body.clone()).ok())
        else {
            return Ok(None);
        };
        let stored = match part {
            PartRef::Tool(index) => body.parts.iter().find_map(|candidate| match candidate {
                MessagePart::Tool(tool) if part_index(&tool.id) == index => Some((
                    tool.result.as_ref().map(render_value),
                    tool.handle.clone(),
                    tool.summary.clone(),
                )),
                _ => None,
            }),
            PartRef::Ui(index) => body.parts.iter().find_map(|candidate| match candidate {
                MessagePart::Ui(ui) if part_index(&ui.id) == index => Some((
                    ui.a2ui.as_ref().map(render_value),
                    ui.handle.clone(),
                    ui.summary.clone(),
                )),
                _ => None,
            }),
        };
        let Some((inline, handle, summary)) = stored else {
            return Ok(None);
        };
        if let Some(content) = inline {
            return Ok(Some(content));
        }
        if let (Some(handle), Some(handles)) = (&handle, &self.handles)
            && let Some(content) = handles.read(handle).await?
        {
            return Ok(Some(content));
        }
        Ok(Some(match (handle, summary) {
            (Some(handle), _) => format!(
                "Stored in the bot's workspace ({} bytes): {}",
                handle.bytes, handle.path
            ),
            (None, Some(summary)) => summary,
            (None, None) => String::new(),
        }))
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[cfg(test)]
mod email_trigger_tests {
    use super::*;
    use serde_json::json;

    fn trigger_entry(source: &str) -> StoredEntry {
        StoredEntry {
            seq: 3,
            message_id: "t".into(),
            sender_id: "owner".into(),
            kind: InboxEntryKind::Trigger,
            envelope: json!({"v": 1, "enc": "none", "body": {
                "source": source, "ref_id": "r", "title": "T", "text": "x"
            }}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn mail_triggers_are_external_and_other_triggers_stay_the_owners() {
        assert_eq!(
            item_of(&trigger_entry("email"), "bot").unwrap().role,
            Role::External
        );
        assert_eq!(
            item_of(&trigger_entry("schedule"), "bot").unwrap().role,
            Role::User
        );
    }
}

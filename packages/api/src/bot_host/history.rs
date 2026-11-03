//! What a headless turn reads: the bot's own copy of the conversation, its last entries up to
//! the message the turn answers, as chat history. A "New topic" marker starts the history over;
//! edits replace their message's text; deleted and sealed messages are left out.

use crate::{
    entity::{bot, inbox_entry, user},
    error::ApiError,
};
use flow_like::flow::copilot::{ChatMessage, ChatRole};
use flow_like_types::{
    bot_message::{MessageBody, MessagePart},
    bot_turn::room_message,
    inbox::{
        ConversationKind, InboxEntryKind, PLAIN_ENCRYPTION, TriggerBody, body_target, is_tombstone,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::collections::HashMap;

/// Entries of the bot's copy a turn reads, the answered message included.
pub(crate) const HISTORY_ENTRIES: u64 = 32;
pub(crate) const MAX_PROMPT_CHARS: usize = 20_000;
pub(crate) const MAX_HISTORY_MESSAGE_CHARS: usize = 8_000;

/// The message to answer and what came before it.
#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    pub prompt: String,
    pub history: Vec<ChatMessage>,
}

#[derive(Debug)]
pub(crate) enum HistoryError {
    Db(ApiError),
    /// The bot has no copy of the message the turn answers.
    TriggerMissing,
    /// The message was deleted, or the server can't read it.
    TriggerUnreadable,
}

impl From<DbErr> for HistoryError {
    fn from(error: DbErr) -> Self {
        Self::Db(error.into())
    }
}

/// One message as the model reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    message_id: String,
    sender_id: String,
    text: String,
}

fn plain_body(envelope: &serde_json::Value) -> Option<&serde_json::Value> {
    (envelope.get("enc").and_then(serde_json::Value::as_str) == Some(PLAIN_ENCRYPTION))
        .then(|| envelope.get("body"))
        .flatten()
}

/// The message's text with a line for each UI block and file it carries.
fn render(body: &MessageBody) -> String {
    let mut text = body.text.clone();
    for part in &body.parts {
        let note = match part {
            MessagePart::Ui(ui) => ui
                .summary
                .as_deref()
                .map(|summary| format!("[UI: {summary}]")),
            MessagePart::Attachment(file) => Some(format!("[file: {}]", file.name)),
            MessagePart::Tool(_) => None,
        };
        if let Some(note) = note {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&note);
        }
    }
    text
}

fn clipped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn lines_of(window: &[inbox_entry::Model]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for entry in window {
        let Some(kind) = InboxEntryKind::parse(&entry.kind) else {
            continue;
        };
        if is_tombstone(&entry.envelope) {
            continue;
        }
        let Some(body) = plain_body(&entry.envelope) else {
            continue;
        };
        match kind {
            InboxEntryKind::Topic => lines.clear(),
            InboxEntryKind::Message => {
                if let Ok(parsed) = serde_json::from_value::<MessageBody>(body.clone())
                    && parsed.approval_response.is_none()
                {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: render(&parsed),
                    });
                }
            }
            InboxEntryKind::Trigger => {
                if let Ok(parsed) = TriggerBody::parse(body) {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: parsed.render(),
                    });
                }
            }
            InboxEntryKind::Edit => {
                let target = body_target(&entry.envelope);
                if let (Some(target), Ok(parsed)) =
                    (target, serde_json::from_value::<MessageBody>(body.clone()))
                    && let Some(line) = lines
                        .iter_mut()
                        .find(|line| line.message_id == target && line.sender_id == entry.sender_id)
                {
                    line.text = render(&parsed);
                }
            }
            _ => {}
        }
    }
    lines
}

/// Display names of people and bots, so the model can tell people apart.
pub(crate) async fn sender_names<C: ConnectionTrait>(
    db: &C,
    ids: &[&str],
) -> Result<HashMap<String, String>, DbErr> {
    let mut names = HashMap::new();
    let people = user::Entity::find()
        .filter(user::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?;
    for person in people {
        if let Some(name) = person
            .name
            .or(person.preferred_username)
            .or(person.username)
            .filter(|name| !name.trim().is_empty())
        {
            names.insert(person.id, name);
        }
    }
    let bots = bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?;
    for bot in bots {
        if let Some(name) = bot.name.filter(|name| !name.trim().is_empty()) {
            names.insert(bot.id, name);
        }
    }
    Ok(names)
}

/// The turn's transcript: the entries up to `trigger_message_id` in the bot's copy of the
/// conversation. In a group each message of someone else is marked as that person's, and says
/// whether the person is the bot's owner (`owner_id`).
pub(crate) async fn transcript<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    trigger_message_id: &str,
    kind: ConversationKind,
    owner_id: &str,
) -> Result<Transcript, HistoryError> {
    let own = |query: sea_orm::Select<inbox_entry::Entity>| {
        query
            .filter(inbox_entry::Column::InboxId.eq(bot_id))
            .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
    };
    let trigger = own(inbox_entry::Entity::find())
        .filter(inbox_entry::Column::MessageId.eq(trigger_message_id))
        .one(db)
        .await?
        .ok_or(HistoryError::TriggerMissing)?;
    let upto = match trigger.seq {
        Some(seq) => inbox_entry::Column::Seq.lte(seq),
        None => inbox_entry::Column::CreatedAt.lte(trigger.created_at),
    };
    let mut window = own(inbox_entry::Entity::find())
        .filter(upto)
        .order_by_desc(inbox_entry::Column::Seq)
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .order_by_desc(inbox_entry::Column::Id)
        .limit(HISTORY_ENTRIES)
        .all(db)
        .await?;
    window.reverse();
    let lines = lines_of(&window);
    let answered = match lines.last() {
        Some(line) if line.message_id == trigger_message_id && line.sender_id != bot_id => {
            line.clone()
        }
        _ => return Err(HistoryError::TriggerUnreadable),
    };
    let names = if kind == ConversationKind::Group {
        let ids: Vec<&str> = lines.iter().map(|line| line.sender_id.as_str()).collect();
        sender_names(db, &ids).await?
    } else {
        HashMap::new()
    };
    let spoken = |line: &Line| {
        if line.sender_id == bot_id || kind != ConversationKind::Group {
            return line.text.clone();
        }
        let name = names.get(&line.sender_id).unwrap_or(&line.sender_id);
        room_message(name, line.sender_id == owner_id, &line.text)
    };
    let mut history: Vec<ChatMessage> = lines
        .iter()
        .map(|line| ChatMessage {
            role: if line.sender_id == bot_id {
                ChatRole::Assistant
            } else {
                ChatRole::User
            },
            content: clipped(&spoken(line), MAX_HISTORY_MESSAGE_CHARS),
            images: None,
        })
        .collect();
    history.pop();
    Ok(Transcript {
        prompt: clipped(&spoken(&answered), MAX_PROMPT_CHARS),
        history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, kind: &str, sender: &str, body: serde_json::Value) -> inbox_entry::Model {
        inbox_entry::Model {
            id: format!("e-{id}"),
            inbox_id: "bot_a".into(),
            conversation_id: "c1".into(),
            message_id: id.into(),
            sender_id: sender.into(),
            kind: kind.to_ascii_uppercase(),
            envelope: json!({ "v": 1, "enc": "none", "body": body }),
            created_at: chrono::DateTime::from_timestamp(1_800_000_000, 0)
                .unwrap()
                .fixed_offset(),
            seq: None,
        }
    }

    #[test]
    fn edits_replace_text_topics_restart_and_tombstones_drop() {
        let mut gone = entry("m3", "message", "user-a", json!({ "text": "x" }));
        gone.envelope = flow_like_types::inbox::tombstone_envelope();
        let window = vec![
            entry("m0", "message", "user-a", json!({ "text": "old topic" })),
            entry("t1", "topic", "user-a", json!({})),
            entry("m1", "message", "user-a", json!({ "text": "first" })),
            entry("m2", "message", "bot_a", json!({ "text": "answer" })),
            entry(
                "ed",
                "edit",
                "user-a",
                json!({ "text": "first, fixed", "target": "m1" }),
            ),
            gone,
            entry("m4", "message", "user-a", json!({ "text": "last" })),
        ];
        let lines = lines_of(&window);
        let texts: Vec<_> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["first, fixed", "answer", "last"]);
    }

    #[test]
    fn another_senders_edit_changes_nothing_and_sealed_entries_are_skipped() {
        let mut sealed = entry("m2", "message", "user-b", json!({}));
        sealed.envelope = json!({ "v": 1, "enc": "x25519", "ct": "AAAA" });
        let window = vec![
            entry("m1", "message", "user-a", json!({ "text": "mine" })),
            entry(
                "ed",
                "edit",
                "user-b",
                json!({ "text": "forged", "target": "m1" }),
            ),
            sealed,
        ];
        let lines = lines_of(&window);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "mine");
    }

    #[test]
    fn ui_blocks_and_files_show_as_notes_and_tool_calls_stay_out() {
        let body: MessageBody = serde_json::from_value(json!({
            "text": "see",
            "parts": [
                { "kind": "ui", "id": "ui1", "summary": "a chart" },
                { "kind": "tool", "id": "t1", "call_id": "c", "name": "x" },
                { "kind": "attachment", "id": "a1", "name": "r.pdf",
                  "handle": { "app_id": "a", "path": "r.pdf", "bytes": 1, "sha256": "0", "mime": "application/pdf" } }
            ]
        }))
        .unwrap();
        assert_eq!(render(&body), "see\n[UI: a chart]\n[file: r.pdf]");
    }
}

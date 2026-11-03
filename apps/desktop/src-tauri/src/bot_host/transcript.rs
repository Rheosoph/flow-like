//! What a turn reads: the conversation up to the message it answers, as chat history. A "New
//! topic" marker starts the history over, edits replace their message's text, deleted messages
//! are left out.

use flow_like::{copilot::UnifiedChatMessage, flow::copilot::ChatRole};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::{MessageBody, MessagePart},
    inbox::{ConversationKind, InboxEntryKind, body_target, is_tombstone},
};
use std::collections::HashMap;

pub const HISTORY_ENTRIES: usize = 32;
const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_HISTORY_MESSAGE_CHARS: usize = 8_000;

#[derive(Debug, Clone)]
pub struct Transcript {
    pub prompt: String,
    pub history: Vec<UnifiedChatMessage>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Unreadable {
    Missing,
    Deleted,
}

struct Line {
    message_id: String,
    sender_id: String,
    text: String,
}

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

fn body_of(entry: &InboxEntry) -> Option<MessageBody> {
    super::rule::parse_body(entry)
}

fn lines_of(entries: &[InboxEntry]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for entry in entries {
        if is_tombstone(&entry.envelope) {
            continue;
        }
        match entry.kind {
            InboxEntryKind::Topic => lines.clear(),
            InboxEntryKind::Message | InboxEntryKind::Trigger => {
                if let Some(body) = body_of(entry) {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: render(&body),
                    });
                }
            }
            InboxEntryKind::Edit => {
                if let (Some(target), Some(body)) = (body_target(&entry.envelope), body_of(entry))
                    && let Some(line) = lines
                        .iter_mut()
                        .find(|line| line.message_id == target && line.sender_id == entry.sender_id)
                {
                    line.text = render(&body);
                }
            }
            _ => {}
        }
    }
    lines
}

fn clipped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The transcript of `entries` (oldest first) up to `trigger_message_id`, which must be the last
/// readable message. In a group each message of someone else starts with its sender's name.
pub fn transcript(
    bot_id: &str,
    kind: ConversationKind,
    trigger_message_id: &str,
    entries: &[InboxEntry],
    names: &HashMap<String, String>,
) -> Result<Transcript, Unreadable> {
    let upto = entries
        .iter()
        .position(|entry| entry.message_id == trigger_message_id)
        .ok_or(Unreadable::Missing)?;
    let start = (upto + 1).saturating_sub(HISTORY_ENTRIES);
    let lines = lines_of(&entries[start..=upto]);
    let answered = match lines.last() {
        Some(line) if line.message_id == trigger_message_id && line.sender_id != bot_id => line,
        _ => return Err(Unreadable::Deleted),
    };
    let spoken = |line: &Line| {
        if line.sender_id == bot_id || kind != ConversationKind::Group {
            return line.text.clone();
        }
        let name = names.get(&line.sender_id).unwrap_or(&line.sender_id);
        format!("{name}: {}", line.text)
    };
    let prompt = clipped(&spoken(answered), MAX_PROMPT_CHARS);
    let history = lines[..lines.len() - 1]
        .iter()
        .map(|line| UnifiedChatMessage {
            role: if line.sender_id == bot_id {
                ChatRole::Assistant
            } else {
                ChatRole::User
            },
            content: clipped(&spoken(line), MAX_HISTORY_MESSAGE_CHARS),
            images: None,
        })
        .collect();
    Ok(Transcript { prompt, history })
}

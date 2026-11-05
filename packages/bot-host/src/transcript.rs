//! What a turn reads: the conversation up to the message it answers, as chat history. A "New
//! topic" marker starts the history over, edits replace their message's text, deleted messages
//! are left out.

use flow_like::{
    copilot::UnifiedChatMessage,
    flow::copilot::{ChatRole, ReactionNote},
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::{MessageBody, MessagePart},
    bot_turn::room_message,
    inbox::{
        ConversationKind, InboxEntryKind, ReactionEvent, TranscriptBody, body_target, is_tombstone,
        net_reactions,
    },
};
use std::collections::HashMap;

pub const HISTORY_ENTRIES: usize = 32;
const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_HISTORY_MESSAGE_CHARS: usize = 8_000;

#[derive(Debug, Clone)]
pub struct Transcript {
    pub prompt: String,
    pub history: Vec<UnifiedChatMessage>,
    /// Something since the bot's last message names it or replies to it, or the conversation is
    /// direct; otherwise the bot reads along.
    pub addressed: bool,
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
    mentions: Vec<String>,
    reply_to: Option<String>,
}

fn addressed_to(lines: &[Line], bot_id: &str) -> bool {
    let since = lines
        .iter()
        .rposition(|line| line.sender_id == bot_id)
        .map_or(0, |last| last + 1);
    lines[since..].iter().any(|line| {
        line.mentions.iter().any(|mention| mention == bot_id)
            || line.reply_to.as_deref().is_some_and(|target| {
                lines
                    .iter()
                    .any(|own| own.message_id == target && own.sender_id == bot_id)
            })
    })
}

fn render(body: &MessageBody) -> String {
    let mut text = body.text.clone();
    for part in &body.parts {
        let note = match part {
            MessagePart::Ui(ui) => ui
                .summary
                .as_deref()
                .map(|summary| format!("[UI: {summary}]")),
            MessagePart::Attachment(file) => Some(file.note()),
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
    crate::rule::parse_body(entry)
}

fn spoken_of(entry: &InboxEntry) -> Option<TranscriptBody> {
    let body = entry
        .envelope
        .get("body")
        .filter(|_| crate::rule::is_plain(entry))?;
    TranscriptBody::parse(body).ok()
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
                        mentions: body.mentions.clone(),
                        reply_to: body.reply_to.clone(),
                    });
                }
            }
            InboxEntryKind::Transcript => {
                if let Some(spoken) = spoken_of(entry) {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: spoken.render(),
                        mentions: spoken.mentions,
                        reply_to: None,
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
/// readable message. In a group each message of someone else is marked as that person's, and whether they are `owner_id`.
pub fn transcript(
    bot_id: &str,
    kind: ConversationKind,
    trigger_message_id: &str,
    entries: &[InboxEntry],
    names: &HashMap<String, String>,
    owner_id: &str,
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
        room_message(name, line.sender_id == owner_id, &line.text)
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
    let addressed = kind != ConversationKind::Group || addressed_to(&lines, bot_id);
    Ok(Transcript {
        prompt,
        history,
        addressed,
    })
}

const EXCERPT_CHARS: usize = 120;

/// What a turn woken by reactions reads, and the reactions it answers.
#[derive(Debug, Clone)]
pub struct ReactionTurn {
    pub transcript: Transcript,
    pub reactions: Vec<ReactionNote>,
}

/// The reactions in place among `answered` (entry ids) on messages `bot_id` sent, from `entries`
/// (oldest first, the newest window), with the conversation as history. A reaction taken back
/// later counts as gone even if it came after the turn's trigger. New messages since the bot's
/// last one are the prompt, and when they address the bot it answers them rather than the
/// reactions.
pub fn reaction_turn(
    bot_id: &str,
    kind: ConversationKind,
    answered: &[String],
    entries: &[InboxEntry],
    names: &HashMap<String, String>,
    owner_id: &str,
) -> ReactionTurn {
    let start = entries.len().saturating_sub(HISTORY_ENTRIES);
    let window = &entries[start..];
    let lines = lines_of(window);
    let events: Vec<ReactionEvent> = window
        .iter()
        .filter(|entry| {
            entry.kind == InboxEntryKind::Reaction
                && crate::rule::is_plain(entry)
                && !is_tombstone(&entry.envelope)
        })
        .filter_map(|entry| {
            ReactionEvent::parse(&entry.message_id, &entry.sender_id, &entry.envelope)
        })
        .collect();
    let named = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let reactions: Vec<ReactionNote> = net_reactions(&events)
        .into_iter()
        .filter(|event| answered.contains(&event.message_id))
        .filter_map(|event| {
            let own = lines
                .iter()
                .find(|line| line.message_id == event.target && line.sender_id == bot_id)?;
            Some(ReactionNote {
                by: named(&event.sender_id),
                emoji: event.emoji,
                excerpt: clipped(&own.text.replace('\n', " "), EXCERPT_CHARS),
            })
        })
        .collect();
    let fresh = lines
        .iter()
        .rposition(|line| line.sender_id == bot_id)
        .map_or(lines.len(), |last| lines.len() - last - 1);
    let addressed = fresh == 0 || kind != ConversationKind::Group || addressed_to(&lines, bot_id);
    let spoken = |line: &Line| {
        if line.sender_id == bot_id || kind != ConversationKind::Group {
            return line.text.clone();
        }
        room_message(
            &named(&line.sender_id),
            line.sender_id == owner_id,
            &line.text,
        )
    };
    let as_history = |line: &Line| UnifiedChatMessage {
        role: if line.sender_id == bot_id {
            ChatRole::Assistant
        } else {
            ChatRole::User
        },
        content: clipped(&spoken(line), MAX_HISTORY_MESSAGE_CHARS),
        images: None,
    };
    let reaction_text = reactions
        .iter()
        .map(|note| {
            format!(
                "{} reacted {} to your message: \"{}\"",
                note.by, note.emoji, note.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (prompt, history, reactions) = match lines.split_last() {
        Some((last, earlier)) if fresh > 0 => {
            let prompt = if reaction_text.is_empty() {
                spoken(last)
            } else {
                format!("{}\n\n{reaction_text}", spoken(last))
            };
            let kept = if addressed { Vec::new() } else { reactions };
            (
                clipped(&prompt, MAX_PROMPT_CHARS),
                earlier.iter().map(as_history).collect(),
                kept,
            )
        }
        _ => (
            reaction_text,
            lines.iter().map(as_history).collect(),
            reactions,
        ),
    };
    ReactionTurn {
        transcript: Transcript {
            prompt,
            history,
            addressed,
        },
        reactions,
    }
}

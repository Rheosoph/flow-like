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
    bot_tool_trail::{TrailEntry, TrailSection, trail_of},
    bot_turn::room_message,
    bot_user_context::{ContextHistory, ContextLine},
    inbox::{
        ConversationKind, InboxEntryKind, InterruptedSpeech, ReactionEvent, TranscriptBody,
        body_target, is_tombstone, net_reactions,
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
    /// Epoch milliseconds of the newest "New topic" marker in the conversation, if any.
    pub topic_at: Option<i64>,
}

/// When the newest live "New topic" marker of `entries` was written.
pub fn topic_at(entries: &[InboxEntry]) -> Option<i64> {
    entries
        .iter()
        .filter(|entry| entry.kind == InboxEntryKind::Topic && !is_tombstone(&entry.envelope))
        .map(|entry| entry.created_at.timestamp_millis())
        .max()
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
    /// A scheduled or webhook trigger: it always addresses the bot.
    from_trigger: bool,
    /// A context message shown as a value-free note: edits never restore its text.
    withheld: bool,
}

fn addressed_to(lines: &[Line], bot_id: &str) -> bool {
    let since = lines
        .iter()
        .rposition(|line| line.sender_id == bot_id)
        .map_or(0, |last| last + 1);
    lines[since..].iter().any(|line| {
        line.from_trigger
            || line.mentions.iter().any(|mention| mention == bot_id)
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
    if let Some(note) = &body.note {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&note.line());
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

fn readable_bodies(entries: &[InboxEntry]) -> impl Iterator<Item = (&str, &serde_json::Value)> {
    entries
        .iter()
        .filter(|entry| !is_tombstone(&entry.envelope) && crate::rule::is_plain(entry))
        .map(|entry| (entry.sender_id.as_str(), &entry.envelope["body"]))
}

/// What `bot_id` knows of the context shared in `window` (oldest first); of the `later` entries,
/// written after the turn's trigger, only revocations count.
fn context_of(window: &[InboxEntry], later: &[InboxEntry], bot_id: &str) -> ContextHistory {
    ContextHistory::with_later(
        readable_bodies(window),
        readable_bodies(later),
        bot_id,
        chrono::Utc::now().timestamp_millis(),
    )
}

/// The session snapshots still usable in `entries` (oldest first), for a turn whose history
/// comes from memory and so leaves them out. `entries` are all the entries, oldest first; the
/// window ends at `trigger_message_id`, and a revocation after it still ends a session.
pub fn shared_context(
    entries: &[InboxEntry],
    trigger_message_id: &str,
    bot_id: &str,
) -> Option<String> {
    let upto = entries
        .iter()
        .position(|entry| entry.message_id == trigger_message_id)
        .map_or(entries.len(), |index| index + 1);
    let start = upto.saturating_sub(HISTORY_ENTRIES);
    context_of(&entries[start..upto], &entries[upto..], bot_id).live_note()
}

fn lines_of(entries: &[InboxEntry], later: &[InboxEntry], bot_id: &str) -> Vec<Line> {
    let live = || {
        entries
            .iter()
            .filter(|entry| !is_tombstone(&entry.envelope))
    };
    let context = context_of(entries, later, bot_id);
    let mut lines: Vec<Line> = Vec::new();
    let mut cut_off: Vec<InterruptedSpeech> = Vec::new();
    for entry in live() {
        let context = match context.line(&entry.sender_id, &entry.envelope["body"]) {
            ContextLine::Hidden => continue,
            ContextLine::Text(text) => Some(text),
            ContextLine::Ordinary => None,
        };
        match (entry.kind, context) {
            (InboxEntryKind::Topic, _) => lines.clear(),
            (InboxEntryKind::Message, Some(text)) => lines.push(Line {
                message_id: entry.message_id.clone(),
                sender_id: entry.sender_id.clone(),
                text,
                mentions: Vec::new(),
                reply_to: None,
                from_trigger: false,
                withheld: true,
            }),
            (InboxEntryKind::Message | InboxEntryKind::Trigger, None) => {
                if let Some(body) = body_of(entry)
                    && body.approval_response.is_none()
                {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: render(&body),
                        mentions: body.mentions.clone(),
                        reply_to: body.reply_to.clone(),
                        from_trigger: entry.kind == InboxEntryKind::Trigger,
                        withheld: false,
                    });
                }
            }
            (InboxEntryKind::Transcript, None) => {
                if let Some(spoken) = spoken_of(entry) {
                    cut_off.extend(spoken.interrupted.clone());
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: spoken.render(),
                        mentions: spoken.mentions,
                        reply_to: None,
                        from_trigger: false,
                        withheld: false,
                    });
                }
            }
            (InboxEntryKind::Edit, None) => {
                if let (Some(target), Some(body)) = (body_target(&entry.envelope), body_of(entry))
                    && let Some(line) = lines.iter_mut().find(|line| {
                        line.message_id == target
                            && line.sender_id == entry.sender_id
                            && !line.withheld
                    })
                {
                    line.text = render(&body);
                }
            }
            _ => {}
        }
    }
    for cut in &cut_off {
        if let Some(line) = lines
            .iter_mut()
            .find(|line| line.message_id == cut.message_id && !line.withheld)
        {
            line.text = cut.heard_part(&line.text);
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
    let lines = lines_of(&entries[start..=upto], &entries[upto + 1..], bot_id);
    if let Some(prompt) = flow_like_types::bot_user_context::response_prompt(
        &entries[upto].envelope["body"],
        bot_id,
        &entries[upto].sender_id,
        chrono::Utc::now().timestamp_millis(),
    ) {
        return Ok(Transcript {
            prompt,
            history: render_history(
                bot_id,
                kind,
                &entries[start..upto],
                names,
                owner_id,
                HISTORY_ENTRIES,
            ),
            addressed: true,
            topic_at: topic_at(&entries[..=upto]),
        });
    }
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
        topic_at: topic_at(&entries[..=upto]),
    })
}

/// The conversation a delayed answer continues, including the bot's latest reply. Entries are
/// oldest first; edits, deletions and topic boundaries follow the normal turn's history rules.
pub(crate) fn continuation_history(
    bot_id: &str,
    kind: ConversationKind,
    entries: &[InboxEntry],
    names: &HashMap<String, String>,
    owner_id: &str,
) -> Vec<UnifiedChatMessage> {
    render_history(bot_id, kind, entries, names, owner_id, HISTORY_ENTRIES)
}

/// The latest `limit` spoken lines from `entries`, oldest first. Topic boundaries, edits and
/// tombstones are applied before the limit, so activity entries do not displace messages.
pub fn render_history(
    bot_id: &str,
    kind: ConversationKind,
    entries: &[InboxEntry],
    names: &HashMap<String, String>,
    owner_id: &str,
    limit: usize,
) -> Vec<UnifiedChatMessage> {
    let lines = lines_of(entries, &[], bot_id);
    let start = lines.len().saturating_sub(limit);
    lines
        .into_iter()
        .skip(start)
        .map(|line| {
            let own = line.sender_id == bot_id;
            let content = if own || kind != ConversationKind::Group {
                line.text
            } else {
                room_message(
                    names.get(&line.sender_id).unwrap_or(&line.sender_id),
                    line.sender_id == owner_id,
                    &line.text,
                )
            };
            UnifiedChatMessage {
                role: if own {
                    ChatRole::Assistant
                } else {
                    ChatRole::User
                },
                content: clipped(&content, MAX_HISTORY_MESSAGE_CHARS),
                images: None,
            }
        })
        .collect()
}

/// The trail section of `bot_id`'s replies in `entries` (oldest first) since the newest "New
/// topic" marker: their tool calls, each with the message it answered.
pub fn tool_trail(bot_id: &str, entries: &[InboxEntry]) -> Option<TrailSection> {
    trail_of(
        bot_id,
        entries.iter().map(|entry| TrailEntry {
            message_id: &entry.message_id,
            sender_id: &entry.sender_id,
            kind: entry.kind,
            envelope: &entry.envelope,
        }),
    )
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
    let lines = lines_of(window, &[], bot_id);
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
            topic_at: topic_at(entries),
        },
        reactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::inbox::{plain_envelope, tombstone_envelope};
    use serde_json::{Value, json};

    fn entry(id: &str, sender: &str, kind: InboxEntryKind, body: Value) -> InboxEntry {
        InboxEntry {
            id: id.to_string(),
            message_id: id.to_string(),
            conversation_id: "conversation".to_string(),
            sender_id: sender.to_string(),
            kind,
            envelope: plain_envelope(body),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z").unwrap(),
            seq: None,
            cursor: String::new(),
        }
    }

    #[test]
    fn continuation_limits_spoken_lines_after_ignoring_trailing_activity() {
        let mut entries: Vec<_> = (0..40)
            .map(|index| {
                entry(
                    &format!("message-{index}"),
                    if index % 2 == 0 { "owner" } else { "bot" },
                    InboxEntryKind::Message,
                    json!({ "text": format!("Message {index}") }),
                )
            })
            .collect();
        entries.extend((0..40).map(|index| {
            entry(
                &format!("activity-{index}"),
                "bot",
                InboxEntryKind::Activity,
                json!({}),
            )
        }));

        let history = continuation_history(
            "bot",
            ConversationKind::Direct,
            &entries,
            &HashMap::new(),
            "owner",
        );

        assert_eq!(history.len(), 32);
        for (message, index) in history.iter().zip(8..40) {
            assert_eq!(message.content, format!("Message {index}"));
            assert_eq!(matches!(message.role, ChatRole::Assistant), index % 2 == 1);
        }
    }

    fn shared_entries(decision: &str) -> Vec<InboxEntry> {
        let now = chrono::Utc::now().timestamp_millis();
        let request = json!({
            "id": "request", "bot_id": "bot", "subject_id": "local", "conversation_id": "conversation",
            "purpose": "Find nearby places", "fields": ["location"], "precision": "approximate",
            "audience": "direct", "audience_member_ids": ["bot", "local"],
            "created_at": now - 120_000, "expires_at": now + 180_000,
        });
        let response = json!({
            "request_id": "request", "bot_id": "bot", "subject_id": "local",
            "conversation_id": "conversation", "decision": decision,
            "captured_at": now - 60_000, "expires_at": now + 240_000,
            "source": {"session_id": "s1", "platform": "web"},
            "values": {"location": {"latitude": 48.17, "longitude": 11.6, "accuracy_m": 1500, "precision": "approximate"}},
            "unavailable": {},
        });
        vec![
            entry(
                "normal",
                "local",
                InboxEntryKind::Message,
                json!({"text":"Find nearby places"}),
            ),
            entry(
                "request",
                "bot",
                InboxEntryKind::Message,
                json!({"text":"Location request", "user_context_request": request}),
            ),
            entry(
                "answer",
                "local",
                InboxEntryKind::Message,
                json!({"text":"Shared", "user_context_response": response}),
            ),
            entry(
                "reply",
                "bot",
                InboxEntryKind::Message,
                json!({"text":"Sensitive nearby answer", "user_context_derived":"request"}),
            ),
        ]
    }

    fn history_of(entries: &[InboxEntry]) -> Vec<String> {
        render_history(
            "bot",
            ConversationKind::Direct,
            entries,
            &HashMap::new(),
            "local",
            32,
        )
        .into_iter()
        .map(|message| message.content)
        .collect()
    }

    #[test]
    fn values_shared_once_leave_only_value_free_notes_in_later_history() {
        let mut entries = shared_entries("once");
        entries.push(entry(
            "edit",
            "bot",
            InboxEntryKind::Edit,
            json!({"target":"reply","text":"Sensitive edited answer"}),
        ));
        entries.push(entry(
            "malformed",
            "bot",
            InboxEntryKind::Message,
            json!({"text":"Sensitive card", "user_context_request":{"id":"malformed"}}),
        ));
        let history = history_of(&entries);
        assert_eq!(history[0], "Find nearby places");
        assert_eq!(history[1], "[You asked for user context (location).]");
        assert!(history[2].contains("(location) once"));
        assert!(history[3].contains("withheld"));
        assert_eq!(history.len(), 4);
        assert!(
            history
                .iter()
                .all(|line| !line.contains("Sensitive") && !line.contains("48.17"))
        );
    }

    #[test]
    fn a_session_answer_keeps_the_snapshot_and_the_reply_in_later_turns() {
        let history = history_of(&shared_entries("session"));
        assert!(history[2].contains("for this session"));
        assert!(history[2].contains("48.17"));
        assert_eq!(history[3], "Sensitive nearby answer");
        assert!(
            shared_context(&shared_entries("session"), "reply", "bot")
                .is_some_and(|note| note.contains("48.17"))
        );
        assert!(shared_context(&shared_entries("once"), "reply", "bot").is_none());
        let mut group = shared_entries("session");
        group[1].envelope["body"]["user_context_request"]["audience"] = json!("group");
        let group = render_history(
            "bot",
            ConversationKind::Group,
            &group,
            &HashMap::new(),
            "local",
            32,
        );
        assert!(group.iter().all(|line| !line.content.contains("48.17")));
    }

    #[test]
    fn a_revocation_after_the_trigger_ends_the_shared_session() {
        let mut entries = shared_entries("session");
        entries.push(entry(
            "revoked",
            "local",
            InboxEntryKind::Message,
            json!({"text":"Revoked", "user_context_revoke": {
                "request_id": "request", "bot_id": "bot", "subject_id": "local",
                "conversation_id": "conversation",
                "revoked_at": chrono::Utc::now().timestamp_millis(),
            }}),
        ));
        assert!(shared_context(&entries, "reply", "bot").is_none());
        assert!(shared_context(&entries, "revoked", "bot").is_none());
        let before = &entries[..entries.len() - 1];
        assert!(shared_context(before, "reply", "bot").is_some());
    }

    #[test]
    fn approval_answers_are_no_lines_and_triggers_address_the_bot() {
        let entries = vec![
            entry(
                "ask",
                "guest",
                InboxEntryKind::Message,
                json!({"text":"Please run it"}),
            ),
            entry(
                "approved",
                "guest",
                InboxEntryKind::Message,
                json!({"text":"Approved", "approval_response": {
                    "approval_id": "a1", "decision": "once",
                }}),
            ),
        ];
        let lines = lines_of(&entries, &[], "bot");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].message_id, "ask");

        let trigger = entry(
            "tick",
            "system",
            InboxEntryKind::Trigger,
            json!({"source":"schedule","ref_id":"task","title":"Daily","text":"Check the report"}),
        );
        let lines = lines_of(&[trigger], &[], "bot");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].from_trigger);
        assert!(addressed_to(&lines, "bot"));
    }

    #[test]
    fn user_context_wake_rule_targets_only_the_requesting_bot() {
        let card = entry(
            "request",
            "bot",
            InboxEntryKind::Message,
            json!({"text":"Share context", "user_context_request":{"id":"request"}, "mentions":["other-bot"]}),
        );
        assert!(
            crate::rule::waiting(
                ConversationKind::Group,
                "other-bot",
                &[card.clone()],
                card.created_at,
                false
            )
            .is_none()
        );
        let answer = entry(
            "answer",
            "local",
            InboxEntryKind::Message,
            json!({"text":"Shared context", "user_context_response":{"bot_id":"bot"}, "mentions":["bot"]}),
        );
        assert!(
            crate::rule::waiting(
                ConversationKind::Group,
                "bot",
                &[answer.clone()],
                answer.created_at,
                false
            )
            .is_some()
        );
        assert!(
            crate::rule::waiting(
                ConversationKind::Group,
                "other-bot",
                &[answer.clone()],
                answer.created_at,
                false
            )
            .is_none()
        );
        let reply = entry(
            "reply",
            "bot",
            InboxEntryKind::Message,
            json!({"text":"Context answer", "user_context_derived":"request", "mentions":["other-bot"]}),
        );
        assert!(
            crate::rule::waiting(
                ConversationKind::Group,
                "other-bot",
                &[reply.clone()],
                reply.created_at,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn another_bots_linked_file_reaches_the_room_with_its_whole_link() {
        let link = "https://vault.s3.eu-central-1.amazonaws.com/offers/Offer.pdf?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=abc123";
        let entries = vec![entry(
            "made",
            "maker",
            InboxEntryKind::Message,
            json!({
                "text": "The offer is ready.",
                "parts": [{ "kind": "attachment", "id": "a1", "name": "Offer.pdf", "url": link, "mime": "application/pdf" }]
            }),
        )];
        let names = HashMap::from([("maker".to_string(), "Maker".to_string())]);
        let note = format!("[file: Offer.pdf (link: {link})]");

        let turn = transcript(
            "reader",
            ConversationKind::Group,
            "made",
            &entries,
            &names,
            "owner",
        )
        .unwrap();
        assert!(turn.prompt.contains(&note), "{}", turn.prompt);

        let history = render_history("other", ConversationKind::Group, &entries, &names, "owner", 8);
        assert!(history[0].content.contains(&note), "{}", history[0].content);
    }

    #[test]
    fn rendered_history_applies_topics_edits_and_tombstones_before_its_limit() {
        let mut deleted = entry(
            "deleted",
            "guest",
            InboxEntryKind::Message,
            json!({ "text": "Removed text" }),
        );
        deleted.envelope = tombstone_envelope();
        let entries = vec![
            entry(
                "old",
                "owner",
                InboxEntryKind::Message,
                json!({ "text": "Old topic" }),
            ),
            entry("topic", "owner", InboxEntryKind::Topic, json!({})),
            entry(
                "question",
                "guest",
                InboxEntryKind::Message,
                json!({ "text": "Old question" }),
            ),
            entry(
                "answer",
                "bot",
                InboxEntryKind::Message,
                json!({ "text": "I am checking." }),
            ),
            entry(
                "edit",
                "guest",
                InboxEntryKind::Edit,
                json!({ "target": "question", "text": "Corrected question" }),
            ),
            deleted,
        ];
        let names = HashMap::from([("guest".to_string(), "Guest".to_string())]);

        let history = render_history("bot", ConversationKind::Group, &entries, &names, "owner", 8);

        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0].content,
            room_message("Guest", false, "Corrected question")
        );
        assert!(matches!(history[0].role, ChatRole::User));
        assert_eq!(history[1].content, "I am checking.");
        assert!(matches!(history[1].role, ChatRole::Assistant));
    }
}

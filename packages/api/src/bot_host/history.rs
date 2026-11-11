//! What a headless turn reads: the bot's own copy of the conversation, its last entries up to
//! the message the turn answers, as chat history. A "New topic" marker starts the history over;
//! edits replace their message's text; deleted and sealed messages are left out.

use crate::{
    entity::{bot, inbox_entry, user},
    error::ApiError,
};
use flow_like::flow::copilot::{ChatMessage, ChatRole, ReactionNote};
use flow_like_types::{
    bot_message::{MessageBody, MessagePart},
    bot_tool_trail::{TrailEntry, TrailSection, trail_of},
    bot_turn::{clip_keeping_tail, room_message},
    bot_user_context::{ContextHistory, ContextLine},
    inbox::{
        ConversationKind, InboxEntryKind, PLAIN_ENCRYPTION, ReactionEvent, TranscriptBody,
        TriggerBody, body_target, is_tombstone, net_reactions,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use std::collections::HashMap;

/// Entries of the bot's copy a turn reads, the answered message included.
pub(crate) const HISTORY_ENTRIES: u64 = 32;
/// Entries after the trigger read for revocations only.
const LATER_ENTRIES: u64 = 16;
/// The entry kinds that make up what the model reads; reactions, member and activity entries
/// do not take a place in the window.
const SPOKEN_KINDS: [&str; 5] = ["MESSAGE", "TRIGGER", "TRANSCRIPT", "EDIT", "TOPIC"];
pub(crate) const MAX_PROMPT_CHARS: usize = 20_000;
pub(crate) const MAX_HISTORY_MESSAGE_CHARS: usize = 8_000;

/// The message to answer and what came before it.
#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    pub prompt: String,
    pub history: Vec<ChatMessage>,
    /// Something since the bot's last message names it or replies to it, or the conversation is
    /// direct; otherwise the bot reads along.
    pub addressed: bool,
    /// Epoch milliseconds of the latest "New topic" marker in the entries read, when there is one.
    pub topic_at: Option<i64>,
    /// The tool calls of the bot's replies in the entries read, as the trail section shows them.
    pub trail: Option<TrailSection>,
    /// What the entries read put into the turn's taint.
    pub reads: flow_like::flow::copilot::TurnReads,
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

impl From<ApiError> for HistoryError {
    fn from(error: ApiError) -> Self {
        Self::Db(error)
    }
}

/// One message as the model reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    message_id: String,
    sender_id: String,
    text: String,
    /// How many characters at the end of `text` are file and UI notes, which a cut keeps whole.
    keep: usize,
    mentions: Vec<String>,
    reply_to: Option<String>,
    from_trigger: bool,
    /// A context message shown as a value-free note: edits never restore its text.
    withheld: bool,
}

/// Whether a line since the bot's own last line names the bot, replies to one of its lines, or
/// is a trigger for it.
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

pub(super) fn plain_body(envelope: &serde_json::Value) -> Option<&serde_json::Value> {
    (envelope.get("enc").and_then(serde_json::Value::as_str) == Some(PLAIN_ENCRYPTION))
        .then(|| envelope.get("body"))
        .flatten()
}

/// The message's text with a line for each UI block and file it carries.
pub(super) fn render(body: &MessageBody) -> String {
    rendered(body).0
}

/// [`render`], and how many characters at its end the UI and file lines take.
fn rendered(body: &MessageBody) -> (String, usize) {
    let mut text = body.text.clone();
    let written = text.chars().count();
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
    let keep = text.chars().count() - written;
    (text, keep)
}

/// When the latest "New topic" marker among `window` was written.
fn topic_at_of(window: &[inbox_entry::Model]) -> Option<i64> {
    window
        .iter()
        .filter(|entry| {
            InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Topic)
                && !is_tombstone(&entry.envelope)
                && plain_body(&entry.envelope).is_some()
        })
        .map(|entry| entry.created_at.timestamp_millis())
        .max()
}

fn clipped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn readable(
    window: &[inbox_entry::Model],
) -> impl Iterator<Item = (&inbox_entry::Model, InboxEntryKind, &serde_json::Value)> {
    window.iter().filter_map(|entry| {
        let kind = InboxEntryKind::parse(&entry.kind)?;
        let body = plain_body(&entry.envelope).filter(|_| !is_tombstone(&entry.envelope))?;
        Some((entry, kind, body))
    })
}

fn context_of(
    window: &[inbox_entry::Model],
    later: &[inbox_entry::Model],
    bot_id: &str,
) -> ContextHistory {
    ContextHistory::with_later(
        readable(window).map(|(entry, _, body)| (entry.sender_id.as_str(), body)),
        readable(later).map(|(entry, _, body)| (entry.sender_id.as_str(), body)),
        bot_id,
        chrono::Utc::now().timestamp_millis(),
    )
}

async fn recent_messages<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<Vec<inbox_entry::Model>, DbErr> {
    let mut window = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Kind.eq("MESSAGE"))
        .order_by_desc(inbox_entry::Column::Seq)
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .order_by_desc(inbox_entry::Column::Id)
        .limit(HISTORY_ENTRIES)
        .all(db)
        .await?;
    window.reverse();
    Ok(window)
}

/// The session snapshots still usable in the bot's copy of the conversation, for a turn whose
/// history comes from memory and so leaves them out.
pub(crate) async fn shared_context<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<Option<String>, DbErr> {
    let window = recent_messages(db, bot_id, conversation_id).await?;
    Ok(context_of(&window, &[], bot_id).live_note())
}

/// The request id of the most recent session snapshot still live in the bot's copy of the
/// conversation.
pub(crate) async fn live_request_id<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<Option<String>, DbErr> {
    let window = recent_messages(db, bot_id, conversation_id).await?;
    Ok(context_of(&window, &[], bot_id)
        .live_request_id()
        .map(str::to_owned))
}

fn lines_of(
    window: &[inbox_entry::Model],
    later: &[inbox_entry::Model],
    bot_id: &str,
) -> Vec<Line> {
    let context = context_of(window, later, bot_id);
    let mut lines: Vec<Line> = Vec::new();
    for (entry, kind, body) in readable(window) {
        let context = match context.line(&entry.sender_id, body) {
            ContextLine::Hidden => continue,
            ContextLine::Text(text) => Some(text),
            ContextLine::Ordinary => None,
        };
        match (kind, context) {
            (InboxEntryKind::Topic, _) => lines.clear(),
            (InboxEntryKind::Message, Some(text)) => lines.push(Line {
                message_id: entry.message_id.clone(),
                sender_id: entry.sender_id.clone(),
                text,
                keep: 0,
                mentions: Vec::new(),
                reply_to: None,
                from_trigger: false,
                withheld: true,
            }),
            (InboxEntryKind::Message, None) => {
                if let Ok(parsed) = serde_json::from_value::<MessageBody>(body.clone())
                    && parsed.approval_response.is_none()
                {
                    let (text, keep) = rendered(&parsed);
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text,
                        keep,
                        mentions: parsed.mentions.clone(),
                        reply_to: parsed.reply_to.clone(),
                        from_trigger: false,
                        withheld: false,
                    });
                }
            }
            (InboxEntryKind::Trigger, None) => {
                if let Ok(parsed) = TriggerBody::parse(body) {
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: parsed.render(),
                        keep: 0,
                        mentions: Vec::new(),
                        reply_to: None,
                        from_trigger: true,
                        withheld: false,
                    });
                }
            }
            (InboxEntryKind::Transcript, None) => {
                if let Ok(parsed) = TranscriptBody::parse(body) {
                    if let Some(interrupted) = &parsed.interrupted
                        && let Some(cut) = lines.iter_mut().find(|line| {
                            line.message_id == interrupted.message_id && !line.withheld
                        })
                    {
                        cut.text = interrupted.heard_part(&cut.text);
                        cut.keep = 0;
                    }
                    lines.push(Line {
                        message_id: entry.message_id.clone(),
                        sender_id: entry.sender_id.clone(),
                        text: parsed.render(),
                        keep: 0,
                        mentions: parsed.mentions,
                        reply_to: None,
                        from_trigger: false,
                        withheld: false,
                    });
                }
            }
            (InboxEntryKind::Edit, None) => {
                let target = body_target(&entry.envelope);
                if let (Some(target), Ok(parsed)) =
                    (target, serde_json::from_value::<MessageBody>(body.clone()))
                    && let Some(line) = lines.iter_mut().find(|line| {
                        line.message_id == target
                            && line.sender_id == entry.sender_id
                            && !line.withheld
                    })
                {
                    (line.text, line.keep) = rendered(&parsed);
                }
            }
            _ => {}
        }
    }
    lines
}

/// Display names of people and bots, so the model can tell people apart. A person is named
/// as every member of a conversation may see them: never by login name or address.
pub(crate) async fn sender_names<C: ConnectionTrait>(
    db: &C,
    ids: &[&str],
) -> Result<HashMap<String, String>, DbErr> {
    use crate::routes::user::identity::public_display_name;
    let mut names = HashMap::new();
    if ids.is_empty() {
        return Ok(names);
    }
    let people = user::Entity::find()
        .select_only()
        .columns([
            user::Column::Id,
            user::Column::Name,
            user::Column::PreferredUsername,
            user::Column::Email,
        ])
        .filter(user::Column::Id.is_in(ids.iter().copied()))
        .into_tuple::<(String, Option<String>, Option<String>, Option<String>)>()
        .all(db)
        .await?;
    for (id, name, handle, email) in people {
        if let Some(name) =
            public_display_name(name.as_deref(), handle.as_deref(), email.as_deref())
        {
            names.insert(id, name);
        }
    }
    let bots = bot::Entity::find()
        .select_only()
        .columns([bot::Column::Id, bot::Column::Name, bot::Column::SystemKey])
        .filter(bot::Column::Id.is_in(ids.iter().copied()))
        .into_tuple::<(String, Option<String>, Option<String>)>()
        .all(db)
        .await?;
    for (id, name, system) in bots {
        let name = name.filter(|name| !name.trim().is_empty()).or_else(|| {
            use flow_like_types::bot::BotSystemKind;
            system
                .as_deref()
                .and_then(BotSystemKind::parse)
                .map(|kind| match kind {
                    BotSystemKind::General => "Assistant".to_owned(),
                    BotSystemKind::Flux => "Flux".to_owned(),
                })
        });
        if let Some(name) = name {
            names.insert(id, name);
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
    let (upto, after) = match trigger.seq {
        Some(seq) => (
            inbox_entry::Column::Seq.lte(seq),
            inbox_entry::Column::Seq.gt(seq),
        ),
        None => (
            inbox_entry::Column::CreatedAt.lte(trigger.created_at),
            inbox_entry::Column::CreatedAt.gt(trigger.created_at),
        ),
    };
    let later = own(inbox_entry::Entity::find())
        .filter(after)
        .filter(inbox_entry::Column::Kind.eq("MESSAGE"))
        .order_by_asc(inbox_entry::Column::Seq)
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .order_by_asc(inbox_entry::Column::Id)
        .limit(LATER_ENTRIES)
        .all(db)
        .await?;
    let mut window = own(inbox_entry::Entity::find())
        .filter(upto)
        .filter(inbox_entry::Column::Kind.is_in(SPOKEN_KINDS))
        .order_by_desc(inbox_entry::Column::Seq)
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .order_by_desc(inbox_entry::Column::Id)
        .limit(HISTORY_ENTRIES)
        .all(db)
        .await?;
    window.reverse();
    let mut lines = lines_of(&window, &later, bot_id);
    let context_prompt = flow_like_types::bot_user_context::response_prompt(
        &trigger.envelope["body"],
        bot_id,
        &trigger.sender_id,
        chrono::Utc::now().timestamp_millis(),
    );
    if context_prompt.is_some() {
        lines.retain(|line| line.message_id != trigger_message_id);
    }
    let answered = if let Some(prompt) = context_prompt.as_ref() {
        Line {
            message_id: trigger.message_id.clone(),
            sender_id: trigger.sender_id.clone(),
            text: prompt.clone(),
            keep: 0,
            mentions: vec![bot_id.to_owned()],
            reply_to: None,
            from_trigger: false,
            withheld: false,
        }
    } else {
        match lines.last() {
            Some(line) if line.message_id == trigger_message_id && line.sender_id != bot_id => {
                line.clone()
            }
            _ => return Err(HistoryError::TriggerUnreadable),
        }
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
            content: clip_keeping_tail(&spoken(line), line.keep, MAX_HISTORY_MESSAGE_CHARS),
            images: None,
        })
        .collect();
    if context_prompt.is_none() {
        history.pop();
    }
    Ok(Transcript {
        prompt: clip_keeping_tail(&spoken(&answered), answered.keep, MAX_PROMPT_CHARS),
        history,
        addressed: context_prompt.is_some()
            || kind != ConversationKind::Group
            || addressed_to(&lines, bot_id),
        topic_at: topic_at_of(&window),
        trail: tool_trail(bot_id, &window),
        reads: super::taint::reads_of(db, &window, bot_id).await?,
    })
}

/// The trail section of `bot_id`'s replies among `window` (oldest first).
fn tool_trail(bot_id: &str, window: &[inbox_entry::Model]) -> Option<TrailSection> {
    trail_of(
        bot_id,
        window.iter().filter_map(|entry| {
            Some(TrailEntry {
                message_id: &entry.message_id,
                sender_id: &entry.sender_id,
                kind: InboxEntryKind::parse(&entry.kind)?,
                envelope: &entry.envelope,
            })
        }),
    )
}

/// Recent context for a continuation whose trigger lives in another conversation. Activity does
/// not displace spoken lines. Edits and topic markers still apply within the bounded scan.
pub(crate) async fn recent_history<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    kind: ConversationKind,
    owner_id: &str,
) -> Result<Vec<ChatMessage>, HistoryError> {
    const MAX_HISTORY_PAGES: u64 = 32;
    let mut window = Vec::new();
    let mut lines = Vec::new();
    for page in 0..MAX_HISTORY_PAGES {
        let batch = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(bot_id))
            .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
            .filter(inbox_entry::Column::Kind.is_in(SPOKEN_KINDS))
            .order_by_desc(inbox_entry::Column::Seq)
            .order_by_desc(inbox_entry::Column::CreatedAt)
            .order_by_desc(inbox_entry::Column::Id)
            .offset(page * HISTORY_ENTRIES)
            .limit(HISTORY_ENTRIES)
            .all(db)
            .await?;
        let exhausted = batch.len() < HISTORY_ENTRIES as usize;
        let topic = batch.iter().any(|entry| {
            InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Topic)
                && !is_tombstone(&entry.envelope)
                && plain_body(&entry.envelope).is_some()
        });
        window.extend(batch);
        window.reverse();
        lines = lines_of(&window, &[], bot_id);
        window.reverse();
        if lines.len() >= HISTORY_ENTRIES as usize || exhausted || topic {
            break;
        }
    }
    let skipped = lines.len().saturating_sub(HISTORY_ENTRIES as usize);
    lines.drain(..skipped);
    let names = if kind == ConversationKind::Group {
        let ids: Vec<&str> = lines.iter().map(|line| line.sender_id.as_str()).collect();
        sender_names(db, &ids).await?
    } else {
        HashMap::new()
    };
    Ok(lines
        .iter()
        .map(|line| {
            let own = line.sender_id == bot_id;
            let text = if own || kind != ConversationKind::Group {
                line.text.clone()
            } else {
                let name = names.get(&line.sender_id).unwrap_or(&line.sender_id);
                room_message(name, line.sender_id == owner_id, &line.text)
            };
            ChatMessage {
                role: if own {
                    ChatRole::Assistant
                } else {
                    ChatRole::User
                },
                content: clip_keeping_tail(&text, line.keep, MAX_HISTORY_MESSAGE_CHARS),
                images: None,
            }
        })
        .collect())
}

/// What a turn woken by reactions reads: the conversation's recent messages, and the reactions in
/// place on the bot's own messages among the entries the turn answers.
#[derive(Debug, Clone)]
pub(crate) struct ReactionTurn {
    pub transcript: Transcript,
    pub reactions: Vec<ReactionNote>,
    pub newest_message_id: Option<String>,
}

const EXCERPT_CHARS: usize = 120;

/// What kind of entry the bot's copy holds under `message_id`.
pub(crate) async fn entry_kind<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> Result<Option<InboxEntryKind>, DbErr> {
    Ok(inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .one(db)
        .await?
        .and_then(|entry| InboxEntryKind::parse(&entry.kind)))
}

/// Whether the entry is a reaction event, parsed.
fn reaction_of(entry: &inbox_entry::Model) -> Option<ReactionEvent> {
    (InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Reaction)
        && !is_tombstone(&entry.envelope)
        && plain_body(&entry.envelope).is_some())
    .then(|| ReactionEvent::parse(&entry.message_id, &entry.sender_id, &entry.envelope))
    .flatten()
}

/// The reactions in place among `answered` (the turn's entry ids) on messages `bot_id` sent,
/// from the newest entries of its copy, oldest first. Later removals count even when they came
/// after the turn's trigger.
pub(crate) async fn reaction_turn<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    answered: &[String],
    kind: ConversationKind,
    owner_id: &str,
) -> Result<ReactionTurn, HistoryError> {
    let mut window = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Kind.is_in(SPOKEN_KINDS.into_iter().chain(["REACTION"])))
        .order_by_desc(inbox_entry::Column::Seq)
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .order_by_desc(inbox_entry::Column::Id)
        .limit(HISTORY_ENTRIES)
        .all(db)
        .await?;
    window.reverse();
    let lines = lines_of(&window, &[], bot_id);
    let events: Vec<ReactionEvent> = window.iter().filter_map(reaction_of).collect();
    let in_place: Vec<ReactionEvent> = net_reactions(&events)
        .into_iter()
        .filter(|event| answered.contains(&event.message_id))
        .collect();
    let ids: Vec<&str> = in_place
        .iter()
        .map(|event| event.sender_id.as_str())
        .chain(lines.iter().map(|line| line.sender_id.as_str()))
        .collect();
    let names = sender_names(db, &ids).await?;
    let reactions: Vec<ReactionNote> = in_place
        .iter()
        .filter_map(|event| {
            let own = lines
                .iter()
                .find(|line| line.message_id == event.target && line.sender_id == bot_id)?;
            Some(ReactionNote {
                by: names
                    .get(&event.sender_id)
                    .cloned()
                    .unwrap_or_else(|| event.sender_id.clone()),
                emoji: event.emoji.clone(),
                excerpt: clipped(&own.text.replace('\n', " "), EXCERPT_CHARS),
            })
        })
        .collect();
    let spoken = |line: &Line| {
        if line.sender_id == bot_id || kind != ConversationKind::Group {
            return line.text.clone();
        }
        let name = names.get(&line.sender_id).unwrap_or(&line.sender_id);
        room_message(name, line.sender_id == owner_id, &line.text)
    };
    let fresh = lines
        .iter()
        .rposition(|line| line.sender_id == bot_id)
        .map_or(lines.len(), |last| lines.len() - last - 1);
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
    let addressed = fresh == 0 || kind != ConversationKind::Group || addressed_to(&lines, bot_id);
    let as_history = |line: &Line| ChatMessage {
        role: if line.sender_id == bot_id {
            ChatRole::Assistant
        } else {
            ChatRole::User
        },
        content: clip_keeping_tail(&spoken(line), line.keep, MAX_HISTORY_MESSAGE_CHARS),
        images: None,
    };
    let (prompt, history, reactions) = match lines.split_last() {
        Some((last, earlier)) if fresh > 0 => {
            let prompt = if reaction_text.is_empty() {
                clip_keeping_tail(&spoken(last), last.keep, MAX_PROMPT_CHARS)
            } else {
                let room = MAX_PROMPT_CHARS.saturating_sub(reaction_text.chars().count() + 2);
                let said = clip_keeping_tail(&spoken(last), last.keep, room);
                let said_chars = said.chars().count();
                clipped(
                    &format!("{said}\n\n{reaction_text}"),
                    MAX_PROMPT_CHARS.max(said_chars),
                )
            };
            let kept = if addressed { Vec::new() } else { reactions };
            (prompt, earlier.iter().map(as_history).collect(), kept)
        }
        _ => (
            reaction_text,
            lines.iter().map(as_history).collect(),
            reactions,
        ),
    };
    Ok(ReactionTurn {
        newest_message_id: window.last().map(|entry| entry.message_id.clone()),
        transcript: Transcript {
            prompt,
            history,
            addressed,
            topic_at: topic_at_of(&window),
            trail: None,
            reads: super::taint::reads_of(db, &window, bot_id).await?,
        },
        reactions,
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
            cleared: Some(false),
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
        let lines = lines_of(&window, &[], "bot_a");
        let texts: Vec<_> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["first, fixed", "answer", "last"]);
    }

    #[test]
    fn a_line_addresses_the_bot_by_mention_or_reply_only_since_the_bots_last_line() {
        let line = |id: &str, sender: &str, mentions: &[&str], reply_to: Option<&str>| Line {
            message_id: id.into(),
            sender_id: sender.into(),
            text: String::new(),
            keep: 0,
            mentions: mentions.iter().map(|m| m.to_string()).collect(),
            reply_to: reply_to.map(str::to_string),
            from_trigger: false,
            withheld: false,
        };
        let quiet = [
            line("m1", "user-a", &[], None),
            line("m2", "user-b", &["bot_b"], None),
        ];
        assert!(!addressed_to(&quiet, "bot_a"));
        assert!(addressed_to(&quiet, "bot_b"));
        let answered = [
            line("m1", "user-a", &["bot_a"], None),
            line("a1", "bot_a", &[], Some("m1")),
            line("m2", "user-a", &[], None),
        ];
        assert!(!addressed_to(&answered, "bot_a"));
        let replied = [
            line("a1", "bot_a", &[], None),
            line("m2", "user-a", &[], Some("a1")),
        ];
        assert!(addressed_to(&replied, "bot_a"));
        let batched = [
            line("a1", "bot_a", &[], None),
            line("m2", "user-a", &["bot_a"], None),
            line("m3", "user-a", &[], None),
        ];
        assert!(addressed_to(&batched, "bot_a"));
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
        let lines = lines_of(&window, &[], "bot_a");
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

    #[test]
    fn a_transcript_that_spoke_over_a_reply_cuts_it_at_the_heard_share() {
        let spoken = |extra: serde_json::Value| {
            let mut body =
                json!({ "call": "c_1", "text": "stop", "offset_ms": 0, "duration_ms": 1 });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            body
        };
        let window = vec![
            entry(
                "m1",
                "message",
                "bot_a",
                json!({ "text": "The weather is sunny today and tomorrow." }),
            ),
            entry(
                "m2",
                "message",
                "bot_a",
                json!({ "text": "Untouched reply." }),
            ),
            entry(
                "t1",
                "transcript",
                "user-a",
                spoken(
                    json!({ "interrupted": { "message_id": "m1", "heard_chars": 10, "spoken_chars": 40 } }),
                ),
            ),
            entry(
                "t2",
                "transcript",
                "user-a",
                spoken(
                    json!({ "interrupted": { "message_id": "gone", "heard_chars": 1, "spoken_chars": 4 } }),
                ),
            ),
        ];
        let lines = lines_of(&window, &[], "bot_a");
        assert!(lines[0].text.starts_with("The…") || lines[0].text.starts_with("The"));
        assert!(lines[0].text.contains("cut off"));
        assert!(!lines[0].text.contains("sunny"));
        assert_eq!(lines[1].text, "Untouched reply.");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_revocation_after_the_trigger_withholds_the_session_snapshot() {
        use flow_like_types::bot_user_context::{
            REQUEST_FIELD, RESPONSE_FIELD, REVOKE_FIELD, UserContextAudience, UserContextField,
            UserContextPrecision, UserContextRequest, UserContextRequestArgs,
        };
        let now = chrono::Utc::now().timestamp_millis();
        let request = UserContextRequest::new(
            "ctx1".into(),
            "bot_a".into(),
            "c1".into(),
            UserContextRequestArgs {
                subject_id: "user-a".into(),
                fields: vec![UserContextField::Timezone],
                purpose: "Arrange a time".into(),
                precision: UserContextPrecision::Approximate,
                status_change: None,
            },
            UserContextAudience::Direct,
            vec!["bot_a".into(), "user-a".into()],
            now,
        );
        let answer = json!({"text":"Shared.","reply_to":"ctx1","mentions":["bot_a"],RESPONSE_FIELD:{
            "request_id":"ctx1","bot_id":"bot_a","subject_id":"user-a","conversation_id":"c1",
            "decision":"session","captured_at":now,"expires_at":now+300_000,
            "source":{"session_id":"s1","platform":"web"},
            "values":{"timezone":{"name":"Europe/Berlin"}},"unavailable":{}}});
        let window = vec![
            entry("ctx1", "message", "bot_a", json!({REQUEST_FIELD: request})),
            entry("ans", "message", "user-a", answer),
            entry("m1", "message", "user-a", json!({"text":"when?"})),
        ];
        let revoke = entry(
            "rev",
            "message",
            "user-a",
            json!({"text":"Revoked.",REVOKE_FIELD:{
                "request_id":"ctx1","bot_id":"bot_a","subject_id":"user-a",
                "conversation_id":"c1","revoked_at":now}}),
        );
        assert!(context_of(&window, &[], "bot_a").live_note().is_some());
        assert!(
            context_of(&window, &[revoke], "bot_a")
                .live_note()
                .is_none()
        );
    }
}

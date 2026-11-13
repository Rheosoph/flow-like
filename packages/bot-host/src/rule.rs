//! Which message a local bot has yet to answer. The rule reads the newest entries of a
//! conversation and mirrors what the hub's ledger queues a turn for.

use chrono::{DateTime, FixedOffset};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::MessageBody,
    bot_notify::reply_is_proactive,
    bot_turn::{MAX_HOPS, TurnAudience},
    inbox::{
        ConversationKind, InboxEntryKind, PLAIN_ENCRYPTION, ReactionEvent, TranscriptBody,
        TriggerBody, is_tombstone, net_reactions,
    },
};
use std::collections::HashSet;

const HOST_REPLY_PREFIXES: [&str; 2] = ["re-", "rf-"];

/// Whether a chat window answers the messages it marked `interactive` itself: while one is live,
/// or a marked message is younger than the grace, the host leaves it alone.
#[derive(Debug, Clone, Copy)]
pub struct Hold {
    pub now: DateTime<FixedOffset>,
    pub grace: chrono::Duration,
    pub live: bool,
}

impl Hold {
    fn keeps(&self, message: &Message) -> bool {
        let marked = message
            .body
            .as_ref()
            .is_some_and(|body| body.interactive.is_some());
        marked && (self.live || message.entry.created_at > self.now - self.grace)
    }
}

/// The message a turn answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub message_id: String,
    pub seq: Option<i64>,
    /// Bot turns that led to the message; a reply carries one more.
    pub hops: u32,
    /// The person did not just ask for the reply: a trigger or another bot woke the bot.
    pub proactive: bool,
    /// Whose message woke the bot; another bot of the same owner speaks for the owner in a
    /// direct conversation.
    pub requester_id: String,
    pub audience: TurnAudience,
    /// The message names the bot, replies to it, or is in a direct conversation; otherwise the
    /// bot reads along and may stay silent.
    pub addressed: bool,
}

struct Message<'a> {
    entry: &'a InboxEntry,
    body: Option<MessageBody>,
}

impl Message<'_> {
    fn deleted(&self) -> bool {
        is_tombstone(&self.entry.envelope)
    }

    fn reply_to(&self) -> Option<&str> {
        self.body.as_ref()?.reply_to.as_deref()
    }
}

fn is_approval_entry(message: &Message) -> bool {
    message
        .body
        .as_ref()
        .is_some_and(|body| body.approval.is_some() || body.approval_response.is_some())
}

fn bot_acknowledgement(message: &Message) -> bool {
    // These wake rules serve the local store, which permits only its owner and validated bots.
    // Account turns check the hub's member kind before applying the same exclusion.
    matches!(
        message.entry.kind,
        InboxEntryKind::Message | InboxEntryKind::Transcript
    ) && message.entry.sender_id != flow_like_local_bots::LOCAL_OWNER_ID
        && message
            .entry
            .envelope
            .get("body")
            .is_some_and(flow_like::flow::copilot::decision::is_acknowledgement_body)
}

fn messages(entries: &[InboxEntry]) -> Vec<Message<'_>> {
    entries
        .iter()
        .filter(|entry| entry.kind.wakes_bot() || entry.kind == InboxEntryKind::Transcript)
        .map(|entry| Message {
            entry,
            body: parse_body(entry),
        })
        .filter(|message| !is_approval_entry(message))
        .filter(|message| {
            !flow_like_types::bot_user_context::is_silent_body(&message.entry.envelope["body"])
        })
        .collect()
}

/// The turns whose approval card the person has not answered yet: such a turn sleeps.
fn parked_turns(entries: &[InboxEntry], bot_id: &str) -> HashSet<String> {
    let bodies: Vec<(&InboxEntry, MessageBody)> = entries
        .iter()
        .filter(|entry| entry.kind.wakes_bot())
        .filter_map(|entry| parse_body(entry).map(|body| (entry, body)))
        .collect();
    let answered: HashSet<&str> = bodies
        .iter()
        .filter_map(|(_, body)| body.approval_response.as_ref())
        .map(|response| response.approval_id.as_str())
        .collect();
    bodies
        .iter()
        .filter(|(entry, _)| entry.sender_id == bot_id)
        .filter_map(|(_, body)| body.approval.as_ref())
        .filter(|card| !answered.contains(card.id.as_str()))
        .map(|card| card.turn_id.clone())
        .collect()
}

pub fn is_plain(entry: &InboxEntry) -> bool {
    entry
        .envelope
        .get("enc")
        .and_then(serde_json::Value::as_str)
        == Some(PLAIN_ENCRYPTION)
}

/// Triggers and spoken lines carry their rendered text and mentions into the host's turn rules.
pub fn parse_body(entry: &InboxEntry) -> Option<MessageBody> {
    let body = entry.envelope.get("body").filter(|_| is_plain(entry))?;
    if entry.kind == InboxEntryKind::Trigger {
        let trigger = TriggerBody::parse(body).ok()?;
        return Some(MessageBody {
            text: trigger.render(),
            mentions: trigger.bot_id.into_iter().collect(),
            ..MessageBody::default()
        });
    }
    if entry.kind == InboxEntryKind::Transcript {
        let spoken = TranscriptBody::parse(body).ok()?;
        return Some(MessageBody {
            text: spoken.render(),
            mentions: spoken.mentions,
            interactive: spoken.interactive,
            ..MessageBody::default()
        });
    }
    serde_json::from_value(body.clone()).ok()
}

fn hops_of(message: &Message, bot_sender: bool) -> u32 {
    let declared = message
        .body
        .as_ref()
        .and_then(|body| body.hops)
        .unwrap_or(0);
    if bot_sender {
        declared.max(1)
    } else {
        declared
    }
}

fn is_host_reply(message_id: &str) -> bool {
    HOST_REPLY_PREFIXES
        .iter()
        .any(|prefix| message_id.starts_with(prefix))
}

/// How many messages, newest first, the bot has answered: its newest message answers everything
/// before it, and a host reply answers up to the message it replies to, so a message that landed
/// while the turn ran stays unanswered.
fn answered_upto(messages: &[Message], bot_id: &str) -> usize {
    let Some(own) = messages
        .iter()
        .position(|message| message.entry.sender_id == bot_id)
    else {
        return messages.len();
    };
    let reply = &messages[own];
    if !is_host_reply(&reply.entry.message_id) {
        return own;
    }
    reply
        .reply_to()
        .and_then(|target| {
            messages
                .iter()
                .skip(own + 1)
                .position(|message| message.entry.message_id == target)
        })
        .map_or(own, |offset| own + 1 + offset)
}

fn addressed(
    kind: ConversationKind,
    bot_id: &str,
    message: &Message,
    messages: &[Message],
) -> bool {
    if kind == ConversationKind::Direct {
        return true;
    }
    let Some(body) = &message.body else {
        return false;
    };
    if message.entry.kind == InboxEntryKind::Trigger {
        return body.mentions.is_empty() || body.mentions.iter().any(|mention| mention == bot_id);
    }
    body.mentions.iter().any(|mention| mention == bot_id)
        || body.reply_to.as_deref().is_some_and(|target| {
            messages.iter().any(|candidate| {
                candidate.entry.message_id == target && candidate.entry.sender_id == bot_id
            })
        })
}

/// The newest message of someone else that `bot_id` has yet to answer, from `entries` (newest
/// first). Direct conversations always wake the bot; in a group it wakes when the bot is
/// mentioned or replied to, and for a person's message also when the bot reads along (unless
/// `mentioned_only`). A message of a bot counts at least one hop and none beyond [`MAX_HOPS`]
/// wakes anyone. Edits, topics, reactions, member events, tombstones and messages sent before
/// `not_before` are no trigger. A bot with a decision model (`has_decision_model`) also reads
/// along with spoken call transcripts; without one it would answer every utterance.
pub fn waiting(
    kind: ConversationKind,
    bot_id: &str,
    entries: &[InboxEntry],
    not_before: DateTime<FixedOffset>,
    mentioned_only: bool,
    has_decision_model: bool,
) -> Option<Waiting> {
    waiting_held(
        kind,
        bot_id,
        entries,
        not_before,
        None,
        mentioned_only,
        has_decision_model,
    )
}

fn reads_along(
    kind: ConversationKind,
    message: &Message,
    mentioned_only: bool,
    has_decision_model: bool,
) -> bool {
    kind == ConversationKind::Group
        && !mentioned_only
        && message.entry.sender_id == flow_like_local_bots::LOCAL_OWNER_ID
        && match message.entry.kind {
            InboxEntryKind::Message => true,
            InboxEntryKind::Transcript => has_decision_model,
            _ => false,
        }
}

/// [`waiting`] that leaves the messages an interactive chat holds to that chat.
pub fn waiting_held(
    kind: ConversationKind,
    bot_id: &str,
    entries: &[InboxEntry],
    not_before: DateTime<FixedOffset>,
    hold: Option<Hold>,
    mentioned_only: bool,
    has_decision_model: bool,
) -> Option<Waiting> {
    let parked = parked_turns(entries, bot_id);
    let messages = messages(entries);
    let unanswered = answered_upto(&messages, bot_id);
    messages[..unanswered]
        .iter()
        .find(|message| {
            let sender = message.entry.sender_id.as_str();
            let bot_sender = sender != flow_like_local_bots::LOCAL_OWNER_ID;
            sender != bot_id
                && context_recipient(message, bot_id)
                && !bot_acknowledgement(message)
                && !message.deleted()
                && message.body.is_some()
                && message.entry.created_at >= not_before
                && !hold.is_some_and(|hold| hold.keeps(message))
                && hops_of(message, bot_sender) <= MAX_HOPS
                && (addressed(kind, bot_id, message, &messages)
                    || reads_along(kind, message, mentioned_only, has_decision_model))
        })
        .filter(|message| {
            !parked.contains(&flow_like_types::bot_turn::turn_id(
                bot_id,
                &message.entry.message_id,
            ))
        })
        .map(|message| {
            let hops = hops_of(
                message,
                message.entry.sender_id != flow_like_local_bots::LOCAL_OWNER_ID,
            );
            let sender = message.entry.sender_id.as_str();
            let requester_id = if kind == ConversationKind::Direct {
                flow_like_local_bots::LOCAL_OWNER_ID
            } else {
                sender
            };
            Waiting {
                requester_id: requester_id.to_string(),
                audience: TurnAudience::of(kind),
                addressed: addressed(kind, bot_id, message, &messages),
                message_id: message.entry.message_id.clone(),
                seq: message.entry.seq,
                hops,
                proactive: reply_is_proactive(message.entry.kind, hops),
            }
        })
        .or_else(|| reacted_to(kind, bot_id, entries, not_before))
}

/// A named participant releases a persisted dependency even when their reply does not mention
/// the waiting bot. The ordinary replay, hop, approval and interactive-chat guards still apply.
pub fn waiting_participant(
    kind: ConversationKind,
    bot_id: &str,
    participant_id: &str,
    after_seq: i64,
    entries: &[InboxEntry],
    not_before: DateTime<FixedOffset>,
    hold: Option<Hold>,
) -> Option<Waiting> {
    let parked = parked_turns(entries, bot_id);
    let messages = messages(entries);
    let unanswered = answered_upto(&messages, bot_id);
    let message = messages[..unanswered].iter().find(|message| {
        message.entry.sender_id == participant_id
            && context_recipient(message, bot_id)
            && !bot_acknowledgement(message)
            && participant_id != bot_id
            && matches!(
                message.entry.kind,
                InboxEntryKind::Message | InboxEntryKind::Transcript
            )
            && message.entry.seq.is_some_and(|seq| seq > after_seq)
            && !message.deleted()
            && message.body.is_some()
            && message.entry.created_at >= not_before
            && !hold.is_some_and(|hold| hold.keeps(message))
            && hops_of(
                message,
                participant_id != flow_like_local_bots::LOCAL_OWNER_ID,
            ) <= MAX_HOPS
    })?;
    if parked.contains(&flow_like_types::bot_turn::turn_id(
        bot_id,
        &message.entry.message_id,
    )) {
        return None;
    }
    let hops = hops_of(
        message,
        participant_id != flow_like_local_bots::LOCAL_OWNER_ID,
    );
    Some(Waiting {
        message_id: message.entry.message_id.clone(),
        seq: message.entry.seq,
        hops,
        proactive: reply_is_proactive(message.entry.kind, hops),
        requester_id: participant_id.to_owned(),
        audience: TurnAudience::of(kind),
        addressed: addressed(kind, bot_id, message, &messages),
    })
}

fn context_recipient(message: &Message, bot_id: &str) -> bool {
    if message.entry.envelope["body"]
        .get(flow_like_types::bot_user_context::DERIVED_FIELD)
        .is_some()
    {
        return false;
    }
    match message.entry.envelope["body"].get(flow_like_types::bot_user_context::RESPONSE_FIELD) {
        Some(response) => {
            response.get("bot_id").and_then(serde_json::Value::as_str) == Some(bot_id)
        }
        None => true,
    }
}

/// The newest reaction of a person still in place on a message `bot_id` sent after its last one.
/// A reaction wakes the bot in groups and direct conversations alike; a bot's reaction wakes
/// nobody.
fn reacted_to(
    kind: ConversationKind,
    bot_id: &str,
    entries: &[InboxEntry],
    not_before: DateTime<FixedOffset>,
) -> Option<Waiting> {
    let after_last_own = entries
        .iter()
        .position(|entry| entry.sender_id == bot_id && entry.kind == InboxEntryKind::Message)
        .unwrap_or(entries.len());
    let events: Vec<(&InboxEntry, ReactionEvent)> = entries[..after_last_own]
        .iter()
        .rev()
        .filter(|entry| entry.kind == InboxEntryKind::Reaction && is_plain(entry))
        .filter(|entry| !is_tombstone(&entry.envelope))
        .filter(|entry| entry.sender_id == flow_like_local_bots::LOCAL_OWNER_ID)
        .filter_map(|entry| {
            ReactionEvent::parse(&entry.message_id, &entry.sender_id, &entry.envelope)
                .map(|event| (entry, event))
        })
        .collect();
    let plain: Vec<ReactionEvent> = events.iter().map(|(_, event)| event.clone()).collect();
    let in_place: HashSet<String> = net_reactions(&plain)
        .into_iter()
        .map(|event| event.message_id)
        .collect();
    events
        .iter()
        .rev()
        .filter(|(entry, event)| {
            in_place.contains(&event.message_id)
                && entry.created_at >= not_before
                && entries.iter().any(|candidate| {
                    candidate.message_id == event.target
                        && candidate.sender_id == bot_id
                        && candidate.kind == InboxEntryKind::Message
                        && !is_tombstone(&candidate.envelope)
                })
        })
        .map(|(entry, _)| Waiting {
            message_id: entry.message_id.clone(),
            seq: entry.seq,
            hops: 0,
            proactive: false,
            requester_id: flow_like_local_bots::LOCAL_OWNER_ID.to_string(),
            audience: TurnAudience::of(kind),
            addressed: true,
        })
        .next()
}

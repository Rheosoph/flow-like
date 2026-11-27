//! Microsoft Teams and Discord chats a bot is part of. Each chat has a guest room of the owner and
//! the bot ([`crate::routes::inbox::channel_rooms`]); a message from the chat is written there as
//! a `channel` trigger naming the bot, and the turn it queues answers the outside person who
//! wrote it, not the owner: its requester is that person's [`outside_requester`] id, so the room
//! rules for people other than the owner apply (no owner-only tools, read-only memory, no owner
//! memory, a daily turn cap per person). The bot's reply goes back to the chat: Teams from the
//! server ([`after_reply`]), Discord from the owner's desktop, which holds the connection and
//! waits for the turn.

pub(crate) mod routes;

pub(crate) use routes::routes;

use crate::{
    bot_host,
    bot_notify::{Reply, bot_limit_guard},
    entity::{bot, bot_channel_link, bot_turn},
    error::ApiError,
    routes::inbox::{
        channel_rooms::{ChannelChat, open_channel_room_in, post_channel_trigger_in},
        now_millis, wake,
    },
    state::AppState,
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot::BotSystemKind,
    bot_turn::{STORED_QUEUED, turn_id},
    inbox::{TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS, TriggerBody, TriggerSource},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, sea_query::Expr,
};
use serde_json::{Value, json};

/// Trigger message ids of chat messages start with this.
pub(crate) const CHANNEL_MESSAGE_PREFIX: &str = "ch-";
/// Requester ids of outside people start with this.
pub(crate) const OUTSIDE_REQUESTER_PREFIX: &str = "ext:";
/// Turns one chat may start in an hour, whoever writes.
pub(crate) const LINK_HOURLY_TURNS: u64 = 120;
/// Turns one outside person may start with a bot in a day, as for room members.
pub(crate) const PERSON_DAILY_TURNS: u64 = 20;
pub(crate) const MAX_ATTACHMENTS: usize = 10;
pub(crate) const MAX_META_BYTES: usize = 4 * 1024;
pub(crate) const MAX_CHAT_TITLE_CHARS: usize = 100;
pub(crate) const MAX_SENDER_NAME_CHARS: usize = 80;
pub(crate) const MAX_ID_CHARS: usize = 512;
/// What a message's text may use of the trigger, leaving room for the sender and the links.
const MAX_MESSAGE_CHARS: usize = 6_000;
pub(crate) const CHANNEL_NOT_ALLOWED: &str = "BOT_CHANNEL_NOT_ALLOWED";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelProvider {
    Teams,
    Discord,
}

impl ChannelProvider {
    pub(crate) fn stored(self) -> &'static str {
        match self {
            Self::Teams => "teams",
            Self::Discord => "discord",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "teams" => Some(Self::Teams),
            "discord" => Some(Self::Discord),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Teams => "Microsoft Teams",
            Self::Discord => "Discord",
        }
    }

    fn room_prefix(self) -> &'static str {
        match self {
            Self::Teams => "Teams",
            Self::Discord => "Discord",
        }
    }
}

/// Who wrote, as the provider names them. `id` is stable per person within the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutsideSender {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutsideAttachment {
    pub name: String,
    pub url: String,
    pub content_type: Option<String>,
}

/// One message from an outside chat, addressed to the bot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutsideMessage {
    pub provider: ChannelProvider,
    /// The provider's stable id of the chat.
    pub chat_key: String,
    /// The chat's name without the provider, e.g. "Sales" or "#general".
    pub chat_title: String,
    /// The provider's id of the message; a repeat is answered once.
    pub message_id: String,
    pub sender: OutsideSender,
    pub text: String,
    pub attachments: Vec<OutsideAttachment>,
    /// Provider data kept on the link for replies, replaced by every message.
    pub meta: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostStatus {
    Queued,
    Limited,
    Duplicate,
}

impl PostStatus {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Limited => "limited",
            Self::Duplicate => "duplicate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Posted {
    pub link_id: String,
    pub conversation_id: String,
    pub trigger_message_id: String,
    /// `None` when the message was refused for a limit.
    pub turn_id: Option<String>,
    pub status: PostStatus,
    /// What to tell the chat when the message was refused.
    pub notice: Option<String>,
}

/// A bot's reply that goes back to an outside chat.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelReply<'a> {
    pub turn_id: &'a str,
    pub bot_id: &'a str,
    pub trigger_message_id: &'a str,
    pub message_id: &'a str,
    pub text: &'a str,
}

fn digest(parts: &[&str]) -> String {
    blake3::hash(parts.join("\0").as_bytes()).to_hex()[..32].to_string()
}

/// The requester id of an outside person: never a Flow-Like sub or bot id.
pub(crate) fn outside_requester(provider: ChannelProvider, sender_id: &str) -> String {
    format!(
        "{OUTSIDE_REQUESTER_PREFIX}{}:{}",
        provider.stored(),
        digest(&[provider.stored(), sender_id])
    )
}

pub(crate) fn link_id(bot_id: &str, provider: ChannelProvider, chat_key: &str) -> String {
    format!("chl_{}", digest(&[bot_id, provider.stored(), chat_key]))
}

pub(crate) fn trigger_message_id(bot_id: &str, provider: ChannelProvider, chat_key: &str, message_id: &str) -> String {
    format!(
        "{CHANNEL_MESSAGE_PREFIX}{}",
        digest(&[bot_id, provider.stored(), chat_key, message_id])
    )
}

fn clip(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// The bots outside people may talk to: the owner's live bots, not Flux, whose FlowPilot tools
/// build and change the owner's apps.
pub(crate) fn ensure_channel_bot(bot: &bot::Model) -> Result<(), ApiError> {
    if bot.deleted_at.is_some() {
        return Err(ApiError::not_found(format!("Bot '{}' not found", bot.id)));
    }
    if bot.system_key.as_deref() == Some(BotSystemKind::Flux.as_str()) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            CHANNEL_NOT_ALLOWED,
            "Flux builds and changes your apps, so it can't join Teams or Discord chats; use another bot or a duplicate without FlowPilot",
        ));
    }
    Ok(())
}

pub(crate) fn validate(message: &OutsideMessage) -> Result<(), ApiError> {
    let required = [
        ("chat", message.chat_key.as_str()),
        ("message id", message.message_id.as_str()),
        ("sender id", message.sender.id.as_str()),
    ];
    for (what, value) in required {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_ID_CHARS {
            return Err(ApiError::bad_request(format!(
                "A chat message needs a {what} of 1 to {MAX_ID_CHARS} characters"
            )));
        }
    }
    if message.text.trim().is_empty() && message.attachments.is_empty() {
        return Err(ApiError::bad_request(
            "A chat message needs text or an attachment",
        ));
    }
    if message.text.chars().count() > TRIGGER_TEXT_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "A chat message has at most {TRIGGER_TEXT_MAX_CHARS} characters"
        )));
    }
    if message.attachments.len() > MAX_ATTACHMENTS {
        return Err(ApiError::bad_request(format!(
            "A chat message carries at most {MAX_ATTACHMENTS} attachments"
        )));
    }
    if let Some(bad) = message
        .attachments
        .iter()
        .find(|attachment| !attachment.url.starts_with("https://") || attachment.url.chars().count() > 2_048)
    {
        return Err(ApiError::bad_request(format!(
            "Attachment '{}' needs an https link of at most 2048 characters",
            clip(&bad.name, 80)
        )));
    }
    if message.meta.to_string().len() > MAX_META_BYTES {
        return Err(ApiError::bad_request(format!(
            "Chat data for replies has at most {MAX_META_BYTES} bytes"
        )));
    }
    Ok(())
}

fn sender_name(message: &OutsideMessage) -> String {
    let name = clip(&message.sender.name, MAX_SENDER_NAME_CHARS);
    if name.is_empty() {
        format!("Someone on {}", message.provider.label())
    } else {
        name
    }
}

fn chat_title(message: &OutsideMessage) -> String {
    let title = clip(&message.chat_title, MAX_CHAT_TITLE_CHARS);
    if title.is_empty() {
        message.provider.label().to_string()
    } else {
        title
    }
}

/// The trigger a chat message becomes: the sender's words as the text, who wrote where as
/// untrusted payload.
pub(crate) fn trigger_body(bot_id: &str, link_id: &str, message: &OutsideMessage) -> TriggerBody {
    let sender = sender_name(message);
    let provider = message.provider.label();
    let mut text = format!("{sender} ({provider}): {}", clip(&message.text, MAX_MESSAGE_CHARS));
    for attachment in &message.attachments {
        text.push_str(&format!(
            "\n[attachment: {}]({})",
            clip(&attachment.name, 120).replace(['[', ']'], ""),
            attachment.url
        ));
    }
    TriggerBody {
        monitoring: false,
        source: TriggerSource::Channel,
        ref_id: link_id.to_string(),
        title: clip(&format!("{sender} on {provider}"), TRIGGER_TITLE_MAX_CHARS),
        text: clip(&text, TRIGGER_TEXT_MAX_CHARS),
        payload: Some(json!({
            "provider": message.provider.stored(),
            "chat": { "title": chat_title(message) },
            "sender": {
                "id": outside_requester(message.provider, &message.sender.id),
                "name": sender,
            },
            "attachments": message.attachments.iter().map(|attachment| json!({
                "name": attachment.name,
                "url": attachment.url,
                "content_type": attachment.content_type,
            })).collect::<Vec<_>>(),
            "untrusted": true,
            "owner": false,
        })),
        payload_handle: None,
        bot_id: Some(bot_id.to_string()),
    }
}

async fn outside_turns<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    requester: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    Ok(bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OriginId.eq(requester))
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(24)))
        .count(db)
        .await?)
}

async fn chat_turns<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    Ok(bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::ConversationId.eq(conversation_id))
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(1)))
        .count(db)
        .await?)
}

const PERSON_LIMIT_NOTICE: &str = "I can only answer so many messages a day from one person, and that limit is used up. Please try again tomorrow.";
const CHAT_LIMIT_NOTICE: &str = "This chat has sent me a lot of messages in the last hour. Please try again in a little while.";
const NOT_QUEUED_NOTICE: &str = "I can't take this message right now. Please try again later.";

/// Writes the chat message into the chat's guest room, opening it on first use, and queues the
/// bot's turn with the outside person as its requester. A repeat of the message returns the
/// first turn. Refused for a limit, it queues nothing and says what to tell the chat.
pub(crate) async fn post_outside_message(
    state: &AppState,
    bot: &bot::Model,
    message: &OutsideMessage,
) -> Result<Posted, ApiError> {
    ensure_channel_bot(bot)?;
    validate(message)?;
    let provider = message.provider;
    let link = link_id(&bot.id, provider, &message.chat_key);
    let trigger = trigger_message_id(&bot.id, provider, &message.chat_key, &message.message_id);
    let turn = turn_id(&bot.id, &trigger);
    let requester = outside_requester(provider, &message.sender.id);
    let _serial = bot_limit_guard("channel", &bot.id).await;
    let existing = bot_channel_link::Entity::find_by_id(&link).one(&state.db).await?;
    if let Some(existing) = &existing
        && bot_turn::Entity::find_by_id(&turn).one(&state.db).await?.is_some()
    {
        return Ok(Posted {
            link_id: link,
            conversation_id: existing.conversation_id.clone(),
            trigger_message_id: trigger,
            turn_id: Some(turn),
            status: PostStatus::Duplicate,
            notice: None,
        });
    }
    let now = now_millis();
    let limited = if outside_turns(&state.db, &bot.id, &requester, now).await? >= PERSON_DAILY_TURNS
    {
        Some(PERSON_LIMIT_NOTICE)
    } else if let Some(existing) = &existing
        && chat_turns(&state.db, &bot.id, &existing.conversation_id, now).await? >= LINK_HOURLY_TURNS
    {
        Some(CHAT_LIMIT_NOTICE)
    } else {
        None
    };
    if let Some(notice) = limited {
        tracing::info!(bot_id = bot.id, link_id = link, "A chat message was refused for a channel limit");
        return Ok(Posted {
            conversation_id: existing.map(|link| link.conversation_id).unwrap_or_default(),
            link_id: link,
            trigger_message_id: trigger,
            turn_id: None,
            status: PostStatus::Limited,
            notice: Some(notice.to_string()),
        });
    }
    let body = trigger_body(&bot.id, &link, message);
    let room_title = format!("{} · {}", provider.room_prefix(), chat_title(message));
    let chat = chat_title(message);
    let meta = message.meta.to_string();
    let chat_key = message.chat_key.trim();
    let (row, sent) = state
        .transaction(|txn| {
            let chat = ChannelChat {
                link_id: link.clone(),
                owner_id: bot.owner_id.clone(),
                bot_id: bot.id.clone(),
                provider: provider.stored().to_string(),
                chat_key: chat_key.to_string(),
                room_title: room_title.clone(),
                chat_title: chat.clone(),
                meta: meta.clone(),
            };
            let (trigger, turn, requester, body) =
                (trigger.clone(), turn.clone(), requester.clone(), body.clone());
            Box::pin(async move {
                let row = open_channel_room_in(txn, &chat, now).await?;
                let sent = post_channel_trigger_in(txn, &row, &trigger, &body, now).await?;
                hand_to_requester_in(txn, &turn, &requester).await?;
                Ok::<_, ApiError>((row, sent))
            })
        })
        .await?;
    wake(state, sent.wake).await;
    bot_host::dispatch(state, &sent.bots).await;
    let queued = bot_turn::Entity::find_by_id(&turn).one(&state.db).await?.is_some();
    Ok(Posted {
        link_id: row.id,
        conversation_id: row.conversation_id,
        trigger_message_id: trigger,
        turn_id: queued.then_some(turn),
        status: if queued {
            PostStatus::Queued
        } else {
            PostStatus::Limited
        },
        notice: (!queued).then(|| NOT_QUEUED_NOTICE.to_string()),
    })
}

/// The turn a chat message queued answers the outside person who wrote it. Its trigger comes
/// from the owner, the only person in the room, so the ledger queued it with the owner as
/// requester; this moves it to the outside person before any host can claim it.
async fn hand_to_requester_in<C: ConnectionTrait>(
    txn: &C,
    turn: &str,
    requester: &str,
) -> Result<(), ApiError> {
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::RequesterId, Expr::value(requester))
        .col_expr(bot_turn::Column::OriginId, Expr::value(requester))
        .filter(bot_turn::Column::Id.eq(turn))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .exec(txn)
        .await?;
    Ok(())
}

/// The link whose room holds `conversation_id`.
pub(crate) async fn link_of_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<Option<bot_channel_link::Model>, ApiError> {
    Ok(bot_channel_link::Entity::find()
        .filter(bot_channel_link::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?)
}

/// Sends a bot's reply to a chat message back to its chat. Returns whether the reply belongs to
/// a chat, in which case the owner is not notified of it: the reply went to the people who asked.
/// A reply to the owner's own message in the room stays in the app.
pub(crate) async fn after_reply(state: &AppState, reply: &Reply<'_>) -> bool {
    match deliver(state, reply).await {
        Ok(channel) => channel,
        Err(error) => {
            tracing::warn!(%error, bot_id = reply.bot_id, turn_id = reply.turn_id, "Could not send a bot's reply to its chat");
            true
        }
    }
}

async fn deliver(state: &AppState, reply: &Reply<'_>) -> Result<bool, ApiError> {
    let Some(turn) = bot_turn::Entity::find_by_id(reply.turn_id).one(&state.db).await? else {
        return Ok(false);
    };
    if turn.bot_id != reply.bot_id {
        return Ok(false);
    }
    let Some(link) = link_of_conversation(&state.db, &turn.conversation_id).await? else {
        return Ok(false);
    };
    if !turn.trigger_message_id.starts_with(CHANNEL_MESSAGE_PREFIX) {
        return Ok(false);
    }
    let channel_reply = ChannelReply {
        turn_id: reply.turn_id,
        bot_id: reply.bot_id,
        trigger_message_id: &turn.trigger_message_id,
        message_id: reply.message_id,
        text: reply.text,
    };
    match ChannelProvider::parse(&link.provider) {
        Some(ChannelProvider::Teams) => {
            crate::teams::bot::deliver_reply(state, &link, &channel_reply).await?;
        }
        Some(ChannelProvider::Discord) | None => {}
    }
    Ok(true)
}

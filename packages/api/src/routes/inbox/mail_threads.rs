//! A bot's mail threads: each thread of mail is one group conversation owned by the bot with its
//! owner as the other member, and a `BotMailThread` row marks it as a mail thread. Inbound mail
//! is a `trigger` entry from the owner that names the bot; a mail the bot sends is recorded as a
//! message from the bot. Later mails reach their thread through the `Message-ID` rows in
//! `BotMailMessage`.

use super::{
    conversations::{NewConversation, normalize_title, start_group},
    entries::{Content, Outgoing, Sent, fan_out, plain_links, repeat_of},
    lock_conversation,
    members::Principal,
    triggers::trigger_outgoing,
};
use crate::{
    entity::{bot_mail_message, bot_mail_thread},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    create_id,
    inbox::{
        ConversationEncryption, ConversationKind, InboxEntryKind, PrincipalKind, TriggerBody,
        plain_envelope,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, sea_query::OnConflict,
};
use serde_json::{Value, json};

pub(crate) const MAX_PARTICIPANTS: usize = 50;
const SNIPPET_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    In,
    Out,
}

impl Direction {
    pub(crate) fn stored(self) -> &'static str {
        match self {
            Self::In => "IN",
            Self::Out => "OUT",
        }
    }
}

pub(crate) fn normalize_message_id(raw: &str) -> Option<String> {
    let id = raw
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase();
    (!id.is_empty() && id.len() <= 512 && !id.chars().any(|c| c.is_control() || c.is_whitespace()))
        .then_some(id)
}

fn snippet(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(SNIPPET_CHARS).collect()
}

/// The thread, of this bot, that already holds one of these `Message-ID`s.
pub(crate) async fn thread_of_any<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    message_ids: &[String],
) -> Result<Option<String>, ApiError> {
    if message_ids.is_empty() {
        return Ok(None);
    }
    Ok(bot_mail_message::Entity::find()
        .select_only()
        .column(bot_mail_message::Column::ThreadId)
        .filter(bot_mail_message::Column::BotId.eq(bot_id))
        .filter(bot_mail_message::Column::MessageId.is_in(message_ids.iter().cloned()))
        .order_by_desc(bot_mail_message::Column::CreatedAt)
        .into_tuple::<String>()
        .one(db)
        .await?)
}

pub(crate) async fn holds_message<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    message_id: &str,
) -> Result<bool, ApiError> {
    Ok(thread_of_any(db, bot_id, &[message_id.to_owned()])
        .await?
        .is_some())
}

/// Starts the thread's conversation (the owner and the bot) and its marker row.
pub(crate) async fn start_thread_in<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    subject: &str,
    participants: &[String],
    id: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<String, ApiError> {
    let id = id.unwrap_or_else(create_id);
    let title = normalize_title(Some(format!("Mail: {}", subject.trim())))?;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title,
        members: vec![bot_id.to_owned()],
        encryption: ConversationEncryption::Plain,
    };
    let bot = Principal {
        id: bot_id.to_owned(),
        kind: PrincipalKind::Bot,
    };
    start_group(txn, &id, owner_id, &plan, &[bot], now).await?;
    bot_mail_thread::Entity::insert(bot_mail_thread::ActiveModel {
        id: Set(id.clone()),
        bot_id: Set(bot_id.to_owned()),
        owner_id: Set(owner_id.to_owned()),
        subject: Set(subject.trim().chars().take(300).collect()),
        participants: Set(serde_json::to_string(
            &participants
                .iter()
                .take(MAX_PARTICIPANTS)
                .collect::<Vec<_>>(),
        )?),
        last_message_at: Set(now),
        last_snippet: Set(None),
        message_count: Set(0),
        created_at: Set(now),
    })
    .exec_without_returning(txn)
    .await?;
    Ok(id)
}

/// Records one mail under its `Message-ID`; `false` when the bot already holds that mail.
pub(crate) async fn link_message_in<C: ConnectionTrait>(
    db: &C,
    thread_id: &str,
    bot_id: &str,
    message_id: &str,
    direction: Direction,
    entry_message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let inserted = bot_mail_message::Entity::insert(bot_mail_message::ActiveModel {
        id: Set(create_id()),
        thread_id: Set(thread_id.to_owned()),
        bot_id: Set(bot_id.to_owned()),
        message_id: Set(message_id.to_owned()),
        direction: Set(direction.stored().to_owned()),
        entry_message_id: Set(entry_message_id.to_owned()),
        created_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            bot_mail_message::Column::BotId,
            bot_mail_message::Column::MessageId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(inserted == 1)
}

/// Moves the thread's last activity, count and correspondents after a mail.
pub(crate) async fn touch_thread_in<C: ConnectionTrait>(
    db: &C,
    thread_id: &str,
    new_participants: &[String],
    text: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let Some(row) = bot_mail_thread::Entity::find_by_id(thread_id)
        .one(db)
        .await?
    else {
        return Err(ApiError::not_found("Mail thread not found"));
    };
    let mut participants: Vec<String> = serde_json::from_str(&row.participants).unwrap_or_default();
    for address in new_participants {
        let address = address.trim().to_ascii_lowercase();
        if !address.is_empty()
            && participants.len() < MAX_PARTICIPANTS
            && !participants.contains(&address)
        {
            participants.push(address);
        }
    }
    let count = row.message_count.saturating_add(1);
    let mut active: bot_mail_thread::ActiveModel = row.into();
    active.participants = Set(serde_json::to_string(&participants)?);
    active.last_message_at = Set(now);
    active.last_snippet = Set(Some(snippet(text)));
    active.message_count = Set(count);
    bot_mail_thread::Entity::update(active).exec(db).await?;
    Ok(())
}

/// Writes the inbound mail as a trigger from the owner that names the bot.
pub(crate) async fn post_thread_trigger_in(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    owner_id: &str,
    message_id: &str,
    body: &TriggerBody,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    write_trigger_in(txn, conversation_id, owner_id, message_id, body, now).await
}

pub(crate) async fn write_trigger_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    owner_id: &str,
    message_id: &str,
    body: &TriggerBody,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let outgoing = trigger_outgoing(message_id, body)?;
    if let Some(sent) = repeat_of(txn, conversation_id, owner_id, message_id).await? {
        return Ok(sent);
    }
    fan_out(txn, conversation_id, owner_id, 0, &outgoing, now).await
}

/// Records a mail the bot sent as a message from the bot, so the owner reads both directions in
/// one thread. `mail` rides in the body next to the markdown text.
pub(crate) async fn record_outbound_in(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    bot_id: &str,
    message_id: &str,
    text: &str,
    mail: Value,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    write_outbound_in(txn, conversation_id, bot_id, message_id, text, mail, now).await
}

pub(crate) async fn write_outbound_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    message_id: &str,
    text: &str,
    mail: Value,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let envelope = plain_envelope(json!({ "text": text, "mail": mail }));
    let links = plain_links(InboxEntryKind::Message, &envelope)?;
    let outgoing = Outgoing {
        message_id: message_id.to_owned(),
        kind: InboxEntryKind::Message,
        content: Content::Plain(envelope),
        links,
    };
    if let Some(sent) = repeat_of(txn, conversation_id, bot_id, message_id).await? {
        return Ok(sent);
    }
    fan_out(txn, conversation_id, bot_id, 0, &outgoing, now).await
}

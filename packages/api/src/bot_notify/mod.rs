//! Notifying a bot's owner: replies the owner did not just ask for (a trigger's answer, a bot's
//! answer to another bot) and the `message_owner` tool. Every notification is titled with the bot's
//! name, grouped under the bot (`source_run_id = bot:<id>`) and opens the bot's chat. The decision
//! rules live in `flow_like_types::bot_notify`; the desktop host applies the same ones.

use crate::{
    entity::{
        bot, bot_turn, conversation, inbox_entry, notification,
        sea_orm_active_enums::NotificationType,
    },
    push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent},
    routes::inbox::{
        approvals::{is_owner_chat, open_conversation},
        exchange::{message_outgoing, post_as_bot},
        now_millis,
    },
    state::AppState,
};
use async_trait::async_trait;
use flow_like_bot_tools::{NoticeReceipt, OwnerNotice, OwnerNotifier, TurnSite};
use flow_like_types::{
    Result, anyhow,
    bot_notify::{
        TOOL_SOURCE, chat_link, group_key, notifications_muted, preview, reply_is_proactive,
    },
    create_id,
    inbox::InboxEntryKind,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
};

#[cfg(test)]
mod tests;

const FAILURE_REPLY_PREFIX: &str = "rf-";
const LANE_COUNT: usize = 64;

static LANES: LazyLock<Vec<Arc<flow_like_types::tokio::sync::Mutex<()>>>> = LazyLock::new(|| {
    (0..LANE_COUNT)
        .map(|_| Arc::new(flow_like_types::tokio::sync::Mutex::new(())))
        .collect()
});

/// Serializes check-then-act limits for one bot inside this process; held across the count and
/// the write that the count guards.
pub(crate) async fn bot_limit_guard(
    domain: &str,
    bot_id: &str,
) -> flow_like_types::tokio::sync::OwnedMutexGuard<()> {
    let digest = blake3::hash(format!("{domain}\0{bot_id}").as_bytes());
    let lane = usize::from(digest.as_bytes()[0]) % LANE_COUNT;
    LANES[lane].clone().lock_owned().await
}

const OWNER_MESSAGE_PREFIX: &str = "mo-";

#[async_trait]
pub(crate) trait NoticeSink: Send + Sync {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()>;
}

pub(crate) struct StateSink<'a>(pub(crate) &'a AppState);

#[async_trait]
impl NoticeSink for StateSink<'_> {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()> {
        dispatch_notification_idempotent(self.0, id, input)
            .await
            .map(|_| ())
            .map_err(|error| anyhow!("Notifying the owner failed: {error}"))
    }
}

const NO_PREVIEW: &str = "Sent you a message";

/// The plain text of a reply's envelope; empty for an encrypted or text-less message.
pub(crate) fn text_of(envelope: Option<&serde_json::Value>) -> String {
    envelope
        .and_then(|envelope| envelope.get("body"))
        .and_then(|body| body.get("text"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A reply a bot committed for a turn.
pub(crate) struct Reply<'a> {
    pub turn_id: &'a str,
    pub bot_id: &'a str,
    pub message_id: &'a str,
    pub text: &'a str,
    /// The turn slept for an owner's approval; the owner did not just ask for this reply.
    pub resumed: bool,
}

pub(crate) fn display_name(bot: &bot::Model) -> String {
    bot.name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match bot.system_key.as_deref() {
            Some("flux") => "Flux".to_string(),
            _ => "Assistant".to_string(),
        })
}

fn stable_id(bot_id: &str, key: &str) -> String {
    let digest = blake3::hash(format!("{bot_id}\0{key}").as_bytes()).to_hex();
    format!("bn-{}", &digest[..32])
}

fn input_for(
    bot: &bot::Model,
    body: String,
    link: Option<String>,
    node: String,
) -> DispatchNotificationInput {
    DispatchNotificationInput {
        notification_priority: Default::default(),
        bot_context: Some(flow_like_types::bot_notify::BotNotificationContext {
            bot_id: bot.id.clone(),
            ..Default::default()
        }),
        user_id: bot.owner_id.clone(),
        app_id: None,
        title: display_name(bot),
        description: Some(body),
        icon: None,
        image: None,
        link: Some(link.unwrap_or_else(|| chat_link(&bot.id))),
        notification_type: NotificationType::System,
        source_run_id: Some(group_key(&bot.id)),
        source_node_id: Some(node),
    }
}

async fn live_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<Option<bot::Model>> {
    Ok(bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?)
}

async fn trigger_kind<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<Option<(InboxEntryKind, Option<String>)>> {
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::ConversationId.eq(turn.conversation_id.as_str()))
        .filter(inbox_entry::Column::MessageId.eq(turn.trigger_message_id.as_str()))
        .one(db)
        .await?;
    Ok(entry.and_then(|entry| {
        let kind = InboxEntryKind::parse(&entry.kind)?;
        let body = &entry.envelope["body"];
        let task = (body["source"].as_str() == Some("schedule"))
            .then(|| {
                body["ref"]
                    .as_str()
                    .or_else(|| body["ref_id"].as_str())
                    .map(str::to_string)
            })
            .flatten();
        Some((kind, task))
    }))
}

/// Notifies the owner of `reply.bot_id` when the reply answers something other than a person's
/// message. A repeat of the same reply notifies once. Returns whether a notification was sent.
pub(crate) async fn notify_for_reply<C: ConnectionTrait>(
    db: &C,
    sink: &dyn NoticeSink,
    reply: &Reply<'_>,
) -> Result<bool> {
    if reply.message_id.starts_with(FAILURE_REPLY_PREFIX) {
        return Ok(false);
    }
    let Some(turn) = bot_turn::Entity::find_by_id(reply.turn_id).one(db).await? else {
        return Ok(false);
    };
    if turn.bot_id != reply.bot_id {
        return Ok(false);
    }
    let posted = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(reply.bot_id))
        .filter(inbox_entry::Column::MessageId.eq(reply.message_id))
        .filter(inbox_entry::Column::SenderId.eq(reply.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&turn.conversation_id))
        .one(db)
        .await?;
    if posted.is_some_and(|entry| {
        entry.envelope["body"][flow_like::flow::copilot::decision_event::REPLY_DELIVERY_FIELD]
            .as_str()
            == Some("respond")
            && entry.envelope["body"].get("error").is_none()
            && entry.envelope["body"].get("approval").is_none()
    }) {
        return Ok(false);
    }
    let Some((kind, task_id)) = trigger_kind(db, &turn).await? else {
        return Ok(false);
    };
    let hops = u32::try_from(turn.hops).unwrap_or(0);
    if !reply.resumed && !reply_is_proactive(kind, hops) {
        return Ok(false);
    }
    let Some(bot) = live_bot(db, reply.bot_id).await? else {
        return Ok(false);
    };
    if bot.owner_id != turn.owner_id || notifications_muted(&bot.appearance) {
        return Ok(false);
    }
    let id = stable_id(&bot.id, reply.message_id);
    if notification::Entity::find_by_id(id.as_str())
        .one(db)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let body = Some(preview(reply.text))
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| NO_PREVIEW.to_string());
    let mut input = input_for(&bot, body, None, reply.message_id.to_string());
    if let Some(context) = &mut input.bot_context {
        context.conversation_id = Some(turn.conversation_id.clone());
        context.conversation_encryption = conversation::Entity::find_by_id(&turn.conversation_id)
            .one(db)
            .await?
            .and_then(|row| flow_like_types::inbox::ConversationEncryption::parse(&row.encryption))
            .map(|encryption| encryption.as_str().to_string());
        context.message_id = Some(reply.message_id.to_string());
        context.turn_id = Some(turn.id.clone());
        context.task_id = task_id;
    }
    sink.send(&id, input).await?;
    Ok(true)
}

/// The post-commit hook of a committed reply; failures are logged, never raised.
pub(crate) async fn after_reply(state: &AppState, reply: &Reply<'_>) {
    if let Err(error) = notify_for_reply(&state.db, &StateSink(state), reply).await {
        tracing::warn!(%error, bot_id = reply.bot_id, "Could not notify the owner of a bot's reply");
    }
}

/// `message_owner` for one bot on the server.
pub(crate) struct ServerNotifier {
    state: AppState,
    bot_id: String,
    site: Option<TurnSite>,
    turn_id: Option<String>,
}

impl ServerNotifier {
    pub(crate) fn new(state: &AppState, bot_id: &str, site: Option<TurnSite>) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_string(),
            site,
            turn_id: None,
        }
    }

    /// A message the turn repeats is the same message: same id, one post, one push.
    pub(crate) fn with_turn(mut self, turn_id: &str) -> Self {
        self.turn_id = Some(turn_id.to_string());
        self
    }
}

/// Writes a message into a conversation as a bot.
#[async_trait]
pub(crate) trait OwnerChat: Send + Sync {
    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()>;
}

pub(crate) struct StateChat<'a>(pub(crate) &'a AppState);

#[async_trait]
impl OwnerChat for StateChat<'_> {
    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let outgoing = message_outgoing(message_id, serde_json::json!({ "text": text, "hops": 0 }))
            .map_err(|error| anyhow!("Messaging the owner failed: {error}"))?;
        post_as_bot(self.0, conversation_id, bot_id, 0, outgoing)
            .await
            .map(|_| ())
            .map_err(|error| anyhow!("Messaging the owner failed: {error}"))
    }
}

/// Distinct messages the bot wrote to its owner since `since_ms`, muted or not.
pub(crate) async fn count_tool_messages<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    since_ms: i64,
) -> Result<usize> {
    let since = chrono::DateTime::from_timestamp_millis(since_ms)
        .ok_or_else(|| anyhow!("Bad window start {since_ms}"))?
        .fixed_offset();
    let rows = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::SenderId.eq(bot_id))
        .filter(inbox_entry::Column::MessageId.starts_with(OWNER_MESSAGE_PREFIX))
        .filter(inbox_entry::Column::CreatedAt.gte(since))
        .all(db)
        .await?;
    let distinct: HashSet<&str> = rows.iter().map(|row| row.message_id.as_str()).collect();
    Ok(distinct.len())
}

async fn in_owner_chat<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    site: Option<&TurnSite>,
) -> Result<bool> {
    match site {
        Some(TurnSite::OwnerLive) => Ok(true),
        Some(TurnSite::Conversation(id)) if !id.is_empty() => {
            is_owner_chat(db, owner_id, bot_id, id)
                .await
                .map_err(|error| anyhow!("{error}"))
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
pub(crate) async fn deliver_owner_message<C: ConnectionTrait>(
    db: &C,
    sink: &dyn NoticeSink,
    chat: &dyn OwnerChat,
    bot_id: &str,
    site: Option<&TurnSite>,
    notice: OwnerNotice,
) -> Result<NoticeReceipt> {
    deliver_owner_message_in_turn(db, sink, chat, bot_id, site, None, notice).await
}

fn owner_message_id(turn_id: Option<&str>, text: &str) -> String {
    match turn_id {
        Some(turn_id) => {
            let digest = blake3::hash(format!("{turn_id}\0{text}").as_bytes()).to_hex();
            format!("{OWNER_MESSAGE_PREFIX}{}", &digest[..32])
        }
        None => format!("{OWNER_MESSAGE_PREFIX}{}", create_id()),
    }
}

/// Writes the message into the bot's direct chat with its owner and pushes it once, unless the
/// turn runs in that chat or the owner reads it live. The chat is only the push when it is end-to-end encrypted.
/// Within one turn the same text is written and pushed once.
pub(crate) async fn deliver_owner_message_in_turn<C: ConnectionTrait>(
    db: &C,
    sink: &dyn NoticeSink,
    chat: &dyn OwnerChat,
    bot_id: &str,
    site: Option<&TurnSite>,
    turn_id: Option<&str>,
    notice: OwnerNotice,
) -> Result<NoticeReceipt> {
    let bot = live_bot(db, bot_id)
        .await?
        .ok_or_else(|| anyhow!("Bot '{bot_id}' no longer exists"))?;
    if in_owner_chat(db, &bot.owner_id, bot_id, site).await? {
        return Ok(NoticeReceipt {
            in_owner_chat: true,
            ..Default::default()
        });
    }
    let message_id = owner_message_id(turn_id, &notice.message());
    let conversation = open_conversation(db, &bot.owner_id, bot_id, "", now_millis())
        .await
        .map_err(|error| anyhow!("Opening the chat of bot '{bot_id}' failed: {error}"))?;
    let posted = match &conversation {
        Some(conversation_id) => {
            chat.post(conversation_id, bot_id, &message_id, &notice.message())
                .await?;
            true
        }
        None => false,
    };
    if notifications_muted(&bot.appearance) {
        return Ok(NoticeReceipt {
            id: message_id,
            muted: true,
            posted,
            ..Default::default()
        });
    }
    let node = format!("{TOOL_SOURCE}:{message_id}");
    let mut input = input_for(&bot, preview(&notice.push_body()), None, node);
    if let Some(context) = &mut input.bot_context {
        context.conversation_encryption = conversation.as_ref().map(|_| "plain".to_string());
        context.conversation_id = conversation;
        context.message_id = Some(message_id.clone());
    }
    sink.send(&stable_id(bot_id, &message_id), input).await?;
    Ok(NoticeReceipt {
        id: message_id,
        muted: false,
        posted,
        ..Default::default()
    })
}

#[async_trait]
impl OwnerNotifier for ServerNotifier {
    async fn sent_since(&self, since_ms: i64) -> Result<usize> {
        count_tool_messages(&self.state.db, &self.bot_id, since_ms).await
    }

    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
        deliver_owner_message_in_turn(
            &self.state.db,
            &StateSink(&self.state),
            &StateChat(&self.state),
            &self.bot_id,
            self.site.as_ref(),
            self.turn_id.as_deref(),
            notice,
        )
        .await
    }
}

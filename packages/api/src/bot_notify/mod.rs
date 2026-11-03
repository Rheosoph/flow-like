//! Notifying a bot's owner: replies the owner did not just ask for (a trigger's answer, a bot's
//! answer to another bot) and the `notify_owner` tool. Every notification is titled with the bot's
//! name, grouped under the bot (`source_run_id = bot:<id>`) and opens the bot's chat. The decision
//! rules live in `flow_like_types::bot_notify`; the desktop host applies the same ones.

use crate::{
    entity::{
        bot, bot_turn, inbox_entry, notification,
        sea_orm_active_enums::NotificationType,
    },
    push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent},
    state::AppState,
};
use async_trait::async_trait;
use flow_like_bot_tools::{NoticeReceipt, OwnerNotice, OwnerNotifier};
use flow_like_types::{
    Result, anyhow,
    bot_notify::{
        TOOL_SOURCE, chat_link, group_key, notifications_muted,
        preview, reply_is_proactive,
    },
    create_id,
    inbox::InboxEntryKind,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};

#[cfg(test)]
mod tests;

const FAILURE_REPLY_PREFIX: &str = "rf-";

#[async_trait]
pub(crate) trait NoticeSink: Send + Sync {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()>;
}

struct StateSink<'a>(&'a AppState);

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

fn display_name(bot: &bot::Model) -> String {
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

fn input_for(bot: &bot::Model, body: String, link: Option<String>, node: String) -> DispatchNotificationInput {
    DispatchNotificationInput {
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
) -> Result<Option<InboxEntryKind>> {
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::ConversationId.eq(turn.conversation_id.as_str()))
        .filter(inbox_entry::Column::MessageId.eq(turn.trigger_message_id.as_str()))
        .one(db)
        .await?;
    Ok(entry.and_then(|entry| InboxEntryKind::parse(&entry.kind)))
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
    let Some(kind) = trigger_kind(db, &turn).await? else {
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
    if notification::Entity::find_by_id(id.as_str()).one(db).await?.is_some() {
        return Ok(false);
    }
    let body = Some(preview(reply.text))
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| NO_PREVIEW.to_string());
    let input = input_for(&bot, body, None, reply.message_id.to_string());
    sink.send(&id, input).await?;
    Ok(true)
}

/// The post-commit hook of a committed reply; failures are logged, never raised.
pub(crate) async fn after_reply(state: &AppState, reply: &Reply<'_>) {
    if let Err(error) = notify_for_reply(&state.db, &StateSink(state), reply).await {
        tracing::warn!(%error, bot_id = reply.bot_id, "Could not notify the owner of a bot's reply");
    }
}

/// `notify_owner` for one bot on the server.
pub(crate) struct ServerNotifier {
    state: AppState,
    bot_id: String,
}

impl ServerNotifier {
    pub(crate) fn new(state: &AppState, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_string(),
        }
    }
}

pub(crate) async fn count_tool_notices<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    since_ms: i64,
) -> Result<usize> {
    let since = chrono::DateTime::from_timestamp_millis(since_ms)
        .ok_or_else(|| anyhow!("Bad window start {since_ms}"))?
        .fixed_offset();
    let count = notification::Entity::find()
        .filter(notification::Column::UserId.eq(owner_id))
        .filter(notification::Column::SourceRunId.eq(group_key(bot_id)))
        .filter(notification::Column::SourceNodeId.starts_with(TOOL_SOURCE))
        .filter(notification::Column::CreatedAt.gte(since))
        .count(db)
        .await?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

pub(crate) async fn deliver_tool_notice<C: ConnectionTrait>(
    db: &C,
    sink: &dyn NoticeSink,
    bot_id: &str,
    notice: OwnerNotice,
) -> Result<NoticeReceipt> {
    let bot = live_bot(db, bot_id)
        .await?
        .ok_or_else(|| anyhow!("Bot '{bot_id}' no longer exists"))?;
    if notifications_muted(&bot.appearance) {
        return Ok(NoticeReceipt {
            id: String::new(),
            muted: true,
        });
    }
    let id = create_id();
    let node = format!("{TOOL_SOURCE}:{id}");
    let text = format!("{}: {}", notice.title, notice.body);
    let input = input_for(&bot, text, notice.link, node);
    sink.send(&id, input).await?;
    Ok(NoticeReceipt { id, muted: false })
}

#[async_trait]
impl OwnerNotifier for ServerNotifier {
    async fn sent_since(&self, since_ms: i64) -> Result<usize> {
        let bot = live_bot(&self.state.db, &self.bot_id)
            .await?
            .ok_or_else(|| anyhow!("Bot '{}' no longer exists", self.bot_id))?;
        count_tool_notices(&self.state.db, &bot.owner_id, &self.bot_id, since_ms).await
    }

    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
        deliver_tool_notice(&self.state.db, &StateSink(&self.state), &self.bot_id, notice).await
    }
}


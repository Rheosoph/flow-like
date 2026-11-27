//! Guest rooms of outside chats: every Microsoft Teams conversation or Discord channel a bot is
//! part of has one group room of the owner and the bot, found again through its
//! `BotChannelLink`. The chat's messages are written into it as `channel` triggers from the
//! owner, naming the bot; the turn they queue answers the outside person, not the owner.

use super::{
    conversations::{NewConversation, normalize_title, start_group},
    entries::Sent,
    lock_conversation,
    members::Principal,
    triggers::write_trigger_in,
};
use crate::{
    db::coordination::coordinate,
    entity::{bot_channel_link, conversation_member},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_effective::BotRoomLimits,
    create_id,
    inbox::{ConversationEncryption, ConversationKind, PrincipalKind, TriggerBody},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set,
    sea_query::Expr,
};

/// Outside people reach no app of the owner through the bot until the owner allows apps for the
/// room; the bot answers only the messages written to it and keeps the room's knowledge to itself.
pub(crate) fn guest_limits() -> BotRoomLimits {
    BotRoomLimits {
        apps: Some(Vec::new()),
        mentioned_only: true,
        ..BotRoomLimits::default()
    }
}

/// The chat a message came from, as its link stores it.
pub(crate) struct ChannelChat {
    pub link_id: String,
    pub owner_id: String,
    pub bot_id: String,
    pub provider: String,
    pub chat_key: String,
    /// The room title, e.g. "Teams · Sales".
    pub room_title: String,
    pub chat_title: String,
    pub meta: String,
}

/// The chat's link with its room, both created on first use. A later message refreshes the
/// title, the provider data and when the chat was last seen. Serialized per link, so two first
/// messages of one chat open one room.
pub(crate) async fn open_channel_room_in(
    txn: &DatabaseTransaction,
    chat: &ChannelChat,
    now: DateTime<FixedOffset>,
) -> Result<bot_channel_link::Model, ApiError> {
    coordinate(txn, "bot-channel-link", &[chat.link_id.as_str()]).await?;
    if let Some(link) = bot_channel_link::Entity::find_by_id(&chat.link_id)
        .one(txn)
        .await?
    {
        let mut active: bot_channel_link::ActiveModel = link.into();
        active.chat_title = Set(chat.chat_title.clone());
        active.meta = Set(chat.meta.clone());
        active.last_seen_at = Set(now);
        return Ok(active.update(txn).await?);
    }
    let conversation_id = create_id();
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: normalize_title(Some(chat.room_title.clone()))?,
        members: vec![chat.bot_id.clone()],
        encryption: ConversationEncryption::Plain,
    };
    let bot = Principal {
        id: chat.bot_id.clone(),
        kind: PrincipalKind::Bot,
    };
    start_group(txn, &conversation_id, &chat.owner_id, &plan, &[bot], now).await?;
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Limits,
            Expr::value(serde_json::to_value(guest_limits()).unwrap_or_default()),
        )
        .filter(conversation_member::Column::ConversationId.eq(&conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(&chat.bot_id))
        .exec(txn)
        .await?;
    Ok(bot_channel_link::ActiveModel {
        id: Set(chat.link_id.clone()),
        bot_id: Set(chat.bot_id.clone()),
        owner_id: Set(chat.owner_id.clone()),
        provider: Set(chat.provider.clone()),
        chat_key: Set(chat.chat_key.clone()),
        chat_title: Set(chat.chat_title.clone()),
        conversation_id: Set(conversation_id),
        meta: Set(chat.meta.clone()),
        created_at: Set(now),
        last_seen_at: Set(now),
    }
    .insert(txn)
    .await?)
}

/// Writes the chat message into the link's room as a trigger from the owner, under the room's
/// lock. A repeat of `message_id` returns the first copy.
pub(crate) async fn post_channel_trigger_in(
    txn: &DatabaseTransaction,
    link: &bot_channel_link::Model,
    message_id: &str,
    body: &TriggerBody,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, &link.conversation_id).await?;
    write_trigger_in(
        txn,
        &link.conversation_id,
        &link.owner_id,
        message_id,
        body,
        now,
    )
    .await
}

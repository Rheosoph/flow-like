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
    entity::{bot_channel_link, bot_turn, conversation_member},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_turn::{STORED_QUEUED, turn_id},
    create_id,
    inbox::{ConversationEncryption, ConversationKind, PrincipalKind, TriggerBody},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    Set, sea_query::Expr,
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
    room_of_chat_in(txn, chat, now).await
}

async fn room_of_chat_in<C: ConnectionTrait>(
    txn: &C,
    chat: &ChannelChat,
    now: DateTime<FixedOffset>,
) -> Result<bot_channel_link::Model, ApiError> {
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
/// lock, and hands the turn it queues to `requester`, the outside person who wrote it. A repeat
/// of `message_id` returns the first copy.
pub(crate) async fn post_channel_trigger_in(
    txn: &DatabaseTransaction,
    link: &bot_channel_link::Model,
    message_id: &str,
    body: &TriggerBody,
    requester: &str,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, &link.conversation_id).await?;
    write_channel_trigger_in(txn, link, message_id, body, requester, now).await
}

/// The owner, the only person in the room, writes the trigger, so the ledger queues its turn with
/// the owner as requester; the turn is moved to the outside person before any host can claim it,
/// and the room rules for people other than the owner apply to it.
async fn write_channel_trigger_in<C: ConnectionTrait>(
    txn: &C,
    link: &bot_channel_link::Model,
    message_id: &str,
    body: &TriggerBody,
    requester: &str,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let sent = write_trigger_in(
        txn,
        &link.conversation_id,
        &link.owner_id,
        message_id,
        body,
        now,
    )
    .await?;
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::RequesterId, Expr::value(requester))
        .col_expr(bot_turn::Column::OriginId, Expr::value(requester))
        .filter(bot_turn::Column::Id.eq(turn_id(&link.bot_id, message_id)))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .exec(txn)
        .await?;
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::conversation,
        routes::inbox::fixture::{at, database, insert_bot},
    };
    use flow_like_types::{bot_turn::TurnAudience, inbox::TriggerSource};
    use sea_orm::{DatabaseConnection, Schema};

    async fn setup() -> DatabaseConnection {
        let db = database().await;
        let schema = Schema::new(db.get_database_backend());
        db.execute(&schema.create_table_from_entity(bot_channel_link::Entity))
            .await
            .unwrap();
        insert_bot(&db, "bot_c", "user-a", "SERVER").await;
        db
    }

    fn chat(title: &str) -> ChannelChat {
        ChannelChat {
            link_id: "chl_1".into(),
            owner_id: "user-a".into(),
            bot_id: "bot_c".into(),
            provider: "teams".into(),
            chat_key: "tenant:conversation".into(),
            room_title: format!("Teams · {title}"),
            chat_title: title.into(),
            meta: "{}".into(),
        }
    }

    fn message() -> TriggerBody {
        TriggerBody {
            monitoring: false,
            source: TriggerSource::Channel,
            ref_id: "chl_1".into(),
            title: "Ann on Microsoft Teams".into(),
            text: "Ann (Microsoft Teams): hello".into(),
            payload: None,
            payload_handle: None,
            bot_id: Some("bot_c".into()),
        }
    }

    #[tokio::test]
    async fn a_chat_keeps_one_guest_room_where_the_bot_reaches_no_app() {
        let db = setup().await;
        let first = room_of_chat_in(&db, &chat("Sales"), at(10)).await.unwrap();
        let again = room_of_chat_in(&db, &chat("Sales team"), at(20))
            .await
            .unwrap();
        assert_eq!(first.conversation_id, again.conversation_id);
        assert_eq!(again.chat_title, "Sales team");
        assert_eq!(again.last_seen_at, at(20));

        let room = conversation::Entity::find_by_id(&first.conversation_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ConversationKind::parse(&room.kind),
            Some(ConversationKind::Group)
        );
        assert_eq!(room.title.as_deref(), Some("Teams · Sales"));

        let members = conversation_member::Entity::find()
            .filter(conversation_member::Column::ConversationId.eq(&first.conversation_id))
            .all(&db)
            .await
            .unwrap();
        let mut ids: Vec<&str> = members.iter().map(|m| m.principal_id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["bot_c", "user-a"]);
        let limits = members
            .iter()
            .find(|member| member.principal_id == "bot_c")
            .and_then(|member| member.limits.clone())
            .map(|limits| serde_json::from_value::<BotRoomLimits>(limits).unwrap());
        assert_eq!(limits, Some(guest_limits()));
    }

    #[tokio::test]
    async fn a_chat_message_queues_one_turn_that_answers_the_outside_person() {
        let db = setup().await;
        let link = room_of_chat_in(&db, &chat("Sales"), at(10)).await.unwrap();
        for when in [11, 12] {
            write_channel_trigger_in(&db, &link, "ch-1", &message(), "ext:teams:ann", at(when))
                .await
                .unwrap();
        }
        let turns = bot_turn::Entity::find()
            .filter(bot_turn::Column::BotId.eq("bot_c"))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(turns.len(), 1);
        let turn = &turns[0];
        assert_eq!(turn.conversation_id, link.conversation_id);
        assert_eq!(turn.requester_id.as_deref(), Some("ext:teams:ann"));
        assert_eq!(turn.origin_id.as_deref(), Some("ext:teams:ann"));
        assert_eq!(turn.audience, TurnAudience::Room.stored());
    }
}

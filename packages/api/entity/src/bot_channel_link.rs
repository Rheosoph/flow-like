//! A Microsoft Teams or Discord chat a bot is part of, with the guest room its messages go to.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotChannelLink")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_name = "botId", column_type = "Text")]
    pub bot_id: String,
    #[sea_orm(column_name = "ownerId", column_type = "Text")]
    pub owner_id: String,
    #[sea_orm(column_type = "Text")]
    pub provider: String,
    #[sea_orm(column_name = "chatKey", column_type = "Text")]
    pub chat_key: String,
    #[sea_orm(column_name = "chatTitle", column_type = "Text")]
    pub chat_title: String,
    #[sea_orm(column_name = "conversationId", column_type = "Text")]
    pub conversation_id: String,
    #[sea_orm(column_type = "Text")]
    pub meta: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "lastSeenAt")]
    pub last_seen_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

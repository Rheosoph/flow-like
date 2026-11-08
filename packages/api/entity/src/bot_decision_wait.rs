//! A bot's pending room decision, separate from approval turns and semantic memory.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotDecisionWait")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "botId")]
    pub bot_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "conversationId")]
    pub conversation_id: String,
    #[sea_orm(column_name = "participantId")]
    pub participant_id: String,
    #[sea_orm(column_name = "triggerMessageId")]
    pub trigger_message_id: String,
    #[sea_orm(column_name = "afterSeq")]
    pub after_seq: i64,
    #[sea_orm(column_name = "expiresAt")]
    pub expires_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

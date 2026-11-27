//! A person's chat with another owner's bot of an org they share (`BotChat` in the contracts
//! crate). Status is stored uppercase.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeamChat")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "botId")]
    pub bot_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "userId")]
    pub user_id: String,
    #[sea_orm(column_name = "ownerId")]
    pub owner_id: String,
    pub status: String,
    #[sea_orm(column_name = "conversationId", unique)]
    pub conversation_id: Option<String>,
    pub limits: Option<Json>,
    pub note: Option<String>,
    #[sea_orm(column_name = "requestedAt")]
    pub requested_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "decidedAt")]
    pub decided_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_name = "endedAt")]
    pub ended_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

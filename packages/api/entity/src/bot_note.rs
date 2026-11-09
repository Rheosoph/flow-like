//! A bot's keyed working note in one conversation (`bot_notes` in the contracts crate).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotNote")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "botId")]
    pub bot_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "conversationId")]
    pub conversation_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub title: String,
    pub status: Option<String>,
    pub body: String,
    pub state: String,
    pub version: i32,
    pub classification: Option<String>,
    pub pii: bool,
    #[sea_orm(column_name = "updatedBy")]
    pub updated_by: String,
    pub history: Json,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

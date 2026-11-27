//! A team of bots and people (`bot_team` in the contracts crate). Teams nest through `parent_id`.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeam")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    #[sea_orm(column_name = "parentId")]
    pub parent_id: Option<String>,
    #[sea_orm(column_name = "leadBotId")]
    pub lead_bot_id: Option<String>,
    #[sea_orm(column_name = "leadOfferBotId")]
    pub lead_offer_bot_id: Option<String>,
    #[sea_orm(column_name = "leadOfferBy")]
    pub lead_offer_by: Option<String>,
    #[sea_orm(column_name = "leadOfferAt")]
    pub lead_offer_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_name = "directAsks")]
    pub direct_asks: bool,
    #[sea_orm(column_name = "conversationId", unique)]
    pub conversation_id: Option<String>,
    #[sea_orm(column_name = "createdBy")]
    pub created_by: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

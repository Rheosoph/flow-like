//! An app pinned to a bot team: `PENDING` until a team admin approves a member's pin.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeamApp")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "teamId")]
    pub team_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "appId")]
    pub app_id: String,
    pub status: String,
    #[sea_orm(column_name = "pinnedBy")]
    pub pinned_by: String,
    #[sea_orm(column_name = "approvedBy")]
    pub approved_by: Option<String>,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "approvedAt")]
    pub approved_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

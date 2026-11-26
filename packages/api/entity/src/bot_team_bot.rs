//! A bot in a bot team: `PENDING` until its owner accepts, active in at most one team.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeamBot")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "teamId")]
    pub team_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "botId")]
    pub bot_id: String,
    pub status: String,
    #[sea_orm(column_name = "directAgreed")]
    pub direct_agreed: bool,
    pub limits: Option<Json>,
    #[sea_orm(column_name = "invitedBy")]
    pub invited_by: Option<String>,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "acceptedAt")]
    pub accepted_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

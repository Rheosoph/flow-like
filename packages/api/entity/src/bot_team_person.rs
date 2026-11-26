//! A person in a bot team: `ADMIN` or `MEMBER`, `PENDING` until they accept.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeamPerson")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "teamId")]
    pub team_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "userId")]
    pub user_id: String,
    pub role: String,
    pub status: String,
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

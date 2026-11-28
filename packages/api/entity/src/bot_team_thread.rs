//! A person's thread with a bot team: where their `#Team` requests go and the team's lead answers.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTeamThread")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "teamId")]
    pub team_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "userId")]
    pub user_id: String,
    #[sea_orm(column_name = "conversationId", unique)]
    pub conversation_id: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

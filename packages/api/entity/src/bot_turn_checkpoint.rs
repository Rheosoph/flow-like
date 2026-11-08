//! The last durable execution boundary of a leased bot turn.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotTurnCheckpoint")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "turnId")]
    pub turn_id: String,
    pub revision: i64,
    #[sea_orm(column_name = "claimAttempt")]
    pub claim_attempt: i32,
    #[sea_orm(column_name = "recoveryAttempts")]
    pub recovery_attempts: i32,
    #[sea_orm(column_name = "startedAtMs")]
    pub started_at_ms: i64,
    #[sea_orm(column_name = "deadlineAtMs")]
    pub deadline_at_ms: i64,
    pub state: Json,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::bot_turn::Entity",
        from = "Column::TurnId",
        to = "super::bot_turn::Column::Id",
        on_delete = "Cascade",
        on_update = "Cascade"
    )]
    BotTurn,
}

impl Related<super::bot_turn::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::BotTurn.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

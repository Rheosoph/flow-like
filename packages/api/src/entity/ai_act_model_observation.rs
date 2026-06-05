//! `SeaORM` Entity for a live attached-model observation (EU AI Act).

use super::sea_orm_active_enums::AiGpaiPosture;
use super::sea_orm_active_enums::AiModelSource;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "AiActModelObservation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_name = "appId", column_type = "Text")]
    pub app_id: String,
    #[sea_orm(column_name = "modelId", column_type = "Text")]
    pub model_id: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub provider: Option<String>,
    pub source: AiModelSource,
    pub posture: AiGpaiPosture,
    pub hosted: bool,
    #[sea_orm(column_name = "openLicence")]
    pub open_licence: bool,
    #[sea_orm(column_name = "systemicRisk")]
    pub systemic_risk: bool,
    pub vetted: bool,
    #[sea_orm(column_name = "dynamicSelector")]
    pub dynamic_selector: bool,
    #[sea_orm(column_name = "driftFlagged")]
    pub drift_flagged: bool,
    #[sea_orm(column_name = "firstSeenAt")]
    pub first_seen_at: DateTime,
    #[sea_orm(column_name = "lastSeenAt")]
    pub last_seen_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::app::Entity",
        from = "Column::AppId",
        to = "super::app::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    App,
}

impl Related<super::app::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::App.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

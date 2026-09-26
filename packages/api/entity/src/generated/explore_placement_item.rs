use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "ExplorePlacementItem")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub edition: String,
    #[sea_orm(
        column_name = "placementId",
        primary_key,
        auto_increment = false,
        column_type = "Text"
    )]
    pub placement_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub position: i32,
    #[sea_orm(column_name = "itemKind", column_type = "Text")]
    pub item_kind: crate::sea_orm_active_enums::ExploreItemKind,
    #[sea_orm(column_name = "itemId", column_type = "Text")]
    pub item_id: String,
    #[sea_orm(column_type = "JsonBinary")]
    pub overrides: Json,
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "ExplorePlacement")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub edition: String,
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_name = "slotKey", column_type = "Text")]
    pub slot_key: String,
    pub position: i32,
    #[sea_orm(column_type = "Text")]
    pub kind: crate::sea_orm_active_enums::ExplorePlacementKind,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    pub enabled: bool,
    #[sea_orm(column_name = "startsAt")]
    pub starts_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_name = "endsAt")]
    pub ends_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_type = "JsonBinary")]
    pub audience: crate::json_types::StringList,
    #[sea_orm(column_type = "JsonBinary")]
    pub content: Json,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "LockedItem")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_name = "appId", column_type = "Text")]
    pub app_id: String,
    #[sea_orm(column_type = "Text")]
    pub scope: crate::sea_orm_active_enums::LockedItemScope,
    #[sea_orm(column_name = "ownerId", column_type = "Text", nullable)]
    pub owner_id: Option<String>,
    #[sea_orm(column_name = "pathKey", column_type = "Text")]
    pub path_key: String,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub folder: String,
    #[sea_orm(column_name = "contentType", column_type = "Text", nullable)]
    pub content_type: Option<String>,
    #[sea_orm(column_name = "latestVersion")]
    pub latest_version: i32,
    #[sea_orm(column_name = "latestSize")]
    pub latest_size: i64,
    #[sea_orm(column_name = "liveVersions")]
    pub live_versions: i32,
    #[sea_orm(column_name = "liveSize")]
    pub live_size: i64,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub metadata: Option<Json>,
    #[sea_orm(column_name = "createdBy", column_type = "Text")]
    pub created_by: String,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

impl ActiveModelBehavior for ActiveModel {}

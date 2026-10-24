use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "LockedItemVersion")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_name = "itemId", column_type = "Text")]
    pub item_id: String,
    #[sea_orm(column_name = "appId", column_type = "Text")]
    pub app_id: String,
    pub version: Option<i32>,
    #[sea_orm(column_type = "Text")]
    pub status: crate::sea_orm_active_enums::LockedVersionStatus,
    #[sea_orm(column_name = "objectKey", column_type = "Text")]
    pub object_key: String,
    #[sea_orm(column_name = "providerVersionId", column_type = "Text", nullable)]
    pub provider_version_id: Option<String>,
    #[sea_orm(column_name = "fileName", column_type = "Text")]
    pub file_name: String,
    #[sea_orm(column_name = "contentType", column_type = "Text")]
    pub content_type: String,
    pub size: i64,
    #[sea_orm(column_type = "Text")]
    pub sha256: String,
    #[sea_orm(column_name = "checksumVerified")]
    pub checksum_verified: bool,
    #[sea_orm(column_name = "retentionMode", column_type = "Text")]
    pub retention_mode: crate::sea_orm_active_enums::LockedRetentionMode,
    #[sea_orm(column_name = "retainUntil")]
    pub retain_until: DateTimeWithTimeZone,
    #[sea_orm(column_type = "Text")]
    pub enforcement: crate::sea_orm_active_enums::LockedEnforcement,
    #[sea_orm(column_name = "legalHold")]
    pub legal_hold: bool,
    #[sea_orm(column_name = "deleteAfterRetention")]
    pub delete_after_retention: Option<bool>,
    #[sea_orm(column_type = "Text", nullable)]
    pub comment: Option<String>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub metadata: Option<Json>,
    #[sea_orm(column_name = "uploadedBy", column_type = "Text")]
    pub uploaded_by: String,
    #[sea_orm(column_name = "uploadExpiresAt")]
    pub upload_expires_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "createdAt")]
    pub created_at: DateTimeWithTimeZone,
    #[sea_orm(column_name = "lockedAt")]
    pub locked_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_name = "purgedAt")]
    pub purged_at: Option<DateTimeWithTimeZone>,
    #[sea_orm(column_name = "purgedBy", column_type = "Text", nullable)]
    pub purged_by: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

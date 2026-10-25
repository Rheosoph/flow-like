use super::config::LockedProvider;
use crate::entity::sea_orm_active_enums::{
    LockedEnforcement, LockedItemScope, LockedRetentionMode, LockedVersionStatus,
};
use crate::entity::{locked_item, locked_item_version};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// What this deployment's locked storage can do.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedStorageStatus {
    pub enabled: bool,
    pub provider: Option<LockedProvider>,
    #[schema(value_type = Option<String>, example = "object")]
    pub enforcement: Option<LockedEnforcement>,
    #[schema(value_type = String, example = "compliance")]
    pub retention_mode: LockedRetentionMode,
    pub default_retention_days: u32,
    pub min_retention_days: u32,
    pub max_retention_days: u32,
    pub max_file_bytes: u64,
    pub supports_legal_hold: bool,
    pub supports_retention_extension: bool,
}

/// One immutable stored version of a locked file.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedItemVersion {
    pub id: String,
    pub item_id: String,
    /// Assigned when the upload is committed.
    pub version: Option<i32>,
    #[schema(value_type = String, example = "locked")]
    pub status: LockedVersionStatus,
    pub file_name: String,
    pub content_type: String,
    pub size: i64,
    /// Lowercase hex SHA-256 of the stored bytes.
    pub sha256: String,
    /// Whether the store or the API confirmed the bytes match `sha256`.
    pub checksum_verified: bool,
    pub provider_version_id: Option<String>,
    #[schema(value_type = String, example = "compliance")]
    pub retention_mode: LockedRetentionMode,
    /// Until then the version can be neither changed nor deleted.
    pub retain_until: DateTime<Utc>,
    #[schema(value_type = String, example = "object")]
    pub enforcement: LockedEnforcement,
    pub legal_hold: bool,
    /// Purged automatically once retention ends and no legal hold remains.
    pub delete_after_retention: bool,
    pub comment: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
    pub uploaded_by: String,
    pub created_at: DateTime<Utc>,
    pub locked_at: Option<DateTime<Utc>>,
    pub purged_at: Option<DateTime<Utc>>,
    pub purged_by: Option<String>,
    /// Locked, retention ended and no legal hold: the bytes may be deleted.
    pub purgeable: bool,
}

/// A locked file: the record every version of it belongs to.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedItem {
    pub id: String,
    pub app_id: String,
    #[schema(value_type = String, example = "project")]
    pub scope: LockedItemScope,
    pub owner_id: Option<String>,
    pub name: String,
    pub folder: String,
    pub content_type: Option<String>,
    pub latest_version: i32,
    pub latest_size: i64,
    /// Versions whose bytes are still stored.
    pub live_versions: i32,
    /// Bytes counted against the storage quota.
    pub live_size: i64,
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The newest stored version.
    pub latest: Option<LockedItemVersion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedItemDetail {
    pub item: LockedItem,
    /// Newest first. Pending uploads appear only for their uploader and admins.
    pub versions: Vec<LockedItemVersion>,
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct ListLockedItemsRequest {
    #[schema(value_type = Option<String>, example = "project")]
    pub scope: Option<LockedItemScope>,
    pub folder: Option<String>,
    #[serde(default)]
    pub recursive: bool,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ListLockedItemsResponse {
    pub items: Vec<LockedItem>,
    /// Immediate child folders of the listed folder (first page of a non-recursive listing).
    pub folders: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct GetLockedItemRequest {
    pub item_id: Option<String>,
    #[schema(value_type = Option<String>, example = "project")]
    pub scope: Option<LockedItemScope>,
    pub folder: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IfExists {
    #[default]
    Error,
    NewVersion,
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct CreateLockedUploadRequest {
    /// Set to upload a new version of this item.
    pub item_id: Option<String>,
    #[schema(value_type = Option<String>, example = "project")]
    pub scope: Option<LockedItemScope>,
    pub folder: Option<String>,
    /// Display name of a new item; defaults to `file_name`.
    pub name: Option<String>,
    pub file_name: String,
    pub content_type: Option<String>,
    pub size: u64,
    /// Lowercase hex SHA-256 of the exact bytes that will be uploaded.
    pub sha256: String,
    pub retention_days: Option<u32>,
    /// Purge this version automatically once its retention ends. Project files need
    /// an admin, as for any purge.
    #[serde(default)]
    pub delete_after_retention: bool,
    pub comment: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub if_exists: IfExists,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedUploadTarget {
    pub url: String,
    pub method: String,
    /// Send exactly these headers; they are part of the signature.
    pub headers: BTreeMap<String, String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedUploadTicket {
    pub item: LockedItem,
    pub version: LockedItemVersion,
    pub upload: LockedUploadTarget,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct LockedVersionRef {
    pub item_id: String,
    pub version_id: String,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct DownloadLockedItemRequest {
    pub item_id: String,
    /// Version number; the newest stored version when omitted.
    pub version: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LockedDownload {
    pub item_id: String,
    pub version_id: String,
    pub version: i32,
    pub file_name: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ExtendLockedRetentionRequest {
    pub item_id: String,
    pub version_id: String,
    pub retain_until: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct SetLockedLegalHoldRequest {
    pub item_id: String,
    pub version_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct SetLockedAutoDeleteRequest {
    pub item_id: String,
    pub version_id: String,
    /// Purge the version automatically once its retention ends and no legal hold remains.
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct PurgeLockedItemRequest {
    pub item_id: String,
    /// Purge one version; every expired version of the item when omitted.
    pub version_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct LockedEmpty {}

impl LockedItemVersion {
    pub fn from_model(model: locked_item_version::Model, now: DateTime<Utc>) -> Self {
        let retain_until = model.retain_until.with_timezone(&Utc);
        Self {
            purgeable: model.status == LockedVersionStatus::Locked
                && !model.legal_hold
                && retain_until <= now,
            id: model.id,
            item_id: model.item_id,
            version: model.version,
            status: model.status,
            file_name: model.file_name,
            content_type: model.content_type,
            size: model.size,
            sha256: model.sha256,
            checksum_verified: model.checksum_verified,
            provider_version_id: model.provider_version_id,
            retention_mode: model.retention_mode,
            retain_until,
            enforcement: model.enforcement,
            legal_hold: model.legal_hold,
            delete_after_retention: model.delete_after_retention.unwrap_or(false),
            comment: model.comment,
            metadata: model.metadata,
            uploaded_by: model.uploaded_by,
            created_at: model.created_at.with_timezone(&Utc),
            locked_at: model.locked_at.map(|at| at.with_timezone(&Utc)),
            purged_at: model.purged_at.map(|at| at.with_timezone(&Utc)),
            purged_by: model.purged_by,
        }
    }
}

impl LockedItem {
    pub fn from_model(model: locked_item::Model, latest: Option<LockedItemVersion>) -> Self {
        Self {
            id: model.id,
            app_id: model.app_id,
            scope: model.scope,
            owner_id: model.owner_id,
            name: model.name,
            folder: model.folder,
            content_type: model.content_type,
            latest_version: model.latest_version,
            latest_size: model.latest_size,
            live_versions: model.live_versions,
            live_size: model.live_size,
            metadata: model.metadata,
            created_by: model.created_by,
            created_at: model.created_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
            latest,
        }
    }
}

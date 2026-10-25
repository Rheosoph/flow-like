use flow_like_types::{Value, json::Map};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type LockedMetadata = Map<String, Value>;

/// Who a locked file belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LockedScope {
    /// Shared by the whole project.
    #[default]
    Project,
    /// Private to one user.
    User,
}

/// How strictly the retention lock is enforced by the storage provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LockedRetentionMode {
    /// Privileged storage administrators could lift the lock.
    Governance,
    /// Nobody, including the storage account owner, can lift the lock before it ends.
    Compliance,
}

/// Where the write-once guarantee is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LockedEnforcement {
    /// A retention lock set on the stored object version itself.
    Object,
    /// A retention policy of the whole bucket or container.
    Bucket,
}

/// Lifecycle state of a locked file version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LockedVersionStatus {
    /// Upload started but not committed yet.
    Pending,
    /// Committed and protected by its retention lock.
    Locked,
    /// Bytes deleted after the retention ended; the record is kept as evidence.
    Purged,
}

impl LockedVersionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Locked => "locked",
            Self::Purged => "purged",
        }
    }
}

/// One immutable, write-once version of a locked file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LockedItemVersion {
    /// Version ID, used to download, extend, hold or purge this exact version.
    pub id: String,
    /// ID of the locked file this version belongs to.
    pub item_id: String,
    /// Version number starting at 1, assigned at commit; null while pending.
    pub version: Option<u32>,
    /// Lifecycle state: pending, locked or purged.
    pub status: LockedVersionStatus,
    /// File name the bytes were uploaded as.
    pub file_name: String,
    /// MIME type of the stored bytes.
    pub content_type: String,
    /// Size of the stored bytes.
    pub size: u64,
    /// Lowercase hex SHA-256 of the stored bytes.
    pub sha256: String,
    /// Whether the storage provider or the server confirmed the SHA-256 of the stored bytes.
    pub checksum_verified: bool,
    /// Storage provider's version identifier (S3 VersionId, GCS generation or Azure version ID).
    pub provider_version_id: Option<String>,
    /// Retention mode: governance or compliance.
    pub retention_mode: LockedRetentionMode,
    /// Date until which the object cannot be modified or deleted (RFC 3339).
    pub retain_until: String,
    /// Whether the lock is set on the object or by a bucket-wide policy.
    pub enforcement: LockedEnforcement,
    /// Whether a legal hold blocks deletion regardless of the retention date.
    pub legal_hold: bool,
    /// Whether the server deletes the bytes automatically once the retention ends and no legal hold remains.
    #[serde(default)]
    pub delete_after_retention: bool,
    /// Comment given at upload.
    pub comment: Option<String>,
    /// Metadata given at upload.
    pub metadata: Option<LockedMetadata>,
    /// User ID of the uploader.
    pub uploaded_by: String,
    /// When the upload was started (RFC 3339).
    pub created_at: String,
    /// When the version was committed and locked (RFC 3339); null while pending.
    pub locked_at: Option<String>,
    /// When the stored bytes were deleted after the retention ended (RFC 3339).
    pub purged_at: Option<String>,
    /// User ID of whoever deleted the stored bytes.
    pub purged_by: Option<String>,
    /// Whether the bytes may be deleted now: locked, retention ended and no legal hold.
    #[serde(default)]
    pub purgeable: bool,
}

/// A locked file as the server indexes it: its identity plus its newest locked version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LockedItem {
    /// Locked file ID.
    pub id: String,
    /// App the file belongs to.
    pub app_id: String,
    /// Whether the file is shared by the project or private to a user.
    pub scope: LockedScope,
    /// Owning user for user-scoped files; null for project files.
    pub owner_id: Option<String>,
    /// Display name, unique within its folder.
    pub name: String,
    /// Virtual folder without leading or trailing slash; empty for the root.
    pub folder: String,
    /// MIME type of the newest version.
    pub content_type: Option<String>,
    /// Highest committed version number; 0 before the first commit.
    pub latest_version: u32,
    /// Size of the newest stored version.
    pub latest_size: u64,
    /// Number of committed versions whose bytes are still stored.
    #[serde(default)]
    pub live_versions: u32,
    /// Bytes of all stored versions, charged to the storage quota.
    #[serde(default)]
    pub live_size: u64,
    /// Metadata of the file.
    pub metadata: Option<LockedMetadata>,
    /// User ID of whoever created the file.
    pub created_by: String,
    /// When the file was created (RFC 3339).
    pub created_at: String,
    /// When the file last changed (RFC 3339).
    pub updated_at: String,
    /// Newest locked version.
    pub latest: Option<LockedItemVersion>,
}

/// A locked file with every version the caller may see, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LockedItemDetail {
    pub item: LockedItem,
    pub versions: Vec<LockedItemVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LockedUploadTarget {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LockedUploadTicket {
    pub item: LockedItem,
    pub version: LockedItemVersion,
    pub upload: LockedUploadTarget,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LockedDownload {
    pub version_id: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

/// Locked storage configuration of the server.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LockedStorageStatus {
    /// Whether the server has locked storage configured.
    pub enabled: bool,
    /// Storage provider: s3, r2, gcs or azure; null when disabled.
    pub provider: Option<String>,
    /// Whether locks are set per object or by a bucket-wide policy; null when disabled.
    pub enforcement: Option<LockedEnforcement>,
    /// Retention mode applied to new versions.
    pub retention_mode: LockedRetentionMode,
    /// Retention in days applied when an upload asks for none.
    pub default_retention_days: u32,
    /// Shortest retention in days an upload may ask for.
    pub min_retention_days: u32,
    /// Longest retention in days an upload may ask for.
    pub max_retention_days: u32,
    /// Largest size of one version.
    pub max_file_bytes: u64,
    /// Whether versions can be put under legal hold.
    pub supports_legal_hold: bool,
    /// Whether the retention of a locked version can be extended.
    pub supports_retention_extension: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListLockedItemsResponse {
    pub items: Vec<LockedItem>,
    #[serde(default)]
    pub folders: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListLockedItemsRequest<'a> {
    pub scope: LockedScope,
    pub folder: &'a str,
    pub recursive: bool,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub search: &'a str,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<&'a str>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GetLockedItemRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<LockedScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockedIfExists {
    #[default]
    Error,
    NewVersion,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateLockedUploadRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<LockedScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub file_name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub delete_after_retention: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LockedMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_exists: Option<LockedIfExists>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockedVersionRef<'a> {
    pub item_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockedDownloadRequest<'a> {
    pub item_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockedRetentionRequest<'a> {
    pub item_id: &'a str,
    pub version_id: &'a str,
    pub retain_until: &'a str,
}

/// Switches a per-version flag: the legal hold or automatic deletion.
#[derive(Debug, Clone, Serialize)]
pub struct LockedVersionFlagRequest<'a> {
    pub item_id: &'a str,
    pub version_id: &'a str,
    pub enabled: bool,
}

/// A locked (write-once, revision-safe) file: the file's identity together with one of its
/// versions, flattened into a single record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LockedFile {
    /// Locked file ID, used by the other Locked File nodes.
    pub item_id: String,
    /// App the file belongs to.
    pub app_id: String,
    /// Whether the file is shared by the project or private to a user.
    pub scope: LockedScope,
    /// Owning user for user-scoped files; null for project files.
    pub owner_id: Option<String>,
    /// Display name, unique within its folder.
    pub name: String,
    /// Virtual folder without leading or trailing slash; empty for the root.
    pub folder: String,
    /// Folder and name joined with "/", without a leading slash.
    pub path: String,
    /// File name the bytes of this version were uploaded as.
    pub file_name: String,
    /// MIME type of this version.
    pub content_type: Option<String>,
    /// Size of this version.
    pub size: u64,
    /// Version number starting at 1; null while pending or when the file has no version.
    pub version: Option<u32>,
    /// Version ID, used to download, extend, hold or purge this exact version.
    pub version_id: Option<String>,
    /// Lifecycle state of this version: pending, locked or purged.
    pub status: Option<LockedVersionStatus>,
    /// Lowercase hex SHA-256 of the stored bytes.
    pub sha256: Option<String>,
    /// Whether the storage provider or the server confirmed the SHA-256 of the stored bytes.
    pub checksum_verified: bool,
    /// Storage provider's version identifier (S3 VersionId, GCS generation or Azure version ID).
    pub provider_version_id: Option<String>,
    /// Retention mode: governance or compliance.
    pub retention_mode: Option<LockedRetentionMode>,
    /// Date until which the object cannot be modified or deleted (RFC 3339).
    pub retain_until: Option<String>,
    /// Whether the lock is set on the object or by a bucket-wide policy.
    pub enforcement: Option<LockedEnforcement>,
    /// Whether a legal hold blocks deletion regardless of the retention date.
    pub legal_hold: bool,
    /// Whether the server deletes this version's bytes automatically once the retention ends and no legal hold remains.
    pub delete_after_retention: bool,
    /// Whether this version's bytes may be deleted now: locked, retention ended and no legal hold.
    pub purgeable: bool,
    /// Comment given when this version was uploaded.
    pub comment: Option<String>,
    /// Metadata of the file.
    pub metadata: Option<LockedMetadata>,
    /// Metadata given when this version was uploaded.
    pub version_metadata: Option<LockedMetadata>,
    /// User ID of whoever uploaded this version.
    pub uploaded_by: Option<String>,
    /// When this version was committed and locked (RFC 3339).
    pub locked_at: Option<String>,
    /// When this version's bytes were deleted after the retention ended (RFC 3339).
    pub purged_at: Option<String>,
    /// Highest committed version number of the file.
    pub latest_version: u32,
    /// Number of committed versions whose bytes are still stored.
    pub live_versions: u32,
    /// Bytes of all stored versions, charged to the storage quota.
    pub live_size: u64,
    /// User ID of whoever created the file.
    pub created_by: String,
    /// When the file was created (RFC 3339).
    pub created_at: String,
    /// When the file last changed (RFC 3339).
    pub updated_at: String,
}

pub fn locked_path(folder: &str, name: &str) -> String {
    match folder.trim_matches('/') {
        "" => name.to_owned(),
        folder => format!("{folder}/{name}"),
    }
}

impl LockedFile {
    pub fn new(item: &LockedItem, version: Option<&LockedItemVersion>) -> Self {
        Self {
            item_id: item.id.clone(),
            app_id: item.app_id.clone(),
            scope: item.scope,
            owner_id: item.owner_id.clone(),
            name: item.name.clone(),
            folder: item.folder.clone(),
            path: locked_path(&item.folder, &item.name),
            file_name: version.map_or_else(|| item.name.clone(), |v| v.file_name.clone()),
            content_type: version
                .map(|v| v.content_type.clone())
                .or_else(|| item.content_type.clone()),
            size: version.map_or(item.latest_size, |v| v.size),
            version: version.and_then(|v| v.version),
            version_id: version.map(|v| v.id.clone()),
            status: version.map(|v| v.status),
            sha256: version.map(|v| v.sha256.clone()),
            checksum_verified: version.is_some_and(|v| v.checksum_verified),
            provider_version_id: version.and_then(|v| v.provider_version_id.clone()),
            retention_mode: version.map(|v| v.retention_mode),
            retain_until: version.map(|v| v.retain_until.clone()),
            enforcement: version.map(|v| v.enforcement),
            legal_hold: version.is_some_and(|v| v.legal_hold),
            delete_after_retention: version.is_some_and(|v| v.delete_after_retention),
            purgeable: version.is_some_and(|v| v.purgeable),
            comment: version.and_then(|v| v.comment.clone()),
            metadata: item.metadata.clone(),
            version_metadata: version.and_then(|v| v.metadata.clone()),
            uploaded_by: version.map(|v| v.uploaded_by.clone()),
            locked_at: version.and_then(|v| v.locked_at.clone()),
            purged_at: version.and_then(|v| v.purged_at.clone()),
            latest_version: item.latest_version,
            live_versions: item.live_versions,
            live_size: item.live_size,
            created_by: item.created_by.clone(),
            created_at: item.created_at.clone(),
            updated_at: item.updated_at.clone(),
        }
    }
}

impl From<&LockedItem> for LockedFile {
    fn from(item: &LockedItem) -> Self {
        Self::new(item, item.latest.as_ref())
    }
}

impl From<&LockedItemDetail> for LockedFile {
    fn from(detail: &LockedItemDetail) -> Self {
        let version = detail.item.latest.as_ref().or(detail.versions.first());
        Self::new(&detail.item, version)
    }
}

impl LockedItemDetail {
    pub fn version_by_id(&self, version_id: &str) -> Option<&LockedItemVersion> {
        self.versions
            .iter()
            .find(|version| version.id == version_id)
    }

    pub fn version_by_number(&self, number: u32) -> Option<&LockedItemVersion> {
        self.versions
            .iter()
            .find(|version| version.version == Some(number))
    }

    pub fn latest_locked(&self) -> Option<&LockedItemVersion> {
        self.item.latest.as_ref().or_else(|| {
            self.versions
                .iter()
                .find(|version| version.status == LockedVersionStatus::Locked)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_value, json};

    fn version(number: u32, status: &str) -> Value {
        json!({
            "id": format!("v{number}"), "item_id": "item-1", "version": number, "status": status,
            "file_name": "invoice.pdf", "content_type": "application/pdf", "size": 1024,
            "sha256": "ab".repeat(32), "checksum_verified": true, "provider_version_id": "pv",
            "retention_mode": "compliance", "retain_until": "2036-10-21T00:00:00Z",
            "enforcement": "object", "legal_hold": false, "comment": null, "metadata": null,
            "uploaded_by": "user-1", "created_at": "2026-10-21T00:00:00Z",
            "locked_at": "2026-10-21T00:00:01Z", "purged_at": null, "purged_by": null,
            "purgeable": false,
        })
    }

    fn detail(latest: Value) -> LockedItemDetail {
        from_value(json!({
            "item": {
                "id": "item-1", "app_id": "app-1", "scope": "project", "owner_id": null,
                "name": "Invoice", "folder": "invoices/2026", "content_type": "application/pdf",
                "latest_version": 2, "latest_size": 1024, "live_versions": 2, "live_size": 2048,
                "metadata": { "customer": "ACME" }, "created_by": "user-1",
                "created_at": "2026-10-20T00:00:00Z", "updated_at": "2026-10-21T00:00:01Z",
                "latest": latest, "added_later": true,
            },
            "versions": [version(2, "locked"), version(1, "purged")],
        }))
        .unwrap()
    }

    #[test]
    fn a_file_flattens_the_item_and_its_latest_version() {
        let file = LockedFile::from(&detail(version(2, "locked")));
        assert_eq!(file.path, "invoices/2026/Invoice");
        assert_eq!(file.version, Some(2));
        assert_eq!(file.version_id.as_deref(), Some("v2"));
        assert_eq!(file.status, Some(LockedVersionStatus::Locked));
        assert_eq!(file.sha256, Some("ab".repeat(32)));
        assert_eq!(file.retention_mode, Some(LockedRetentionMode::Compliance));
        assert_eq!(file.enforcement, Some(LockedEnforcement::Object));
        assert_eq!((file.live_versions, file.live_size), (2, 2048));
        assert_eq!(file.metadata.unwrap()["customer"], json!("ACME"));
    }

    #[test]
    fn a_file_without_a_locked_version_shows_its_newest_record() {
        let detail = detail(Value::Null);
        let file = LockedFile::from(&detail);
        assert_eq!(file.version_id.as_deref(), Some("v2"));
        assert_eq!(detail.version_by_number(1).unwrap().id, "v1");
        assert_eq!(
            detail.version_by_number(1).unwrap().status,
            LockedVersionStatus::Purged
        );
        assert_eq!(locked_path("", "a.pdf"), "a.pdf");
    }
}

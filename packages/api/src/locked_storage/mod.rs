//! Write-once, versioned files in a bucket with object lock (Revisionssicherheit).
//! Only this process holds bucket credentials; clients receive signed URLs for one
//! exact object. Design and provider matrix: todo/locked-storage.md.

#[cfg(feature = "azure")]
mod azure;
mod backend;
mod config;
#[cfg(feature = "gcp")]
mod gcs;
mod paths;
#[cfg(feature = "aws")]
mod s3;
mod service;
mod types;

pub use backend::{ChecksumCheck, LockedBackend, SignedUpload, StoredObject, UploadSpec};
pub use config::{LockedProvider, LockedStorageConfig};
pub use service::{
    Caller, DOWNLOAD_URL_TTL, LockedStorage, UPLOAD_URL_TTL, abort, commit, disabled_status,
    download, extend_retention, get, list, purge, set_auto_delete, set_legal_hold, status,
    sweep_abandoned_uploads, sweep_expired_versions, upload,
};
pub use types::*;

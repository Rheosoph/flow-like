//! Fork files cross between the hub and a desktop only through signed URLs.
//!
//! The server picks every object key and signs one request per file, so no
//! client ever holds a storage credential or chooses where a file lands.
//! Whatever a fork writes on the way, re-keyed meta artifacts for a download
//! and staged files of an upload, lives under `tmp/`, which the content
//! bucket's lifecycle rules reclaim, so an abandoned transfer cleans itself up.

use super::ids::derive_id;
use crate::{credentials::temporary_prefixes, error::ApiError, state::AppState};
use flow_like_storage::{Path, files::store::FlowLikeStore, object_store::ObjectStore};
use flow_like_types::storage_paths::is_legacy_training_ledger_path;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use utoipa::ToSchema;

/// Most files one transfer request signs or moves.
pub const MAX_TRANSFER_BATCH: usize = 500;
const TRANSFER_URL_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_CONTENT_PATH_LEN: usize = 1024;
const UPLOAD_STAGING_DIR: &str = "fork-upload";
const DOWNLOAD_STAGING_DIR: &str = "fork-offline";
const CONTENT_ROOTS: [&str; 4] = ["metadata/", "media/", "upload/", "storage/"];
const REKEYED_METADATA_DIRS: [&str; 3] = [
    "metadata/widgets/",
    "metadata/templates/",
    "metadata/pages/",
];

/// One file of a fork transfer and the single request that moves it.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SignedFile {
    /// Path relative to the app root: where the file lands for a download,
    /// what it is in the local app for an upload.
    pub path: String,
    pub url: String,
    /// Headers the request has to carry, such as `x-ms-blob-type` for an
    /// upload to Azure.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Signs transfer URLs on the content bucket.
pub struct TransferSigner {
    store: FlowLikeStore,
    ttl: Duration,
}

impl TransferSigner {
    pub async fn content(state: &AppState) -> Result<Self, ApiError> {
        let master = state.master_credentials().await?;
        let store = master.to_store(false).await?;
        if !matches!(
            store,
            FlowLikeStore::AWS(_)
                | FlowLikeStore::Azure(_)
                | FlowLikeStore::Google(_)
                | FlowLikeStore::Signed(_)
        ) {
            return Err(ApiError::not_implemented(
                "this deployment's content storage cannot sign transfer URLs, so forks cannot move files",
            ));
        }
        Ok(Self {
            ttl: master.signing_ttl(TRANSFER_URL_TTL),
            store,
        })
    }

    pub fn store(&self) -> Arc<dyn ObjectStore> {
        self.store.as_generic()
    }

    pub fn expires_at(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::from_std(self.ttl).unwrap_or_default()
    }

    pub async fn download(
        &self,
        path: String,
        key: &Path,
        size: Option<u64>,
    ) -> Result<SignedFile, ApiError> {
        let url = self.store.sign("GET", key, self.ttl).await?;
        Ok(SignedFile {
            path,
            url: url.to_string(),
            headers: BTreeMap::new(),
            size,
        })
    }

    pub async fn upload(
        &self,
        path: String,
        key: &Path,
        size: Option<u64>,
    ) -> Result<SignedFile, ApiError> {
        let url = self.store.sign("PUT", key, self.ttl).await?;
        let mut headers = BTreeMap::new();
        if matches!(self.store, FlowLikeStore::Azure(_)) {
            headers.insert("x-ms-blob-type".to_string(), "BlockBlob".to_string());
        }
        Ok(SignedFile {
            path,
            url: url.to_string(),
            headers,
            size,
        })
    }
}

/// Where an upload's files wait until the server moves them into the app.
pub fn upload_staging_prefix(sub: &str, app_id: &str, job_id: &str) -> Path {
    let (user_prefix, _) = temporary_prefixes(sub, app_id);
    Path::from(user_prefix)
        .join(UPLOAD_STAGING_DIR)
        .join(job_id)
}

/// Where an offline fork's re-keyed meta artifacts wait for the desktop.
pub fn download_staging_prefix(app_id: &str, session_id: &str) -> Path {
    let (_, global_prefix) = temporary_prefixes("", app_id);
    Path::from(global_prefix)
        .join(DOWNLOAD_STAGING_DIR)
        .join(session_id)
}

/// Whether an upload may put a file at `path` (relative to the app root):
/// only below the content roots, never a meta artifact, never the
/// device-local training ledger, and never a path that escapes its root.
pub fn is_uploadable_content_path(path: &str) -> bool {
    path.len() <= MAX_CONTENT_PATH_LEN
        && CONTENT_ROOTS.iter().any(|root| path.starts_with(root))
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        && !is_legacy_training_ledger_path(path)
}

/// The path a content file takes in a copy whose widget, template and page
/// ids were derived from `seed`: their metadata lives under those ids.
pub fn rekeyed_content_path(path: &str, seed: &str) -> String {
    for dir in REKEYED_METADATA_DIRS {
        if let Some((id, tail)) = path.strip_prefix(dir).and_then(|rest| rest.split_once('/')) {
            return format!("{dir}{}/{tail}", derive_id(seed, id));
        }
    }
    path.to_string()
}

/// `key` relative to `prefix`, or `None` when it lies elsewhere.
pub fn relative_key(key: &Path, prefix: &Path) -> Option<String> {
    key.as_ref()
        .strip_prefix(prefix.as_ref())
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_stay_inside_the_content_roots() {
        for path in [
            "metadata/en.meta",
            "media/icon.webp",
            "upload/a b/c.csv",
            "storage/db/t.lance/data/0.lance",
        ] {
            assert!(is_uploadable_content_path(path), "{path}");
        }
        for path in [
            "manifest.app",
            "board.board",
            "events/e.event",
            "upload/../manifest.app",
            "upload//x",
            "/upload/x",
            "storage/inspection/jobs.sqlite",
            "tmp/x",
        ] {
            assert!(!is_uploadable_content_path(path), "{path}");
        }
        assert!(!is_uploadable_content_path(&format!(
            "upload/{}",
            "x".repeat(MAX_CONTENT_PATH_LEN)
        )));
    }

    #[test]
    fn metadata_of_rekeyed_artifacts_moves_to_the_derived_ids() {
        assert_eq!(
            rekeyed_content_path("metadata/widgets/w1/en.meta", "job"),
            format!("metadata/widgets/{}/en.meta", derive_id("job", "w1"))
        );
        assert_eq!(
            rekeyed_content_path("metadata/pages/p1/de.meta", "job"),
            format!("metadata/pages/{}/de.meta", derive_id("job", "p1"))
        );
        for path in ["metadata/en.meta", "media/x.webp", "upload/rpa/b1/a.png"] {
            assert_eq!(rekeyed_content_path(path, "job"), path);
        }
    }

    #[test]
    fn staging_lives_under_tmp_so_lifecycle_rules_reclaim_it() {
        let upload = upload_staging_prefix("auth0|user", "app", "job");
        assert!(upload.as_ref().starts_with("tmp/user/"), "{upload}");
        assert!(upload.as_ref().ends_with("/fork-upload/job"), "{upload}");
        let download = download_staging_prefix("app", "session");
        assert_eq!(
            download.as_ref(),
            "tmp/global/apps/app/fork-offline/session"
        );
        assert_eq!(
            relative_key(&Path::from("tmp/a/b/c"), &Path::from("tmp/a")),
            Some("b/c".to_string())
        );
        assert_eq!(
            relative_key(&Path::from("tmp/ab"), &Path::from("tmp/a")),
            None
        );
    }
}

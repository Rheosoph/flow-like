//! File-size accounting for released content on Azure.
//!
//! Each `BlobCreated`/`BlobDeleted` under `apps/` and `users/` becomes one observation for the
//! shared `flow-like-file-accounting` ledger, which settles App, User and payer capacity in
//! PostgreSQL. Plain blobs carry their Event Grid `sequencer`; Lance datasets are accounted as
//! one directory row measured by listing the content container. The Cosmos `files` container is
//! read-only: it supplies the pre-SQL size of an object, or of a dataset's files, once.

use crate::blob_events::BlobEvent;
use flow_like_azure_data::cosmos::{CosmosClient, QueryParameter};
use flow_like_db::DbDialect;
use flow_like_file_accounting::{
    Observation, apply_batch, dataset_prefix, measurement_sequencer, normalize_sequencer, observe,
};
use flow_like_storage::object_store::{ObjectStore, path::Path};
use futures_util::TryStreamExt;
use sea_orm::{DatabaseConnection, DbErr};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub const DEFAULT_FILES_CONTAINER: &str = "files";

const DATASET_LISTING_TIMEOUT: Duration = Duration::from_secs(60);
const LEGACY_PAGE_SIZE: i32 = 1_000;

pub struct FileTrackingContext {
    pub cosmos: CosmosClient,
    pub files_container: String,
    pub content_container: String,
    pub store: Arc<dyn ObjectStore>,
    pub database: DatabaseConnection,
    pub dialect: DbDialect,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileTrackingError {
    /// The message can never be processed; poison it.
    Permanent(&'static str),
    /// Transient store failure; release the message for a retry.
    Retryable(&'static str),
}

/// Legacy ledger row: only the fields the baseline import reads.
#[derive(Debug, Deserialize)]
struct LedgerDocument {
    sk: String,
    size: i64,
}

/// Cosmos ids may not contain `/`, `\`, `?` or `#`; blob keys are paths.
fn ledger_id(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

fn cosmos_error(error: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(format!("files ledger request failed: {error}"))
}

/// `None` for events this worker does not account.
fn observation_for(
    event: &BlobEvent,
    content_container: &str,
    dataset_sequencer: &str,
) -> Result<Option<Observation>, FileTrackingError> {
    let (container, key) = event
        .container_and_key()
        .ok_or(FileTrackingError::Permanent("invalid_blob_subject"))?;
    if container != content_container {
        tracing::warn!(
            container,
            "blob event for a container this worker does not track"
        );
        return Ok(None);
    }
    // DeviceArchive metadata accounts for these server-owned payloads separately.
    if key.starts_with("device-archives/") {
        return Ok(None);
    }
    let object_sequencer = if dataset_prefix(key).is_some() {
        String::new()
    } else {
        let raw = event
            .sequencer
            .as_deref()
            .ok_or(FileTrackingError::Permanent("blob_event_without_sequencer"))?;
        normalize_sequencer(raw)
            .map_err(|_| FileTrackingError::Permanent("blob_event_without_sequencer"))?
    };
    observe(content_container, key, &object_sequencer, dataset_sequencer)
        .map(Some)
        .ok_or(FileTrackingError::Permanent("invalid_blob_key_layout"))
}

async fn legacy_size(
    cosmos: CosmosClient,
    files_container: String,
    app_id: String,
    key: String,
) -> Result<i64, DbErr> {
    cosmos
        .read_document::<LedgerDocument>(&files_container, &ledger_id(&key), &app_id)
        .await
        .map(|row| row.map_or(0, |row| row.size))
        .map_err(cosmos_error)
}

async fn legacy_files(
    cosmos: CosmosClient,
    files_container: String,
    app_id: String,
    prefix: String,
) -> Result<Vec<(String, i64)>, DbErr> {
    let parameters = [QueryParameter::new("@prefix", prefix)];
    let mut rows = Vec::new();
    let mut continuation: Option<String> = None;
    loop {
        let page = cosmos
            .query_page::<LedgerDocument>(
                &files_container,
                "SELECT * FROM c WHERE STARTSWITH(c.sk, @prefix)",
                &parameters,
                Some(&app_id),
                LEGACY_PAGE_SIZE,
                continuation.as_deref(),
            )
            .await
            .map_err(cosmos_error)?;
        rows.extend(page.documents.into_iter().map(|row| (row.sk, row.size)));
        match page.continuation {
            Some(next) => continuation = Some(next),
            None => return Ok(rows),
        }
    }
}

async fn dataset_size(store: Arc<dyn ObjectStore>, prefix: String) -> Result<i64, DbErr> {
    // Event keys are the stored, already encoded form; `Path::from` would encode them again.
    let path = Path::parse(prefix.trim_end_matches('/'))
        .map_err(|error| DbErr::Custom(format!("dataset prefix {prefix} is invalid: {error}")))?;
    let listing = async {
        store
            .list(Some(&path))
            .try_fold(0i64, |total, meta| async move {
                Ok(total.saturating_add(i64::try_from(meta.size).unwrap_or(i64::MAX)))
            })
            .await
    };
    match tokio::time::timeout(DATASET_LISTING_TIMEOUT, listing).await {
        Ok(Ok(size)) => Ok(size),
        Ok(Err(error)) => Err(DbErr::Custom(format!("listing {prefix} failed: {error}"))),
        Err(_) => Err(DbErr::Custom(format!("listing {prefix} timed out"))),
    }
}

pub async fn process(
    ctx: &FileTrackingContext,
    event: &BlobEvent,
) -> Result<(), FileTrackingError> {
    let dataset_sequencer = measurement_sequencer(SystemTime::now());
    let Some(observation) = observation_for(event, &ctx.content_container, &dataset_sequencer)?
    else {
        return Ok(());
    };
    let created_size = if event.is_deletion() {
        0
    } else {
        event.content_length.ok_or(FileTrackingError::Permanent(
            "blob_created_without_content_length",
        ))?
    };
    let key = observation.key.clone();

    let results = apply_batch(
        &ctx.database,
        ctx.dialect,
        std::slice::from_ref(&observation),
        |observation| {
            legacy_size(
                ctx.cosmos.clone(),
                ctx.files_container.clone(),
                observation.app_id.clone(),
                observation.key.clone(),
            )
        },
        |observation| {
            legacy_files(
                ctx.cosmos.clone(),
                ctx.files_container.clone(),
                observation.app_id.clone(),
                observation.key.clone(),
            )
        },
        |observation| {
            let store = ctx.store.clone();
            let prefix = observation.is_dataset().then(|| observation.key.clone());
            async move {
                match prefix {
                    Some(prefix) => dataset_size(store, prefix).await,
                    None => Ok(created_size),
                }
            }
        },
    )
    .await;

    match results.into_iter().next() {
        Some(Ok(())) => Ok(()),
        Some(Err(failure)) => {
            tracing::error!(key = %key, error = %failure, "file accounting failed");
            Err(FileTrackingError::Retryable("file_accounting_failed"))
        }
        None => Err(FileTrackingError::Retryable("file_accounting_failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER: &str = "content";
    const MEASURED: &str = "1a2b3c";

    fn event(key: &str, sequencer: Option<&str>) -> BlobEvent {
        BlobEvent {
            id: "e".into(),
            event_type: crate::blob_events::BLOB_CREATED.into(),
            subject: format!("/blobServices/default/containers/{CONTAINER}/blobs/{key}"),
            content_length: Some(10),
            sequencer: sequencer.map(str::to_owned),
        }
    }

    fn observation(
        key: &str,
        sequencer: Option<&str>,
    ) -> Result<Option<Observation>, FileTrackingError> {
        observation_for(&event(key, sequencer), CONTAINER, MEASURED)
    }

    #[test]
    fn plain_blob_uses_normalized_event_sequencer() {
        let seen = observation("apps/a1/files/x.txt", Some("000000000000ABCD"))
            .unwrap()
            .unwrap();
        assert_eq!(seen.key, "apps/a1/files/x.txt");
        assert_eq!(seen.app_id, "a1");
        assert_eq!(seen.sequencer, "abcd");
        assert_eq!(seen.bucket, CONTAINER);
        assert!(!seen.is_dataset());
    }

    #[test]
    fn dataset_file_becomes_directory_at_measurement_sequencer() {
        let seen = observation("users/u1/apps/a1/db/t.lance/data/f.lance", Some("0001"))
            .unwrap()
            .unwrap();
        assert_eq!(seen.key, "users/u1/apps/a1/db/t.lance/");
        assert_eq!(seen.user_id.as_deref(), Some("u1"));
        assert_eq!(seen.sequencer, MEASURED);
        assert!(seen.is_dataset());
    }

    #[test]
    fn dataset_file_without_sequencer_is_accepted() {
        let seen = observation("apps/a1/db/t.lance/data/f.lance", None)
            .unwrap()
            .unwrap();
        assert!(seen.is_dataset());
        assert_eq!(seen.sequencer, MEASURED);
    }

    #[test]
    fn plain_blob_without_valid_sequencer_is_permanent() {
        let expected = Err(FileTrackingError::Permanent("blob_event_without_sequencer"));
        assert_eq!(observation("apps/a1/x.txt", None).map(|_| ()), expected);
        assert_eq!(
            observation("apps/a1/x.txt", Some("zz")).map(|_| ()),
            expected
        );
    }

    #[test]
    fn bad_layout_is_permanent() {
        assert_eq!(
            observation("media/x.png", Some("01")).map(|_| ()),
            Err(FileTrackingError::Permanent("invalid_blob_key_layout"))
        );
    }

    #[test]
    fn device_archives_and_other_containers_are_skipped() {
        assert!(
            observation("device-archives/a.bin", None)
                .unwrap()
                .is_none()
        );
        let mut other = event("apps/a1/x.txt", Some("01"));
        other.subject = "/blobServices/default/containers/other/blobs/apps/a1/x.txt".into();
        assert!(
            observation_for(&other, CONTAINER, MEASURED)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn ledger_ids_are_path_safe_and_stable() {
        let id = ledger_id("apps/a1/storage/db/x.lance");
        assert_eq!(id.len(), 64);
        assert!(!id.contains('/'));
        assert_eq!(id, ledger_id("apps/a1/storage/db/x.lance"));
    }
}

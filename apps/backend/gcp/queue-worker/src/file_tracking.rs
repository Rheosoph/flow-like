//! File-size accounting for released content on GCP, an adapter to
//! `flow-like-file-accounting` like `apps/backend/aws/file-tracker`.
//!
//! Every `OBJECT_FINALIZE`/`OBJECT_DELETE` under `apps/` and `users/` becomes one
//! observation: the object at its generation sequencer, or the enclosing Lance
//! dataset directory, whose size is measured by listing it. The shared core writes the
//! SQL ledger and moves App, User and payer totals. The Firestore `files` collection is
//! read-only: it supplies the pre-SQL baseline of an object or dataset once.

use crate::gcs_events::GcsEvent;
use flow_like_db::DbDialect;
use flow_like_file_accounting::{self as accounting, Observation};
use flow_like_gcp_data::firestore::{FilterOperator, FirestoreClient, QueryFilter};
use flow_like_storage::Path;
use flow_like_storage::object_store::ObjectStore;
use futures::TryStreamExt;
use sea_orm::{DatabaseConnection, DbErr};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub const DEFAULT_FILES_COLLECTION: &str = "files";

const DATASET_LIST_TIMEOUT: Duration = Duration::from_secs(60);
const LEGACY_PAGE_SIZE: i32 = 500;

pub struct FileTrackingContext {
    pub firestore: FirestoreClient,
    pub files_collection: String,
    pub content_bucket: String,
    pub store: Arc<dyn ObjectStore>,
    pub database: DatabaseConnection,
    pub dialect: DbDialect,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileTrackingError {
    /// The message can never be processed; dead-letter it.
    Permanent(&'static str),
    /// Transient store failure; nack so Pub/Sub redelivers.
    Retryable(&'static str),
}

/// Legacy ledger row, read-only: one per object, queried by app id.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerDocument {
    id: String,
    app_id: String,
    sk: String,
    size: i64,
    user_id: String,
    etag: String,
    #[serde(default)]
    generation: i64,
    updated_at: i64,
}

/// Firestore document IDs may not contain `/`; object names are paths.
fn ledger_id(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

/// `None` when the event is not accounted (`device-archives/`, whose DeviceArchive metadata
/// accounts for them separately).
fn observe_event(
    event: &GcsEvent,
    content_bucket: &str,
    dataset_sequencer: &str,
) -> Result<Option<Observation>, FileTrackingError> {
    if event.object.starts_with("device-archives/") {
        return Ok(None);
    }
    let object_sequencer = if accounting::dataset_prefix(&event.object).is_some() {
        String::new()
    } else {
        if !event.is_deletion() && event.size.is_none() {
            return Err(FileTrackingError::Permanent(
                "object_finalized_without_size",
            ));
        }
        let generation = event
            .generation
            .ok_or(FileTrackingError::Permanent(
                "object_event_without_generation",
            ))
            .and_then(|generation| {
                u64::try_from(generation)
                    .map_err(|_| FileTrackingError::Permanent("object_event_without_generation"))
            })?;
        accounting::generation_sequencer(generation, event.is_deletion())
    };
    accounting::observe(
        content_bucket,
        &event.object,
        &object_sequencer,
        dataset_sequencer,
    )
    .map(Some)
    .ok_or(FileTrackingError::Permanent("invalid_object_key_layout"))
}

async fn list_size(store: Arc<dyn ObjectStore>, prefix: String) -> Result<i64, DbErr> {
    let path = Path::parse(prefix.trim_end_matches('/'))
        .map_err(|error| DbErr::Custom(format!("dataset prefix {prefix} is invalid: {error}")))?;
    let total = async {
        store
            .list(Some(&path))
            .try_fold(0i64, |total, meta| async move {
                Ok(total.saturating_add(i64::try_from(meta.size).unwrap_or(i64::MAX)))
            })
            .await
    };
    match tokio::time::timeout(DATASET_LIST_TIMEOUT, total).await {
        Ok(Ok(total)) => Ok(total),
        Ok(Err(error)) => Err(DbErr::Custom(format!(
            "listing dataset {prefix} failed: {error}"
        ))),
        Err(_) => Err(DbErr::Custom(format!("listing dataset {prefix} timed out"))),
    }
}

async fn legacy_size(
    firestore: FirestoreClient,
    collection: String,
    key: String,
) -> Result<i64, DbErr> {
    firestore
        .read_document::<LedgerDocument>(&collection, &ledger_id(&key))
        .await
        .map(|document| document.map_or(0, |document| document.value.size))
        .map_err(|error| DbErr::Custom(format!("legacy ledger read for {key} failed: {error}")))
}

async fn legacy_files(
    firestore: FirestoreClient,
    collection: String,
    app_id: String,
    prefix: String,
) -> Result<Vec<(String, i64)>, DbErr> {
    let filters = [QueryFilter::new("app_id", FilterOperator::Equal, app_id)];
    let mut files = Vec::new();
    let mut continuation: Option<String> = None;
    loop {
        let page = firestore
            .query_page::<LedgerDocument>(
                &collection,
                &filters,
                &[],
                LEGACY_PAGE_SIZE,
                continuation.as_deref(),
            )
            .await
            .map_err(|error| {
                DbErr::Custom(format!("legacy ledger query for {prefix} failed: {error}"))
            })?;
        files.extend(
            page.documents
                .into_iter()
                .filter(|document| document.value.sk.starts_with(&prefix))
                .map(|document| (document.value.sk, document.value.size)),
        );
        match page.continuation {
            Some(next) => continuation = Some(next),
            None => return Ok(files),
        }
    }
}

pub async fn process(ctx: &FileTrackingContext, event: &GcsEvent) -> Result<(), FileTrackingError> {
    if event.bucket != ctx.content_bucket {
        tracing::warn!(
            bucket = %event.bucket,
            "object event for a bucket this worker does not track"
        );
        return Ok(());
    }
    let dataset_sequencer = accounting::measurement_sequencer(SystemTime::now());
    let Some(observation) = observe_event(event, &ctx.content_bucket, &dataset_sequencer)? else {
        return Ok(());
    };
    let finalized_size = if event.is_deletion() {
        0
    } else {
        event.size.unwrap_or_default()
    };

    let results = accounting::apply_batch(
        &ctx.database,
        ctx.dialect,
        std::slice::from_ref(&observation),
        |observation| {
            legacy_size(
                ctx.firestore.clone(),
                ctx.files_collection.clone(),
                observation.key.clone(),
            )
        },
        |observation| {
            legacy_files(
                ctx.firestore.clone(),
                ctx.files_collection.clone(),
                observation.app_id.clone(),
                observation.key.clone(),
            )
        },
        |observation| {
            let store = ctx.store.clone();
            let prefix = observation.key.clone();
            let dataset = observation.is_dataset();
            async move {
                if dataset {
                    list_size(store, prefix).await
                } else {
                    Ok(finalized_size)
                }
            }
        },
    )
    .await;

    match results.into_iter().next() {
        Some(Err(error)) => {
            tracing::error!(key = %observation.key, error = %error, "file accounting failed");
            Err(FileTrackingError::Retryable("file_accounting_failed"))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcs_events::{OBJECT_DELETE, OBJECT_FINALIZE};

    const BUCKET: &str = "content";
    const MEASURED: &str = "abc";

    fn event(kind: &str, object: &str, generation: Option<i64>) -> GcsEvent {
        GcsEvent {
            id: String::new(),
            event_type: kind.into(),
            bucket: BUCKET.into(),
            object: object.into(),
            size: Some(10),
            generation,
            etag: None,
        }
    }

    fn observed(event: &GcsEvent) -> Result<Option<Observation>, FileTrackingError> {
        observe_event(event, BUCKET, MEASURED)
    }

    #[test]
    fn finalize_is_stamped_with_its_generation() {
        let o = observed(&event(OBJECT_FINALIZE, "apps/a1/x.txt", Some(7)))
            .unwrap()
            .unwrap();
        assert_eq!(o.sequencer, accounting::generation_sequencer(7, false));
        assert_eq!(o.key, "apps/a1/x.txt");
        assert_eq!(o.bucket, BUCKET);
        assert_eq!(o.app_id, "a1");
        assert_eq!(o.user_id, None);
    }

    #[test]
    fn delete_orders_right_after_its_finalize() {
        let finalize = observed(&event(OBJECT_FINALIZE, "users/u1/apps/a1/x", Some(7)))
            .unwrap()
            .unwrap();
        let delete = observed(&event(OBJECT_DELETE, "users/u1/apps/a1/x", Some(7)))
            .unwrap()
            .unwrap();
        let newer = accounting::generation_sequencer(8, false);
        assert_eq!(finalize.user_id.as_deref(), Some("u1"));
        let order = |s: &str| (s.len(), s.to_owned());
        assert!(order(&delete.sequencer) > order(&finalize.sequencer));
        assert!(order(&delete.sequencer) < order(&newer));
    }

    #[test]
    fn lance_file_becomes_the_dataset_directory() {
        let o = observed(&event(
            OBJECT_FINALIZE,
            "apps/a1/storage/db/t.lance/data/0.lance",
            None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(o.key, "apps/a1/storage/db/t.lance/");
        assert_eq!(o.sequencer, MEASURED);
        assert!(o.is_dataset());
    }

    #[test]
    fn dataset_file_without_generation_is_accepted() {
        let mut e = event(
            OBJECT_FINALIZE,
            "apps/a1/t.lance/_versions/1.manifest",
            None,
        );
        e.size = None;
        assert!(observed(&e).unwrap().is_some());
    }

    #[test]
    fn plain_object_requires_a_generation() {
        let missing = observed(&event(OBJECT_FINALIZE, "apps/a1/x", None));
        assert_eq!(
            missing.unwrap_err(),
            FileTrackingError::Permanent("object_event_without_generation")
        );
        assert!(observed(&event(OBJECT_DELETE, "apps/a1/x", Some(-1))).is_err());
    }

    #[test]
    fn finalize_requires_a_size() {
        let mut e = event(OBJECT_FINALIZE, "apps/a1/x", Some(1));
        e.size = None;
        assert_eq!(
            observed(&e).unwrap_err(),
            FileTrackingError::Permanent("object_finalized_without_size")
        );
    }

    #[test]
    fn bad_layout_is_permanent() {
        assert_eq!(
            observed(&event(OBJECT_FINALIZE, "media/x.png", Some(1))).unwrap_err(),
            FileTrackingError::Permanent("invalid_object_key_layout")
        );
    }

    #[test]
    fn device_archives_are_skipped() {
        let e = event(OBJECT_FINALIZE, "device-archives/a1/x", None);
        assert!(observed(&e).unwrap().is_none());
    }

    #[test]
    fn ledger_ids_are_path_safe_and_stable() {
        let id = ledger_id("apps/a1/storage/db/x.lance");
        assert_eq!(id.len(), 64);
        assert!(!id.contains('/'));
        assert_eq!(id, ledger_id("apps/a1/storage/db/x.lance"));
    }
}

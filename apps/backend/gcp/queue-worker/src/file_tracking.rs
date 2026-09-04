//! File-size accounting for released content, the GCP port of
//! `apps/backend/aws/file-tracker`.
//!
//! Every `OBJECT_FINALIZE`/`OBJECT_DELETE` under `apps/` and `users/` updates a
//! per-object ledger row in the Firestore `files` collection and applies the size
//! delta to `App.totalSize` (and `User.totalSize` for user-scoped keys) inside
//! one PostgreSQL transaction, reached through the self-rotating Cloud SQL IAM
//! token. The ledger, not the event, is the source of the previous size, exactly
//! like the DynamoDB table on AWS and the Cosmos container on Azure.
//!
//! Two differences from the Cosmos original are forced by Firestore. The
//! partition key becomes an ordinary indexed field (`app_id`), because Firestore
//! has no partition key and its IAM cannot scope to a collection let alone to a
//! partition. And the concurrency token moves out of the document: Cosmos
//! carries `_etag` in the body, Firestore keeps `updateTime` in the envelope, so
//! every conditional write is driven by the value the read returned alongside
//! the document.

use crate::gcs_events::GcsEvent;
use flow_like_db::{AsDbConflict, DbConflict, DbDialect, RetryPolicy, retry_transaction};
use flow_like_gcp_data::firestore::{FirestoreClient, FirestoreError, MutationOutcome};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FILES_COLLECTION: &str = "files";

pub struct FileTrackingContext {
    pub firestore: FirestoreClient,
    pub files_collection: String,
    pub content_bucket: String,
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

/// Ledger row: one per object, queried by app id.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerDocument {
    id: String,
    /// The Cosmos `pk` under its Firestore name. It is still the only field the
    /// `files` index covers, so the query shape survives the rename.
    app_id: String,
    sk: String,
    size: i64,
    user_id: String,
    etag: String,
    /// The GCS generation of the event that last wrote this row. Zero means the
    /// row predates generation tracking and is never treated as stale.
    #[serde(default)]
    generation: i64,
    updated_at: i64,
}

/// What it takes to put the ledger back the way it was.
///
/// Every arm carries the `updateTime` the write produced, so an undo can only
/// ever remove the exact version this event wrote. A newer event that landed in
/// between owns the row, and its value is the correct one to keep.
#[derive(Debug)]
enum LedgerRollback {
    None,
    /// The event created a row that did not exist before.
    Remove {
        update_time: Option<String>,
    },
    /// The event overwrote an existing row.
    Restore {
        previous: Box<LedgerDocument>,
        update_time: Option<String>,
    },
    /// The event removed a row.
    Recreate {
        previous: Box<LedgerDocument>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyIdentity {
    user_id: Option<String>,
    app_id: String,
}

/// `apps/{app_id}/...` or `users/{sub}/apps/{app_id}/...`, mirroring the AWS
/// parser (which also does not validate the third `apps` segment).
fn parse_key_identity(key: &str) -> Option<KeyIdentity> {
    let parts: Vec<&str> = key.split('/').collect();
    match parts.as_slice() {
        ["apps", app_id, ..] if !app_id.is_empty() => Some(KeyIdentity {
            user_id: None,
            app_id: (*app_id).to_string(),
        }),
        ["users", user_id, _, app_id, ..] if !user_id.is_empty() && !app_id.is_empty() => {
            Some(KeyIdentity {
                user_id: Some((*user_id).to_string()),
                app_id: (*app_id).to_string(),
            })
        }
        _ => None,
    }
}

/// Firestore document IDs may not contain `/`; object names are paths.
fn ledger_id(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

/// Azure's `sequencer` is fixed-width hex, so staleness there was a
/// lexicographic `>=`. A GCS generation is a decimal integer of varying width,
/// so the same test has to be numeric — compared as text, `"9"` outranks `"10"`
/// and every carry into a new digit would silently drop a fresh event.
///
/// This is the `OBJECT_FINALIZE` predicate only. `>=` is right there because a
/// redelivered finalize carries the generation the row already holds and must
/// be a no-op; it is wrong for a delete, see [`is_stale_deletion`].
fn is_stale(existing: &LedgerDocument, incoming: Option<i64>) -> bool {
    match incoming {
        Some(generation) => existing.generation != 0 && existing.generation >= generation,
        None => false,
    }
}

/// The `OBJECT_DELETE` predicate, and the second place a GCS generation differs
/// from an Event Grid `sequencer`. A sequencer is a property of the *event*, so
/// BlobCreated and DeleteBlob for one blob carry distinct, increasing values and
/// `>=` tells them apart. A generation is a property of the *object version*:
/// the OBJECT_DELETE for a live object carries exactly the generation its
/// OBJECT_FINALIZE carried, which is the value `record_creation` stored. Under
/// `>=` every deletion would compare equal, be skipped as stale, and the ledger
/// row — and `App.totalSize` with it — would never come down. So a delete is
/// stale only when the row was written by a strictly *newer* generation, i.e. the
/// object was overwritten after the version this delete refers to.
fn is_stale_deletion(existing: &LedgerDocument, incoming: Option<i64>) -> bool {
    matches!(incoming, Some(generation) if existing.generation != 0 && existing.generation > generation)
}

fn map_firestore(error: FirestoreError) -> FileTrackingError {
    tracing::warn!(error = %error, "files ledger request failed");
    FileTrackingError::Retryable("files_ledger_unavailable")
}

/// Cosmos scoped every read to a partition key, so a row belonging to another
/// app could never come back from a read this worker issued. Firestore has no
/// partition key, so that guarantee has to be restated as an explicit check:
/// the ledger ID is a hash of the object name and object names are namespaced
/// per app, which means a mismatch is either a hash collision or a ledger being
/// written by something other than this worker. Both are conditions to refuse
/// rather than to absorb into somebody else's size total.
fn ensure_same_app(existing: &LedgerDocument, app_id: &str) -> Result<(), FileTrackingError> {
    if existing.app_id != app_id {
        return Err(FileTrackingError::Permanent("files_ledger_app_mismatch"));
    }
    Ok(())
}

pub async fn process(ctx: &FileTrackingContext, event: &GcsEvent) -> Result<(), FileTrackingError> {
    if event.bucket != ctx.content_bucket {
        tracing::warn!(
            bucket = %event.bucket,
            "object event for a bucket this worker does not track"
        );
        return Ok(());
    }
    let identity = parse_key_identity(&event.object)
        .ok_or(FileTrackingError::Permanent("invalid_object_key_layout"))?;
    let id = ledger_id(&event.object);

    let (delta, rollback) = if event.is_deletion() {
        let (size, rollback) = record_deletion(ctx, &id, &identity.app_id, event).await?;
        (-size, rollback)
    } else {
        let size = event.size.ok_or(FileTrackingError::Permanent(
            "object_finalized_without_size",
        ))?;
        let (previous, rollback) = record_creation(ctx, &id, &identity, size, event).await?;
        (size - previous, rollback)
    };

    if delta == 0 {
        return Ok(());
    }

    if let Err(error) = apply_usage_delta(
        &ctx.database,
        ctx.dialect,
        identity.user_id.as_deref(),
        &identity.app_id,
        delta,
    )
    .await
    {
        // The stored object is deliberately left alone. Deleting it to "undo"
        // the event would publish another notification into this very
        // subscription and the worker would be feeding itself — the AWS original
        // names this as Lambda recursion and refuses it for the same reason.
        // Only the ledger is rolled back, so the redelivery recomputes the same
        // delta against the same previous size instead of seeing its own
        // half-applied write and computing zero.
        rollback_ledger(ctx, &id, rollback).await;
        return Err(error);
    }
    Ok(())
}

/// Upsert the ledger row and return the previous size (0 if absent, `size` when
/// the event is stale so that the caller computes a zero delta).
async fn record_creation(
    ctx: &FileTrackingContext,
    id: &str,
    identity: &KeyIdentity,
    size: i64,
    event: &GcsEvent,
) -> Result<(i64, LedgerRollback), FileTrackingError> {
    let existing = ctx
        .firestore
        .read_document::<LedgerDocument>(&ctx.files_collection, id)
        .await
        .map_err(map_firestore)?;

    if let Some(existing) = &existing {
        ensure_same_app(&existing.value, &identity.app_id)?;
        if is_stale(&existing.value, event.generation) {
            tracing::info!(object = %event.object, "ignoring stale OBJECT_FINALIZE event");
            return Ok((size, LedgerRollback::None));
        }
    }

    let document = LedgerDocument {
        id: id.to_string(),
        app_id: identity.app_id.clone(),
        sk: event.object.clone(),
        size,
        user_id: identity.user_id.clone().unwrap_or_default(),
        etag: event.etag.clone().unwrap_or_default(),
        generation: event.generation.unwrap_or_default(),
        updated_at: chrono::Utc::now().timestamp(),
    };

    let (outcome, update_time) = match &existing {
        Some(current) => {
            write_ledger(ctx, id, &document, Some(current.update_time.as_deref())).await?
        }
        None => write_ledger(ctx, id, &document, None).await?,
    };

    match outcome {
        MutationOutcome::Applied => match existing {
            Some(current) => {
                let previous_size = current.value.size;
                Ok((
                    previous_size,
                    LedgerRollback::Restore {
                        previous: Box::new(current.value),
                        update_time,
                    },
                ))
            }
            None => Ok((0, LedgerRollback::Remove { update_time })),
        },
        // Another delivery moved the row between the read and the write; a retry
        // re-reads it.
        MutationOutcome::Conflict
        | MutationOutcome::PreconditionFailed
        | MutationOutcome::NotFound => {
            Err(FileTrackingError::Retryable("files_ledger_write_conflict"))
        }
    }
}

/// Delete the ledger row and return the size it held (0 if absent or stale).
async fn record_deletion(
    ctx: &FileTrackingContext,
    id: &str,
    app_id: &str,
    event: &GcsEvent,
) -> Result<(i64, LedgerRollback), FileTrackingError> {
    let Some(existing) = ctx
        .firestore
        .read_document::<LedgerDocument>(&ctx.files_collection, id)
        .await
        .map_err(map_firestore)?
    else {
        tracing::warn!(object = %event.object, "no ledger row for deleted object");
        return Ok((0, LedgerRollback::None));
    };
    ensure_same_app(&existing.value, app_id)?;
    if is_stale_deletion(&existing.value, event.generation) {
        tracing::info!(object = %event.object, "ignoring stale OBJECT_DELETE event");
        return Ok((0, LedgerRollback::None));
    }

    match ctx
        .firestore
        .delete_document(&ctx.files_collection, id, existing.update_time.as_deref())
        .await
        .map_err(map_firestore)?
    {
        MutationOutcome::Applied => Ok((
            existing.value.size,
            LedgerRollback::Recreate {
                previous: Box::new(existing.value),
            },
        )),
        MutationOutcome::NotFound => Ok((0, LedgerRollback::None)),
        MutationOutcome::Conflict | MutationOutcome::PreconditionFailed => {
            Err(FileTrackingError::Retryable("files_ledger_write_conflict"))
        }
    }
}

/// Write one ledger row and return the `updateTime` it produced.
///
/// This goes through `commit` rather than through `create_document` /
/// `replace_document` for one reason: only `commit` reports the resulting
/// `updateTime`, and without it a rollback would have to write blind.
async fn write_ledger(
    ctx: &FileTrackingContext,
    id: &str,
    document: &LedgerDocument,
    update_time: Option<Option<&str>>,
) -> Result<(MutationOutcome, Option<String>), FileTrackingError> {
    let mut batch = ctx.firestore.write_batch();
    match update_time {
        Some(update_time) => batch.replace(&ctx.files_collection, id, document, None, update_time),
        None => batch.create(&ctx.files_collection, id, document, None),
    }
    .map(|_| ())
    .map_err(map_firestore)?;

    let outcome = ctx.firestore.commit(batch).await.map_err(map_firestore)?;
    Ok((
        outcome.outcome,
        outcome.update_times.into_iter().next().flatten(),
    ))
}

/// Undo this event's ledger write. Best effort: the delivery is already failing
/// and will be redelivered, and a rollback that itself fails leaves the row
/// owned by whoever wrote it last, which the next delivery reconciles against.
async fn rollback_ledger(ctx: &FileTrackingContext, id: &str, rollback: LedgerRollback) {
    let outcome = match &rollback {
        LedgerRollback::None => return,
        LedgerRollback::Remove { update_time } => {
            let Some(update_time) = update_time.as_deref() else {
                tracing::error!(
                    ledger_id = %id,
                    "ledger row has no updateTime to roll back against; leaving it in place"
                );
                return;
            };
            ctx.firestore
                .delete_document(&ctx.files_collection, id, Some(update_time))
                .await
        }
        LedgerRollback::Restore {
            previous,
            update_time,
        } => {
            let Some(update_time) = update_time.as_deref() else {
                tracing::error!(
                    ledger_id = %id,
                    "ledger row has no updateTime to roll back against; leaving it in place"
                );
                return;
            };
            ctx.firestore
                .replace_document(
                    &ctx.files_collection,
                    id,
                    previous.as_ref(),
                    None,
                    Some(update_time),
                )
                .await
        }
        // The row was deleted, so there is no `updateTime` to swap against.
        // Create-if-absent is the equivalent guard: a row that has reappeared
        // belongs to a newer event and must not be overwritten with this one's
        // stale snapshot.
        LedgerRollback::Recreate { previous } => {
            ctx.firestore
                .create_document(&ctx.files_collection, id, previous.as_ref(), None)
                .await
        }
    };

    match outcome {
        Ok(MutationOutcome::Applied) => {
            tracing::warn!(ledger_id = %id, "rolled the files ledger back after a usage-update failure")
        }
        Ok(outcome) => tracing::warn!(
            ledger_id = %id,
            ?outcome,
            "a newer event owns the ledger row; leaving its value in place"
        ),
        Err(error) => tracing::error!(
            ledger_id = %id,
            error = %error,
            "files ledger rollback failed; the row may over-report until the next event"
        ),
    }
}

/// `UPDATE "App" SET "totalSize" = "totalSize" + delta` (and the same for
/// `"User"` when the key is user-scoped) in one retried transaction; a missing
/// row is a permanent failure like on AWS. Relative increments keep concurrent
/// events safe without a read-modify-write, and a lost commit race on this hot
/// row is re-run by `retry_transaction`; an unknown commit outcome is not,
/// because applying the delta twice would over-count.
async fn apply_usage_delta(
    database: &DatabaseConnection,
    dialect: DbDialect,
    user_id: Option<&str>,
    app_id: &str,
    delta: i64,
) -> Result<(), FileTrackingError> {
    let app_key = app_id.to_owned();
    let user_key = user_id.map(str::to_owned);
    retry_transaction::<_, (), UsageError>(
        database,
        dialect,
        None,
        &RetryPolicy::default(),
        move |txn| {
            let app_id = app_key.clone();
            let user_id = user_key.clone();
            Box::pin(async move {
                let app_rows = txn
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"UPDATE "public"."App" SET "totalSize" = "totalSize" + $1 WHERE "id" = $2"#,
                        [delta.into(), app_id.into()],
                    ))
                    .await?
                    .rows_affected();
                if app_rows == 0 {
                    return Err(UsageError::AppNotFound);
                }

                if let Some(user_id) = user_id {
                    let user_rows = txn
                        .execute_raw(Statement::from_sql_and_values(
                            DatabaseBackend::Postgres,
                            r#"UPDATE "public"."User" SET "totalSize" = "totalSize" + $1 WHERE "id" = $2"#,
                            [delta.into(), user_id.into()],
                        ))
                        .await?
                        .rows_affected();
                    if user_rows == 0 {
                        return Err(UsageError::UserNotFound);
                    }
                }
                Ok(())
            })
        },
    )
    .await
    .map_err(|error| match error {
        UsageError::AppNotFound => FileTrackingError::Permanent("usage_app_not_found"),
        UsageError::UserNotFound => FileTrackingError::Permanent("usage_user_not_found"),
        UsageError::Db(error) => {
            tracing::warn!(error = %error, app_id, "usage update failed");
            FileTrackingError::Retryable("usage_database_unavailable")
        }
    })
}

#[derive(Debug)]
enum UsageError {
    Db(DbErr),
    AppNotFound,
    UserNotFound,
}

impl From<DbErr> for UsageError {
    fn from(error: DbErr) -> Self {
        Self::Db(error)
    }
}

impl AsDbConflict for UsageError {
    fn db_conflict(&self) -> Option<DbConflict> {
        match self {
            Self::Db(error) => error.db_conflict(),
            Self::AppNotFound | Self::UserNotFound => None,
        }
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(error) => write!(f, "{error}"),
            Self::AppNotFound => f.write_str("app not found"),
            Self::UserNotFound => f.write_str("user not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(generation: i64) -> LedgerDocument {
        LedgerDocument {
            id: "i".into(),
            app_id: "a".into(),
            sk: "k".into(),
            size: 1,
            user_id: String::new(),
            etag: String::new(),
            generation,
            updated_at: 0,
        }
    }

    #[test]
    fn identity_from_app_and_user_keys() {
        assert_eq!(
            parse_key_identity("apps/a1/storage/db/x.lance"),
            Some(KeyIdentity {
                user_id: None,
                app_id: "a1".into()
            })
        );
        assert_eq!(
            parse_key_identity("users/u1/apps/a1/db/x"),
            Some(KeyIdentity {
                user_id: Some("u1".into()),
                app_id: "a1".into()
            })
        );
        assert_eq!(parse_key_identity("media/x.png"), None);
        assert_eq!(parse_key_identity("users/u1"), None);
        assert_eq!(parse_key_identity("apps//x"), None);
    }

    /// The lexicographic form this replaces would call generation 1_755_254_400
    /// stale against 999_999_999, because `"9"` sorts above `"1"`. The
    /// equal-generation case is stale for a finalize only: that is a redelivery
    /// of the write that produced the row.
    #[test]
    fn stale_detection_compares_generations_numerically() {
        let existing = row(1_755_254_400_000_002);
        assert!(is_stale(&existing, Some(1_755_254_400_000_001)));
        assert!(is_stale(&existing, Some(1_755_254_400_000_002)));
        assert!(!is_stale(&existing, Some(1_755_254_400_000_003)));
        assert!(!is_stale(&existing, None));
        assert!(!is_stale(&row(999_999_999), Some(1_755_254_400_000_001)));
    }

    /// An OBJECT_DELETE carries the same generation as the OBJECT_FINALIZE that
    /// wrote the row, so equality is the *normal* delete and must go through;
    /// only a strictly newer row (the object was overwritten first) is stale.
    #[test]
    fn deletion_staleness_is_strict() {
        let generation = 1_755_254_400_000_002;
        assert!(!is_stale_deletion(&row(generation), Some(generation)));
        assert!(is_stale_deletion(&row(generation), Some(generation - 1)));
        assert!(!is_stale_deletion(&row(generation), Some(generation + 1)));
        assert!(!is_stale_deletion(&row(generation), None));
        assert!(!is_stale_deletion(&row(0), Some(generation)));
    }

    /// A row written before generations were tracked must not swallow the first
    /// event that carries one.
    #[test]
    fn rows_without_a_generation_are_never_stale() {
        assert!(!is_stale(&row(0), Some(1)));
        assert!(!is_stale_deletion(&row(0), Some(1)));
    }

    #[test]
    fn ledger_ids_are_path_safe_and_stable() {
        let id = ledger_id("apps/a1/storage/db/x.lance");
        assert_eq!(id.len(), 64);
        assert!(!id.contains('/'));
        assert_eq!(id, ledger_id("apps/a1/storage/db/x.lance"));
    }
}

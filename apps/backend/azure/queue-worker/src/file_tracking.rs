//! File-size accounting for released content, the Azure port of
//! `apps/backend/aws/file-tracker`.
//!
//! Every `BlobCreated`/`BlobDeleted` under `apps/` and `users/` updates a
//! per-blob ledger row in the Cosmos `files` container (`pk` = app id) and
//! applies the size delta to `App.totalSize` (and `User.totalSize` for
//! user-scoped keys) inside one PostgreSQL transaction. The ledger, not the
//! event, is the source of the previous size, exactly like the DynamoDB table
//! on AWS. Two things are stricter than the AWS original: rows carry the Event
//! Grid `sequencer`, so an out-of-order or duplicate event never regresses the
//! ledger, and every ledger write is ETag-conditional.

use crate::blob_events::BlobEvent;
use flow_like_azure_data::cosmos::{CosmosClient, CosmosError, MutationOutcome};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, TransactionTrait};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FILES_CONTAINER: &str = "files";

pub struct FileTrackingContext {
    pub cosmos: CosmosClient,
    pub files_container: String,
    pub content_container: String,
    pub database: DatabaseConnection,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileTrackingError {
    /// The message can never be processed; poison it.
    Permanent(&'static str),
    /// Transient store failure; release the message for a retry.
    Retryable(&'static str),
}

/// Ledger row: one per blob, partitioned by app id.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LedgerDocument {
    id: String,
    pk: String,
    sk: String,
    size: i64,
    user_id: String,
    etag: String,
    #[serde(default)]
    sequencer: String,
    updated_at: i64,
    #[serde(rename = "_etag", default, skip_serializing)]
    cosmos_etag: Option<String>,
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

/// Cosmos ids may not contain `/`, `\`, `?` or `#`; blob keys are paths.
fn ledger_id(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

fn is_stale(existing: &LedgerDocument, incoming: Option<&str>) -> bool {
    match incoming {
        Some(sequencer) => {
            !existing.sequencer.is_empty() && existing.sequencer.as_str() >= sequencer
        }
        None => false,
    }
}

fn map_cosmos(error: CosmosError) -> FileTrackingError {
    tracing::warn!(error = %error, "files ledger request failed");
    FileTrackingError::Retryable("files_ledger_unavailable")
}

pub async fn process(
    ctx: &FileTrackingContext,
    event: &BlobEvent,
) -> Result<(), FileTrackingError> {
    let (container, key) = event
        .container_and_key()
        .ok_or(FileTrackingError::Permanent("invalid_blob_subject"))?;
    if container != ctx.content_container {
        tracing::warn!(
            container,
            "blob event for a container this worker does not track"
        );
        return Ok(());
    }
    let identity =
        parse_key_identity(key).ok_or(FileTrackingError::Permanent("invalid_blob_key_layout"))?;

    let delta = if event.is_deletion() {
        -record_deletion(ctx, &identity.app_id, key, event.sequencer.as_deref()).await?
    } else {
        let size = event.content_length.ok_or(FileTrackingError::Permanent(
            "blob_created_without_content_length",
        ))?;
        let previous = record_creation(ctx, &identity, key, size, event).await?;
        size - previous
    };

    if delta == 0 {
        return Ok(());
    }
    apply_usage_delta(
        &ctx.database,
        identity.user_id.as_deref(),
        &identity.app_id,
        delta,
    )
    .await
}

/// Upsert the ledger row and return the previous size (0 if absent or stale).
async fn record_creation(
    ctx: &FileTrackingContext,
    identity: &KeyIdentity,
    key: &str,
    size: i64,
    event: &BlobEvent,
) -> Result<i64, FileTrackingError> {
    let id = ledger_id(key);
    let existing: Option<LedgerDocument> = ctx
        .cosmos
        .read_document(&ctx.files_container, &id, &identity.app_id)
        .await
        .map_err(map_cosmos)?;

    if let Some(existing) = &existing {
        if is_stale(existing, event.sequencer.as_deref()) {
            tracing::info!(key, "ignoring stale BlobCreated event");
            return Ok(size);
        }
    }

    let document = LedgerDocument {
        id: id.clone(),
        pk: identity.app_id.clone(),
        sk: key.to_string(),
        size,
        user_id: identity.user_id.clone().unwrap_or_default(),
        etag: event.etag.clone().unwrap_or_default(),
        sequencer: event.sequencer.clone().unwrap_or_default(),
        updated_at: chrono::Utc::now().timestamp(),
        cosmos_etag: None,
    };

    let outcome = match &existing {
        Some(current) => ctx
            .cosmos
            .replace_document(
                &ctx.files_container,
                &id,
                &identity.app_id,
                &document,
                current.cosmos_etag.as_deref(),
            )
            .await
            .map_err(map_cosmos)?,
        None => ctx
            .cosmos
            .create_document(&ctx.files_container, &identity.app_id, &document)
            .await
            .map_err(map_cosmos)?,
    };
    match outcome {
        MutationOutcome::Applied => Ok(existing.map(|row| row.size).unwrap_or(0)),
        // Another replica moved the row between read and write; a retry re-reads it.
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
    app_id: &str,
    key: &str,
    sequencer: Option<&str>,
) -> Result<i64, FileTrackingError> {
    let id = ledger_id(key);
    let Some(existing): Option<LedgerDocument> = ctx
        .cosmos
        .read_document(&ctx.files_container, &id, app_id)
        .await
        .map_err(map_cosmos)?
    else {
        tracing::warn!(key, "no ledger row for deleted blob");
        return Ok(0);
    };
    if is_stale(&existing, sequencer) {
        tracing::info!(key, "ignoring stale BlobDeleted event");
        return Ok(0);
    }
    match ctx
        .cosmos
        .delete_document(
            &ctx.files_container,
            &id,
            app_id,
            existing.cosmos_etag.as_deref(),
        )
        .await
        .map_err(map_cosmos)?
    {
        MutationOutcome::Applied => Ok(existing.size),
        MutationOutcome::NotFound => Ok(0),
        MutationOutcome::Conflict | MutationOutcome::PreconditionFailed => {
            Err(FileTrackingError::Retryable("files_ledger_write_conflict"))
        }
    }
}

/// `UPDATE "App" SET "totalSize" = "totalSize" + delta` (and the same for
/// `"User"` when the key is user-scoped) in one transaction; a missing row is a
/// permanent failure like on AWS. Relative increments keep concurrent events
/// safe without a read-modify-write.
async fn apply_usage_delta(
    database: &DatabaseConnection,
    user_id: Option<&str>,
    app_id: &str,
    delta: i64,
) -> Result<(), FileTrackingError> {
    let txn = database.begin().await.map_err(|error| {
        tracing::warn!(error = %error, "usage transaction could not start");
        FileTrackingError::Retryable("usage_database_unavailable")
    })?;

    let app_rows = txn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE "public"."App" SET "totalSize" = "totalSize" + $1 WHERE "id" = $2"#,
            [delta.into(), app_id.into()],
        ))
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, app_id, "app usage update failed");
            FileTrackingError::Retryable("usage_database_unavailable")
        })?
        .rows_affected();
    if app_rows == 0 {
        return Err(FileTrackingError::Permanent("usage_app_not_found"));
    }

    if let Some(user_id) = user_id {
        let user_rows = txn
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"UPDATE "public"."User" SET "totalSize" = "totalSize" + $1 WHERE "id" = $2"#,
                [delta.into(), user_id.into()],
            ))
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "user usage update failed");
                FileTrackingError::Retryable("usage_database_unavailable")
            })?
            .rows_affected();
        if user_rows == 0 {
            return Err(FileTrackingError::Permanent("usage_user_not_found"));
        }
    }

    txn.commit().await.map_err(|error| {
        tracing::warn!(error = %error, "usage transaction commit failed");
        FileTrackingError::Retryable("usage_database_unavailable")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn stale_detection_uses_sequencer_order() {
        let row = LedgerDocument {
            id: "i".into(),
            pk: "a".into(),
            sk: "k".into(),
            size: 1,
            user_id: String::new(),
            etag: String::new(),
            sequencer: "00000000000004420000000000028963".into(),
            updated_at: 0,
            cosmos_etag: None,
        };
        assert!(is_stale(&row, Some("00000000000004420000000000028962")));
        assert!(is_stale(&row, Some("00000000000004420000000000028963")));
        assert!(!is_stale(&row, Some("00000000000004420000000000028964")));
        assert!(!is_stale(&row, None));
    }

    #[test]
    fn ledger_ids_are_path_safe_and_stable() {
        let id = ledger_id("apps/a1/storage/db/x.lance");
        assert_eq!(id.len(), 64);
        assert!(!id.contains('/'));
        assert_eq!(id, ledger_id("apps/a1/storage/db/x.lance"));
    }
}

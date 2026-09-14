//! Private execution inputs have a shorter lifetime than the billing ledger.
//! Expiring a prompt or staged request never releases an unknown reservation.

use flow_like_storage::object_store::{ObjectStore, path::Path};
use futures_util::{StreamExt, TryStreamExt, stream};
use sea_orm::{ConnectionTrait, Statement};

use crate::{error::ApiError, state::AppState};

const MAX_OPERATIONS: usize = 25;
const MAX_OBJECTS: usize = 250;
const READER_GRACE_MS: i64 = 300_000;
const UNKNOWN_INPUT_RETENTION_MS: i64 = 86_400_000;

#[derive(Clone, Copy)]
pub enum Scope<'a> {
    App(&'a str),
    Payer(&'a str),
}

impl Scope<'_> {
    fn column(&self) -> &'static str {
        match self {
            Self::App(_) => "appId",
            Self::Payer(_) => "payerId",
        }
    }
    fn value(&self) -> &str {
        match self {
            Self::App(id) | Self::Payer(id) => id,
        }
    }
}

/// Mark only a bounded page per deletion pass. Workers observe cancellation while
/// retained quota rows still allow their terminal receipts to be reconciled.
pub async fn request_cancellation(state: &AppState, scope: Scope<'_>) -> Result<bool, ApiError> {
    let sql = format!(
        r#"SELECT id FROM "QuotaOperation" WHERE "{}" = $1 AND status != 'finalized' AND "cancelRequested" = false ORDER BY id LIMIT 100"#,
        scope.column()
    );
    let rows = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            state.db.get_database_backend(),
            sql,
            [scope.value().into()],
        ))
        .await?;
    for row in &rows {
        let id: String = row.try_get("", "id")?;
        state.db.execute_raw(Statement::from_sql_and_values(state.db.get_database_backend(),
            r#"UPDATE "QuotaOperation" SET "cancelRequested" = true WHERE id = $1 AND status != 'finalized'"#,[id.into()])).await?;
    }
    Ok(rows.len() < 100)
}

fn private_prefixes(operation_id: &str, finalized: bool) -> Vec<String> {
    let hash = blake3::hash(operation_id.as_bytes()).to_hex();
    let mut prefixes = vec![format!("system/dispatch/{hash}")];
    if !operation_id.is_empty()
        && operation_id.len() <= 128
        && operation_id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
    {
        prefixes.push(format!("system/hosted-ai/{operation_id}"));
    }
    if finalized {
        prefixes.push(format!("system/quota/{hash}"));
    }
    prefixes
}

/// Return the number of operations fully cleaned. Each pass has fixed row/object
/// budgets and resumes from retained markers. Unknown operations retain billing
/// evidence and reservations even after their private input has expired.
pub async fn cleanup(state: &AppState, scope: Option<Scope<'_>>) -> Result<u64, ApiError> {
    let now = chrono::Utc::now().timestamp_millis();
    let filter = scope
        .as_ref()
        .map(|scope| format!(r#" AND "{}" = $3"#, scope.column()))
        .unwrap_or_default();
    let sql = format!(
        r#"SELECT id,status FROM "QuotaOperation" WHERE "payloadsDeletedAt" IS NULL AND ((status = 'finalized' AND deadline < $1 AND (kind != 'workflow' OR "createdAt" < $2)) OR (status != 'finalized' AND deadline < $2)){filter} ORDER BY deadline LIMIT {MAX_OPERATIONS}"#
    );
    let mut values = vec![
        (now - READER_GRACE_MS).into(),
        (now - UNKNOWN_INPUT_RETENTION_MS).into(),
    ];
    if let Some(scope) = scope {
        values.push(scope.value().into());
    }
    let rows = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            state.db.get_database_backend(),
            sql,
            values,
        ))
        .await?;
    let mut completed = 0;
    let started = std::time::Instant::now();
    let store = state.meta_bucket.as_generic();
    for row in rows {
        let operation_id: String = row.try_get("", "id")?;
        let finalized = row.try_get::<String>("", "status")? == "finalized";
        let mut done = true;
        for prefix in private_prefixes(&operation_id, finalized) {
            if started.elapsed() >= std::time::Duration::from_secs(5) {
                return Ok(completed);
            }
            let result = flow_like_types::tokio::time::timeout(
                std::time::Duration::from_secs(3),
                cleanup_prefix(store.as_ref(), &prefix),
            )
            .await;
            match result {
                Ok(result) => done &= result?,
                Err(_) => return Ok(completed),
            }
        }
        if done {
            state.db.execute_raw(Statement::from_sql_and_values(state.db.get_database_backend(),
                r#"UPDATE "QuotaOperation" SET "payloadsDeletedAt" = $2 WHERE id = $1 AND "payloadsDeletedAt" IS NULL"#,[operation_id.into(),now.into()])).await?;
            completed += 1;
        }
    }
    Ok(completed)
}

async fn cleanup_prefix(store: &dyn ObjectStore, prefix: &str) -> Result<bool, ApiError> {
    let boundary = format!("{prefix}/");
    let entries = store
        .list(Some(&Path::from(prefix)))
        .try_filter(move |entry| {
            futures_util::future::ready(entry.location.as_ref().starts_with(&boundary))
        })
        .take(MAX_OBJECTS + 1)
        .try_collect::<Vec<_>>()
        .await?;
    let done = entries.len() <= MAX_OBJECTS;
    store
        .delete_stream(
            stream::iter(
                entries
                    .into_iter()
                    .take(MAX_OBJECTS)
                    .map(|entry| Ok(entry.location)),
            )
            .boxed(),
        )
        .try_for_each(|_| async { Ok(()) })
        .await?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn private_cleanup_is_bounded_and_does_not_cross_an_operation_boundary() {
        use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};
        let store = InMemory::new();
        for i in 0..=MAX_OBJECTS {
            store
                .put(
                    &Path::from(format!("system/hosted-ai/op/{i:08}")),
                    vec![0u8].into(),
                )
                .await
                .unwrap();
        }
        let sibling = Path::from("system/hosted-ai/op-other/request.json");
        store.put(&sibling, vec![1u8].into()).await.unwrap();
        assert!(!cleanup_prefix(&store, "system/hosted-ai/op").await.unwrap());
        assert_eq!(
            store
                .list(Some(&Path::from("system/hosted-ai/op")))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(cleanup_prefix(&store, "system/hosted-ai/op").await.unwrap());
        assert!(store.head(&sibling).await.is_ok());
    }

    #[test]
    fn only_finalized_operations_lose_their_billing_receipt() {
        assert!(
            private_prefixes("operation_1", true)
                .iter()
                .any(|p| p.starts_with("system/quota/"))
        );
        assert!(
            !private_prefixes("operation_1", false)
                .iter()
                .any(|p| p.starts_with("system/quota/"))
        );
        assert!(
            !private_prefixes("../other", false)
                .iter()
                .any(|p| p.starts_with("system/hosted-ai/"))
        );
    }
}

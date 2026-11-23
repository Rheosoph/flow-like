//! Routine maintenance of hosted Lance tables.
//!
//! A table only degrades when it is written, and the AWS file tracker touches a dataset's
//! accounting row (`objectKey` ending in `.lance/`) on every write. A pass therefore looks only
//! at tables written recently and idle since, checks their manifest against the thresholds in
//! [`LanceDBVectorStore::maintenance_needs`], and compacts or re-indexes them. History is never
//! pruned; that stays an explicit Optimize.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use flow_like_storage::databases::vector::lancedb::{LanceDBVectorStore, MaintenanceNeeds};
use flow_like_types::maintenance::TableMaintenanceResult;
use sea_orm::{ConnectionTrait, DbErr, Statement};

use crate::{credentials::RuntimeCredentials, error::ApiError, state::AppState};

/// Writers get this long to finish a burst before their table is touched, so maintenance
/// rarely races a run's own commits.
const IDLE: Duration = Duration::from_secs(120);
/// Longer than the schedule interval, so every write burst is checked even if a pass fails.
const DEFAULT_LOOKBACK: Duration = Duration::from_secs(1_800);
const LOOKBACK_ENV: &str = "TABLE_MAINTENANCE_LOOKBACK_SECS";
const MAX_CANDIDATES: u32 = 200;
/// The maintenance runner gives up on the response after 120 s.
const START_BUDGET: Duration = Duration::from_secs(45);
/// A table still running at this point is abandoned before its commit; Lance leaves only
/// unreferenced files behind, never a partial version.
const TABLE_BUDGET: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
struct TableLocation {
    app_id: String,
    user: Option<String>,
    table: String,
}

/// The lancedb table a dataset directory holds; `None` for Lance data outside a database.
fn locate(key: &str) -> Option<TableLocation> {
    let parts = key.strip_suffix(".lance/")?.split('/').collect::<Vec<_>>();
    let (app_id, user, table) = match parts.as_slice() {
        ["apps", app, "storage", "db", table] => (*app, None, *table),
        ["users", user, "apps", app, "db", table] => (*app, Some(*user), *table),
        _ => return None,
    };
    let named = !app_id.is_empty() && !table.is_empty() && user.is_none_or(|user| !user.is_empty());
    named.then(|| TableLocation {
        app_id: app_id.to_owned(),
        user: user.map(str::to_owned),
        table: table.to_owned(),
    })
}

fn lookback() -> Duration {
    std::env::var(LOOKBACK_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(DEFAULT_LOOKBACK, Duration::from_secs)
}

async fn recently_written(
    state: &AppState,
    after: DateTime<Utc>,
    before: DateTime<Utc>,
) -> Result<Vec<TableLocation>, DbErr> {
    let rows = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            state.db.get_database_backend(),
            format!(
                r#"SELECT "objectKey" FROM "FileAccountingObject"
                WHERE "updatedAt" > $1 AND "updatedAt" <= $2 AND "size" > 0 AND "objectKey" LIKE '%.lance/'
                ORDER BY "updatedAt" LIMIT {MAX_CANDIDATES}"#
            ),
            [after.into(), before.into()],
        ))
        .await?;
    let mut locations = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(location) = locate(&row.try_get::<String>("", "objectKey")?) {
            locations.push(location);
        }
    }
    Ok(locations)
}

async fn maintain_table(
    credentials: &RuntimeCredentials,
    location: &TableLocation,
) -> flow_like_types::Result<MaintenanceNeeds> {
    let builder = match &location.user {
        Some(user) => credentials.to_db_scoped(user, &location.app_id).await?,
        None => credentials.to_db(&location.app_id).await?,
    };
    let store =
        LanceDBVectorStore::from_connection(builder.execute().await?, location.table.clone()).await;
    let needs = store.maintenance_needs().await?;
    store.maintain(needs).await?;
    Ok(needs)
}

pub async fn run_pass(state: &AppState) -> Result<TableMaintenanceResult, ApiError> {
    let started = Instant::now();
    let now = Utc::now();
    let candidates = recently_written(state, now - lookback(), now - IDLE).await?;
    let credentials = state.master_credentials().await?;
    let mut result = TableMaintenanceResult::default();
    for (index, location) in candidates.iter().enumerate() {
        if started.elapsed() >= START_BUDGET {
            result.deferred = (candidates.len() - index) as u64;
            break;
        }
        result.evaluated += 1;
        match tokio::time::timeout(TABLE_BUDGET, maintain_table(&credentials, location)).await {
            Ok(Ok(needs)) => {
                result.compacted += u64::from(needs.compact);
                result.reindexed += u64::from(needs.index);
                if needs.any() {
                    tracing::info!(
                        app_id = %location.app_id,
                        table = %location.table,
                        user_scoped = location.user.is_some(),
                        compacted = needs.compact,
                        reindexed = needs.index,
                        "Maintained Lance table"
                    );
                }
            }
            Ok(Err(error)) => {
                result.failed += 1;
                tracing::warn!(
                    app_id = %location.app_id,
                    table = %location.table,
                    user_scoped = location.user.is_some(),
                    error = %error,
                    "Lance table maintenance failed"
                );
            }
            Err(_) => {
                result.failed += 1;
                tracing::warn!(
                    app_id = %location.app_id,
                    table = %location.table,
                    user_scoped = location.user.is_some(),
                    budget_secs = TABLE_BUDGET.as_secs(),
                    "Lance table maintenance ran out of time"
                );
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_directories_map_to_their_database_tables() {
        assert_eq!(
            locate("apps/app/storage/db/notes.lance/"),
            Some(TableLocation {
                app_id: "app".into(),
                user: None,
                table: "notes".into(),
            })
        );
        assert_eq!(
            locate("users/sub/apps/app/db/notes.lance/"),
            Some(TableLocation {
                app_id: "app".into(),
                user: Some("sub".into()),
                table: "notes".into(),
            })
        );
        for elsewhere in [
            "apps/app/storage/graph/nodes.lance/",
            "users/sub/apps/app/storage/db/notes.lance/",
            "apps/app/storage/db/nested/notes.lance/",
            "apps//storage/db/notes.lance/",
            "apps/app/storage/db/.lance/",
            "users//apps/app/db/notes.lance/",
            "apps/app/storage/db/notes.lance",
        ] {
            assert_eq!(locate(elsewhere), None, "{elsewhere}");
        }
    }
}

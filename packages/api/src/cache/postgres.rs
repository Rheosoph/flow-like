//! PostgreSQL cache backend.
//!
//! The default backend: no extra infrastructure, but no native TTL either. Expired rows
//! are hidden on read and physically removed by the cache sweeper.
//!
//! `try_insert` relies on `ON CONFLICT ... DO UPDATE ... WHERE`, which Postgres supports
//! and MySQL does not. The workspace targets Postgres only (`sqlx-postgres`); porting
//! this backend elsewhere would need that guard re-expressed, or concurrent callers
//! could all believe they claimed the key.

use super::types::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use flow_like_types::cache::CacheScope;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr,
    EntityTrait, PaginatorTrait, QueryFilter, QueryResult, Set, Statement, TransactionTrait,
    sea_query::{Expr, OnConflict},
};
use std::sync::Arc;

use crate::entity::{app_cache_entry, sea_orm_active_enums::CacheScope as EntityCacheScope};

#[derive(Debug, Clone)]
pub struct PostgresCacheStore {
    db: Arc<DatabaseConnection>,
}

impl PostgresCacheStore {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

fn scope_to_entity(scope: CacheScope) -> EntityCacheScope {
    match scope {
        CacheScope::App => EntityCacheScope::App,
        CacheScope::User => EntityCacheScope::User,
    }
}

fn ts_to_datetime(ts: i64) -> sea_orm::prelude::DateTime {
    Utc.timestamp_millis_opt(ts)
        .single()
        .unwrap_or_else(|| Utc.timestamp_nanos(0))
        .naive_utc()
}

fn datetime_to_ts(dt: sea_orm::prelude::DateTime) -> i64 {
    dt.and_utc().timestamp_millis()
}

fn model_to_entry(model: app_cache_entry::Model) -> CacheEntry {
    CacheEntry {
        key: model.key,
        value: model.value,
        expires_at: model.expires_at.map(datetime_to_ts),
        updated_at: datetime_to_ts(model.updated_at),
    }
}

fn key_condition(key: &CacheKey) -> Condition {
    Condition::all()
        .add(app_cache_entry::Column::AppId.eq(key.app_id.clone()))
        .add(app_cache_entry::Column::Scope.eq(scope_to_entity(key.scope)))
        .add(app_cache_entry::Column::UserId.eq(key.user_id.clone()))
        .add(app_cache_entry::Column::Namespace.eq(key.namespace.clone()))
        .add(app_cache_entry::Column::Key.eq(key.key.clone()))
}

/// Matches rows that have not expired: either no TTL at all, or one still in the future.
fn live_condition(now: sea_orm::prelude::DateTime) -> Condition {
    Condition::any()
        .add(app_cache_entry::Column::ExpiresAt.is_null())
        .add(app_cache_entry::Column::ExpiresAt.gt(now))
}

fn active_model(
    entry: &SetCacheEntry,
    now: sea_orm::prelude::DateTime,
) -> app_cache_entry::ActiveModel {
    app_cache_entry::ActiveModel {
        app_id: Set(entry.key.app_id.clone()),
        scope: Set(scope_to_entity(entry.key.scope)),
        user_id: Set(entry.key.user_id.clone()),
        namespace: Set(entry.key.namespace.clone()),
        key: Set(entry.key.key.clone()),
        value: Set(entry.value.clone()),
        expires_at: Set(entry.expires_at.map(ts_to_datetime)),
        created_at: Set(now),
        updated_at: Set(now),
    }
}

/// Ceiling on the expired-row probe, and therefore on what it may scan.
///
/// A `LIMIT` inside a subquery is an optimization fence Postgres never pulls up, so the
/// count stops here instead of walking a cache table that could hold millions of rows.
/// Reaching the cap means "more than `EXPIRED_SCAN_CAP - 1`", which is what
/// [`CacheStoreStats::expired_pending_capped`] tells the dashboard to render.
const EXPIRED_SCAN_CAP: i64 = 10_001;

/// Timeouts applied to every statistics probe.
///
/// `set_config(..., true)` is `SET LOCAL`, so the values die with the surrounding
/// transaction rather than leaking into the next request that borrows this pooled
/// connection. The lock timeout is the one that earns its keep: `pg_total_relation_size`
/// opens the relation with an `AccessShareLock`, so a migration holding
/// `AccessExclusiveLock` would otherwise hang the admin dashboard indefinitely.
const PROBE_TIMEOUTS_SQL: &str =
    "SELECT set_config('statement_timeout', '3s', true), set_config('lock_timeout', '1s', true)";

/// Planner estimates for the cache table, resolved through `search_path` exactly as the
/// ORM's own statements are. `to_regclass` yields NULL rather than raising when the table
/// is absent, so a fresh database degrades to "no numbers" instead of an error.
const TABLE_ESTIMATE_SQL: &str = r#"SELECT
        COALESCE(stats.n_live_tup, 0)::bigint AS live_rows,
        EXTRACT(EPOCH FROM GREATEST(stats.last_analyze, stats.last_autoanalyze))::bigint
            AS analyzed_at
    FROM pg_stat_all_tables stats
    WHERE stats.relid = to_regclass('"AppCacheEntry"')"#;

/// Heap plus indexes plus TOAST. The size functions carry no privilege check at all, but
/// they do return NULL for a relation dropped between the catalog scan and the call.
const TABLE_SIZE_SQL: &str = r#"SELECT pg_total_relation_size(cls.oid)::bigint AS total_bytes
    FROM pg_class cls
    WHERE cls.oid = to_regclass('"AppCacheEntry"')"#;

/// `ORDER BY` is load-bearing, not cosmetic: without it the planner may pick a sequential
/// scan, which still honours the `LIMIT` but reads the whole table first whenever few
/// rows actually match — the exact shape of a healthy, well-swept cache. The ordering
/// makes the index on `expiresAt` the cheaper plan.
///
/// The cutoff is bound as a naive timestamp because `expiresAt` is
/// `timestamp without time zone`; comparing it against `now()` would silently shift the
/// boundary by the session's UTC offset.
const EXPIRED_PENDING_SQL: &str = r#"SELECT count(*)::bigint AS expired
    FROM (
        SELECT 1
        FROM "AppCacheEntry"
        WHERE "expiresAt" <= $1
        ORDER BY "expiresAt"
        LIMIT $2
    ) capped"#;

/// Run one statistics probe under its own transaction and timeouts.
///
/// Each probe gets a transaction of its own because one failed statement poisons the rest
/// of a Postgres transaction: sharing one would let a lock timeout on the size function
/// erase the row estimate that had already succeeded.
async fn probe(
    db: &DatabaseConnection,
    statement: Statement,
) -> Result<Option<QueryResult>, DbErr> {
    let timeouts = Statement::from_string(DatabaseBackend::Postgres, PROBE_TIMEOUTS_SQL);
    let txn = db.begin().await?;
    txn.execute(timeouts).await?;
    let row = txn.query_one(statement).await?;
    txn.commit().await?;
    Ok(row)
}

fn probed_column<T: sea_orm::TryGetable>(
    result: &Result<Option<QueryResult>, DbErr>,
    column: &str,
) -> Option<T> {
    result
        .as_ref()
        .ok()?
        .as_ref()?
        .try_get::<T>("", column)
        .ok()
}

#[async_trait]
impl CacheStore for PostgresCacheStore {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        let model = app_cache_entry::Entity::find()
            .filter(key_condition(key))
            .one(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        let Some(model) = model else {
            return Ok(None);
        };

        let entry = model_to_entry(model);
        if entry.is_expired_at(Utc::now().timestamp_millis()) {
            // The sweeper will reclaim the row; the caller must see a miss right now.
            return Ok(None);
        }

        Ok(Some(entry))
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        // Liveness is pushed into SQL so an expired row reads as absent without the
        // value ever leaving the database.
        let count = app_cache_entry::Entity::find()
            .filter(key_condition(key))
            .filter(live_condition(Utc::now().naive_utc()))
            .count(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(count > 0)
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now = Utc::now().naive_utc();

        app_cache_entry::Entity::insert(active_model(&entry, now))
            .on_conflict(
                OnConflict::columns([
                    app_cache_entry::Column::AppId,
                    app_cache_entry::Column::Scope,
                    app_cache_entry::Column::UserId,
                    app_cache_entry::Column::Namespace,
                    app_cache_entry::Column::Key,
                ])
                .update_columns([
                    app_cache_entry::Column::Value,
                    app_cache_entry::Column::ExpiresAt,
                    app_cache_entry::Column::UpdatedAt,
                ])
                .to_owned(),
            )
            .exec_without_returning(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(CacheEntry {
            key: entry.key.key,
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: datetime_to_ts(now),
        })
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now = Utc::now().naive_utc();

        // One statement, so two concurrent callers cannot both decide the key is free.
        // The guard on the DO UPDATE branch is what makes this "insert if absent" rather
        // than a plain upsert: an unqualified column in that clause refers to the row
        // already in the table, so the update only fires when that row has expired.
        let affected = app_cache_entry::Entity::insert(active_model(&entry, now))
            .on_conflict(
                OnConflict::columns([
                    app_cache_entry::Column::AppId,
                    app_cache_entry::Column::Scope,
                    app_cache_entry::Column::UserId,
                    app_cache_entry::Column::Namespace,
                    app_cache_entry::Column::Key,
                ])
                .update_columns([
                    app_cache_entry::Column::Value,
                    app_cache_entry::Column::ExpiresAt,
                    app_cache_entry::Column::UpdatedAt,
                ])
                .action_and_where(
                    Expr::col((app_cache_entry::Entity, app_cache_entry::Column::ExpiresAt))
                        .is_not_null()
                        .and(
                            Expr::col((
                                app_cache_entry::Entity,
                                app_cache_entry::Column::ExpiresAt,
                            ))
                            .lt(now),
                        ),
                )
                .to_owned(),
            )
            .exec_without_returning(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        if affected == 0 {
            return Ok(None);
        }

        Ok(Some(CacheEntry {
            key: entry.key.key,
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: datetime_to_ts(now),
        }))
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let result = app_cache_entry::Entity::delete_many()
            .filter(key_condition(key))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected > 0)
    }

    async fn delete_namespace(
        &self,
        app_id: &str,
        scope: CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> Result<i64, CacheStoreError> {
        if namespace.is_empty() {
            return Err(CacheStoreError::InvalidInput(
                "Namespace invalidation requires a non-empty namespace".to_string(),
            ));
        }

        let result = app_cache_entry::Entity::delete_many()
            .filter(
                Condition::all()
                    .add(app_cache_entry::Column::AppId.eq(app_id))
                    .add(app_cache_entry::Column::Scope.eq(scope_to_entity(scope)))
                    .add(app_cache_entry::Column::UserId.eq(user_id))
                    .add(app_cache_entry::Column::Namespace.eq(namespace)),
            )
            .exec(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected as i64)
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        let result = app_cache_entry::Entity::delete_many()
            .filter(app_cache_entry::Column::AppId.eq(app_id))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected as i64)
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        let now = Utc::now().naive_utc();
        let result = app_cache_entry::Entity::delete_many()
            .filter(
                Condition::all()
                    .add(app_cache_entry::Column::ExpiresAt.is_not_null())
                    .add(app_cache_entry::Column::ExpiresAt.lt(now)),
            )
            .exec(self.db.as_ref())
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected as i64)
    }

    async fn stats(&self) -> Result<Option<CacheStoreStats>, CacheStoreError> {
        let db = self.db.as_ref();
        if db.get_database_backend() != DatabaseBackend::Postgres {
            // Every probe below reads a Postgres catalog view. There is no portable
            // equivalent, and guessing at one would be worse than reporting nothing.
            return Ok(None);
        }

        let estimates = probe(
            db,
            Statement::from_string(DatabaseBackend::Postgres, TABLE_ESTIMATE_SQL),
        )
        .await;
        let size = probe(
            db,
            Statement::from_string(DatabaseBackend::Postgres, TABLE_SIZE_SQL),
        )
        .await;
        let expired = probe(
            db,
            Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                EXPIRED_PENDING_SQL,
                [Utc::now().naive_utc().into(), EXPIRED_SCAN_CAP.into()],
            ),
        )
        .await;

        // A role missing one privilege, or a lock held over one relation, must cost only
        // the field it touches. All three failing together points at the connection
        // instead — reporting a healthy cache with no numbers would hide that entirely.
        if let (Err(error), Err(_), Err(_)) = (&estimates, &size, &expired) {
            return Err(CacheStoreError::Database(error.to_string()));
        }

        let expired_pending = probed_column::<i64>(&expired, "expired");

        Ok(Some(CacheStoreStats {
            entries: probed_column::<i64>(&estimates, "live_rows"),
            size_bytes: probed_column::<Option<i64>>(&size, "total_bytes").flatten(),
            expired_pending,
            expired_pending_capped: expired_pending == Some(EXPIRED_SCAN_CAP),
            observed_at: probed_column::<Option<i64>>(&estimates, "analyzed_at")
                .flatten()
                .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single()),
            note: Some(
                "Entry count is the planner's `n_live_tup` estimate, not a COUNT(*): it \
                 drifts between autovacuum runs and includes rows whose TTL has lapsed but \
                 which the sweeper has not reclaimed yet. Size covers the heap, its indexes \
                 and TOAST."
                    .to_string(),
            ),
            ..CacheStoreStats::default()
        }))
    }
}

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
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Set,
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
        .add(app_cache_entry::Column::Key.eq(key.key.clone()))
}

/// Matches rows that have not expired: either no TTL at all, or one still in the future.
fn live_condition(now: sea_orm::prelude::DateTime) -> Condition {
    Condition::any()
        .add(app_cache_entry::Column::ExpiresAt.is_null())
        .add(app_cache_entry::Column::ExpiresAt.gt(now))
}

fn active_model(entry: &SetCacheEntry, now: sea_orm::prelude::DateTime) -> app_cache_entry::ActiveModel {
    app_cache_entry::ActiveModel {
        app_id: Set(entry.key.app_id.clone()),
        scope: Set(scope_to_entity(entry.key.scope)),
        user_id: Set(entry.key.user_id.clone()),
        key: Set(entry.key.key.clone()),
        value: Set(entry.value.clone()),
        expires_at: Set(entry.expires_at.map(ts_to_datetime)),
        created_at: Set(now),
        updated_at: Set(now),
    }
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
}

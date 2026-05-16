//! Orphan-fork cleanup primitive.
//!
//! Cross-mode forks (online↔offline) materialize destination apps in
//! object storage *before* every DB row is committed. If a flow is
//! interrupted — desktop crashes mid-upload, finalize call never
//! arrives, fork session expires — the storage prefix `apps/{id}/...`
//! can outlive any matching DB row. Without a janitor, those orphans
//! accumulate forever.
//!
//! `find_orphan_app_prefixes` walks `apps/` on the master store and
//! lists every top-level app id that has no row in the `App` table.
//! `delete_orphan_app_prefix` deletes one such prefix.
//!
//! Callers (admin endpoint, scheduled task, deployment one-shot) are
//! responsible for wiring this in; this module only provides the
//! primitive so all paths share the same definition of "orphan".

use std::collections::HashSet;

use crate::{entity::app, error::ApiError, state::AppState};
use flow_like_storage::Path;
use flow_like_types::anyhow;
use futures_util::TryStreamExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[derive(Debug, Clone)]
pub struct OrphanPrefix {
    pub app_id: String,
    pub object_count: u64,
    pub total_size_bytes: u64,
}

/// Lists every `apps/{id}/...` prefix on the master store that has no
/// matching row in the `App` table. Pure read; no mutation.
pub async fn find_orphan_app_prefixes(state: &AppState) -> Result<Vec<OrphanPrefix>, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let store = credentials
        .to_store(true)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    // Collect every distinct top-level segment under `apps/` along
    // with running size + count totals — we surface these so callers
    // can preview before deleting.
    let prefix = Path::from("apps");
    let mut counters: std::collections::HashMap<String, (u64, u64)> =
        std::collections::HashMap::new();
    let mut listing = store.list(Some(&prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list apps prefix: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let suffix = match path_str.strip_prefix("apps/") {
            Some(s) => s,
            None => continue,
        };
        let app_id = match suffix.split('/').next() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        let entry = counters.entry(app_id).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.size);
    }

    if counters.is_empty() {
        return Ok(Vec::new());
    }

    let storage_app_ids: Vec<String> = counters.keys().cloned().collect();
    let known_rows = app::Entity::find()
        .filter(app::Column::Id.is_in(storage_app_ids))
        .all(&state.db)
        .await?;
    let known: HashSet<String> = known_rows.into_iter().map(|r| r.id).collect();

    let mut orphans = Vec::new();
    for (app_id, (count, size)) in counters {
        if known.contains(&app_id) {
            continue;
        }
        orphans.push(OrphanPrefix {
            app_id,
            object_count: count,
            total_size_bytes: size,
        });
    }
    orphans.sort_by(|a, b| a.app_id.cmp(&b.app_id));
    Ok(orphans)
}

/// Deletes every object under `apps/{app_id}/...` on the master store.
/// The caller MUST have already verified this is an orphan via
/// `find_orphan_app_prefixes`; this function does not re-check the DB
/// (its concurrent-DB-write defence is the caller's responsibility).
pub async fn delete_orphan_app_prefix(state: &AppState, app_id: &str) -> Result<u64, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let store = credentials
        .to_store(true)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    let prefix = Path::from("apps").child(app_id.to_string());
    let mut deleted = 0u64;
    let locations: Vec<Path> = store
        .list(Some(&prefix))
        .map_ok(|m| m.location)
        .try_collect()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list orphan prefix: {e}")))?;
    for loc in locations {
        store
            .delete(&loc)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("delete orphan object: {e}")))?;
        deleted = deleted.saturating_add(1);
    }
    Ok(deleted)
}

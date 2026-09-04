//! Orphan-fork cleanup primitive.
//!
//! Forks materialize a destination in object storage while a `ForkJob`
//! row tracks them; the App row exists from the job's `allocate` step on.
//! If a job is torn out from under its driver in a way the job sweeper
//! cannot repair, or a pre-job fork session was interrupted, the storage
//! prefix `apps/{id}/...` (and `media/apps/{id}/...`) can outlive every
//! matching row. Without a janitor those orphans accumulate forever.
//!
//! `find_orphan_app_prefixes` walks `apps/` on the meta and content stores
//! plus `media/apps/` and lists every app id that has neither an `App`
//! row nor a live `ForkJob`. `delete_orphan_app_prefix` deletes one such
//! id from all three places.
//!
//! Callers (admin endpoint, scheduled task, deployment one-shot) are
//! responsible for wiring this in; this module only provides the
//! primitive so all paths share the same definition of "orphan".

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{entity::app, error::ApiError, state::AppState};
use flow_like_storage::Path;
use flow_like_types::anyhow;
use futures_util::TryStreamExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::job;

#[derive(Debug, Clone)]
pub struct OrphanPrefix {
    pub app_id: String,
    pub object_count: u64,
    pub total_size_bytes: u64,
}

/// Lists every app id with objects under `apps/{id}/...` on either store
/// or under `media/apps/{id}/...` that has neither an `App` row nor a
/// fork job that is still running. Pure read; no mutation.
pub async fn find_orphan_app_prefixes(state: &AppState) -> Result<Vec<OrphanPrefix>, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let meta_store = credentials
        .to_store(true)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();
    let content_store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    let apps_prefix = Path::from("apps");
    let media_prefix = Path::from("media").child("apps");
    let mut counters: HashMap<String, (u64, u64)> = HashMap::new();
    tally_prefix(&meta_store, &apps_prefix, &mut counters).await?;
    tally_prefix(&content_store, &apps_prefix, &mut counters).await?;
    tally_prefix(&content_store, &media_prefix, &mut counters).await?;

    if counters.is_empty() {
        return Ok(Vec::new());
    }

    let storage_app_ids: Vec<String> = counters.keys().cloned().collect();
    let known_rows = app::Entity::find()
        .filter(app::Column::Id.is_in(storage_app_ids))
        .all(&state.db)
        .await?;
    let mut known: HashSet<String> = known_rows.into_iter().map(|r| r.id).collect();
    known.extend(job::live_dest_app_ids(state).await?);

    let mut orphans: Vec<OrphanPrefix> = counters
        .into_iter()
        .filter(|(app_id, _)| !known.contains(app_id))
        .map(|(app_id, (count, size))| OrphanPrefix {
            app_id,
            object_count: count,
            total_size_bytes: size,
        })
        .collect();
    orphans.sort_by(|a, b| a.app_id.cmp(&b.app_id));
    Ok(orphans)
}

/// Adds every object below `prefix` to the counter of the app id that is
/// its first path segment after the prefix.
async fn tally_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
    counters: &mut HashMap<String, (u64, u64)>,
) -> Result<(), ApiError> {
    let prefix_str = format!("{}/", prefix.as_ref());
    let mut listing = store.list(Some(prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list {}: {e}", prefix.as_ref())))?
    {
        let path_str = item.location.as_ref().to_string();
        let Some(suffix) = path_str.strip_prefix(&prefix_str) else {
            continue;
        };
        let app_id = match suffix.split('/').next() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        let entry = counters.entry(app_id).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.size);
    }
    Ok(())
}

/// Deletes every object under `apps/{app_id}/...` on both stores and
/// under `media/apps/{app_id}/...`, with bounded concurrency. The caller
/// MUST have already verified this is an orphan via
/// `find_orphan_app_prefixes`; this function does not re-check the DB
/// (its concurrent-DB-write defence is the caller's responsibility).
pub async fn delete_orphan_app_prefix(state: &AppState, app_id: &str) -> Result<u64, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let meta_store = credentials
        .to_store(true)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();
    let content_store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    let app_prefix = Path::from("apps").child(app_id.to_string());
    let media_prefix = Path::from("media").child("apps").child(app_id.to_string());
    let mut deleted = 0u64;
    for (store, prefix, label) in [
        (&meta_store, &app_prefix, "orphan meta prefix"),
        (&content_store, &app_prefix, "orphan content prefix"),
        (&content_store, &media_prefix, "orphan media prefix"),
    ] {
        deleted = deleted.saturating_add(count_prefix(store, prefix, label).await?);
        super::delete_object_prefix(store, prefix, label).await?;
    }
    Ok(deleted)
}

async fn count_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
    label: &str,
) -> Result<u64, ApiError> {
    let mut count = 0u64;
    let mut listing = store.list(Some(prefix));
    while listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list {label}: {e}")))?
        .is_some()
    {
        count = count.saturating_add(1);
    }
    Ok(count)
}

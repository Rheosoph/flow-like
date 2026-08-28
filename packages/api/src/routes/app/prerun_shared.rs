//! Shared helpers for prerun analysis (board + event endpoints).
//!
//! Everything a prerun endpoint returns that depends only on
//! `(app_id, board_id, version)` — runtime variables, OAuth requirements,
//! WASM package metadata, element demand — is a pure function of the board
//! and lives in the compiled [`PrerunManifest`]. This module serves that
//! manifest without loading the board: memory first, then the persisted
//! artifact next to the compiled board, and only on a miss the board itself.
//!
//! The payload carries a stable `signature` hash so callers can detect drift
//! when revalidating and react (rerun / cancel / prompt) on divergence.

use crate::{error::ApiError, state::AppState};
use flow_like::flow::{
    board::{Board, ExecutionMode},
    compiled::{PrerunManifest, decode_manifest, encode_manifest, manifest_path},
    node::NodePermission,
};
use flow_like_storage::{
    Path,
    object_store::{Error as StoreError, ObjectStore, PutPayload},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

/// A runtime-configured variable that needs a value before execution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data_type: String,
    pub value_type: String,
    pub secret: bool,
    pub schema: Option<String>,
}

/// OAuth provider requirement collected from the board's nodes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OAuthRequirement {
    pub provider_id: String,
    pub scopes: Vec<String>,
}

/// Board-derived prerun data — everything that depends only on
/// `(app_id, board_id, version)` and is safe to share across users.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrerunPayload {
    pub runtime_variables: Vec<RuntimeVariable>,
    pub oauth_requirements: Vec<OAuthRequirement>,
    pub requires_local_execution: bool,
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    pub has_wasm_nodes: bool,
    pub wasm_package_ids: Vec<String>,
    pub wasm_package_permissions: HashMap<String, Vec<NodePermission>>,
    /// Stable hash over the board-derived fields. Frontend uses this
    /// to detect drift when revalidating in the background.
    pub signature: String,
}

impl From<&PrerunManifest> for PrerunPayload {
    fn from(manifest: &PrerunManifest) -> Self {
        Self {
            runtime_variables: manifest
                .runtime_variables
                .iter()
                .map(|v| RuntimeVariable {
                    id: v.id.clone(),
                    name: v.name.clone(),
                    description: v.description.clone(),
                    data_type: v.data_type.clone(),
                    value_type: v.value_type.clone(),
                    secret: v.secret,
                    schema: v.schema.clone(),
                })
                .collect(),
            oauth_requirements: manifest
                .oauth_requirements
                .iter()
                .map(|r| OAuthRequirement {
                    provider_id: r.provider_id.clone(),
                    scopes: r.scopes.clone(),
                })
                .collect(),
            requires_local_execution: manifest.requires_local_execution,
            execution_mode: manifest.execution_mode(),
            has_wasm_nodes: manifest.has_wasm_nodes,
            wasm_package_ids: manifest.wasm_package_ids.clone(),
            wasm_package_permissions: manifest.wasm_permissions(),
            signature: manifest.signature.clone(),
        }
    }
}

pub fn parse_version(version_str: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version_str.split('_').collect();
    if parts.len() == 3 {
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].parse().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}

/// `prerun_manifest_cache` key of an immutable board version.
pub fn version_manifest_cache_key(
    app_id: &str,
    board_id: &str,
    version: (u32, u32, u32),
) -> String {
    format!(
        "{app_id}:{board_id}:{}_{}_{}",
        version.0, version.1, version.2
    )
}

fn draft_manifest_cache_key(app_id: &str, board_id: &str, e_tag: &str) -> String {
    format!("{app_id}:{board_id}:{e_tag}")
}

/// Resolve the prerun manifest of `(app, board, version|draft)`.
///
/// Versions are immutable, so their manifest is memoised in process and
/// persisted beside the compiled artifact; the board is only loaded when
/// neither exists. Drafts are keyed by the `.board` object's etag (one HEAD),
/// memoised, and never persisted — they change on every edit.
pub async fn load_prerun_manifest(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Arc<PrerunManifest>, ApiError> {
    let storage_root = Path::from("apps").child(app_id.to_string());
    let meta_store = state.meta_bucket.as_generic();

    let Some(version) = version else {
        let proto_path = Board::proto_path(&storage_root, board_id, None);
        let head = meta_store.head(&proto_path).await.map_err(|e| match e {
            StoreError::NotFound { .. } => {
                ApiError::not_found(format!("Board {board_id} not found"))
            }
            other => ApiError::from(other),
        })?;
        if let Some(e_tag) = head.e_tag.as_deref()
            && let Some(hit) = state
                .prerun_manifest_cache
                .get(&draft_manifest_cache_key(app_id, board_id, e_tag))
        {
            return Ok(hit);
        }

        let cached = state
            .master_board_shared(app_id, board_id, state, None)
            .await?;
        let manifest = Arc::new(PrerunManifest::from_board(&cached.board));
        // Keyed by the etag of the board that produced the manifest: a write
        // racing the HEAD above would otherwise file a newer manifest under
        // the older etag.
        if !cached.e_tag.is_empty() {
            state.prerun_manifest_cache.insert(
                draft_manifest_cache_key(app_id, board_id, &cached.e_tag),
                manifest.clone(),
            );
        }
        return Ok(manifest);
    };

    let cache_key = version_manifest_cache_key(app_id, board_id, version);
    if let Some(hit) = state.prerun_manifest_cache.get(&cache_key) {
        return Ok(hit);
    }

    let path = manifest_path(&storage_root, board_id, version);
    let manifest = match read_persisted_manifest(meta_store.as_ref(), &path).await {
        Some(manifest) => Arc::new(manifest),
        None => {
            let cached = state
                .master_board_shared(app_id, board_id, state, Some(version))
                .await?;
            let manifest = Arc::new(PrerunManifest::from_board(&cached.board));
            persist_prerun_manifest(meta_store.as_ref(), &path, &manifest).await;
            manifest
        }
    };
    state
        .prerun_manifest_cache
        .insert(cache_key, manifest.clone());
    Ok(manifest)
}

/// `None` for a missing, unreadable, or stale-format manifest — every one of
/// those is answered by recomputing from the board and writing a fresh file.
async fn read_persisted_manifest(
    meta_store: &dyn ObjectStore,
    path: &Path,
) -> Option<PrerunManifest> {
    let bytes = match meta_store.get(path).await {
        Ok(result) => match result.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::debug!(path = %path, error = %e, "Prerun manifest read failed");
                return None;
            }
        },
        Err(StoreError::NotFound { .. }) => return None,
        Err(e) => {
            tracing::debug!(path = %path, error = %e, "Prerun manifest read failed");
            return None;
        }
    };
    match decode_manifest(&bytes) {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            tracing::debug!(path = %path, error = %e, "Prerun manifest rejected, recomputing");
            None
        }
    }
}

/// Best-effort write of a version's manifest beside its compiled artifact.
/// Failures only cost a board load on the next cold read.
pub async fn persist_prerun_manifest(
    meta_store: &dyn ObjectStore,
    path: &Path,
    manifest: &PrerunManifest,
) {
    let bytes = match encode_manifest(manifest) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "Prerun manifest encode failed");
            return;
        }
    };
    match meta_store.put(path, PutPayload::from(bytes)).await {
        Ok(_) => tracing::debug!(path = %path, "Prerun manifest persisted"),
        Err(e) => tracing::warn!(path = %path, error = %e, "Prerun manifest persist failed"),
    }
}

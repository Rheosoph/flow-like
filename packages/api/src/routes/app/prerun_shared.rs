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
    compiled::{
        PrerunManifest, decode_manifest, draft_manifest_path, draft_page_manifest_path,
        draft_source_path, encode_manifest, legacy_manifest_path, manifest_path,
        version_page_manifest_path,
    },
    node::NodePermission,
};
use flow_like_storage::{
    Path,
    object_store::{Error as StoreError, GetOptions, ObjectStore, PutPayload},
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
    let version = format!("{}_{}_{}", version.0, version.1, version.2);
    manifest_cache_key("version", &[app_id, board_id, &version])
}

pub fn version_page_manifest_cache_key(
    app_id: &str,
    board_id: &str,
    version: (u32, u32, u32),
    page_id: &str,
    page_revision: &str,
) -> String {
    let version = format!("{}_{}_{}", version.0, version.1, version.2);
    manifest_cache_key(
        "version-page",
        &[app_id, board_id, &version, page_id, page_revision],
    )
}

fn draft_manifest_cache_key(app_id: &str, board_id: &str, e_tag: &str) -> String {
    manifest_cache_key("draft", &[app_id, board_id, e_tag])
}

fn draft_page_manifest_cache_key(
    app_id: &str,
    board_id: &str,
    e_tag: &str,
    page_id: &str,
    page_revision: &str,
) -> String {
    manifest_cache_key(
        "draft-page",
        &[app_id, board_id, e_tag, page_id, page_revision],
    )
}

fn manifest_cache_key(kind: &str, parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like/prerun-process-cache/v1");
    hasher.update(&(kind.len() as u64).to_le_bytes());
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{kind}:{}", hasher.finalize().to_hex())
}

fn hash_canonical_json(hasher: &mut blake3::Hasher, value: &serde_json::Value) {
    match value {
        serde_json::Value::Null => {
            hasher.update(&[0]);
        }
        serde_json::Value::Bool(value) => {
            hasher.update(&[1, u8::from(*value)]);
        }
        serde_json::Value::Number(value) => {
            hasher.update(&[2]);
            let value = value.to_string();
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        serde_json::Value::String(value) => {
            hasher.update(&[3]);
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        serde_json::Value::Array(values) => {
            hasher.update(&[4]);
            hasher.update(&(values.len() as u64).to_le_bytes());
            for value in values {
                hash_canonical_json(hasher, value);
            }
        }
        serde_json::Value::Object(values) => {
            hasher.update(&[5]);
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            hasher.update(&(keys.len() as u64).to_le_bytes());
            for key in keys {
                hasher.update(&(key.len() as u64).to_le_bytes());
                hasher.update(key.as_bytes());
                hash_canonical_json(hasher, &values[key]);
            }
        }
    };
}

pub(crate) fn page_payload_revision(
    page: &flow_like::a2ui::Page,
) -> flow_like_types::Result<String> {
    let value = serde_json::to_value(page)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like/page-prerun-cache");
    hash_canonical_json(&mut hasher, &value);
    Ok(hasher.finalize().to_hex().to_string())
}

fn page_manifest_matches_board_authority(
    candidate: &PrerunManifest,
    authority: &PrerunManifest,
    page_id: &str,
) -> bool {
    candidate.page_events.len() == 1
        && candidate.page_events[0].page_id == page_id
        && candidate.runtime_variables == authority.runtime_variables
        && candidate.oauth_requirements == authority.oauth_requirements
        && candidate.requires_local_execution == authority.requires_local_execution
        && candidate.execution_mode == authority.execution_mode
        && candidate.has_wasm_nodes == authority.has_wasm_nodes
        && candidate.wasm_package_ids == authority.wasm_package_ids
        && candidate.wasm_package_permissions == authority.wasm_package_permissions
        && candidate.element_selectors == authority.element_selectors
        && candidate.element_reads_dynamic == authority.element_reads_dynamic
        && candidate.entry_node_ids == authority.entry_node_ids
}

/// Resolve a draft manifest from a Board value already bound to a storage
/// ETag. Page execution uses this form so its manifest, action map, and later
/// dispatch all refer to the same conditional read.
pub fn draft_prerun_manifest_for_cached_board(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: &crate::state::CachedBoard,
) -> Arc<PrerunManifest> {
    let cache_key = (!cached.e_tag.is_empty())
        .then(|| draft_manifest_cache_key(app_id, board_id, &cached.e_tag));
    if let Some(cache_key) = cache_key.as_deref()
        && let Some(hit) = state.prerun_manifest_cache.get(cache_key)
    {
        return hit;
    }

    let manifest = Arc::new(PrerunManifest::from_board(&cached.board));
    if let Some(cache_key) = cache_key {
        state
            .prerun_manifest_cache
            .insert(cache_key, manifest.clone());
    }
    manifest
}

/// Read one ETag-keyed Board manifest through a caller-owned process cache.
/// The object store remains authoritative so a new Lambda instance can reuse
/// work completed by an earlier invocation.
async fn load_etag_prerun_manifest(
    cache: &moka::sync::Cache<String, Arc<PrerunManifest>>,
    meta_store: &dyn ObjectStore,
    app_id: &str,
    board_id: &str,
    e_tag: &str,
) -> Result<Option<Arc<PrerunManifest>>, ApiError> {
    let e_tag = e_tag.trim();
    if e_tag.is_empty() {
        return Ok(None);
    }
    let cache_key = draft_manifest_cache_key(app_id, board_id, e_tag);
    let path = draft_manifest_path(app_id, board_id, e_tag);
    if let Some(hit) = cache.get(&cache_key) {
        if hit.page_events.is_empty() {
            ensure_cached_manifest_shared(meta_store, &path, &hit).await?;
            return Ok(Some(hit));
        }
        cache.invalidate(&cache_key);
    }

    let Some(stored) = read_persisted_manifest(meta_store, &path).await else {
        return Ok(None);
    };
    if !stored.page_events.is_empty() {
        tracing::debug!(path = %path, "Board-only prerun authority contains Page bindings");
        return Ok(None);
    }
    let stored = Arc::new(stored);
    cache.insert(cache_key, stored.clone());
    Ok(Some(stored))
}

/// A process cache may avoid decoding an immutable manifest, but it cannot be
/// its only copy. A lifecycle deletion is repaired from the cached exact bytes
/// before the caller may use that authority. Other storage errors fail closed.
async fn ensure_cached_manifest_shared(
    meta_store: &dyn ObjectStore,
    path: &Path,
    manifest: &PrerunManifest,
) -> Result<(), ApiError> {
    match meta_store.head(path).await {
        Ok(_) => Ok(()),
        Err(StoreError::NotFound { .. }) => {
            persist_prerun_manifest_required(meta_store, path, manifest).await
        }
        Err(error) => Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Failed to verify shared prerun authority at {path}: {error}"
        ))),
    }
}

/// Load or persist the board-only prerun authority for an exact draft ETag.
/// A cold API instance reads the content-addressed artifact before deriving it
/// from the Board, so an unchanged ETag does not rebuild the manifest.
pub async fn ensure_draft_prerun_manifest(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: &crate::state::CachedBoard,
) -> Result<Arc<PrerunManifest>, ApiError> {
    let e_tag = cached.e_tag.trim();
    if e_tag.is_empty() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Latest board '{board_id}' has no storage ETag"
        )));
    }
    let meta_store = state.meta_bucket.as_generic();
    if let Some(hit) = load_etag_prerun_manifest(
        &state.prerun_manifest_cache,
        meta_store.as_ref(),
        app_id,
        board_id,
        e_tag,
    )
    .await?
    {
        return Ok(hit);
    }
    let cache_key = draft_manifest_cache_key(app_id, board_id, e_tag);
    let path = draft_manifest_path(app_id, board_id, e_tag);

    let manifest = Arc::new(PrerunManifest::from_board(&cached.board));
    persist_prerun_manifest_required(meta_store.as_ref(), &path, &manifest).await?;
    state
        .prerun_manifest_cache
        .insert(cache_key, manifest.clone());
    Ok(manifest)
}

/// Load or persist one pinned Page's immutable execution map.
///
/// The board-only entry authority stays at `manifest_path`; Page-dependent
/// signatures use a separate key so one Page publication cannot invalidate an
/// already queued callback or race another Page on a stateless API instance.
pub async fn ensure_versioned_page_prerun_manifest(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: (u32, u32, u32),
    board: &Board,
    requested_page: &flow_like::a2ui::Page,
    board_authority: Arc<PrerunManifest>,
) -> Result<Arc<PrerunManifest>, ApiError> {
    let page_revision = page_payload_revision(requested_page).map_err(ApiError::internal_error)?;
    let cache_key = version_page_manifest_cache_key(
        app_id,
        board_id,
        version,
        &requested_page.id,
        &page_revision,
    );
    let storage_root = Path::from("apps").child(app_id.to_string());
    let path = version_page_manifest_path(
        &storage_root,
        board_id,
        version,
        &requested_page.id,
        &page_revision,
    );
    let meta_store = state.meta_bucket.as_generic();
    if let Some(hit) = state.prerun_manifest_cache.get(&cache_key) {
        if page_manifest_matches_board_authority(&hit, &board_authority, &requested_page.id) {
            ensure_cached_manifest_shared(meta_store.as_ref(), &path, &hit).await?;
            return Ok(hit);
        }
        state.prerun_manifest_cache.invalidate(&cache_key);
    }
    if let Some(stored) = read_persisted_manifest(meta_store.as_ref(), &path)
        .await
        .filter(|stored| {
            page_manifest_matches_board_authority(stored, &board_authority, &requested_page.id)
        })
    {
        let stored = Arc::new(stored);
        state
            .prerun_manifest_cache
            .insert(cache_key, stored.clone());
        return Ok(stored);
    }

    let manifest = Arc::new(
        PrerunManifest::from_board_and_page(board, requested_page).map_err(|error| {
            tracing::warn!(
                board_id,
                page_id = %requested_page.id,
                version = ?version,
                error = %error,
                "Published Page execution contract is invalid"
            );
            ApiError::bad_request("The Page execution contract is invalid")
        })?,
    );
    persist_prerun_manifest_required(meta_store.as_ref(), &path, &manifest).await?;
    state
        .prerun_manifest_cache
        .insert(cache_key, manifest.clone());
    Ok(manifest)
}

fn board_changed_while_starting_error() -> ApiError {
    ApiError::bad_request("The Board changed while starting this run; reload the Page")
}

/// A failed `if_match` read of the exact Board source means someone saved the
/// board between contract resolution and snapshot capture — a client-actionable
/// staleness, not a storage failure.
fn exact_board_source_read_error(source_path: &Path, e_tag: &str, error: StoreError) -> ApiError {
    match error {
        StoreError::Precondition { .. } => board_changed_while_starting_error(),
        error => ApiError::internal_error(flow_like_types::anyhow!(
            "Failed to read exact Board source {source_path} at ETag {e_tag}: {error}"
        )),
    }
}

/// Persist the exact compressed Board source selected by a draft ETag. Exact
/// Page runs can then rebuild a missing registry-specific artifact without
/// reading a newer floating Board or applying hydration twice.
pub async fn ensure_draft_board_snapshot(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: &crate::state::CachedBoard,
) -> Result<(), ApiError> {
    let e_tag = cached.e_tag.trim();
    if e_tag.is_empty() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Latest board '{board_id}' has no storage ETag"
        )));
    }
    let path = draft_source_path(app_id, board_id, e_tag);
    let meta_store = state.meta_bucket.as_generic();
    match meta_store.head(&path).await {
        Ok(_) => return Ok(()),
        Err(StoreError::NotFound { .. }) => {}
        Err(error) => {
            return Err(ApiError::internal_error(flow_like_types::anyhow!(
                "Failed to inspect exact Board snapshot at {path}: {error}"
            )));
        }
    }

    let storage_root = Path::from("apps").child(app_id.to_string());
    let source_path = Board::proto_path(&storage_root, board_id, None);
    let source = meta_store
        .get_opts(
            &source_path,
            GetOptions {
                if_match: Some(e_tag.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| exact_board_source_read_error(&source_path, e_tag, error))?;
    if source.meta.e_tag.as_deref() != Some(e_tag) {
        // A store that does not honor `if_match` reports the same race here.
        return Err(board_changed_while_starting_error());
    }
    let bytes = source.bytes().await.map_err(|error| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "Failed to read exact Board source bytes from {source_path}: {error}"
        ))
    })?;
    meta_store
        .put(&path, PutPayload::from(bytes))
        .await
        .map_err(|error| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "Failed to persist exact Board snapshot at {path}: {error}"
            ))
        })?;
    tracing::debug!(path = %path, "Exact Board snapshot persisted");
    Ok(())
}

/// Load the prerun authority selected by a signed immutable version or draft
/// ETag. An ETag miss never falls back to the current draft.
pub async fn load_exact_prerun_manifest(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
    e_tag: Option<&str>,
) -> Result<Arc<PrerunManifest>, ApiError> {
    match (
        version,
        e_tag.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(version), None) => load_prerun_manifest(state, app_id, board_id, Some(version)).await,
        (None, Some(e_tag)) => {
            let meta_store = state.meta_bucket.as_generic();
            if let Some(hit) = load_etag_prerun_manifest(
                &state.prerun_manifest_cache,
                meta_store.as_ref(),
                app_id,
                board_id,
                e_tag,
            )
            .await?
            {
                return Ok(hit);
            }
            // Lifecycle rules on the drafts prefix may sweep the ETag-keyed
            // manifest while the exact Board snapshot for that ETag survives.
            // `PrerunManifest::from_board` is deterministic, so rebuilding
            // from the snapshot reproduces the signed authority.
            if let Some(rebuilt) = rebuild_etag_manifest_from_snapshot(
                &state.prerun_manifest_cache,
                &meta_store,
                app_id,
                board_id,
                e_tag,
                |proto| async move {
                    let app_state = state
                        .master_state(state)
                        .await
                        .map_err(ApiError::internal_error)?;
                    let storage_root = Path::from("apps").child(app_id.to_string());
                    Ok(Board::from_loaded_proto(proto, storage_root, app_state).await)
                },
            )
            .await?
            {
                return Ok(rebuilt);
            }
            Err(ApiError::bad_request(
                "The exact Page execution authority is unavailable; reload the Page",
            ))
        }
        _ => Err(ApiError::bad_request(
            "The Page execution board selector is invalid",
        )),
    }
}

/// Rebuild the board-only prerun authority for an exact draft ETag from the
/// snapshot `ensure_draft_board_snapshot` persisted for that same ETag.
/// `Ok(None)` when the snapshot is also gone — the caller keeps failing closed.
async fn rebuild_etag_manifest_from_snapshot<F, Fut>(
    cache: &moka::sync::Cache<String, Arc<PrerunManifest>>,
    meta_store: &Arc<dyn ObjectStore>,
    app_id: &str,
    board_id: &str,
    e_tag: &str,
    hydrate: F,
) -> Result<Option<Arc<PrerunManifest>>, ApiError>
where
    F: FnOnce(flow_like_types::proto::Board) -> Fut,
    Fut: std::future::Future<Output = Result<Board, ApiError>>,
{
    let snapshot_path = draft_source_path(app_id, board_id, e_tag);
    let proto: flow_like_types::proto::Board = match flow_like::utils::compression::from_compressed(
        meta_store.clone(),
        snapshot_path.clone(),
    )
    .await
    {
        Ok(proto) => proto,
        Err(error) => {
            if matches!(
                error.downcast_ref::<StoreError>(),
                Some(StoreError::NotFound { .. })
            ) {
                return Ok(None);
            }
            return Err(ApiError::internal_error(flow_like_types::anyhow!(
                "Failed to read exact Board snapshot at {snapshot_path}: {error}"
            )));
        }
    };

    let board = hydrate(proto).await?;
    if board.id != board_id {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Exact Board snapshot at {snapshot_path} contains board {}",
            board.id
        )));
    }

    let manifest = Arc::new(PrerunManifest::from_board(&board));
    let path = draft_manifest_path(app_id, board_id, e_tag);
    persist_prerun_manifest_required(meta_store.as_ref(), &path, &manifest).await?;
    cache.insert(
        draft_manifest_cache_key(app_id, board_id, e_tag),
        manifest.clone(),
    );
    tracing::info!(
        path = %path,
        "Rebuilt swept ETag prerun authority from its exact Board snapshot"
    );
    Ok(Some(manifest))
}

/// Resolve the Page-aware prerun artifact for an ETag-bound draft. The key
/// includes both the Board ETag and a canonical Page payload revision because
/// those two objects are persisted in sequence rather than atomically.
pub async fn draft_page_prerun_manifest_for_cached_board(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: &crate::state::CachedBoard,
    board_authority: &PrerunManifest,
    page: &flow_like::a2ui::Page,
) -> Result<Arc<PrerunManifest>, ApiError> {
    let e_tag = cached.e_tag.trim();
    if e_tag.is_empty() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Latest board '{board_id}' has no storage ETag"
        )));
    }
    let page_revision = page_payload_revision(page).map_err(ApiError::internal_error)?;
    let cache_key =
        draft_page_manifest_cache_key(app_id, board_id, e_tag, &page.id, &page_revision);
    let path = draft_page_manifest_path(app_id, board_id, e_tag, &page.id, &page_revision);
    let meta_store = state.meta_bucket.as_generic();
    if let Some(hit) = state.prerun_manifest_cache.get(&cache_key) {
        if page_manifest_matches_board_authority(&hit, board_authority, &page.id) {
            ensure_cached_manifest_shared(meta_store.as_ref(), &path, &hit).await?;
            return Ok(hit);
        }
        state.prerun_manifest_cache.invalidate(&cache_key);
    }

    if let Some(stored) = read_persisted_manifest(meta_store.as_ref(), &path)
        .await
        .filter(|stored| page_manifest_matches_board_authority(stored, board_authority, &page.id))
    {
        let stored = Arc::new(stored);
        state
            .prerun_manifest_cache
            .insert(cache_key, stored.clone());
        return Ok(stored);
    }

    let manifest = Arc::new(
        PrerunManifest::from_board_and_page(&cached.board, page).map_err(|error| {
            tracing::warn!(
                board_id,
                page_id = %page.id,
                error = %error,
                "Page execution contract is invalid"
            );
            ApiError::bad_request("The Page execution contract is invalid")
        })?,
    );
    persist_prerun_manifest_required(meta_store.as_ref(), &path, &manifest).await?;
    state
        .prerun_manifest_cache
        .insert(cache_key, manifest.clone());
    Ok(manifest)
}

#[cfg(test)]
mod draft_manifest_cache_tests {
    use super::{
        PersistedVersionManifest, draft_manifest_cache_key, draft_manifest_path,
        draft_page_manifest_path, draft_source_path, ensure_cached_manifest_shared,
        exact_board_source_read_error, hash_canonical_json, legacy_manifest_path,
        load_etag_prerun_manifest, manifest_path, page_manifest_matches_board_authority,
        page_payload_revision, persist_prerun_manifest, read_persisted_manifest,
        read_version_manifest, rebuild_etag_manifest_from_snapshot, version_manifest_cache_key,
        version_page_manifest_cache_key, version_page_manifest_path,
    };
    use flow_like::a2ui::Page;
    use flow_like::flow::{board::Board, compiled::PrerunManifest};
    use flow_like_storage::{
        Path,
        object_store::{Error as StoreError, ObjectStore, memory::InMemory},
    };
    use flow_like_types::{FromProto, ToProto};
    use std::sync::Arc;

    fn digest(value: serde_json::Value) -> String {
        let mut hasher = blake3::Hasher::new();
        hash_canonical_json(&mut hasher, &value);
        hasher.finalize().to_hex().to_string()
    }

    #[test]
    fn page_cache_revision_is_independent_of_json_object_order() {
        assert_eq!(
            digest(serde_json::json!({"a": 1, "b": {"x": true, "y": false}})),
            digest(serde_json::json!({"b": {"y": false, "x": true}, "a": 1}))
        );
        assert_ne!(
            digest(serde_json::json!({"actions": ["a", "b"]})),
            digest(serde_json::json!({"actions": ["b", "a"]}))
        );
    }

    #[test]
    fn version_and_draft_cache_selectors_have_disjoint_namespaces() {
        assert_ne!(
            version_manifest_cache_key("app-1", "board-1", (1, 2, 3)),
            draft_manifest_cache_key("app-1", "board-1", "1_2_3")
        );
        assert_ne!(
            draft_manifest_cache_key("app:one", "board", "etag"),
            draft_manifest_cache_key("app", "one:board", "etag")
        );
    }

    #[test]
    fn page_manifest_read_requires_the_exact_board_authority_and_page() {
        let board = Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        let authority = PrerunManifest::from_board(&board);
        let page = Page::new("page-1", "Page", "/");
        let candidate = PrerunManifest::from_board_and_page(&board, &page).unwrap();
        assert!(page_manifest_matches_board_authority(
            &candidate, &authority, "page-1"
        ));
        assert!(!page_manifest_matches_board_authority(
            &candidate, &authority, "page-2"
        ));

        let mut different_board = board.clone();
        different_board.execution_mode = flow_like::flow::board::ExecutionMode::Local;
        let different_authority = PrerunManifest::from_board(&different_board);
        assert!(!page_manifest_matches_board_authority(
            &candidate,
            &different_authority,
            "page-1"
        ));
    }

    #[test]
    fn page_payload_revision_changes_with_execution_content() {
        let page = Page::new("page-1", "Page", "/");
        let before = page_payload_revision(&page).unwrap();
        let mut changed = page.clone();
        changed.on_load_event_id = Some("entry-1".into());

        assert_ne!(before, page_payload_revision(&changed).unwrap());
        assert_eq!(before, page_payload_revision(&page).unwrap());
    }

    #[tokio::test]
    async fn fresh_lambda_cache_reads_etag_manifest_from_shared_storage() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let mut persisted_board =
            Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        persisted_board.execution_mode = flow_like::flow::board::ExecutionMode::Local;
        let persisted = PrerunManifest::from_board(&persisted_board);
        let path = draft_manifest_path("app-1", "board-1", "etag-1");
        assert!(persist_prerun_manifest(store.as_ref(), &path, &persisted).await);

        // This cache represents a different cold Lambda from the writer.
        let fresh_cache = moka::sync::Cache::builder().max_capacity(4).build();
        let loaded =
            load_etag_prerun_manifest(&fresh_cache, store.as_ref(), "app-1", "board-1", "etag-1")
                .await
                .unwrap()
                .expect("the persisted ETag artifact should satisfy a cold loader");

        let rebuilt = PrerunManifest::from_board(&Board::new_detached(
            Some("board-1".into()),
            Path::from("apps").child("app-1"),
        ));
        assert_eq!(loaded.as_ref(), &persisted);
        assert_ne!(loaded.signature, rebuilt.signature);
        assert!(
            fresh_cache
                .get(&draft_manifest_cache_key("app-1", "board-1", "etag-1"))
                .is_some()
        );

        store.delete(&path).await.unwrap();
        let warm =
            load_etag_prerun_manifest(&fresh_cache, store.as_ref(), "app-1", "board-1", "etag-1")
                .await
                .unwrap()
                .expect("a warm cache hit should restore its missing shared artifact");
        assert_eq!(warm.as_ref(), &persisted);

        let next_cold_cache = moka::sync::Cache::builder().max_capacity(4).build();
        let next_cold = load_etag_prerun_manifest(
            &next_cold_cache,
            store.as_ref(),
            "app-1",
            "board-1",
            "etag-1",
        )
        .await
        .unwrap()
        .expect("the next cold Lambda should see the restored shared artifact");
        assert_eq!(next_cold.as_ref(), &persisted);
    }

    #[tokio::test]
    async fn swept_etag_manifest_is_rebuilt_from_the_exact_board_snapshot() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let mut board =
            Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        board.execution_mode = flow_like::flow::board::ExecutionMode::Local;
        let authority = PrerunManifest::from_board(&board);
        flow_like::utils::compression::compress_to_file(
            store.clone(),
            draft_source_path("app-1", "board-1", "etag-1"),
            &board.to_proto(),
        )
        .await
        .unwrap();

        let cache = moka::sync::Cache::builder().max_capacity(4).build();
        let rebuilt = rebuild_etag_manifest_from_snapshot(
            &cache,
            &store,
            "app-1",
            "board-1",
            "etag-1",
            |proto| async move { Ok::<_, crate::error::ApiError>(Board::from_proto(proto)) },
        )
        .await
        .unwrap()
        .expect("the surviving snapshot should rebuild the swept manifest");

        // Determinism keeps the rebuilt authority equal to the signed one.
        assert_eq!(rebuilt.as_ref(), &authority);
        assert_eq!(rebuilt.signature, authority.signature);
        assert!(
            cache
                .get(&draft_manifest_cache_key("app-1", "board-1", "etag-1"))
                .is_some()
        );

        // The rebuild is persisted, so the next cold loader reads it directly.
        let fresh_cache = moka::sync::Cache::builder().max_capacity(4).build();
        let reloaded =
            load_etag_prerun_manifest(&fresh_cache, store.as_ref(), "app-1", "board-1", "etag-1")
                .await
                .unwrap()
                .expect("the rebuilt artifact should satisfy a cold loader");
        assert_eq!(reloaded.as_ref(), &authority);

        // A missing snapshot keeps the caller failing closed.
        let missing = rebuild_etag_manifest_from_snapshot(
            &cache,
            &store,
            "app-1",
            "board-1",
            "etag-2",
            |proto| async move { Ok::<_, crate::error::ApiError>(Board::from_proto(proto)) },
        )
        .await
        .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn exact_board_source_if_match_race_maps_to_a_client_actionable_status() {
        let path = Path::from("apps").child("app-1").child("board-1.board");

        let stale = exact_board_source_read_error(
            &path,
            "etag-1",
            StoreError::Precondition {
                path: path.to_string(),
                source: "the board was saved concurrently".into(),
            },
        );
        assert_eq!(stale.status(), axum::http::StatusCode::BAD_REQUEST);

        let broken = exact_board_source_read_error(
            &path,
            "etag-1",
            StoreError::Generic {
                store: "test",
                source: "boom".into(),
            },
        );
        assert_eq!(
            broken.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn board_authority_loader_rejects_a_page_aware_artifact() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let board = Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        let manifest =
            PrerunManifest::from_board_and_page(&board, &Page::new("page-1", "Page", "/")).unwrap();
        let path = draft_manifest_path("app-1", "board-1", "etag-1");
        assert!(persist_prerun_manifest(store.as_ref(), &path, &manifest).await);

        let fresh_cache = moka::sync::Cache::builder().max_capacity(4).build();
        assert!(
            load_etag_prerun_manifest(&fresh_cache, store.as_ref(), "app-1", "board-1", "etag-1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn cached_page_manifest_is_republished_after_shared_deletion() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let board = Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        let cached =
            PrerunManifest::from_board_and_page(&board, &Page::new("page-1", "Page", "/")).unwrap();
        let path =
            draft_page_manifest_path("app-1", "board-1", "etag-1", "page-1", "page-revision-1");
        assert!(persist_prerun_manifest(store.as_ref(), &path, &cached).await);
        store.delete(&path).await.unwrap();

        ensure_cached_manifest_shared(store.as_ref(), &path, &cached)
            .await
            .unwrap();
        assert_eq!(
            read_persisted_manifest(store.as_ref(), &path).await,
            Some(cached)
        );
    }

    #[tokio::test]
    async fn cached_version_page_manifest_is_republished_after_shared_deletion() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let storage_root = Path::from("apps").child("app-1");
        let board = Board::new_detached(Some("board-1".into()), storage_root.clone());
        let page = Page::new("page-1", "Page", "/");
        let cached = PrerunManifest::from_board_and_page(&board, &page).unwrap();
        let revision = page_payload_revision(&page).unwrap();
        let path =
            version_page_manifest_path(&storage_root, "board-1", (1, 2, 3), &page.id, &revision);
        assert!(persist_prerun_manifest(store.as_ref(), &path, &cached).await);
        store.delete(&path).await.unwrap();

        ensure_cached_manifest_shared(store.as_ref(), &path, &cached)
            .await
            .unwrap();
        assert_eq!(
            read_persisted_manifest(store.as_ref(), &path).await,
            Some(cached)
        );
    }

    #[tokio::test]
    async fn cached_version_manifest_is_republished_after_shared_deletion() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let board = Board::new_detached(Some("board-1".into()), Path::from("apps").child("app-1"));
        let cached = PrerunManifest::from_board(&board);
        let storage_root = Path::from("apps").child("app-1");
        let path = manifest_path(&storage_root, "board-1", (1, 2, 3));
        assert!(persist_prerun_manifest(store.as_ref(), &path, &cached).await);
        store.delete(&path).await.unwrap();

        ensure_cached_manifest_shared(store.as_ref(), &path, &cached)
            .await
            .unwrap();
        assert_eq!(
            read_persisted_manifest(store.as_ref(), &path).await,
            Some(cached)
        );
    }

    #[tokio::test]
    async fn pinned_pages_publish_to_disjoint_keys_without_changing_callback_authority() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let storage_root = Path::from("apps").child("app-1");
        let mut board = Board::new_detached(Some("board-1".into()), storage_root.clone());
        board.page_ids = vec!["page-1".into(), "page-2".into()];
        let authority = PrerunManifest::from_board(&board);
        let authority_path = manifest_path(&storage_root, "board-1", (1, 2, 3));
        assert!(persist_prerun_manifest(store.as_ref(), &authority_path, &authority).await);
        let minted_callback_revision = authority.signature.clone();

        let page_1 = Page::new("page-1", "First", "/first");
        let page_2 = Page::new("page-2", "Second", "/second");
        let revision_1 = page_payload_revision(&page_1).unwrap();
        let revision_2 = page_payload_revision(&page_2).unwrap();
        let path_1 = version_page_manifest_path(
            &storage_root,
            "board-1",
            (1, 2, 3),
            &page_1.id,
            &revision_1,
        );
        let path_2 = version_page_manifest_path(
            &storage_root,
            "board-1",
            (1, 2, 3),
            &page_2.id,
            &revision_2,
        );
        assert_ne!(path_1, path_2);
        assert_ne!(
            version_page_manifest_cache_key("app-1", "board-1", (1, 2, 3), &page_1.id, &revision_1,),
            version_page_manifest_cache_key("app-1", "board-1", (1, 2, 3), &page_2.id, &revision_2,)
        );

        let manifest_1 = PrerunManifest::from_board_and_page(&board, &page_1).unwrap();
        let manifest_2 = PrerunManifest::from_board_and_page(&board, &page_2).unwrap();
        let (write_1, write_2) = tokio::join!(
            persist_prerun_manifest(store.as_ref(), &path_1, &manifest_1),
            persist_prerun_manifest(store.as_ref(), &path_2, &manifest_2),
        );
        assert!(write_1 && write_2);
        assert_eq!(
            read_persisted_manifest(store.as_ref(), &path_1).await,
            Some(manifest_1)
        );
        assert_eq!(
            read_persisted_manifest(store.as_ref(), &path_2).await,
            Some(manifest_2)
        );

        let callback_authority = read_persisted_manifest(store.as_ref(), &authority_path)
            .await
            .expect("board authority should remain readable after both Page writes");
        assert!(callback_authority.page_events.is_empty());
        assert_eq!(callback_authority.signature, minted_callback_revision);
    }

    #[tokio::test]
    async fn format_scoped_version_manifest_isolated_from_legacy_writers() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let storage_root = Path::from("apps").child("app-1");
        let current_path = manifest_path(&storage_root, "board-1", (1, 2, 3));
        let legacy_path = legacy_manifest_path(&storage_root, "board-1", (1, 2, 3));
        assert_ne!(current_path, legacy_path);

        let current = PrerunManifest::from_board(&Board::new_detached(
            Some("board-1".into()),
            storage_root.clone(),
        ));
        assert!(persist_prerun_manifest(store.as_ref(), &current_path, &current).await);

        let mut legacy_board = Board::new_detached(Some("board-1".into()), storage_root);
        legacy_board.execution_mode = flow_like::flow::board::ExecutionMode::Local;
        let legacy = PrerunManifest::from_board(&legacy_board);
        assert!(persist_prerun_manifest(store.as_ref(), &legacy_path, &legacy).await);

        match read_version_manifest(store.as_ref(), &current_path, &legacy_path)
            .await
            .expect("current authority should be readable")
        {
            PersistedVersionManifest::Current(loaded) => assert_eq!(loaded, current),
            PersistedVersionManifest::Legacy => panic!("legacy writer shadowed current authority"),
        }
    }

    #[tokio::test]
    async fn version_manifest_reader_reports_legacy_fallback_on_current_miss() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let storage_root = Path::from("apps").child("app-1");
        let current_path = manifest_path(&storage_root, "board-1", (1, 2, 3));
        let legacy_path = legacy_manifest_path(&storage_root, "board-1", (1, 2, 3));
        let legacy =
            PrerunManifest::from_board(&Board::new_detached(Some("board-1".into()), storage_root));
        assert!(persist_prerun_manifest(store.as_ref(), &legacy_path, &legacy).await);

        assert!(matches!(
            read_version_manifest(store.as_ref(), &current_path, &legacy_path).await,
            Some(PersistedVersionManifest::Legacy)
        ));
        assert!(
            read_persisted_manifest(store.as_ref(), &current_path)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn version_board_authority_rejects_page_dependent_bytes() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let storage_root = Path::from("apps").child("app-1");
        let board = Board::new_detached(Some("board-1".into()), storage_root.clone());
        let page_manifest =
            PrerunManifest::from_board_and_page(&board, &Page::new("page-1", "Page", "/")).unwrap();
        let current_path = manifest_path(&storage_root, "board-1", (1, 2, 3));
        let legacy_path = legacy_manifest_path(&storage_root, "board-1", (1, 2, 3));
        assert!(persist_prerun_manifest(store.as_ref(), &current_path, &page_manifest).await);

        assert!(
            read_version_manifest(store.as_ref(), &current_path, &legacy_path)
                .await
                .is_none()
        );
    }
}

/// Resolve the prerun manifest of `(app, board, version|draft)`.
///
/// Versions are immutable, so their manifest is memoised in process and
/// persisted beside the compiled artifact; the board is only loaded when
/// neither exists. Drafts are persisted once per `.board` ETag, so a cold
/// Lambda reads the shared artifact and only a new ETag rebuilds it. The
/// process cache remains an optional warm-path optimization.
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
        if let Some(e_tag) = head.e_tag.as_deref() {
            if let Some(hit) = load_etag_prerun_manifest(
                &state.prerun_manifest_cache,
                meta_store.as_ref(),
                app_id,
                board_id,
                e_tag,
            )
            .await?
            {
                return Ok(hit);
            }
        }

        let cached = state
            .master_board_shared(app_id, board_id, state, None)
            .await?;
        // Keyed by the ETag of the board that produced the manifest. A write
        // racing the HEAD above therefore cannot file a newer manifest under
        // the older ETag.
        if cached.e_tag.trim().is_empty() {
            return Ok(draft_prerun_manifest_for_cached_board(
                state, app_id, board_id, &cached,
            ));
        }
        return ensure_draft_prerun_manifest(state, app_id, board_id, &cached).await;
    };

    let cache_key = version_manifest_cache_key(app_id, board_id, version);
    let path = manifest_path(&storage_root, board_id, version);
    let legacy_path = legacy_manifest_path(&storage_root, board_id, version);
    if let Some(hit) = state.prerun_manifest_cache.get(&cache_key) {
        if hit.page_events.is_empty() {
            ensure_cached_manifest_shared(meta_store.as_ref(), &path, &hit).await?;
            return Ok(hit);
        }
        state.prerun_manifest_cache.invalidate(&cache_key);
    }

    let manifest = match read_version_manifest(meta_store.as_ref(), &path, &legacy_path).await {
        Some(PersistedVersionManifest::Current(manifest)) => Arc::new(manifest),
        Some(PersistedVersionManifest::Legacy) | None => {
            // A legacy artifact proves this version has been analyzed before,
            // but v1/v2 does not contain the complete entry-node authority.
            // Re-derive v3 from the immutable Board and leave the legacy key
            // untouched for old Lambdas during a rolling deployment.
            let cached = state
                .master_board_shared(app_id, board_id, state, Some(version))
                .await?;
            let manifest = Arc::new(PrerunManifest::from_board(&cached.board));
            persist_prerun_manifest_required(meta_store.as_ref(), &path, &manifest).await?;
            manifest
        }
    };
    state
        .prerun_manifest_cache
        .insert(cache_key, manifest.clone());
    Ok(manifest)
}

enum PersistedVersionManifest {
    Current(PrerunManifest),
    Legacy,
}

/// Prefer the format-scoped authority. The legacy object is only a migration
/// signal: callers re-derive current authority from the immutable Board rather
/// than promoting fields that did not exist in the old format.
async fn read_version_manifest(
    meta_store: &dyn ObjectStore,
    current_path: &Path,
    legacy_path: &Path,
) -> Option<PersistedVersionManifest> {
    if let Some(manifest) = read_persisted_manifest(meta_store, current_path).await {
        if manifest.page_events.is_empty() {
            return Some(PersistedVersionManifest::Current(manifest));
        }
        tracing::debug!(
            path = %current_path,
            "Board-only prerun authority contains Page bindings; re-deriving"
        );
    }
    read_persisted_manifest(meta_store, legacy_path)
        .await
        .map(|_| PersistedVersionManifest::Legacy)
}

/// `None` for a missing, unreadable, or stale-format manifest. Callers either
/// recompute from their selected Board or fail closed when only signed exact
/// authority is available.
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
) -> bool {
    let bytes = match encode_manifest(manifest) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "Prerun manifest encode failed");
            return false;
        }
    };
    match meta_store.put(path, PutPayload::from(bytes)).await {
        Ok(_) => {
            tracing::debug!(path = %path, "Prerun manifest persisted");
            true
        }
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "Prerun manifest persist failed");
            false
        }
    }
}

async fn persist_prerun_manifest_required(
    meta_store: &dyn ObjectStore,
    path: &Path,
    manifest: &PrerunManifest,
) -> Result<(), ApiError> {
    let bytes = encode_manifest(manifest).map_err(|error| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "Failed to encode exact prerun authority at {path}: {error}"
        ))
    })?;
    meta_store
        .put(path, PutPayload::from(bytes))
        .await
        .map_err(|error| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "Failed to persist exact prerun authority at {path}: {error}"
            ))
        })?;
    tracing::debug!(path = %path, "Exact prerun authority persisted");
    Ok(())
}

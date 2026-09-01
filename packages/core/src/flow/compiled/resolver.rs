//! Shared template resolution: in-process cache → persisted artifact →
//! compile-from-source with background write-back.
//!
//! Both the cloud executor and the desktop runtime resolve boards through
//! this type; they differ only in cache instance and extra cache-key salt
//! (the executor keys on its per-request WASM bundle signature).

use super::CompiledRunTemplate;
use crate::flow::board::Board;
use crate::state::{FlowLikeState, FlowNodeRegistryInner};
use crate::utils::compression::from_compressed_with_meta;
use flow_like_storage::Path;
use flow_like_storage::object_store::{ObjectMeta, ObjectStore, PutPayload};
use flow_like_types::{Result, anyhow};
use std::sync::Arc;
use std::time::Duration;

pub struct CachedTemplate {
    pub template: Arc<CompiledRunTemplate>,
    pub e_tag: Option<String>,
    pub last_modified: chrono::DateTime<chrono::Utc>,
}

/// True when the storage-level identity recorded at load time still matches
/// what HEAD returns now. Prefers e_tag (cheap, exact), falls back to
/// last_modified when the backend doesn't expose one.
fn meta_unchanged(cached: &CachedTemplate, head: &ObjectMeta) -> bool {
    match (&cached.e_tag, &head.e_tag) {
        (Some(a), Some(b)) => a == b,
        _ => cached.last_modified == head.last_modified,
    }
}

/// In-process cache of compiled run templates.
///
/// A cached template is fully shared across requests and users — its Board
/// view carries no `app_state` — and revalidated by HEAD for floating
/// "latest" boards; pinned versions are immutable and skip revalidation.
pub struct TemplateCache {
    inner: moka::sync::Cache<String, Arc<CachedTemplate>>,
}

impl Default for TemplateCache {
    fn default() -> Self {
        Self::new(
            256,
            Duration::from_secs(30 * 60),
            Duration::from_secs(5 * 60),
        )
    }
}

impl TemplateCache {
    pub fn new(capacity: u64, ttl: Duration, tti: Duration) -> Self {
        Self {
            inner: moka::sync::Cache::builder()
                .max_capacity(capacity)
                .time_to_live(ttl)
                .time_to_idle(tti)
                .build(),
        }
    }

    /// Resolve a board into a shared run template.
    ///
    /// Warm path: this cache (HEAD-revalidated for "latest"). Cold path: the
    /// persisted artifact (`compiled/` for versions, app-scoped
    /// `tmp/apps/{app_id}/compiled/drafts/` keyed by source etag for drafts),
    /// rejected on registry-fingerprint mismatch;
    /// last resort: load the proto, run `node_updates`, compile, and persist
    /// the artifact in the background.
    ///
    /// `key_salt` lets callers segment the cache beyond app/board/version —
    /// the executor passes its WASM bundle signature.
    pub async fn resolve(
        &self,
        state: &Arc<FlowLikeState>,
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
        expected_etag: Option<&str>,
        key_salt: &str,
    ) -> Result<Arc<CompiledRunTemplate>> {
        if version.is_some() && expected_etag.is_some() {
            return Err(anyhow!(
                "A pinned board version cannot also select a Latest ETag"
            ));
        }
        if expected_etag.is_some_and(|etag| etag.trim().is_empty()) {
            return Err(anyhow!(
                "An ETag-bound Latest board requires a non-empty ETag"
            ));
        }
        let expected_etag = expected_etag.map(str::trim).filter(|etag| !etag.is_empty());
        let storage_root = Path::from("apps").child(app_id.to_string());
        let proto_path = Board::proto_path(&storage_root, board_id, version);

        let meta_store = state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or_else(|| anyhow!("Project store not found while loading board {board_id}"))?
            .as_generic();

        let registry = state.node_registry.read().await.node_registry.clone();
        let fingerprint = registry.fingerprint();

        let version_key = match (version, expected_etag) {
            (Some((m, n, p)), None) => format!("{m}_{n}_{p}"),
            (Some(_), Some(_)) => unreachable!("validated above"),
            (None, Some(etag)) => format!("latest@{etag}"),
            (None, None) => "latest".to_string(),
        };
        let fingerprint_hex = blake3::Hash::from_bytes(fingerprint).to_hex();
        let cache_key = format!(
            "{app_id}:{board_id}:{version_key}:{}:{key_salt}",
            &fingerprint_hex.as_str()[..16]
        );

        let mut source_head: Option<ObjectMeta> = None;
        if let Some(cached) = self.inner.get(&cache_key) {
            if version.is_some() || expected_etag.is_some() {
                tracing::debug!(cache_key = %cache_key, "Template cache hit (content-addressed)");
                return Ok(cached.template.clone());
            }
            match meta_store.head(&proto_path).await {
                Ok(head) if meta_unchanged(cached.as_ref(), &head) => {
                    tracing::debug!(cache_key = %cache_key, "Template cache hit (HEAD validated)");
                    return Ok(cached.template.clone());
                }
                Ok(head) => {
                    tracing::debug!(cache_key = %cache_key, "Template cache stale, reloading");
                    self.inner.invalidate(&cache_key);
                    source_head = Some(head);
                }
                Err(e) => {
                    tracing::warn!(cache_key = %cache_key, error = %e, "HEAD failed during template revalidation");
                    self.inner.invalidate(&cache_key);
                }
            }
        }

        // Persisted artifact fast path — skips proto decode and `node_updates`.
        let artifact_path = match (version, expected_etag) {
            (Some(v), None) => Some(super::artifact_path(&storage_root, board_id, v)),
            (Some(_), Some(_)) => unreachable!("validated above"),
            (None, Some(etag)) => Some(super::draft_artifact_path(
                app_id,
                board_id,
                etag,
                &fingerprint,
            )),
            (None, None) => {
                if source_head.is_none() {
                    source_head = meta_store.head(&proto_path).await.ok();
                }
                source_head
                    .as_ref()
                    .and_then(|h| h.e_tag.clone())
                    .map(|e_tag| super::draft_artifact_path(app_id, board_id, &e_tag, &fingerprint))
            }
        };
        if let Some(ref path) = artifact_path
            && let Some(template) = template_from_artifact(
                &meta_store,
                path,
                &fingerprint,
                registry.as_ref(),
                &storage_root,
            )
            .await
        {
            let (e_tag, last_modified) = if let Some(etag) = expected_etag {
                (Some(etag.to_string()), chrono::Utc::now())
            } else {
                source_head
                    .as_ref()
                    .map(|h| (h.e_tag.clone(), h.last_modified))
                    .unwrap_or((None, chrono::Utc::now()))
            };
            self.inner.insert(
                cache_key.clone(),
                Arc::new(CachedTemplate {
                    template: template.clone(),
                    e_tag,
                    last_modified,
                }),
            );
            tracing::debug!(cache_key = %cache_key, "Template built from artifact");
            return Ok(template);
        }

        // Cold path: compile from the source board and persist the artifact.
        // The template's Board view is reconstructed from the compiled form
        // (see `CompiledRunTemplate::from_compiled`), which also guarantees it
        // never carries this request's credentials or logic.
        let (loaded, meta) = if let Some(expected_etag) = expected_etag {
            let source_path = super::draft_source_path(app_id, board_id, expected_etag);
            from_compressed_with_meta(meta_store.clone(), source_path.clone())
                .await
                .map_err(|e| {
                    anyhow!(
                        "Exact source snapshot for Latest board {board_id} is unavailable at {source_path}: {e}"
                    )
                })?
        } else {
            Board::load_proto_with_meta(meta_store.clone(), &storage_root, board_id, version)
                .await
                .map_err(|e| anyhow!("Failed to load board {board_id}: {e}"))?
        };
        let board = Board::from_loaded_proto(loaded, storage_root.clone(), state.clone()).await;

        let compiled_board = super::compile::compile_board_with_catalog(&board, registry.as_ref())
            .map_err(|e| anyhow!("Failed to compile board {board_id}: {e}"))?;
        drop(board);
        let template = Arc::new(
            CompiledRunTemplate::from_compiled(
                &compiled_board,
                registry.as_ref(),
                storage_root.clone(),
            )
            .map_err(|e| anyhow!("Failed to build run template for board {board_id}: {e}"))?,
        );

        // Drafts churn with every save; purge the board's older draft
        // artifacts alongside the write so exactly one survives (there are no
        // lifecycle rules on local stores).
        let write_back = match version {
            Some(v) => Some((super::artifact_path(&storage_root, board_id, v), None)),
            None => expected_etag.or(meta.e_tag.as_deref()).map(|e_tag| {
                // Floating artifacts are content-addressed. Keep prior ETags
                // available to queued exact Page runs; lifecycle cleanup and
                // the desktop startup sweep reclaim them later.
                (
                    super::draft_artifact_path(app_id, board_id, e_tag, &fingerprint),
                    None,
                )
            }),
        };
        if let Some((path, purge_prefix)) = write_back {
            match super::encode_artifact(&compiled_board, &fingerprint) {
                Ok(bytes) => spawn_artifact_write_back(meta_store, path, purge_prefix, bytes),
                Err(e) => {
                    tracing::warn!(board_id = %board_id, error = %e, "Failed to encode compiled board artifact")
                }
            }
        }

        self.inner.insert(
            cache_key.clone(),
            Arc::new(CachedTemplate {
                template: template.clone(),
                e_tag: expected_etag.map(str::to_string).or(meta.e_tag.clone()),
                last_modified: meta.last_modified,
            }),
        );
        tracing::debug!(cache_key = %cache_key, "Template compiled from source");
        Ok(template)
    }
}

/// Try to build a template from a persisted compiled artifact. Any failure —
/// missing object, format/fingerprint mismatch, decode error — returns None
/// and the caller compiles from the source `.board` instead.
async fn template_from_artifact(
    meta_store: &Arc<dyn ObjectStore>,
    artifact_path: &Path,
    fingerprint: &[u8; 32],
    registry: &FlowNodeRegistryInner,
    storage_root: &Path,
) -> Option<Arc<CompiledRunTemplate>> {
    let result = meta_store.get(artifact_path).await.ok()?;
    let bytes = result.bytes().await.ok()?;
    let compiled_board = match super::decode_artifact(&bytes, Some(fingerprint)) {
        Ok(compiled_board) => compiled_board,
        Err(e) => {
            tracing::debug!(path = %artifact_path, error = %e, "Compiled artifact rejected");
            return None;
        }
    };
    match CompiledRunTemplate::from_compiled(&compiled_board, registry, storage_root.clone()) {
        Ok(template) => Some(Arc::new(template)),
        Err(e) => {
            tracing::warn!(path = %artifact_path, error = %e, "Compiled artifact failed template build");
            None
        }
    }
}

/// Persist a compiled artifact in the background. Artifacts are recreatable,
/// so failures only cost warmth, never correctness.
fn spawn_artifact_write_back(
    meta_store: Arc<dyn ObjectStore>,
    artifact_path: Path,
    purge_prefix: Option<Path>,
    bytes: Vec<u8>,
) {
    flow_like_types::tokio::spawn(async move {
        // Executors may lack write access to the meta store (AWS scopes them
        // to GetObject); the API ensures artifacts before dispatch there, so
        // a failed opportunistic write is only diagnostic.
        if let Err(e) = persist_artifact(&meta_store, &artifact_path, purge_prefix, bytes).await {
            tracing::debug!(path = %artifact_path, error = %e, "Compiled board artifact write-back failed");
        } else {
            tracing::debug!(path = %artifact_path, "Compiled board artifact persisted");
        }
    });
}

fn is_app_scoped_draft_artifact(path: &Path) -> bool {
    let parts: Vec<&str> = path.as_ref().split('/').collect();
    parts.len() == 7
        && parts[0] == "tmp"
        && parts[1] == "apps"
        && !parts[2].is_empty()
        && parts[3] == "compiled"
        && parts[4] == "drafts"
        && !parts[5].is_empty()
        && !parts[6].is_empty()
}

/// Delete draft artifacts older than `max_age` from the app-scoped
/// `tmp/apps/` draft namespace. The legacy `tmp/compiled/` namespace is swept
/// too so upgrades eventually reclaim artifacts written by older builds.
/// Desktop runs this once at startup because local stores have no bucket
/// lifecycle rules.
pub async fn sweep_draft_artifacts(
    meta_store: &Arc<dyn ObjectStore>,
    max_age: Duration,
) -> Result<usize> {
    use futures::{StreamExt, TryStreamExt};

    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::days(7));
    let draft_prefix = Path::from("tmp").child("apps");
    let current_cutoff = cutoff;
    let current = meta_store
        .list(Some(&draft_prefix))
        .try_filter(move |meta| {
            futures::future::ready(
                meta.last_modified < current_cutoff && is_app_scoped_draft_artifact(&meta.location),
            )
        })
        .map_ok(|meta| meta.location)
        .boxed();

    let legacy_prefix = Path::from("tmp").child("compiled");
    let legacy = meta_store
        .list(Some(&legacy_prefix))
        .try_filter(move |meta| futures::future::ready(meta.last_modified < cutoff))
        .map_ok(|meta| meta.location)
        .boxed();

    let stale = futures::stream::select(current, legacy).boxed();
    let deleted = meta_store
        .delete_stream(stale)
        .try_collect::<Vec<Path>>()
        .await
        .map_err(|e| anyhow!("draft artifact sweep failed: {e}"))?;
    Ok(deleted.len())
}

/// Write a compiled artifact, first purging any sibling artifacts under
/// `purge_prefix` (stale drafts) down to the one being written.
pub async fn persist_artifact(
    meta_store: &Arc<dyn ObjectStore>,
    artifact_path: &Path,
    purge_prefix: Option<Path>,
    bytes: Vec<u8>,
) -> Result<()> {
    if let Some(prefix) = purge_prefix {
        use futures::{StreamExt, TryStreamExt};
        let stale = meta_store
            .list(Some(&prefix))
            .map_ok(|m| m.location)
            .try_filter(|location| futures::future::ready(location != artifact_path))
            .boxed();
        if let Err(e) = meta_store
            .delete_stream(stale)
            .try_collect::<Vec<Path>>()
            .await
        {
            tracing::debug!(prefix = %prefix, error = %e, "Stale draft artifact purge failed");
        }
    }

    meta_store
        .put(artifact_path, PutPayload::from(bytes))
        .await
        .map_err(|e| anyhow!("failed to persist compiled artifact at {artifact_path}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    #[test]
    fn identifies_only_app_scoped_draft_artifacts() {
        assert!(is_app_scoped_draft_artifact(&Path::from(
            "tmp/apps/app-1/compiled/drafts/board-1/artifact.flcb"
        )));
        assert!(!is_app_scoped_draft_artifact(&Path::from(
            "apps/app-1/compiled/drafts/board-1/artifact.flcb"
        )));
        assert!(!is_app_scoped_draft_artifact(&Path::from(
            "apps/app-1/meta/compiled/board-1/1_0_0.flcb"
        )));
        assert!(!is_app_scoped_draft_artifact(&Path::from(
            "tmp/apps/app-1/compiled/drafts/board-1/nested/artifact.flcb"
        )));
        assert!(!is_app_scoped_draft_artifact(&Path::from(
            "tmp/compiled/app-1/board-1/artifact.flcb"
        )));
    }

    #[tokio::test]
    async fn sweep_removes_current_and_legacy_drafts_without_touching_version_artifacts() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let current =
            crate::flow::compiled::draft_artifact_path("app-1", "board-1", "etag", &[7; 32]);
        let current_manifest =
            crate::flow::compiled::draft_manifest_path("app-1", "board-1", "etag");
        let current_page_manifest = crate::flow::compiled::draft_page_manifest_path(
            "app-1",
            "board-1",
            "etag",
            "page-1",
            "revision-1",
        );
        let current_source = crate::flow::compiled::draft_source_path("app-1", "board-1", "etag");
        let legacy = Path::from("tmp/compiled/app-1/board-1/legacy.flcb");
        let version = Path::from("apps/app-1/meta/compiled/board-1/1_0_0.flcb");
        // The sweep only lists `tmp/`; anything under `apps/` — including the
        // pre-`tmp/` draft location — is out of scope.
        let former_draft_location = Path::from("apps/app-1/compiled/drafts/board-1/old.flcb");

        assert!(
            current
                .as_ref()
                .starts_with("tmp/apps/app-1/compiled/drafts/board-1/")
        );

        for path in [
            &current,
            &current_manifest,
            &current_page_manifest,
            &current_source,
            &legacy,
            &version,
            &former_draft_location,
        ] {
            store
                .put(path, PutPayload::from_static(b"artifact"))
                .await
                .unwrap();
        }

        let deleted = sweep_draft_artifacts(&store, Duration::ZERO).await.unwrap();
        assert_eq!(deleted, 5);
        assert!(store.head(&current).await.is_err());
        assert!(store.head(&current_manifest).await.is_err());
        assert!(store.head(&current_page_manifest).await.is_err());
        assert!(store.head(&current_source).await.is_err());
        assert!(store.head(&legacy).await.is_err());
        assert!(store.head(&version).await.is_ok());
        assert!(store.head(&former_draft_location).await.is_ok());
    }
}

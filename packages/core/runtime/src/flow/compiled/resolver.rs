//! Shared template resolution.
//!
//! Two entry points share one cache type:
//! - [`TemplateCache::resolve`] — the desktop path: in-process cache →
//!   persisted artifact → compile-from-source with background write-back.
//!   It addresses the meta store directly.
//! - [`template_from_bytes`] + [`TemplateCache::get`]/[`TemplateCache::insert`]
//!   — the server-executor path. The API compiles every run ahead of dispatch
//!   and hands the executor a presigned GET for the exact `.flcb`; the
//!   executor decodes those bytes and never touches the meta store, never
//!   reads a board proto and never writes an artifact back.

use super::CompiledRunTemplate;
use crate::flow::board::Board;
use crate::state::{FlowLikeState, FlowNodeRegistryInner};
use crate::utils::compression::from_compressed_with_meta;
use flow_like_storage::Path;
use flow_like_storage::object_store::ObjectStoreExt;
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

    /// The cache key for one (board, content identity, registry, salt) tuple.
    ///
    /// `version_key` is `M_m_p` for a pinned version, `latest@{etag}` for a
    /// floating board pinned to a source ETag, or `latest` when only HEAD
    /// revalidation identifies it. `key_salt` segments the cache beyond
    /// app/board/version — the executor passes its WASM bundle signature.
    pub fn cache_key(
        app_id: &str,
        board_id: &str,
        version_key: &str,
        fingerprint: &[u8; 32],
        key_salt: &str,
    ) -> String {
        let fingerprint_hex = blake3::Hash::from_bytes(*fingerprint).to_hex();
        format!(
            "{app_id}:{board_id}:{version_key}:{}:{key_salt}",
            &fingerprint_hex.as_str()[..16]
        )
    }

    /// A cached template by exact key, without revalidation. Callers that key
    /// on a content identity (source ETag or pinned version) need none.
    pub fn get(&self, key: &str) -> Option<Arc<CompiledRunTemplate>> {
        self.inner.get(key).and_then(|cached| {
            cached.template.ensure_supported_format().ok()?;
            Some(cached.template.clone())
        })
    }

    /// Cache a template the caller built itself. Entries carry no storage
    /// identity because only [`Self::resolve`] revalidates by HEAD, and it
    /// only ever revalidates keys it computed itself.
    pub fn insert(&self, key: String, template: Arc<CompiledRunTemplate>) {
        self.inner.insert(
            key,
            Arc::new(CachedTemplate {
                template,
                e_tag: None,
                last_modified: chrono::Utc::now(),
            }),
        );
    }

    /// Resolve a board into a shared run template (desktop path).
    ///
    /// Warm path: this cache (HEAD-revalidated for "latest"). Cold path: the
    /// persisted artifact (`compiled/` for versions, app-scoped
    /// `tmp/apps/{app_id}/compiled/drafts/` keyed by source etag for drafts),
    /// rejected on registry-fingerprint mismatch;
    /// last resort: load the proto, run `node_updates`, compile, and persist
    /// the artifact in the background.
    ///
    /// An artifact of the same source compiled against another registry is adopted when the
    /// node types the board places are registered identically in this one, so a catalog
    /// change elsewhere does not send the board back to its source.
    ///
    /// `key_salt` lets callers segment the cache beyond app/board/version.
    pub async fn resolve(
        &self,
        state: &Arc<FlowLikeState>,
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
        expected_etag: Option<&str>,
        key_salt: &str,
    ) -> Result<Arc<CompiledRunTemplate>> {
        self.resolve_inner(
            state,
            app_id,
            board_id,
            version,
            expected_etag,
            key_salt,
            false,
        )
        .await
    }

    /// [`Self::resolve`] for callers that read [`CompiledRunTemplate::baseline`], such as a
    /// Page contract. A template whose artifact predates those facts is compiled from the
    /// source once, and the artifact is rewritten with them.
    pub async fn resolve_with_baseline(
        &self,
        state: &Arc<FlowLikeState>,
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
        key_salt: &str,
    ) -> Result<Arc<CompiledRunTemplate>> {
        self.resolve_inner(state, app_id, board_id, version, None, key_salt, true)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn resolve_inner(
        &self,
        state: &Arc<FlowLikeState>,
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
        expected_etag: Option<&str>,
        key_salt: &str,
        require_baseline: bool,
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
        let storage_root = Path::from("apps").join(app_id.to_string());
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
        // A Page start runs the logic the registry holds at this moment. A package reload
        // registers new logic under an unchanged fingerprint, which the cache key cannot see.
        let usable = |template: &CompiledRunTemplate| {
            !require_baseline
                || (template.baseline.is_some() && template.holds_logic_of(registry.as_ref()))
        };

        let version_key = match (version, expected_etag) {
            (Some((m, n, p)), None) => format!("{m}_{n}_{p}"),
            (Some(_), Some(_)) => unreachable!("validated above"),
            (None, Some(etag)) => format!("latest@{etag}"),
            (None, None) => "latest".to_string(),
        };
        let cache_key = Self::cache_key(app_id, board_id, &version_key, &fingerprint, key_salt);

        let mut source_head: Option<ObjectMeta> = None;
        if let Some(cached) = self
            .inner
            .get(&cache_key)
            .filter(|cached| usable(cached.template.as_ref()))
        {
            cached.template.ensure_supported_format()?;
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
        let from_artifact = match artifact_path.as_ref() {
            None => None,
            Some(path) => {
                let exact = template_from_artifact(
                    &meta_store,
                    path,
                    &fingerprint,
                    registry.as_ref(),
                    &storage_root,
                )
                .await
                .filter(|template| usable(template.as_ref()));
                match exact {
                    Some(template) => Some(template),
                    // A queued exact run names one artifact; nothing else may stand in for it.
                    None if expected_etag.is_some() => None,
                    None => {
                        let siblings = sibling_artifacts(
                            &meta_store,
                            app_id,
                            board_id,
                            version,
                            source_head.as_ref(),
                            path,
                        )
                        .await;
                        adopt_artifact(
                            &meta_store,
                            siblings,
                            &fingerprint,
                            registry.as_ref(),
                            &storage_root,
                        )
                        .await
                        .map(|(template, restamped)| {
                            spawn_artifact_write_back(
                                meta_store.clone(),
                                path.clone(),
                                None,
                                restamped,
                            );
                            template
                        })
                    }
                }
            }
        };
        if let Some(template) = from_artifact {
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
        let board = if expected_etag.is_some() {
            Board::from_loaded_proto(loaded, storage_root.clone(), state.clone()).await?
        } else {
            Board::from_loaded_proto_for_version(
                loaded,
                storage_root.clone(),
                state.clone(),
                version,
            )
            .await?
        };

        let compiled_board = super::compile::compile_board_with_catalog(&board, registry.as_ref())
            .map_err(|e| anyhow!("Failed to compile board {board_id}: {e}"))?;
        // Only the source board can supply these; the compiled form drops what they cover.
        let facts = super::BaselineFacts::from_source(&board, registry.as_ref());
        drop(board);
        let mut template = CompiledRunTemplate::from_compiled(
            &compiled_board,
            registry.as_ref(),
            storage_root.clone(),
        )
        .map_err(|e| anyhow!("Failed to build run template for board {board_id}: {e}"))?;
        template.baseline = Some(Arc::new(facts.clone()));
        let template = Arc::new(template);

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
            match super::encode_artifact(&compiled_board, &fingerprint)
                .and_then(|bytes| super::append_baseline_facts(&bytes, &facts))
            {
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
    match template_from_bytes(&bytes, fingerprint, registry, storage_root) {
        Ok(template) => Some(template),
        Err(e) => {
            tracing::debug!(path = %artifact_path, error = %e, "Compiled artifact rejected");
            None
        }
    }
}

/// Build a run template from the bytes of a persisted `.flcb` artifact.
///
/// This is the whole of the server executor's board loading: the API
/// compiled these bytes against a registry whose fingerprint is in the
/// header, and the executor accepts them only if that fingerprint is its
/// own — an artifact compiled against a different node catalog would bake
/// the wrong defaults and pin layouts. The error names both fingerprints so
/// a deploy skew between API and executor is diagnosable from the message.
pub fn template_from_bytes(
    bytes: &[u8],
    fingerprint: &[u8; 32],
    registry: &FlowNodeRegistryInner,
    storage_root: &Path,
) -> Result<Arc<CompiledRunTemplate>> {
    let header = super::peek_header(bytes)?;
    if &header.registry_fingerprint != fingerprint {
        let ours = blake3::Hash::from_bytes(*fingerprint).to_hex();
        let theirs = blake3::Hash::from_bytes(header.registry_fingerprint).to_hex();
        return Err(anyhow!(
            "compiled artifact was built against registry {} but this runtime's registry is {}",
            &theirs.as_str()[..16],
            &ours.as_str()[..16]
        ));
    }
    // from_compiled validates arena indices before reconstructing the Board.
    // Public decode_artifact retains that validation for its own callers.
    let compiled_board = super::decode_archive(bytes, &header)?;
    let mut template =
        CompiledRunTemplate::from_compiled(&compiled_board, registry, storage_root.clone())
            .map_err(|e| anyhow!("compiled artifact failed template build: {e}"))?;
    template.widget_snapshot = super::decode_widget_snapshot(bytes)
        .inspect_err(|error| tracing::warn!(%error, "Ignoring invalid artifact widget snapshot"))
        .ok()
        .flatten()
        .map(Arc::new);
    template.baseline = super::decode_baseline_facts(bytes)
        .inspect_err(|error| tracing::warn!(%error, "Ignoring invalid artifact baseline facts"))
        .ok()
        .flatten()
        .map(Arc::new);
    Ok(Arc::new(template))
}

/// How many artifacts of one source are read while looking for one to adopt.
const ADOPTION_CANDIDATES: usize = 4;

/// Artifacts of this exact source that a registry with another fingerprint compiled, newest
/// first. A version has one artifact path, whose header names the compiling registry; a draft
/// has one path per registry fingerprint.
async fn sibling_artifacts(
    meta_store: &Arc<dyn ObjectStore>,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
    source_head: Option<&ObjectMeta>,
    exact: &Path,
) -> Vec<Path> {
    use futures::TryStreamExt;

    if version.is_some() {
        return vec![exact.clone()];
    }
    let Some(e_tag) = source_head.and_then(|head| head.e_tag.as_deref()) else {
        return Vec::new();
    };
    let prefix = format!("{}_", super::draft_artifact_stem(e_tag));
    let mut siblings: Vec<ObjectMeta> = meta_store
        .list(Some(&super::draft_artifact_dir(app_id, board_id)))
        .try_filter(|meta| {
            futures::future::ready(
                meta.location != *exact
                    && meta
                        .location
                        .filename()
                        .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".flcb")),
            )
        })
        .try_collect()
        .await
        .unwrap_or_default();
    siblings.sort_by_key(|meta| std::cmp::Reverse(meta.last_modified));
    siblings
        .into_iter()
        .take(ADOPTION_CANDIDATES)
        .map(|meta| meta.location)
        .collect()
}

/// The first candidate whose recorded node types are registered identically in `registry`,
/// as a template plus the artifact bytes restamped for this registry's fingerprint.
async fn adopt_artifact(
    meta_store: &Arc<dyn ObjectStore>,
    candidates: Vec<Path>,
    fingerprint: &[u8; 32],
    registry: &FlowNodeRegistryInner,
    storage_root: &Path,
) -> Option<(Arc<CompiledRunTemplate>, Vec<u8>)> {
    for path in candidates {
        let Ok(read) = meta_store.get(&path).await else {
            continue;
        };
        let Ok(bytes) = read.bytes().await else {
            continue;
        };
        match adopt_bytes(&bytes, fingerprint, registry, storage_root) {
            Ok(Some(adopted)) => {
                tracing::debug!(path = %path, "Compiled artifact adopted from another registry");
                return Some(adopted);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(path = %path, error = %error, "Compiled artifact not adoptable")
            }
        }
    }
    None
}

/// An artifact is bound to the whole registry it was compiled against, but its content only
/// depends on the node types its board places. When those are registered identically here,
/// compiling again would reproduce the same board, so the artifact is taken as it is.
fn adopt_bytes(
    bytes: &[u8],
    fingerprint: &[u8; 32],
    registry: &FlowNodeRegistryInner,
    storage_root: &Path,
) -> Result<Option<(Arc<CompiledRunTemplate>, Vec<u8>)>> {
    let header = super::peek_header(bytes)?;
    let Some(facts) = super::decode_baseline_facts(bytes)? else {
        return Ok(None);
    };
    if !facts.holds_for(registry) {
        return Ok(None);
    }
    let compiled = super::decode_archive(bytes, &header)?;
    let mut template =
        CompiledRunTemplate::from_compiled(&compiled, registry, storage_root.clone())?;
    let restamped =
        super::append_baseline_facts(&super::encode_artifact(&compiled, fingerprint)?, &facts)?;
    template.baseline = Some(Arc::new(facts));
    Ok(Some((Arc::new(template), restamped)))
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

/// The draft objects of the source each board has right now. A board that has not changed
/// still starts from them, however long ago they were written. Of several artifacts for that
/// source, one per registry fingerprint, only the newest counts.
async fn current_draft_objects(
    meta_store: &Arc<dyn ObjectStore>,
    drafts: &[ObjectMeta],
    cutoff: chrono::DateTime<chrono::Utc>,
) -> std::collections::HashSet<Path> {
    let mut boards: std::collections::HashMap<(&str, &str), Vec<&ObjectMeta>> =
        std::collections::HashMap::new();
    for meta in drafts {
        let mut parts = meta.location.as_ref().split('/');
        if let (Some(app_id), Some(board_id)) = (parts.nth(2), parts.nth(2)) {
            boards.entry((app_id, board_id)).or_default().push(meta);
        }
    }

    let is_artifact = |meta: &ObjectMeta| {
        meta.location
            .filename()
            .is_some_and(|name| name.ends_with(".flcb"))
    };
    let mut current = std::collections::HashSet::new();
    for ((app_id, board_id), objects) in boards {
        if objects.iter().all(|meta| meta.last_modified >= cutoff) {
            continue;
        }
        let source = Board::proto_path(&Path::from("apps").join(app_id), board_id, None);
        let Some(e_tag) = meta_store
            .head(&source)
            .await
            .ok()
            .and_then(|head| head.e_tag)
        else {
            continue;
        };
        let stem = super::draft_artifact_stem(&e_tag);
        let of_source = || {
            objects.iter().copied().filter(|meta| {
                meta.location
                    .filename()
                    .is_some_and(|name| name.starts_with(&stem))
            })
        };
        let newest_artifact = of_source()
            .filter(|meta| is_artifact(meta))
            .max_by(|left, right| {
                left.last_modified
                    .cmp(&right.last_modified)
                    .then_with(|| left.location.as_ref().cmp(right.location.as_ref()))
            })
            .map(|meta| &meta.location);
        current.extend(
            of_source()
                .filter(|meta| !is_artifact(meta) || Some(&meta.location) == newest_artifact)
                .map(|meta| meta.location.clone()),
        );
    }
    current
}

/// Delete draft artifacts older than `max_age` from the app-scoped
/// `tmp/apps/` draft namespace, except those of the source a board still has
/// ([`current_draft_objects`]). The legacy `tmp/compiled/` namespace is swept
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
    let drafts: Vec<ObjectMeta> = meta_store
        .list(Some(&Path::from("tmp").join("apps")))
        .try_filter(|meta| futures::future::ready(is_app_scoped_draft_artifact(&meta.location)))
        .try_collect()
        .await
        .map_err(|e| anyhow!("draft artifact sweep failed: {e}"))?;
    let kept = current_draft_objects(meta_store, &drafts, cutoff).await;
    let current = futures::stream::iter(
        drafts
            .into_iter()
            .filter(|meta| meta.last_modified < cutoff && !kept.contains(&meta.location))
            .map(|meta| Ok(meta.location))
            .collect::<Vec<_>>(),
    )
    .boxed();

    let legacy_prefix = Path::from("tmp").join("compiled");
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
        let retained_path = artifact_path.clone();
        let stale = meta_store
            .list(Some(&prefix))
            .map_ok(|m| m.location)
            .try_filter(move |location| futures::future::ready(location != &retained_path))
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

    #[tokio::test]
    async fn sweep_keeps_the_artifact_of_the_source_a_board_still_has() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let source = Board::proto_path(&Path::from("apps").join("app-1"), "board-1", None);
        store
            .put(&source, PutPayload::from_static(b"source"))
            .await
            .unwrap();
        let e_tag = store.head(&source).await.unwrap().e_tag.unwrap();
        let draft = crate::flow::compiled::draft_artifact_path;
        let superseded = draft("app-1", "board-1", &e_tag, &[8; 32]);
        let current = draft("app-1", "board-1", &e_tag, &[7; 32]);
        let manifest = crate::flow::compiled::draft_manifest_path("app-1", "board-1", &e_tag);
        let earlier_source = draft("app-1", "board-1", "earlier-etag", &[7; 32]);
        let deleted_board = draft("app-1", "board-2", "any-etag", &[7; 32]);

        for path in [
            &superseded,
            &current,
            &manifest,
            &earlier_source,
            &deleted_board,
        ] {
            store
                .put(path, PutPayload::from_static(b"artifact"))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        let deleted = sweep_draft_artifacts(&store, Duration::ZERO).await.unwrap();
        assert_eq!(deleted, 3);
        assert!(store.head(&current).await.is_ok());
        assert!(store.head(&manifest).await.is_ok());
        assert!(store.head(&superseded).await.is_err());
        assert!(store.head(&earlier_source).await.is_err());
        assert!(store.head(&deleted_board).await.is_err());
    }

    mod resolving {
        use super::*;
        use crate::flow::compiled::{
            artifact_path, compile_board_with_catalog, decode_baseline_facts, draft_artifact_path,
            encode_artifact, peek_header,
        };
        use crate::flow::execution::context::ExecutionContext;
        use crate::flow::node::{Node, NodeLogic};
        use crate::state::FlowLikeConfig;
        use crate::utils::http::HTTPClient;
        use flow_like_storage::files::store::FlowLikeStore;
        use flow_like_types::async_trait;

        const APP: &str = "app";
        const BOARD: &str = "board";

        struct Typed {
            name: &'static str,
            friendly_name: &'static str,
        }

        #[async_trait]
        impl NodeLogic for Typed {
            fn get_node(&self) -> Node {
                Node::new(self.name, self.friendly_name, "", "test")
            }

            async fn run(&self, _: &mut ExecutionContext) -> Result<()> {
                Ok(())
            }
        }

        fn root() -> Path {
            Path::from("apps").join(APP)
        }

        /// A host over `store` whose catalog registers exactly `types`.
        async fn host(
            store: &Arc<InMemory>,
            types: &[(&'static str, &'static str)],
        ) -> (Arc<FlowLikeState>, [u8; 32]) {
            let mut config = FlowLikeConfig::new();
            config.register_app_meta_store(FlowLikeStore::Other(store.clone()));
            let state = Arc::new(FlowLikeState::new(
                config,
                HTTPClient::new_without_refetch(),
            ));
            let logic = types
                .iter()
                .map(|&(name, friendly_name)| {
                    Arc::new(Typed {
                        name,
                        friendly_name,
                    }) as Arc<dyn NodeLogic>
                })
                .collect();
            state.node_registry.write().await.push_nodes(logic);
            let fingerprint = state.node_registry.read().await.node_registry.fingerprint();
            (state, fingerprint)
        }

        async fn save_board(state: &Arc<FlowLikeState>) {
            let mut board = Board::new(Some(BOARD.into()), root(), state.clone());
            board.version = (1, 0, 0);
            let node = Node::new("used", "Used", "", "test");
            board.nodes.insert(node.id.clone(), node);
            board.save(None).await.unwrap();
        }

        /// Artifacts are written by a spawned task; wait until `path` holds one that `accept`s.
        async fn written(
            store: &Arc<InMemory>,
            path: &Path,
            accept: impl Fn(&[u8]) -> bool,
        ) -> Vec<u8> {
            for _ in 0..400 {
                if let Ok(read) = store.get(path).await {
                    let bytes = read.bytes().await.unwrap().to_vec();
                    if accept(&bytes) {
                        return bytes;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("{path} never held the expected artifact");
        }

        async fn draft_path(store: &Arc<InMemory>, fingerprint: &[u8; 32]) -> Path {
            let source = Board::proto_path(&root(), BOARD, None);
            let e_tag = store.head(&source).await.unwrap().e_tag.unwrap();
            draft_artifact_path(APP, BOARD, &e_tag, fingerprint)
        }

        #[tokio::test]
        async fn a_compile_records_the_facts_and_a_later_process_reads_them() {
            let store = Arc::new(InMemory::new());
            let (state, fingerprint) = host(&store, &[("used", "Used")]).await;
            save_board(&state).await;

            let compiled = TemplateCache::default()
                .resolve(&state, APP, BOARD, None, None, "")
                .await
                .unwrap();
            let facts = compiled.baseline.clone().expect("a compile records facts");
            let path = draft_path(&store, &fingerprint).await;
            let bytes = written(&store, &path, |_| true).await;
            assert_eq!(
                decode_baseline_facts(&bytes).unwrap().as_ref(),
                Some(&*facts)
            );

            let later = TemplateCache::default()
                .resolve_with_baseline(&state, APP, BOARD, None, "")
                .await
                .unwrap();
            assert_eq!(later.baseline, Some(facts));
        }

        #[tokio::test]
        async fn an_artifact_without_facts_is_compiled_again_only_when_they_are_needed() {
            let store = Arc::new(InMemory::new());
            let (state, fingerprint) = host(&store, &[("used", "Used")]).await;
            save_board(&state).await;
            let registry = state.node_registry.read().await.node_registry.clone();
            let source = Board::load(root(), BOARD, state.clone(), None)
                .await
                .unwrap();
            let path = draft_path(&store, &fingerprint).await;
            let earlier_build = encode_artifact(
                &compile_board_with_catalog(&source, &registry).unwrap(),
                &fingerprint,
            )
            .unwrap();
            store
                .put(&path, PutPayload::from(earlier_build))
                .await
                .unwrap();

            let cache = TemplateCache::default();
            let plain = cache
                .resolve(&state, APP, BOARD, None, None, "")
                .await
                .unwrap();
            assert!(plain.baseline.is_none());
            let needed = cache
                .resolve_with_baseline(&state, APP, BOARD, None, "")
                .await
                .unwrap();
            assert!(needed.baseline.is_some());
            let afterwards = cache
                .resolve(&state, APP, BOARD, None, None, "")
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&afterwards, &needed));
            written(&store, &path, |bytes| {
                decode_baseline_facts(bytes).is_ok_and(|facts| facts.is_some())
            })
            .await;
        }

        #[tokio::test]
        async fn a_version_artifact_outlives_a_catalog_change_outside_its_node_types() {
            let store = Arc::new(InMemory::new());
            let version = (1, 0, 0);
            let (compiling, _) = host(&store, &[("used", "Used")]).await;
            save_board(&compiling).await;
            let snapshot = Board::proto_path(&root(), BOARD, Some(version));
            store
                .copy(&Board::proto_path(&root(), BOARD, None), &snapshot)
                .await
                .unwrap();
            TemplateCache::default()
                .resolve(&compiling, APP, BOARD, Some(version), None, "")
                .await
                .unwrap();
            let artifact = artifact_path(&root(), BOARD, version);
            written(&store, &artifact, |_| true).await;
            // Without its source the board can only start from the artifact.
            store.delete(&snapshot).await.unwrap();

            let (extended, extended_fingerprint) =
                host(&store, &[("used", "Used"), ("unrelated", "Unrelated")]).await;
            let adopted = TemplateCache::default()
                .resolve_with_baseline(&extended, APP, BOARD, Some(version), "")
                .await
                .expect("a catalog change elsewhere keeps the artifact usable");
            assert!(adopted.baseline.is_some());
            written(&store, &artifact, |bytes| {
                peek_header(bytes)
                    .is_ok_and(|header| header.registry_fingerprint == extended_fingerprint)
            })
            .await;

            let (changed, _) = host(&store, &[("used", "Renamed")]).await;
            assert!(
                TemplateCache::default()
                    .resolve(&changed, APP, BOARD, Some(version), None, "")
                    .await
                    .is_err(),
                "a changed node type must send the board back to its source"
            );
        }

        /// Registers `name` again with the definition it already has, as a package reload does.
        async fn reload(
            state: &Arc<FlowLikeState>,
            name: &'static str,
            friendly_name: &'static str,
        ) {
            let logic: Arc<dyn NodeLogic> = Arc::new(Typed {
                name,
                friendly_name,
            });
            state.node_registry.write().await.push_nodes(vec![logic]);
        }

        #[tokio::test]
        async fn a_page_start_runs_the_logic_a_reload_registered() {
            let store = Arc::new(InMemory::new());
            let version = (1, 0, 0);
            let (state, fingerprint) =
                host(&store, &[("used", "Used"), ("unrelated", "Unrelated")]).await;
            save_board(&state).await;
            let snapshot = Board::proto_path(&root(), BOARD, Some(version));
            store
                .copy(&Board::proto_path(&root(), BOARD, None), &snapshot)
                .await
                .unwrap();
            let cache = TemplateCache::default();
            let page = || cache.resolve_with_baseline(&state, APP, BOARD, Some(version), "");
            let first = page().await.unwrap();
            written(&store, &artifact_path(&root(), BOARD, version), |_| true).await;
            // Without its source the board can only start from the artifact.
            store.delete(&snapshot).await.unwrap();

            reload(&state, "unrelated", "Unrelated").await;
            assert!(
                Arc::ptr_eq(&page().await.unwrap(), &first),
                "a reload of a type the board does not place keeps its template"
            );

            reload(&state, "used", "Used").await;
            let registry = state.node_registry.read().await.node_registry.clone();
            assert_eq!(registry.fingerprint(), fingerprint);
            assert!(!first.holds_logic_of(&registry));
            let plain = cache
                .resolve(&state, APP, BOARD, Some(version), None, "")
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&plain, &first));

            let reloaded = page().await.unwrap();
            assert!(reloaded.holds_logic_of(&registry));
            assert!(reloaded.baseline.is_some());
            assert!(Arc::ptr_eq(&page().await.unwrap(), &reloaded));
        }

        #[tokio::test]
        async fn only_artifacts_of_the_same_source_are_candidates_for_adoption() {
            let memory = Arc::new(InMemory::new());
            let store: Arc<dyn ObjectStore> = memory.clone();
            let source = Board::proto_path(&root(), BOARD, None);
            store
                .put(&source, PutPayload::from_static(b"source"))
                .await
                .unwrap();
            let head = store.head(&source).await.unwrap();
            let e_tag = head.e_tag.clone().unwrap();
            let exact = draft_artifact_path(APP, BOARD, &e_tag, &[1; 32]);
            let sibling = draft_artifact_path(APP, BOARD, &e_tag, &[2; 32]);
            for path in [
                &exact,
                &sibling,
                &draft_artifact_path(APP, BOARD, "another-etag", &[2; 32]),
                &crate::flow::compiled::draft_manifest_path(APP, BOARD, &e_tag),
            ] {
                store
                    .put(path, PutPayload::from_static(b"artifact"))
                    .await
                    .unwrap();
            }

            assert_eq!(
                sibling_artifacts(&store, APP, BOARD, None, Some(&head), &exact).await,
                [sibling]
            );
            assert_eq!(
                sibling_artifacts(&store, APP, BOARD, Some((1, 0, 0)), None, &exact).await,
                [exact]
            );
        }
    }
}

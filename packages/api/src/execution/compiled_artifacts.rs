//! Pre-dispatch compiled-artifact assurance.
//!
//! The executor holds no meta-store credential and has no board source other
//! than the compiled artifact the dispatcher presigns for it, so the API
//! guarantees that artifact exists — for every run — before dispatch, and the
//! dispatcher refuses the run otherwise. The check is designed to be
//! near-free on the hot path:
//!
//! 1. In-memory positive cache keyed by (app, board, version|etag, registry
//!    fingerprint). Version artifacts are trusted for as long as the entry
//!    lives. Draft artifacts live under lifecycle-managed `tmp/`, so an entry
//!    is trusted only while the object's age is inside the retention floor.
//! 2. One ranged GET of the 40-byte artifact envelope — existence *and*
//!    fingerprint validity in a single round trip (on S3 Express One Zone a
//!    GET costs $0.03/M at single-digit-millisecond latency; see
//!    todo/compiled-board-execution.md for the S3-vs-DynamoDB numbers).
//!    Ordinary floating runs need one extra HEAD on the source `.board` for
//!    its ETag. Governed Page runs already carry the ETag they authorized;
//!    their reserved-version guard runs beside the lookup.
//! 3. On miss: compile from the API's prepared-board cache and persist.
//!
//! The registry compiled against is the one the executor will run with: the
//! built-in catalog extended with the run's WASM node definitions (see
//! [`crate::state::State::artifact_registry`]), so boards with WASM nodes
//! compile here too — without the API ever loading a module.

use crate::state::AppState;
use flow_like::flow::board::Board;
use flow_like::flow::compiled;
use flow_like::state::{FlowNodeRegistry, FlowNodeRegistryInner};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::{GetOptions, ObjectStore, PutMode, PutOptions, PutPayload};
use flow_like_storage::{Path, object_store::Error as StoreError};
use flow_like_types::anyhow;
use flow_like_types::dispatch::{ETAG_BOUND_LATEST_VERSION_SENTINEL, WasmPackageRef};
use flow_like_types::tokio::sync::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::Instrument;

/// Storage lifecycle reclaims `tmp/`, where every draft artifact lives, no
/// sooner than this after an object is written: AWS keeps it 30 days, GCP at
/// least one, and the page-action capability TTL already needs a full day.
const DRAFT_MIN_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
/// What still follows a successful check: queue wait, executor start, clock skew.
const DRAFT_USE_HEADROOM: Duration = Duration::from_secs(60 * 60);
const ENVELOPE_LEN: u64 = 40;

/// Until when a draft object written at `written` exists without asking
/// storage again.
pub(crate) fn draft_object_trusted_until(written: SystemTime) -> SystemTime {
    written + DRAFT_MIN_RETENTION - DRAFT_USE_HEADROOM
}

/// `compiled_artifact_cache` value for an artifact written at `written`.
fn artifact_trust(version: Option<(u32, u32, u32)>, written: SystemTime) -> Option<SystemTime> {
    version
        .is_none()
        .then(|| draft_object_trusted_until(written))
}

fn artifact_trusted(trust: Option<SystemTime>) -> bool {
    trust.is_none_or(|until| SystemTime::now() < until)
}

/// What the assurance established: the artifact's object key on the meta
/// store, the source ETag it was compiled from (floating Latest runs; `None`
/// for pinned versions) and the registry fingerprint in its header.
#[derive(Clone, Debug)]
pub struct EnsuredArtifact {
    pub path: Path,
    pub source_etag: Option<String>,
    pub registry_fingerprint: [u8; 32],
}

/// A board the server must not run because its catalog marks some of its nodes
/// `only_offline`. The executor rejects such a board before its first node, so
/// dispatch answers it as a client error instead of compiling it.
#[derive(Debug)]
pub struct LocalOnlyBoard {
    pub board_name: String,
    pub nodes: Vec<String>,
}

impl LocalOnlyBoard {
    pub fn message(&self, event_name: Option<&str>) -> String {
        let subject = match event_name {
            Some(event) => format!("Event \"{event}\" (board \"{}\")", self.board_name),
            None => format!("Board \"{}\"", self.board_name),
        };
        format!(
            "{subject} runs only on your own device, not on a server, because these nodes need your device: {}",
            self.nodes.join(", ")
        )
    }
}

impl std::fmt::Display for LocalOnlyBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message(None))
    }
}

impl std::error::Error for LocalOnlyBoard {}

/// The flag comes from the registry, as the executor's check does: a board can
/// alter its own copy of node metadata.
pub(crate) fn ensure_server_runnable(
    board: &Board,
    registry: &FlowNodeRegistryInner,
) -> flow_like_types::Result<()> {
    let mut nodes: Vec<String> = board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .filter(|node| {
            registry
                .registry
                .get(&node.name)
                .is_some_and(|(catalog, _)| catalog.only_offline)
        })
        .map(|node| node.friendly_name.clone())
        .collect();
    if nodes.is_empty() {
        return Ok(());
    }
    nodes.sort_unstable();
    nodes.dedup();
    Err(LocalOnlyBoard {
        board_name: board.name.clone(),
        nodes,
    }
    .into())
}

fn require_selected_draft_etag(
    board_id: &str,
    selected_etag: Option<&str>,
    loaded_etag: &str,
) -> flow_like_types::Result<()> {
    if selected_etag.is_some_and(|selected| selected != loaded_etag) {
        return Err(anyhow!(
            "Latest board {board_id} changed before its ETag-bound compiled artifact was created"
        ));
    }
    Ok(())
}

/// An ETag-bound run dispatches under the reserved version, so no real
/// snapshot may exist there.
async fn ensure_reserved_version_free(
    meta_store: &dyn ObjectStore,
    storage_root: &Path,
    board_id: &str,
) -> flow_like_types::Result<()> {
    let sentinel_path = Board::proto_path(
        storage_root,
        board_id,
        Some(ETAG_BOUND_LATEST_VERSION_SENTINEL),
    );
    match meta_store
        .head(&sentinel_path)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "artifact.reserved_version.head"
        ))
        .await
    {
        Err(StoreError::NotFound { .. }) => Ok(()),
        Ok(_) => Err(anyhow!(
            "board {board_id} already has a snapshot at the version reserved for ETag-bound Latest dispatch"
        )),
        Err(error) => Err(anyhow!(
            "failed to verify the reserved-version guard for board {board_id}: {error}"
        )),
    }
}

/// When the artifact at `path` was written, if it exists and was compiled
/// against `fingerprint`. One ranged GET of the envelope answers existence,
/// format version and fingerprint together.
pub(super) async fn stored_artifact_written(
    meta_store: &dyn ObjectStore,
    path: &Path,
    fingerprint: &[u8; 32],
) -> Option<SystemTime> {
    async {
        let result = meta_store
            .get_opts(path, GetOptions::new().with_range(Some(0..ENVELOPE_LEN)))
            .await
            .ok()?;
        let written = SystemTime::from(result.meta.last_modified);
        let header_bytes = result.bytes().await.ok()?;
        let header = compiled::peek_header(&header_bytes).ok()?;
        (header.registry_fingerprint == *fingerprint).then_some(written)
    }
    .instrument(tracing::info_span!(
        target: "flow_like::observability",
        "artifact.envelope.get"
    ))
    .await
}

/// Rewrite a draft artifact that is still in use but has aged past the
/// retention floor, restarting its lifecycle clock so this and every other
/// instance can trust it again without a per-run GET. The bytes are unchanged:
/// the path is content-addressed. Returns the new trust deadline; a failed
/// rewrite only leaves the artifact untrusted, so the next run checks it again.
pub(super) async fn refresh_draft_artifact(
    meta_store: &dyn ObjectStore,
    path: &Path,
) -> Option<SystemTime> {
    let refresh = async {
        let bytes = meta_store.get(path).await?.bytes().await?;
        let written = SystemTime::now();
        meta_store.put(path, bytes.into()).await?;
        flow_like_types::Ok(draft_object_trusted_until(written))
    };
    refresh
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "artifact.refresh"
        ))
        .await
        .inspect_err(|error| {
            tracing::warn!(path = %path, error = %error, "Draft artifact refresh failed");
        })
        .ok()
}

/// Ensure the compiled artifact for (board, version|latest) exists on the
/// meta store and matches the fingerprint of the registry the run will use.
#[tracing::instrument(
    target = "flow_like::observability",
    name = "artifact.ensure",
    skip_all,
    fields(app_id, board_id, etag_bound = expected_etag.is_some())
)]
pub async fn ensure_compiled_artifact(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
    expected_etag: Option<&str>,
    wasm_packages: Option<&HashMap<String, WasmPackageRef>>,
) -> flow_like_types::Result<EnsuredArtifact> {
    let registry = state.artifact_registry(wasm_packages).await?;
    let fingerprint = registry.fingerprint();
    let fingerprint_hex = blake3::Hash::from_bytes(fingerprint).to_hex();
    let storage_root = Path::from("apps").join(app_id.to_string());
    let meta_store = state.meta_bucket.as_generic();

    let lookup = async {
        // Lookup 1 (drafts only): the current source etag keys the artifact.
        // Draft artifacts are content-addressed: a queued Page run may still
        // need an older ETag after a later save, so lifecycle cleanup owns
        // reclamation instead of purge-on-write.
        let (artifact_path, cache_key, selected_draft_etag) = match (version, expected_etag) {
            (Some(v), _) => (
                compiled::artifact_path(&storage_root, board_id, v),
                format!(
                    "{app_id}:{board_id}:{}_{}_{}:{}",
                    v.0,
                    v.1,
                    v.2,
                    &fingerprint_hex.as_str()[..16]
                ),
                None,
            ),
            (None, Some(e_tag)) => {
                if e_tag.trim().is_empty() {
                    return Err(anyhow!(
                        "Latest board {board_id} has an empty expected etag"
                    ));
                }
                (
                    compiled::draft_artifact_path(app_id, board_id, e_tag, &fingerprint),
                    format!(
                        "{app_id}:{board_id}:{e_tag}:{}",
                        &fingerprint_hex.as_str()[..16]
                    ),
                    Some(e_tag.to_string()),
                )
            }
            (None, None) => {
                let proto_path = Board::proto_path(&storage_root, board_id, None);
                let head = meta_store
                    .head(&proto_path)
                    .instrument(tracing::info_span!(
                        target: "flow_like::observability",
                        "artifact.source.head"
                    ))
                    .await
                    .map_err(|e| anyhow!("board {board_id} not found for artifact check: {e}"))?;
                let e_tag = head
                    .e_tag
                    .ok_or_else(|| anyhow!("meta store returned no etag for board {board_id}"))?;
                (
                    compiled::draft_artifact_path(app_id, board_id, &e_tag, &fingerprint),
                    format!(
                        "{app_id}:{board_id}:{e_tag}:{}",
                        &fingerprint_hex.as_str()[..16]
                    ),
                    Some(e_tag),
                )
            }
        };
        let present = match state.compiled_artifact_cache.get(&cache_key) {
            Some(trust) if artifact_trusted(trust) => true,
            _ => match stored_artifact_written(meta_store.as_ref(), &artifact_path, &fingerprint)
                .await
            {
                Some(written) => {
                    let trust = artifact_trust(version, written);
                    let trust = if artifact_trusted(trust) {
                        Some(trust)
                    } else {
                        refresh_draft_artifact(meta_store.as_ref(), &artifact_path)
                            .await
                            .map(Some)
                    };
                    if let Some(trust) = trust {
                        state
                            .compiled_artifact_cache
                            .insert(cache_key.clone(), trust);
                    }
                    true
                }
                None => false,
            },
        };
        flow_like_types::Ok((artifact_path, cache_key, selected_draft_etag, present))
    };
    let reserved_version_free = async {
        match expected_etag {
            Some(_) => {
                ensure_reserved_version_free(meta_store.as_ref(), &storage_root, board_id).await
            }
            None => Ok(()),
        }
    };
    let (reserved_version_free, lookup) =
        flow_like_types::tokio::join!(reserved_version_free, lookup);
    reserved_version_free?;
    let (artifact_path, cache_key, selected_draft_etag, present) = lookup?;

    let ensured = EnsuredArtifact {
        path: artifact_path.clone(),
        source_etag: selected_draft_etag.clone(),
        registry_fingerprint: fingerprint,
    };
    if present {
        return Ok(state
            .compiled_widget_artifacts
            .enrich(&meta_store, app_id, board_id, version, ensured)
            .await);
    }

    // Miss. A page open dispatches its load and its widgets' runs together, each to its own
    // instance; they build the artifact once. Requests on this instance queue behind the local
    // lock, other instances behind the lease beside the artifact.
    let target = ArtifactTarget {
        app_id,
        board_id,
        version,
        expected_etag,
        selected_draft_etag: selected_draft_etag.as_deref(),
        storage_root: &storage_root,
        registry: &registry,
        fingerprint,
        artifact_path: &artifact_path,
        cache_key: &cache_key,
    };
    let build_lock = state.artifact_build_lock(&cache_key);
    let _local_turn = build_lock.lock().await;
    let built_while_queued = state
        .compiled_artifact_cache
        .get(&cache_key)
        .is_some_and(artifact_trusted);
    if !built_while_queued {
        match take_build_turn(meta_store.as_ref(), &artifact_path).await {
            BuildTurn::Ours(lease) => {
                let lease = HeldBuildLease {
                    store: meta_store.clone(),
                    path: lease,
                };
                let built = build_artifact(state, &target).await;
                lease.release().await;
                built?;
            }
            BuildTurn::Theirs(lease) => {
                match wait_for_build(meta_store.as_ref(), &artifact_path, &lease, &fingerprint)
                    .await
                {
                    Some(written) => state
                        .compiled_artifact_cache
                        .insert(cache_key.clone(), artifact_trust(version, written)),
                    None => build_artifact(state, &target).await?,
                }
            }
        }
    }
    Ok(state
        .compiled_widget_artifacts
        .enrich(&meta_store, app_id, board_id, version, ensured)
        .await)
}

/// One compiled artifact a dispatch needs, as `ensure_compiled_artifact` resolved it.
#[derive(Clone, Copy)]
struct ArtifactTarget<'a> {
    app_id: &'a str,
    board_id: &'a str,
    version: Option<(u32, u32, u32)>,
    expected_etag: Option<&'a str>,
    selected_draft_etag: Option<&'a str>,
    storage_root: &'a Path,
    registry: &'a Arc<FlowNodeRegistryInner>,
    fingerprint: [u8; 32],
    artifact_path: &'a Path,
    cache_key: &'a str,
}

/// A build lease older than this belongs to a build that died; the next run takes it over.
const BUILD_LEASE_TTL: Duration = Duration::from_secs(60);
/// How long a run waits for another instance's build before building the artifact itself.
const BUILD_WAIT_LIMIT: Duration = Duration::from_secs(45);
const BUILD_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Who builds a missing artifact.
enum BuildTurn {
    /// This request builds and afterwards releases the lease it holds, if any. `None` means the
    /// store could not record one; the build then runs unannounced.
    Ours(Option<Path>),
    /// Another instance holds a live lease for the same artifact.
    Theirs(Path),
}

fn build_lease_path(artifact_path: &Path) -> Path {
    Path::from(format!("{artifact_path}.building"))
}

/// A lease written in the future (clock skew) counts as live.
fn lease_is_live(written: SystemTime, now: SystemTime) -> bool {
    !matches!(now.duration_since(written), Ok(age) if age >= BUILD_LEASE_TTL)
}

/// Claim the build of `artifact_path` with a create-only lease object. Storage arbitrates, so
/// exactly one of the instances racing for a fresh artifact wins.
async fn take_build_turn(meta_store: &dyn ObjectStore, artifact_path: &Path) -> BuildTurn {
    let lease = build_lease_path(artifact_path);
    let claim = async {
        for _ in 0..2 {
            match meta_store
                .put_opts(
                    &lease,
                    PutPayload::from_static(b"building"),
                    PutOptions {
                        mode: PutMode::Create,
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(_) => return BuildTurn::Ours(Some(lease.clone())),
                Err(StoreError::AlreadyExists { .. } | StoreError::Precondition { .. }) => {}
                Err(error) => {
                    tracing::debug!(path = %lease, error = %error, "Artifact build lease unavailable");
                    return BuildTurn::Ours(None);
                }
            }
            match meta_store.head(&lease).await {
                Ok(meta) if lease_is_live(meta.last_modified.into(), SystemTime::now()) => {
                    return BuildTurn::Theirs(lease.clone());
                }
                // The previous holder died mid-build. Two takers may both win this overwrite;
                // they then write the same content-addressed bytes.
                Ok(_) => {
                    return match meta_store
                        .put(&lease, PutPayload::from_static(b"building"))
                        .await
                    {
                        Ok(_) => BuildTurn::Ours(Some(lease.clone())),
                        Err(_) => BuildTurn::Ours(None),
                    };
                }
                // Released between the two calls: claim it again.
                Err(StoreError::NotFound { .. }) => continue,
                Err(_) => return BuildTurn::Ours(None),
            }
        }
        BuildTurn::Ours(None)
    };
    claim
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "artifact.build_turn"
        ))
        .await
}

/// Wait for another instance to persist `artifact_path`. `None` when its lease disappears or
/// expires without an artifact, or the wait limit passes; the caller then builds it itself.
async fn wait_for_build(
    meta_store: &dyn ObjectStore,
    artifact_path: &Path,
    lease: &Path,
    fingerprint: &[u8; 32],
) -> Option<SystemTime> {
    let wait = async {
        let deadline = flow_like_types::tokio::time::Instant::now() + BUILD_WAIT_LIMIT;
        loop {
            flow_like_types::tokio::time::sleep(BUILD_POLL_INTERVAL).await;
            if let Some(written) =
                stored_artifact_written(meta_store, artifact_path, fingerprint).await
            {
                return Some(written);
            }
            let lease_live = match meta_store.head(lease).await {
                Ok(meta) => lease_is_live(meta.last_modified.into(), SystemTime::now()),
                Err(_) => false,
            };
            if !lease_live {
                return stored_artifact_written(meta_store, artifact_path, fingerprint).await;
            }
            if flow_like_types::tokio::time::Instant::now() >= deadline {
                return None;
            }
        }
    };
    wait.instrument(tracing::info_span!(
        target: "flow_like::observability",
        "artifact.build_wait"
    ))
    .await
}

/// The lease of a build in progress. Released when the build ends, and also when the request is
/// dropped mid-build, so other instances do not wait out its TTL for an artifact nobody builds.
struct HeldBuildLease {
    store: Arc<dyn ObjectStore>,
    path: Option<Path>,
}

impl HeldBuildLease {
    async fn release(mut self) {
        release_build_lease(self.store.as_ref(), self.path.take()).await;
    }
}

impl Drop for HeldBuildLease {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() {
            let store = self.store.clone();
            runtime.spawn(async move { release_build_lease(store.as_ref(), Some(path)).await });
        }
    }
}

async fn release_build_lease(meta_store: &dyn ObjectStore, lease: Option<Path>) {
    let Some(lease) = lease else {
        return;
    };
    if let Err(error) = meta_store.delete(&lease).await {
        tracing::debug!(path = %lease, error = %error, "Artifact build lease release failed");
    }
}

/// Load the board the artifact is compiled from, compile it and persist the artifact.
async fn build_artifact(
    state: &AppState,
    target: &ArtifactTarget<'_>,
) -> flow_like_types::Result<()> {
    let ArtifactTarget {
        app_id,
        board_id,
        version,
        expected_etag,
        selected_draft_etag,
        storage_root,
        registry,
        fingerprint,
        artifact_path,
        cache_key,
    } = *target;
    let meta_store = state.meta_bucket.as_generic();
    let board = if let Some(expected_etag) = expected_etag {
        let source_path = compiled::draft_source_path(app_id, board_id, expected_etag);
        let proto = flow_like::utils::compression::from_compressed(
            meta_store.clone(),
            source_path.clone(),
        )
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "artifact.snapshot.get"
        ))
        .await
        .map_err(|error| {
            anyhow!(
                "exact source snapshot for Latest board {board_id} is unavailable at {source_path}: {error}"
            )
        })?;
        let app_state = state.master_state(state).await?;
        let board = Board::from_loaded_proto(proto, storage_root.clone(), app_state).await?;
        if board.id != board_id {
            return Err(anyhow!(
                "exact source snapshot for Latest board {board_id} contains board {}",
                board.id
            ));
        }
        std::sync::Arc::new(board)
    } else {
        let cached = state
            .master_board_shared(app_id, board_id, state, version)
            .await?;
        require_selected_draft_etag(board_id, selected_draft_etag, &cached.e_tag)?;
        cached.board.clone()
    };

    // The prepared board was hydrated against the built-in registry, where
    // `sync_board_node_schemas` skips nodes it cannot find — so a WASM node
    // still carries whatever metadata, pin schemas and permissions were saved,
    // not the package's current ones. Re-hydrate against the overlay registry
    // so the artifact is what the executor's own registry would have produced.
    let board = if board.has_wasm_nodes() && !Arc::ptr_eq(registry, &state.registry) {
        rehydrate_with_registry(
            state,
            meta_store.clone(),
            app_id,
            board_id,
            storage_root,
            version,
            expected_etag,
            selected_draft_etag,
            registry.clone(),
        )
        .await?
    } else {
        board
    };

    ensure_server_runnable(&board, registry)?;
    let compiled_board = compiled::compile::compile_board_with_catalog(&board, registry)?;
    let bytes = compiled::encode_artifact(&compiled_board, &fingerprint)?;
    state
        .compiled_widget_artifacts
        .seed_base(artifact_path, fingerprint, &compiled_board, &bytes);
    let written = SystemTime::now();
    compiled::persist_artifact(&meta_store, artifact_path, None, bytes)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "artifact.persist"
        ))
        .await?;
    state
        .compiled_artifact_cache
        .insert(cache_key.to_string(), artifact_trust(version, written));
    tracing::debug!(
        app_id = %app_id,
        board_id = %board_id,
        path = %artifact_path,
        "Compiled artifact ensured before dispatch"
    );
    Ok(())
}

/// Load the board's source again and hydrate it against `registry`, for boards
/// whose prepared form was hydrated against a registry without their WASM
/// nodes. The hydration state borrows the master state's stores and clients
/// but gets its own registry handle, so the shared master state is never
/// mutated. A floating source that moved since the HEAD that keyed the
/// artifact is refused rather than compiled under the older ETag's path.
#[allow(clippy::too_many_arguments)]
async fn rehydrate_with_registry(
    state: &AppState,
    meta_store: Arc<dyn ObjectStore>,
    app_id: &str,
    board_id: &str,
    storage_root: &Path,
    version: Option<(u32, u32, u32)>,
    expected_etag: Option<&str>,
    selected_etag: Option<&str>,
    registry: Arc<FlowNodeRegistryInner>,
) -> flow_like_types::Result<Arc<Board>> {
    let proto = if let Some(expected_etag) = expected_etag {
        let source_path = compiled::draft_source_path(app_id, board_id, expected_etag);
        flow_like::utils::compression::from_compressed(meta_store, source_path.clone())
            .await
            .map_err(|error| {
                anyhow!(
                    "exact source snapshot for Latest board {board_id} is unavailable at {source_path}: {error}"
                )
            })?
    } else {
        let (proto, meta) =
            Board::load_proto_with_meta(meta_store, storage_root, board_id, version).await?;
        let loaded_etag = meta
            .e_tag
            .ok_or_else(|| anyhow!("meta store returned no etag for board {board_id}"))?;
        require_selected_draft_etag(board_id, selected_etag, &loaded_etag)?;
        proto
    };

    let master = state.master_state(state).await?;
    let mut hydration_state = master.for_execution_run();
    hydration_state.node_registry = Arc::new(RwLock::new(FlowNodeRegistry {
        node_registry: registry,
        parent: None,
    }));
    let board = if expected_etag.is_some() {
        Board::from_loaded_proto(proto, storage_root.clone(), Arc::new(hydration_state)).await?
    } else {
        Board::from_loaded_proto_for_version(
            proto,
            storage_root.clone(),
            Arc::new(hydration_state),
            version,
        )
        .await?
    };
    if board.id != board_id {
        return Err(anyhow!(
            "source for board {board_id} contains board {}",
            board.id
        ));
    }
    Ok(Arc::new(board))
}

#[cfg(test)]
mod tests {
    use super::{
        BUILD_LEASE_TTL, BuildTurn, DRAFT_MIN_RETENTION, LocalOnlyBoard, artifact_trust,
        artifact_trusted, ensure_server_runnable, lease_is_live, refresh_draft_artifact,
        release_build_lease, require_selected_draft_etag, stored_artifact_written, take_build_turn,
        wait_for_build,
    };
    use crate::execution::wasm_node_stubs::WasmNodeStub;
    use flow_like::flow::board::{Board, Layer, LayerType};
    use flow_like::flow::compiled;
    use flow_like::flow::node::Node;
    use flow_like::state::FlowNodeRegistryInner;
    use flow_like_storage::Path;
    use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    #[test]
    fn draft_artifacts_are_trusted_only_inside_the_lifecycle_floor() {
        let now = SystemTime::now();
        assert!(artifact_trusted(artifact_trust(None, now)));
        assert!(!artifact_trusted(artifact_trust(
            None,
            now - DRAFT_MIN_RETENTION
        )));
        assert!(
            !artifact_trusted(artifact_trust(
                None,
                now - Duration::from_secs(23 * 60 * 60)
            )),
            "the last hour of the floor belongs to the dispatch that follows the check"
        );
        assert!(
            artifact_trusted(artifact_trust(
                Some((1, 2, 3)),
                now - 10 * DRAFT_MIN_RETENTION
            )),
            "version artifacts sit outside tmp/ and never expire"
        );
    }

    fn artifact_bytes() -> Vec<u8> {
        let compiled_board = compiled::compile::compile_board(&board(Vec::new(), Vec::new()))
            .expect("an empty board compiles");
        compiled::encode_artifact(&compiled_board, &[7; 32]).unwrap()
    }

    #[flow_like_types::tokio::test]
    async fn one_request_builds_a_missing_artifact_and_the_others_wait_for_it() {
        let store = InMemory::new();
        let path = compiled::draft_artifact_path("app-1", "board-1", "etag-1", &[7; 32]);
        let BuildTurn::Ours(Some(lease)) = take_build_turn(&store, &path).await else {
            panic!("the first request takes the build");
        };
        let BuildTurn::Theirs(held) = take_build_turn(&store, &path).await else {
            panic!("a concurrent request waits for the build");
        };
        assert_eq!(held, lease);

        store.put(&path, artifact_bytes().into()).await.unwrap();
        assert!(
            wait_for_build(&store, &path, &held, &[7; 32])
                .await
                .is_some()
        );
        release_build_lease(&store, Some(lease)).await;
        assert!(matches!(
            take_build_turn(&store, &path).await,
            BuildTurn::Ours(Some(_))
        ));
    }

    #[flow_like_types::tokio::test]
    async fn a_lease_released_without_an_artifact_ends_the_wait() {
        let store = InMemory::new();
        let path = compiled::draft_artifact_path("app-1", "board-1", "etag-1", &[7; 32]);
        let BuildTurn::Ours(Some(lease)) = take_build_turn(&store, &path).await else {
            panic!("the first request takes the build");
        };
        release_build_lease(&store, Some(lease.clone())).await;
        assert_eq!(wait_for_build(&store, &path, &lease, &[7; 32]).await, None);
    }

    #[test]
    fn a_lease_older_than_its_ttl_is_dead_and_one_from_the_future_is_live() {
        let now = SystemTime::now();
        assert!(lease_is_live(now, now));
        assert!(lease_is_live(now + Duration::from_secs(5), now));
        assert!(lease_is_live(
            now - BUILD_LEASE_TTL + Duration::from_secs(1),
            now
        ));
        assert!(!lease_is_live(now - BUILD_LEASE_TTL, now));
    }

    #[flow_like_types::tokio::test]
    async fn refresh_rewrites_identical_bytes_and_restarts_trust() {
        let store = InMemory::new();
        let path = compiled::draft_artifact_path("app-1", "board-1", "etag-1", &[7; 32]);
        let bytes = artifact_bytes();
        store.put(&path, bytes.clone().into()).await.unwrap();

        let trusted_until = refresh_draft_artifact(&store, &path)
            .await
            .expect("an existing artifact can be refreshed");
        assert!(artifact_trusted(Some(trusted_until)));
        let stored = store.get(&path).await.unwrap().bytes().await.unwrap();
        assert_eq!(stored.as_ref(), bytes.as_slice());

        let missing = compiled::draft_artifact_path("app-1", "board-1", "etag-2", &[7; 32]);
        assert!(refresh_draft_artifact(&store, &missing).await.is_none());
    }

    #[flow_like_types::tokio::test]
    async fn envelope_read_reports_the_write_time_of_a_matching_artifact() {
        let store = InMemory::new();
        let path = compiled::draft_artifact_path("app-1", "board-1", "etag-1", &[7; 32]);
        let before = SystemTime::now() - Duration::from_secs(1);
        store.put(&path, artifact_bytes().into()).await.unwrap();

        let written = stored_artifact_written(&store, &path, &[7; 32])
            .await
            .expect("the artifact exists for this fingerprint");
        assert!(written >= before);
        assert!(
            stored_artifact_written(&store, &path, &[8; 32])
                .await
                .is_none()
        );
        let missing = compiled::draft_artifact_path("app-1", "board-1", "etag-2", &[7; 32]);
        assert!(
            stored_artifact_written(&store, &missing, &[7; 32])
                .await
                .is_none()
        );
    }

    fn node(name: &str, friendly_name: &str, only_offline: bool) -> Node {
        let mut node = Node::new(name, friendly_name, "", "Test");
        node.set_only_offline(only_offline);
        node
    }

    fn registry(nodes: &[Node]) -> FlowNodeRegistryInner {
        let mut registry = FlowNodeRegistryInner::new(nodes.len());
        for node in nodes {
            registry.insert(node.clone(), Arc::new(WasmNodeStub::new(node.clone())));
        }
        registry
    }

    fn board(root: Vec<Node>, layered: Vec<Node>) -> Board {
        let mut board = Board::new_detached(Some("board-1".into()), Path::from("apps/app-1"));
        board.name = "Scraper".into();
        board.nodes = root.into_iter().map(|n| (n.id.clone(), n)).collect();
        let mut layer = Layer::new("layer-1".into(), "Login".into(), LayerType::Function);
        layer.nodes = layered.into_iter().map(|n| (n.id.clone(), n)).collect();
        board.layers.insert(layer.id.clone(), layer);
        board
    }

    #[test]
    fn local_only_nodes_are_refused_by_the_catalog_flag_not_the_stored_one() {
        let registry = registry(&[
            node("browser_click", "Click", true),
            node("selector_build", "Build Selector", true),
            node("log_info", "Log", false),
        ]);
        let board = board(
            vec![
                node("browser_click", "Click", false),
                node("log_info", "Log", true),
            ],
            vec![
                node("selector_build", "Build Selector", false),
                node("browser_click", "Click", false),
            ],
        );

        let error = ensure_server_runnable(&board, &registry)
            .expect_err("a board holding local-only nodes must not compile for the server");
        let refusal = error
            .downcast_ref::<LocalOnlyBoard>()
            .expect("the dispatcher maps only this type to a client error");
        assert_eq!(refusal.board_name, "Scraper");
        assert_eq!(refusal.nodes, vec!["Build Selector", "Click"]);
    }

    #[test]
    fn boards_without_local_only_catalog_nodes_compile() {
        let registry = registry(&[node("log_info", "Log", false)]);
        let board = board(
            vec![node("log_info", "Log", true), node("unknown", "Gone", true)],
            Vec::new(),
        );
        ensure_server_runnable(&board, &registry)
            .expect("only catalog restrictions refuse; unknown nodes fail later as before");
    }

    #[test]
    fn a_draft_load_may_not_be_written_under_an_older_head_etag() {
        require_selected_draft_etag("board-1", Some("etag-a"), "etag-a")
            .expect("an unchanged conditional selection is valid");
        assert!(
            require_selected_draft_etag("board-1", Some("etag-a"), "etag-b").is_err(),
            "a concurrent save must stop before bytes for B can be written at A's path"
        );
        require_selected_draft_etag("board-1", None, "etag-b")
            .expect("pinned versions do not use a draft ETag selector");
    }
}

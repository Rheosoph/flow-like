//! Incremental board fetch. See `flow_like::flow::board::sync` for the protocol.
//!
//! The expensive work — hydrating the board and splitting it into tokenised parts — happens once
//! per storage revision and is cached on the ETag; a client whose manifest matches costs one
//! `If-None-Match` GET plus a map compare, and receives an empty diff.
//!
//! Every board writer seeds this cache with the revision it just persisted
//! ([`seed_board_revision`]), so the read that follows every edit — from the writer's own merged
//! apply response, a peer, or an old client's `/sync` — never rebuilds a snapshot. Snapshots are
//! built incrementally from the previous one on this instance and their segments are indexed by
//! token, which is what turns a changed segment into a node-level patch for any client that still
//! holds a recent revision.

use std::sync::Arc;

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        board::secrets::filter_board_secrets,
        template::get_template::VersionQuery,
        wasm_catalog::{app_wasm_nodes_cached, hydrate_board_wasm_metadata},
    },
    state::{AppState, CachedBoard, State, segment_base_key},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State as AxumState},
};
use flow_like::flow::board::sync::{
    BoardSyncRequest, BoardSyncResponse, BoardSyncSnapshot, SyncSegment,
};
use flow_like_types::anyhow;

pub(crate) fn parse_version(version: Option<&str>) -> Result<Option<(u32, u32, u32)>, ApiError> {
    let Some(ver_str) = version else {
        return Ok(None);
    };
    let parts = ver_str
        .split('_')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<u32>, _>>()?;
    match parts.as_slice() {
        [maj, min, pat] => Ok(Some((*maj, *min, *pat))),
        _ => Err(ApiError::bad_request(
            "version must be in MAJOR_MINOR_PATCH format",
        )),
    }
}

/// The tokenised parts of one cached board revision — **the only place a snapshot is built**.
///
/// Keyed on the storage ETag plus the app's WASM catalog fingerprint: the WASM catalog is folded
/// into node metadata before tokenising, so a package change must produce a different snapshot
/// even though the board bytes did not move. The board is cloned, WASM-hydrated and
/// **secret-filtered** before anything is tokenised, so no caller can obtain an unfiltered
/// snapshot. The previous head snapshot of the board (if this instance has one) seeds token reuse,
/// and the result becomes the new head and feeds the segment base index.
#[tracing::instrument(
    name = "board_sync_snapshot",
    skip(state, cached),
    level = "debug",
    fields(app_id, board_id, e_tag = %cached.e_tag)
)]
pub(crate) async fn snapshot_for_cached(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: &CachedBoard,
    version: Option<(u32, u32, u32)>,
) -> Result<Arc<BoardSyncSnapshot>, ApiError> {
    let wasm = app_wasm_nodes_cached(state, app_id).await?;
    let key = format!(
        "{app_id}\u{1f}{board_id}\u{1f}{:?}\u{1f}{}\u{1f}{}",
        version, cached.e_tag, wasm.fingerprint
    );
    if let Some(snapshot) = state.board_sync_cache.get(&key) {
        return Ok(snapshot);
    }

    let builtin_nodes = state.registry.as_ref().get_nodes_shared();
    let mut board = (*cached.board).clone();
    hydrate_board_wasm_metadata(&mut board, &wasm.nodes, &builtin_nodes);
    filter_board_secrets(&mut board);

    // Apps without packages are the common case; there the shared catalog is used as-is rather
    // than copied to append nothing to it.
    let catalog = if wasm.nodes.is_empty() {
        builtin_nodes
    } else {
        let mut catalog = (*builtin_nodes).clone();
        catalog.extend(wasm.nodes.iter().cloned());
        Arc::new(catalog)
    };
    let head_key = State::board_snapshot_head_key(app_id, board_id, version);
    let previous = state.board_snapshot_heads.get(&head_key);
    let snapshot = Arc::new(
        BoardSyncSnapshot::from_board_incremental(&board, &catalog, previous.as_deref())
            .map_err(|error| ApiError::internal_error(anyhow!("board sync snapshot: {error}")))?,
    );
    for (_, segment) in snapshot.segments() {
        state.board_segment_bases.insert(
            segment_base_key(app_id, board_id, &segment.hash),
            segment.clone(),
        );
    }
    state
        .board_snapshot_heads
        .insert(head_key, snapshot.clone());
    // Only a validated revision may be shared: an object without an ETag would pin an
    // unverifiable snapshot to every later request.
    if !cached.e_tag.is_empty() {
        state.board_sync_cache.insert(key, snapshot.clone());
    }
    Ok(snapshot)
}

/// [`snapshot_for_cached`] for the board storage currently holds, validated by one
/// `If-None-Match` round trip.
pub(crate) async fn board_sync_snapshot(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Arc<BoardSyncSnapshot>, ApiError> {
    let cached = state
        .master_board_shared(app_id, board_id, state, version)
        .await?;
    snapshot_for_cached(state, app_id, board_id, &cached, version).await
}

/// Resolves a client's segment token against this instance's base index.
pub(crate) fn segment_base_resolver<'a>(
    state: &'a AppState,
    app_id: &'a str,
    board_id: &'a str,
) -> impl Fn(&str) -> Option<Arc<SyncSegment>> + 'a {
    move |token: &str| {
        state
            .board_segment_bases
            .get(&segment_base_key(app_id, board_id, token))
    }
}

/// What every board writer runs after `save`: pin the persisted board to its ETag **and** build
/// the revision's snapshot from that same board, so the write is the read. Returns the cached
/// entry (for a merged apply tail) when the object could be validated. Snapshot failures are
/// logged, never surfaced — the write is already committed and readers self-heal.
pub(crate) async fn seed_board_revision(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    board: flow_like::flow::board::Board,
    put: &flow_like::flow_like_storage::object_store::PutResult,
) -> Option<Arc<CachedBoard>> {
    let cached = state.seed_board_cache(app_id, board_id, board, put)?;
    if let Err(error) = snapshot_for_cached(state, app_id, board_id, &cached, None).await {
        tracing::warn!(
            app_id,
            board_id,
            "board sync snapshot after write failed; readers will rebuild: {error:?}"
        );
    }
    Some(cached)
}

/// The sync tail of a merged apply: the diff between what the client sent and the revision the
/// write produced. `cached` is the entry [`seed_board_revision`] returned; without one the
/// validated read path is used. Never fails — a committed write must not turn into an error
/// because its tail could not be built; the client then falls back to a plain sync.
pub(crate) async fn board_sync_tail(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    cached: Option<Arc<CachedBoard>>,
    request: &BoardSyncRequest,
) -> Option<BoardSyncResponse> {
    let snapshot = match cached {
        Some(cached) => snapshot_for_cached(state, app_id, board_id, &cached, None).await,
        None => board_sync_snapshot(state, app_id, board_id, None).await,
    };
    match snapshot {
        Ok(snapshot) => {
            let resolver = segment_base_resolver(state, app_id, board_id);
            Some(snapshot.diff(request, &resolver))
        }
        Err(error) => {
            tracing::warn!(
                app_id,
                board_id,
                "merged apply: sync tail unavailable, client will sync separately: {error:?}"
            );
            None
        }
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/sync",
    tag = "boards",
    description = "Fetch only the parts of a board that changed since the manifest the client last received. Send an empty body to receive the whole board.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format (e.g., 1_0_3)")
    ),
    request_body(content = Object, description = "Part tokens the client currently holds"),
    responses(
        (status = 200, description = "Changed board parts plus the current manifest", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/sync",
    skip(state, user, params, request)
)]
pub async fn sync_board(
    AxumState(state): AxumState<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
    Json(request): Json<BoardSyncRequest>,
) -> Result<Json<BoardSyncResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let version = parse_version(params.version.as_deref())?;
    let snapshot = board_sync_snapshot(&state, &app_id, &board_id, version).await?;
    let resolver = segment_base_resolver(&state, &app_id, &board_id);
    Ok(Json(snapshot.diff(&request, &resolver)))
}

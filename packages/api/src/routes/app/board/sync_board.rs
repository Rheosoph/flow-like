//! Incremental board fetch. See `flow_like::flow::board::sync` for the protocol.
//!
//! The expensive work — hydrating the board and splitting it into tokenised parts — happens once
//! per storage revision and is cached on the ETag; a client whose manifest matches costs one
//! `If-None-Match` GET plus a map compare, and receives an empty diff.

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
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::sync::{BoardSyncRequest, BoardSyncResponse, BoardSyncSnapshot};
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

/// The tokenised parts of one board revision, shared by every client polling that revision.
///
/// Keyed on the storage ETag plus the app's WASM catalog fingerprint: the WASM catalog is folded
/// into node metadata before tokenising, so a package change must produce a different snapshot
/// even though the board bytes did not move.
pub(crate) async fn board_sync_snapshot(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Arc<BoardSyncSnapshot>, ApiError> {
    let cached = state
        .master_board_shared(app_id, board_id, state, version)
        .await?;
    let wasm = app_wasm_nodes_cached(state, app_id).await?;
    let key = format!(
        "{app_id}\u{1f}{board_id}\u{1f}{:?}\u{1f}{}\u{1f}{}",
        version, cached.e_tag, wasm.fingerprint
    );
    if let Some(snapshot) = state.board_sync_cache.get(&key) {
        return Ok(snapshot);
    }

    let builtin_nodes = state.registry.as_ref().get_nodes();
    let mut board = (*cached.board).clone();
    hydrate_board_wasm_metadata(&mut board, &wasm.nodes, &builtin_nodes);
    filter_board_secrets(&mut board);

    let mut catalog = builtin_nodes;
    catalog.extend(wasm.nodes.iter().cloned());
    let snapshot = Arc::new(
        BoardSyncSnapshot::from_board(&board, &catalog)
            .map_err(|error| ApiError::internal_error(anyhow!("board sync snapshot: {error}")))?,
    );
    // Only a validated revision may be shared: an object without an ETag would pin an
    // unverifiable snapshot to every later request.
    if !cached.e_tag.is_empty() {
        state.board_sync_cache.insert(key, snapshot.clone());
    }
    Ok(snapshot)
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
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
    Json(request): Json<BoardSyncRequest>,
) -> Result<Json<BoardSyncResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let version = parse_version(params.version.as_deref())?;
    let snapshot = board_sync_snapshot(&state, &app_id, &board_id, version).await?;
    Ok(Json(snapshot.diff(&request)))
}

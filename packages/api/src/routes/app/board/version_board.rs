use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::VersionType;
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Clone, Deserialize, IntoParams)]
pub struct CreateVersionQuery {
    #[param(value_type = Option<String>)]
    pub version_type: Option<VersionType>,
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        CreateVersionQuery
    ),
    responses(
        (status = 200, description = "New version created as (major, minor, patch) tuple", body = (u32, u32, u32)),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn version_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<CreateVersionQuery>,
) -> Result<Json<(u32, u32, u32)>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let _mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    let (version, published) = board
        .create_version_returning_published(params.version_type.unwrap_or(VersionType::Patch), None)
        .await?;
    spawn_compiled_artifact_warmup(&board, published);

    audit_branch!(
        state,
        user,
        app_id,
        "board.version",
        "board",
        board_id,
        format!(
            "Board versioned to {}.{}.{}",
            version.0, version.1, version.2
        )
    );
    Ok(Json(version))
}

/// Eagerly compile the just-published immutable snapshot so the executor's
/// first run of this version starts from the artifact instead of the proto.
///
/// Boards containing WASM nodes are skipped: their `on_update` runs against a
/// per-request bundle registry only the executor has, so its lazy compile is
/// authoritative there. Failures only cost warmth — the executor self-heals.
fn spawn_compiled_artifact_warmup(
    board: &flow_like::flow::board::Board,
    published: (u32, u32, u32),
) {
    let Some(app_state) = board.app_state.clone() else {
        return;
    };
    let has_wasm = board.nodes.values().any(|n| n.wasm.is_some())
        || board
            .layers
            .values()
            .any(|l| l.nodes.values().any(|n| n.wasm.is_some()));
    if has_wasm {
        return;
    }

    let mut snapshot = board.clone();
    snapshot.version = published;
    let board_dir = board.board_dir.clone();
    let board_id = board.id.clone();

    flow_like_types::tokio::spawn(async move {
        let registry = app_state.node_registry.read().await.node_registry.clone();
        let fingerprint = registry.fingerprint();
        let compiled = match flow_like::flow::compiled::compile::compile_board_with_catalog(
            &snapshot,
            registry.as_ref(),
        ) {
            Ok(compiled) => compiled,
            Err(e) => {
                tracing::warn!(board_id = %board_id, error = %e, "Compiled-board warm-up: compile failed");
                return;
            }
        };
        let bytes = match flow_like::flow::compiled::encode_artifact(&compiled, &fingerprint) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(board_id = %board_id, error = %e, "Compiled-board warm-up: encode failed");
                return;
            }
        };
        let store = {
            let guard = app_state.config.read().await;
            guard.stores.app_meta_store.clone()
        };
        let Some(store) = store else {
            return;
        };
        let path = flow_like::flow::compiled::artifact_path(&board_dir, &board_id, published);
        let payload = flow_like::flow_like_storage::object_store::PutPayload::from(bytes);
        match store.as_generic().put(&path, payload).await {
            Ok(_) => {
                tracing::debug!(path = %path, "Compiled-board warm-up artifact persisted")
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "Compiled-board warm-up: persist failed")
            }
        }
    });
}

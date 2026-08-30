use crate::{
    audit_branch, ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{persist_prerun_manifest, version_manifest_cache_key},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::{
    board::{Board, VersionType},
    compiled::{PrerunManifest, manifest_path},
};
use serde::Deserialize;
use std::sync::Arc;
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
    spawn_compiled_artifact_warmup(&state, &app_id, &board, published);

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

/// Eagerly persist the prerun manifest and compile the just-published
/// immutable snapshot so the first prerun and the executor's first run of this
/// version start from artifacts instead of the proto.
///
/// The manifest is registry-independent and always written. Boards containing
/// WASM nodes skip the compile: their `on_update` runs against a per-request
/// bundle registry only the executor has, so its lazy compile is authoritative
/// there. Failures only cost warmth — both readers self-heal.
fn spawn_compiled_artifact_warmup(
    state: &AppState,
    app_id: &str,
    board: &Board,
    published: (u32, u32, u32),
) {
    let Some(app_state) = board.app_state.clone() else {
        return;
    };
    let board_dir = board.board_dir.clone();
    let board_id = board.id.clone();

    let manifest = Arc::new(PrerunManifest::from_board(board));
    state.prerun_manifest_cache.insert(
        version_manifest_cache_key(app_id, &board_id, published),
        manifest.clone(),
    );

    let has_wasm = board.nodes.values().any(|n| n.wasm.is_some())
        || board
            .layers
            .values()
            .any(|l| l.nodes.values().any(|n| n.wasm.is_some()));
    let mut snapshot = board.clone();
    snapshot.version = published;

    flow_like_types::tokio::spawn(async move {
        let store = {
            let guard = app_state.config.read().await;
            guard.stores.app_meta_store.clone()
        };
        let Some(store) = store else {
            return;
        };
        let store = store.as_generic();
        persist_prerun_manifest(
            store.as_ref(),
            &manifest_path(&board_dir, &board_id, published),
            &manifest,
        )
        .await;
        if has_wasm {
            return;
        }

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
        let path = flow_like::flow::compiled::artifact_path(&board_dir, &board_id, published);
        let payload = flow_like::flow_like_storage::object_store::PutPayload::from(bytes);
        match store.put(&path, payload).await {
            Ok(_) => {
                tracing::debug!(path = %path, "Compiled-board warm-up artifact persisted")
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "Compiled-board warm-up: persist failed")
            }
        }
    });
}

use crate::{
    audit_branch, ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{
        page_payload_revision, persist_prerun_manifest, version_manifest_cache_key,
        version_page_manifest_cache_key,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::{
    board::{Board, VersionType},
    compiled::{PrerunManifest, manifest_path, version_page_manifest_path},
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
    let manifest = Arc::new(PrerunManifest::from_board(&board));
    let page_manifests = published_page_prerun_manifests(&board, published).await;
    let manifest_path = manifest_path(&board.board_dir, &board.id, published);
    let manifest_persisted = persist_prerun_manifest(
        state.meta_bucket.as_generic().as_ref(),
        &manifest_path,
        &manifest,
    )
    .await;
    // Board-only entry authority never shares a key with Page action maps, so
    // a later Page write cannot invalidate an already queued callback.
    if manifest_persisted {
        state.prerun_manifest_cache.insert(
            version_manifest_cache_key(&app_id, &board.id, published),
            manifest,
        );
    }
    for (page_id, page_revision, page_manifest) in page_manifests {
        let path = version_page_manifest_path(
            &board.board_dir,
            &board.id,
            published,
            &page_id,
            &page_revision,
        );
        if persist_prerun_manifest(
            state.meta_bucket.as_generic().as_ref(),
            &path,
            &page_manifest,
        )
        .await
        {
            state.prerun_manifest_cache.insert(
                version_page_manifest_cache_key(
                    &app_id,
                    &board.id,
                    published,
                    &page_id,
                    &page_revision,
                ),
                page_manifest,
            );
        }
    }
    spawn_compiled_artifact_warmup(&state, &app_id, &board, published);
    // Publish-triggered regression suites: one indexed projection lookup plus
    // the dispatches, all on a detached task — the publish PATCH is never
    // blocked or failed by them.
    crate::execution::regression::spawn_publish_triggered_suites(
        &state, &sub, &app_id, &board_id, published,
    );

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

/// Build one immutable artifact per Page. A damaged Page does not prevent the
/// board-only authority or unrelated Pages from being published.
async fn published_page_prerun_manifests(
    board: &Board,
    published: (u32, u32, u32),
) -> Vec<(String, String, Arc<PrerunManifest>)> {
    let mut manifests = Vec::with_capacity(board.page_ids.len());
    for page_id in &board.page_ids {
        let page = match board.load_versioned_page(page_id, published, None).await {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(
                    board_id = %board.id,
                    page_id,
                    error = %error,
                    "Published Page prerun artifact could not be built"
                );
                continue;
            }
        };
        let page_revision = match page_payload_revision(&page) {
            Ok(revision) => revision,
            Err(error) => {
                tracing::warn!(
                    board_id = %board.id,
                    page_id,
                    error = %error,
                    "Published Page revision could not be derived"
                );
                continue;
            }
        };
        let manifest = match PrerunManifest::from_board_and_page(board, &page) {
            Ok(manifest) => Arc::new(manifest),
            Err(error) => {
                tracing::warn!(
                    board_id = %board.id,
                    page_id,
                    error = %error,
                    "Published Page execution map is invalid"
                );
                continue;
            }
        };
        manifests.push((page.id, page_revision, manifest));
    }
    manifests
}

/// Eagerly compile the just-published immutable snapshot so the executor's
/// first run of this version finds its artifact already there.
///
/// Board and Page prerun manifests are persisted synchronously before this
/// best-effort task starts because a Lambda may freeze as soon as the response
/// is returned. The pre-dispatch assurance recompiles on a miss or a registry
/// mismatch, so this is warmth, never correctness. It compiles against the
/// registry the dispatch would use — the built-in catalog extended with the
/// app's WASM node definitions — so the fingerprint matches on first run.
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
    let state = state.clone();
    let app_id = app_id.to_string();

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

        let wasm_packages =
            crate::execution::wasm_resolve::resolve_wasm_packages(&state, &app_id).await;
        let registry = match state.artifact_registry(wasm_packages.as_ref()).await {
            Ok(registry) => registry,
            Err(e) => {
                tracing::warn!(board_id = %board_id, error = %e, "Compiled-board warm-up: registry unavailable");
                return;
            }
        };
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

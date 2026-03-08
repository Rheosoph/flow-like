//! WASM Package Registry API

pub mod check_id;
pub mod download;
pub mod hash_check;
mod index;
pub mod join_queue;
pub mod metadata;
pub mod prerun_check;
pub mod publish;
pub mod purchase;
pub mod recompile;
pub mod search;
pub mod server;
pub mod types;
pub mod upload;
pub mod users;

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{delete, get, patch, post, put},
};

pub use server::ServerRegistry;

/// Check that the caller has at least the given `WasmPackagePermission` on a
/// package. Uses the in-memory permission cache (120 s TTL) to avoid repeated
/// DB look-ups. Returns the resolved `WasmPackagePermission` on success.
///
/// Usage:
/// ```ignore
/// let perm = ensure_wasm_permission!(state, &user_id, &package_id, WasmPackagePermission::Maintainer);
/// ```
#[macro_export]
macro_rules! ensure_wasm_permission {
    ($state:expr, $user_id:expr, $package_id:expr, $required:expr) => {{
        use $crate::entity::wasm_package_user;
        use $crate::permission::wasm_package_permission::WasmPackagePermission;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let perm = if let Some(cached) = $state.check_wasm_permission($user_id, $package_id) {
            cached
        } else {
            let record = wasm_package_user::Entity::find()
                .filter(wasm_package_user::Column::PackageId.eq($package_id))
                .filter(wasm_package_user::Column::UserId.eq($user_id))
                .one(&$state.db)
                .await
                .map_err(|e| $crate::error::ApiError::internal(format!("DB error: {}", e)))?;

            let resolved = record
                .map(|u| WasmPackagePermission::from_bits_truncate(u.permission))
                .unwrap_or(WasmPackagePermission::empty());

            $state.put_wasm_permission($user_id, $package_id, resolved);
            resolved
        };

        if !perm.has_permission($required) {
            $state.invalidate_wasm_permission($user_id, $package_id);
            return Err($crate::error::ApiError::FORBIDDEN);
        }
        perm
    }};
}

/// Check whether the caller has *any* permission record on a package (i.e. is
/// a package user at all). Returns `Option<WasmPackagePermission>`.
/// Does **not** fail on missing permission — the caller decides what to do.
#[macro_export]
macro_rules! check_wasm_access {
    ($state:expr, $user_id:expr, $package_id:expr) => {{
        use $crate::entity::wasm_package_user;
        use $crate::permission::wasm_package_permission::WasmPackagePermission;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        if let Some(cached) = $state.check_wasm_permission($user_id, $package_id) {
            if cached.is_empty() {
                None
            } else {
                Some(cached)
            }
        } else {
            let record = wasm_package_user::Entity::find()
                .filter(wasm_package_user::Column::PackageId.eq($package_id))
                .filter(wasm_package_user::Column::UserId.eq($user_id))
                .one(&$state.db)
                .await
                .map_err(|e| $crate::error::ApiError::internal(format!("DB error: {}", e)))?;

            let resolved = record
                .map(|u| WasmPackagePermission::from_bits_truncate(u.permission))
                .unwrap_or(WasmPackagePermission::empty());

            $state.put_wasm_permission($user_id, $package_id, resolved);
            if resolved.is_empty() { None } else { Some(resolved) }
        }
    }};
}

async fn compilation_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Json<flow_like_types::dispatch::CompilationResult>,
) -> Result<
    Json<crate::compilation::callback::CallbackResponse>,
    (StatusCode, Json<crate::compilation::callback::CallbackResponse>),
> {
    let db = std::sync::Arc::new(state.db.clone());
    crate::compilation::callback::handle_compilation_callback(
        State(db),
        headers,
        body,
    )
    .await
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/search", get(search::search))
        .route("/download", post(download::download))
        .route("/publish", post(publish::publish))
        .route("/upload-url", post(upload::get_upload_url))
        .route("/check-id", post(check_id::check_id))
        .route("/recompile", post(recompile::recompile))
        .route("/hash-check", post(hash_check::hash_check))
        .route("/prerun-check", post(prerun_check::prerun_check))
        .route(
            "/compilation-callback",
            post(compilation_callback),
        )
        .route("/package/{id}", get(index::get_package))
        .route("/package/{id}/versions", get(index::get_versions))
        .route(
            "/package/{package_id}/readme",
            get(metadata::get_readme).put(metadata::update_readme),
        )
        .route(
            "/package/{package_id}/meta",
            get(metadata::get_meta).put(metadata::upsert_meta),
        )
        .route(
            "/package/{package_id}/meta/media",
            put(metadata::push_package_media),
        )
        .route(
            "/package/{package_id}/meta/media/{media_id}",
            delete(metadata::remove_package_media),
        )
        .route(
            "/package/{package_id}/request-publication",
            post(metadata::request_publication),
        )
        .route("/package/{package_id}/users", get(users::list_users))
        .route(
            "/package/{package_id}/users/invite",
            post(users::invite_user),
        )
        .route(
            "/package/{package_id}/users/{user_id}",
            patch(users::update_user_permission).delete(users::remove_user),
        )
        .route(
            "/package/{package_id}/access",
            put(join_queue::request_access).get(join_queue::list_access_requests),
        )
        .route(
            "/package/{package_id}/access/{request_id}",
            post(join_queue::accept_access_request)
                .delete(join_queue::reject_access_request),
        )
        .route(
            "/package/{package_id}/purchase",
            post(purchase::purchase),
        )
        .route(
            "/invitation/{invitation_id}/accept",
            post(users::accept_invitation),
        )
        .route(
            "/invitation/{invitation_id}/reject",
            post(users::reject_invitation),
        )
        .route("/invitations/me", get(users::list_my_invitations))
}

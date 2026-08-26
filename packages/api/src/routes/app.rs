use crate::{
    entity::{app, sea_orm_active_enums::Visibility},
    error::ApiError,
    state::AppState,
};
use axum::{
    Router,
    routing::{get, patch, post},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

pub mod internal;

pub mod ai_act;
pub mod analytics;
pub mod api;
pub mod board;
pub mod cache;
pub mod comments;
pub mod connection;
pub mod data;
pub mod db;
pub mod events;
pub mod fork;
pub mod graph;
pub mod groups;
pub mod invoke;
pub mod meta;
pub mod notifications;
pub mod packages;
pub mod page;
pub mod prerun_shared;
pub mod publication;
pub mod roles;
pub mod route;
pub mod sales;
pub mod team;
pub mod template;
pub mod wasm_catalog;
pub mod widget;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(internal::get_apps::get_apps))
        .route("/nodes", get(internal::get_nodes::get_nodes))
        .route("/{app_id}/nodes", get(internal::get_nodes::get_app_nodes))
        .route("/search", get(internal::search_apps::search_apps))
        // Literal segment, mounted on the root router. It must NOT live under
        // the `/{app_id}` nest, where `/{app_id}/templates` would shadow it.
        .route(
            "/templates/search",
            get(internal::search_templates::search_templates),
        )
        .route(
            "/{app_id}",
            get(internal::get_app::get_app)
                .put(internal::upsert_app::upsert_app)
                .delete(internal::delete_app::delete_app),
        )
        .route("/{app_id}/detail", get(internal::get_detail::get_detail))
        .route(
            "/{app_id}/visibility",
            patch(internal::change_visibility::change_visibility),
        )
        .route(
            "/{app_id}/settings/forking",
            patch(internal::change_forking::change_forking)
                .get(internal::change_forking::get_forking),
        )
        .route("/{app_id}/fork", post(fork::online_fork::online_fork))
        .route(
            "/{app_id}/notifications/create",
            post(notifications::create_notification),
        )
        .route(
            "/{app_id}/cache",
            get(cache::read_cache_entry)
                .put(cache::write_cache_entry)
                .delete(cache::delete_cache_entry)
                // Cache values may exceed axum's 2 MB default body limit. The wire form
                // can be larger than the stored compact form (escaped strings), so give
                // it 2x headroom on top of the configured value ceiling.
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::cache::CacheLimits::from_env()
                        .max_value_bytes
                        .saturating_mul(2)
                        .saturating_add(64 * 1024),
                )),
        )
        .route("/{app_id}/cache/exists", get(cache::cache_entry_exists))
        .route(
            "/{app_id}/cache/namespace",
            axum::routing::delete(cache::delete_cache_namespace),
        )
        .nest("/{app_id}/templates", template::routes())
        .nest("/{app_id}/widgets", widget::routes())
        .nest("/{app_id}/pages", page::routes())
        .nest("/{app_id}/board", board::routes())
        .nest("/{app_id}/meta", meta::routes())
        .nest("/{app_id}/roles", roles::routes())
        .nest("/{app_id}/team", team::routes())
        .nest("/{app_id}/connections", connection::routes())
        .nest("/{app_id}/groups", groups::routes())
        .nest("/{app_id}/analytics", analytics::routes())
        .nest("/{app_id}/sales", sales::routes())
        .nest("/{app_id}/events", events::routes())
        .nest("/{app_id}/fork", fork::routes())
        .merge(fork::root_routes())
        .nest("/{app_id}/comments", comments::routes())
        .nest("/{app_id}/publication", publication::routes())
        .nest("/{app_id}/packages", packages::routes())
        .nest("/{app_id}/data", data::routes())
        .nest("/{app_id}/invoke", invoke::routes())
        .nest("/{app_id}/db", db::routes())
        .nest("/{app_id}/graph", graph::routes())
        .nest("/{app_id}/api", api::routes())
        .nest("/{app_id}/routes", route::routes())
        .nest("/{app_id}/ai-act", ai_act::routes())
}

#[macro_export]
macro_rules! ensure_permission {
    ($user:expr, $app_id:expr, $state:expr, $perm:expr) => {{
        let sub = $user.app_permission($app_id, $state).await?;
        if !sub.has_permission($perm) {
            if let Ok(user_id) = sub.sub() {
                $state.invalidate_permission(&user_id, $app_id);
            }
            return Err($crate::error::ApiError::FORBIDDEN);
        }
        sub
    }};
}

/// Like `ensure_permission!`, but passes when the principal holds ANY of the
/// listed permissions (e.g. `ReadFiles` OR `ReadDatabase`).
#[macro_export]
macro_rules! ensure_any_permission {
    ($user:expr, $app_id:expr, $state:expr, $($perm:expr),+ $(,)?) => {{
        let sub = $user.app_permission($app_id, $state).await?;
        if !($(sub.has_permission($perm))||+) {
            if let Ok(user_id) = sub.sub() {
                $state.invalidate_permission(&user_id, $app_id);
            }
            return Err($crate::error::ApiError::FORBIDDEN);
        }
        sub
    }};
}

#[macro_export]
macro_rules! ensure_in_project {
    ($user:expr, $app_id:expr, $state:expr) => {{
        let sub = $user.app_permission($app_id, $state).await?;
        sub
    }};
}

#[macro_export]
macro_rules! ensure_permissions {
    ($user:expr, $app_id:expr, $state:expr, $perms:expr) => {{
        let sub = $user.app_permission($app_id, $state).await?;
        for perm in $perms.iter() {
            if !sub.has_permission(perm) {
                return Err($crate::error::ApiError::FORBIDDEN);
            }
        }
        sub
    }};
}

pub async fn ensure_app_publicly_visible(
    app_id: &str,
    state: &AppState,
) -> Result<app::Model, ApiError> {
    app::Entity::find_by_id(app_id)
        .filter(
            app::Column::Visibility
                .eq(Visibility::Public)
                .or(app::Column::Visibility.eq(Visibility::PublicRequestAccess)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)
}

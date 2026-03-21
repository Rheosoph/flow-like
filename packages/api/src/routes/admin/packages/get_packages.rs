//! List packages for admin

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::registry::server::PackageDetails;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::get_stats::RegistryStatsResponse;

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_stats: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct ListResponse {
    pub packages: Vec<PackageDetails>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<RegistryStatsResponse>,
}

#[utoipa::path(
    get,
    path = "/admin/packages",
    tag = "admin",
    params(ListQuery),
    responses(
        (status = 200, description = "List of packages", body = ListResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
pub async fn get_packages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("WASM registry not configured"))?;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).min(100);

    let (packages, total_count) = registry
        .list_packages_admin(query.status.as_deref(), offset, limit)
        .await?;

    let stats = if query.include_stats.unwrap_or(false) {
        let raw_stats = registry.get_stats().await?;
        Some(RegistryStatsResponse {
            total_packages: raw_stats.total_packages,
            total_versions: raw_stats.total_versions,
            total_downloads: raw_stats.total_downloads,
            pending_review: raw_stats.pending_review,
            active_packages: raw_stats.active_packages,
            rejected_packages: raw_stats.rejected_packages,
        })
    } else {
        None
    };

    Ok(Json(ListResponse {
        packages,
        total_count,
        offset,
        limit,
        stats,
    }))
}

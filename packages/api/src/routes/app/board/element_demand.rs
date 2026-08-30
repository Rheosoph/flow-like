//! Element demand of a board: which page elements its workflow reads.
//!
//! Answered from the prerun manifest, so the board is never loaded. A run
//! then ships only the selected elements (plus the one that triggered it);
//! clients fall back to the full element map when this cannot be fetched.

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{load_prerun_manifest, parse_version},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ElementDemandQuery {
    /// Board version in MAJOR_MINOR_PATCH format. Omitted means latest.
    pub version: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ElementDemandResponse {
    /// Element selectors the board reads: `pageId/elementId`, `elementId`,
    /// `instanceId/childId`, `host:KEY`, `type:X`, `glob:PATTERN`,
    /// `children:KEY`, `parent:KEY`, `values:instanceId`.
    pub selectors: Vec<String>,
    /// Whether the board also resolves element references at run time, so the
    /// static selectors alone are not exhaustive.
    pub dynamic: bool,
    /// Stable hash of the board-derived prerun data; changes whenever the
    /// demand changes.
    pub signature: String,
}

/// Lists the page elements a board's workflow reads, so a run can send only
/// those instead of the whole page.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/element-demand",
    tag = "execution",
    description = "Get the page elements a board reads before running it.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ElementDemandQuery
    ),
    responses(
        (status = 200, description = "Element selectors the board reads", body = ElementDemandResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/element-demand",
    skip(state, user, query)
)]
pub async fn get_element_demand(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<ElementDemandQuery>,
) -> Result<Json<ElementDemandResponse>, ApiError> {
    super::ensure_connected_app_board_invoke_denied(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);

    let version = query.version.as_deref().and_then(parse_version);
    let manifest = load_prerun_manifest(&state, &app_id, &board_id, version).await?;

    Ok(Json(ElementDemandResponse {
        selectors: manifest.element_selectors.clone(),
        dynamic: manifest.element_reads_dynamic,
        signature: manifest.signature.clone(),
    }))
}

//! Package search endpoint

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use super::types::{SearchFilters, SearchResults};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(flatten)]
    pub filters: SearchFilters,
    #[serde(default)]
    pub include_own: bool,
}

/// GET /registry/search
/// Search packages with visibility filtering. Public packages are always shown.
/// When `include_own` is true, packages the caller has access to are also included.
#[utoipa::path(
    get,
    path = "/registry/search",
    tag = "registry",
    params(
        ("query" = Option<String>, Query, description = "Search query matching name, description, keywords"),
        ("category" = Option<String>, Query, description = "Filter by category"),
        ("keywords" = Option<Vec<String>>, Query, description = "Filter by keywords"),
        ("author" = Option<String>, Query, description = "Filter by author"),
        ("verified_only" = Option<bool>, Query, description = "Only show verified packages"),
        ("include_deprecated" = Option<bool>, Query, description = "Include deprecated packages"),
        ("offset" = Option<usize>, Query, description = "Pagination offset"),
        ("limit" = Option<usize>, Query, description = "Pagination limit"),
        ("sort_by" = Option<String>, Query, description = "Sort field: relevance, name, downloads, updated_at, created_at"),
        ("sort_desc" = Option<bool>, Query, description = "Sort direction (descending if true)"),
        ("include_own" = Option<bool>, Query, description = "Include private packages the caller has access to")
    ),
    responses(
        (status = 200, description = "Search results", body = SearchResults),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn search(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let caller_id = user.sub().ok();

    let results = registry
        .search_with_visibility(&query.filters, caller_id.as_deref(), query.include_own)
        .await?;
    Ok(Json(results))
}

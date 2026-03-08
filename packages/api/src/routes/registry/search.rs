//! Package search endpoint

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use super::types::{SearchFilters, SearchResults, SortField};
use serde::Deserialize;

fn default_limit() -> usize { 50 }

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub verified_only: bool,
    #[serde(default)]
    pub include_deprecated: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub sort_by: SortField,
    #[serde(default)]
    pub sort_desc: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub include_own: bool,
}

impl SearchQuery {
    fn into_parts(self) -> (SearchFilters, bool) {
        let filters = SearchFilters {
            query: self.query,
            category: self.category,
            keywords: self.keywords,
            author: self.author,
            verified_only: self.verified_only,
            include_deprecated: self.include_deprecated,
            offset: self.offset,
            limit: self.limit,
            sort_by: self.sort_by,
            sort_desc: self.sort_desc,
            language: self.language,
        };
        (filters, self.include_own)
    }
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
    let (filters, include_own) = query.into_parts();

    let results = registry
        .search_with_visibility(&filters, caller_id.as_deref(), include_own)
        .await?;
    Ok(Json(results))
}

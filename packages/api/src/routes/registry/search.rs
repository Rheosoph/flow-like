//! Package search endpoint

use super::types::{PackageAccessFilter, PackageSummary, SearchFilters, SearchResults, SortField};
use crate::entity::wasm_package_user;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Deserialize;
use std::collections::HashMap;

const MAX_SEARCH_IDS: usize = 100;

/// Mark which results the caller may install, so pickers can send them to the
/// store instead of failing later, and the caller's permission bits on each.
/// One query for the whole page.
async fn mark_viewer_access(
    state: &AppState,
    caller_id: &str,
    packages: &mut [PackageSummary],
) -> Result<(), ApiError> {
    if packages.is_empty() {
        return Ok(());
    }
    let held: HashMap<String, i64> = wasm_package_user::Entity::find()
        .select_only()
        .column(wasm_package_user::Column::PackageId)
        .column(wasm_package_user::Column::Permission)
        .filter(wasm_package_user::Column::UserId.eq(caller_id))
        .filter(wasm_package_user::Column::PackageId.is_in(packages.iter().map(|p| p.id.clone())))
        .filter(wasm_package_user::Column::Permission.ne(0))
        .into_tuple()
        .all(&state.db)
        .await?
        .into_iter()
        .collect();
    for package in packages {
        let permission = held.get(&package.id).copied();
        let free = package.visibility == "public" && package.price <= 0;
        package.viewer_has_access = Some(free || permission.is_some());
        package.viewer_permission = permission;
    }
    Ok(())
}

fn parse_ids(raw: Option<&str>) -> Result<Option<Vec<String>>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let ids: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(String::from)
        .collect();
    if ids.len() > MAX_SEARCH_IDS {
        return Err(ApiError::bad_request(format!(
            "ids accepts at most {MAX_SEARCH_IDS} package ids, got {}",
            ids.len()
        )));
    }
    Ok(Some(ids))
}

fn default_limit() -> usize {
    50
}

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
    pub include_disabled: bool,
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
    #[serde(default)]
    pub owned_only: bool,
    #[serde(default)]
    pub access: Option<PackageAccessFilter>,
    #[serde(default)]
    pub ids: Option<String>,
}

impl SearchQuery {
    fn into_parts(self) -> (SearchFilters, bool, bool) {
        let filters = SearchFilters {
            query: self.query,
            category: self.category,
            keywords: self.keywords,
            author: self.author,
            verified_only: self.verified_only,
            include_deprecated: self.include_deprecated,
            include_disabled: self.include_disabled,
            offset: self.offset,
            limit: self.limit,
            sort_by: self.sort_by,
            sort_desc: self.sort_desc,
            language: self.language,
        };
        (filters, self.include_own, self.owned_only)
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
        ("include_disabled" = Option<bool>, Query, description = "Include disabled (soft-deleted) packages"),
        ("offset" = Option<usize>, Query, description = "Pagination offset"),
        ("limit" = Option<usize>, Query, description = "Pagination limit"),
        ("sort_by" = Option<String>, Query, description = "Sort field: relevance, name, downloads, updated_at, created_at"),
        ("sort_desc" = Option<bool>, Query, description = "Sort direction (descending if true)"),
        ("include_own" = Option<bool>, Query, description = "Include private packages the caller has access to"),
        ("owned_only" = Option<bool>, Query, description = "Return only packages the caller owns or has access to"),
        ("access" = Option<String>, Query, description = "Return only your packages with this access: maintainer (owner or maintainer), library (shared with you or bought). Implies owned_only"),
        ("ids" = Option<String>, Query, description = "Comma-separated package ids to restrict results to (at most 100); an empty list matches nothing")
    ),
    responses(
        (status = 200, description = "Search results", body = SearchResults),
        (status = 400, description = "Unknown access filter or more than 100 package ids"),
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
    let access = query.access;
    let ids = parse_ids(query.ids.as_deref())?;
    let (filters, include_own, owned_only) = query.into_parts();

    let mut results = registry
        .search_with_visibility(
            &filters,
            caller_id.as_deref(),
            include_own,
            owned_only,
            access,
            ids.as_deref(),
        )
        .await?;

    if let Some(caller_id) = caller_id.as_deref() {
        mark_viewer_access(&state, caller_id, &mut results.packages).await?;
    }

    if let Ok(master_creds) = state.master_credentials().await
        && let Ok(store) = master_creds.to_store(false).await
    {
        for pkg in &mut results.packages {
            if let Some(meta) = &mut pkg.metadata {
                meta.presign_media(&pkg.id, &store).await;
            }
        }
    }

    Ok(Json(results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{StatusCode, Uri};

    fn search_query(query: &str) -> Option<SearchQuery> {
        let uri: Uri = format!("/registry/search?{query}").parse().unwrap();
        Query::<SearchQuery>::try_from_uri(&uri)
            .ok()
            .map(|Query(query)| query)
    }

    #[test]
    fn ids_absent_means_no_restriction() {
        assert_eq!(parse_ids(None).unwrap(), None);
    }

    #[test]
    fn ids_are_trimmed_and_empty_entries_dropped() {
        assert_eq!(
            parse_ids(Some(" a , ,b,,c ")).unwrap(),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn ids_without_entries_restrict_to_nothing() {
        assert_eq!(parse_ids(Some("")).unwrap(), Some(vec![]));
        assert_eq!(parse_ids(Some(" , ,")).unwrap(), Some(vec![]));
    }

    #[test]
    fn ids_accept_the_limit_and_reject_more() {
        let at_limit = (0..MAX_SEARCH_IDS)
            .map(|i| format!("pkg-{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse_ids(Some(&at_limit)).unwrap().map(|ids| ids.len()),
            Some(MAX_SEARCH_IDS)
        );

        let over_limit = format!("{at_limit},pkg-extra");
        let error = parse_ids(Some(&over_limit)).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn ids_limit_counts_entries_after_dropping_empty_ones() {
        let padded = format!(
            "{},,,",
            (0..MAX_SEARCH_IDS)
                .map(|i| format!("pkg-{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse_ids(Some(&padded)).is_ok());
    }

    #[test]
    fn query_string_parses_access_and_ids() {
        let query = search_query("access=maintainer&ids=a,b").unwrap();
        assert_eq!(query.access, Some(PackageAccessFilter::Maintainer));
        assert_eq!(query.ids.as_deref(), Some("a,b"));

        let query = search_query("access=library").unwrap();
        assert_eq!(query.access, Some(PackageAccessFilter::Library));

        let query = search_query("owned_only=true").unwrap();
        assert_eq!(query.access, None);
        assert_eq!(query.ids, None);
    }

    #[test]
    fn query_string_rejects_unknown_access() {
        assert!(search_query("access=owner").is_none());
    }
}

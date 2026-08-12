use crate::{
    ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use flow_like::a2ui::widget::Page;
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// A pinned board version addresses an immutable snapshot, so a client may hold it without
/// asking again. The current page can change at any moment and has to be revalidated —
/// `no-cache` requires exactly that while still letting a 304 answer the revalidation.
const CURRENT_PAGE_CACHE: &str = "private, no-cache";
const VERSIONED_PAGE_CACHE: &str = "private, max-age=600";

/// Derived from the encoded page rather than from the `Page` row's timestamp: any rewrite of
/// the stored payload changes the tag, so a writer that does not touch the row (template
/// instantiation, fork, a board-level restore) can never leave clients pinned to stale content.
fn page_etag(body: &[u8]) -> String {
    format!("\"{}\"", blake3::hash(body).to_hex())
}

fn if_none_match_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|candidate| {
                let candidate = candidate.trim();
                candidate == "*"
                    || candidate == etag
                    || candidate
                        .strip_prefix("W/")
                        .is_some_and(|inner| inner == etag)
            })
        })
}

fn page_response(page: &Page, headers: &HeaderMap, versioned: bool) -> Result<Response, ApiError> {
    let body = flow_like_types::json::to_vec(page)
        .map_err(|e| ApiError::internal_error(anyhow!("failed to encode page: {e}")))?;
    let etag = page_etag(&body);
    let cache_control = if versioned {
        VERSIONED_PAGE_CACHE
    } else {
        CURRENT_PAGE_CACHE
    };

    // A shared browser cache keys only on the URL, and two accounts on one device address the
    // same page URL. Their requests differ solely by credential, so the stored entry has to be
    // scoped to it.
    if if_none_match_matches(headers, &etag) {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, cache_control),
                (header::VARY, header::AUTHORIZATION.as_str()),
            ],
        )
            .into_response());
    }

    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ETAG, etag.as_str()),
            (header::CACHE_CONTROL, cache_control),
            (header::VARY, header::AUTHORIZATION.as_str()),
        ],
        body,
    )
        .into_response())
}

#[derive(Deserialize, Debug, IntoParams, ToSchema)]
pub struct VersionQuery {
    /// expected format: "MAJOR_MINOR_PATCH", e.g. "1_0_3"
    pub version: Option<String>,
    /// Exact owning board. When supplied, lookup never falls through to another board.
    pub board_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/pages/{page_id}",
    tag = "pages",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("page_id" = String, Path, description = "Page ID"),
        VersionQuery
    ),
    responses(
        (status = 200, description = "Page details", body = Object),
        (status = 304, description = "The cached copy of this page is still current"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Page not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/pages/{page_id}",
    skip(state, user, params, headers)
)]
pub async fn get_page(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, page_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);

    let requested_board_id = params.board_id.filter(|id| !id.trim().is_empty());
    let version_opt = if let Some(ver_str) = params.version {
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::internal_error(anyhow!(
                    "version must be in MAJOR_MINOR_PATCH format"
                )));
            }
        }
    } else {
        None
    };

    // The Page DB row carries the owning `board_id`; trying that
    // board first avoids scanning every board on the app. A missing
    // hint, an unresolvable hint, or a load failure all fall through
    // to a full scan so stale/orphaned DB rows can't make a page
    // permanently unreachable. The same scan strategy is used by the
    // desktop's `get_page` Tauri command.
    let row = page::Entity::find_by_id(&page_id)
        .filter(page::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?;
    let board_hint = requested_board_id
        .clone()
        .or_else(|| row.and_then(|r| r.board_id));

    let app = state.master_app(&user.sub()?, &app_id, &state).await?;

    let try_board = |board_id: String| {
        let app = &app;
        let page_id = &page_id;
        async move {
            let board = app.open_board(board_id, None, version_opt).await.ok()?;
            let board_guard = board.lock().await;
            match version_opt {
                Some(v) => board_guard.load_versioned_page(page_id, v, None).await.ok(),
                None => board_guard.load_page(page_id, None).await.ok(),
            }
        }
    };

    let versioned = version_opt.is_some();

    if let Some(board_id) = board_hint
        && let Some(page) = try_board(board_id).await
    {
        return page_response(&page, &headers, versioned);
    }
    if requested_board_id.is_some() {
        return Err(ApiError::NOT_FOUND);
    }

    for board_id in app.boards.iter() {
        if let Some(page) = try_board(board_id.clone()).await {
            return page_response(&page, &headers, versioned);
        }
    }

    Err(ApiError::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, value.parse().unwrap());
        headers
    }

    #[test]
    fn matches_exact_and_weak_and_wildcard_tags() {
        let etag = page_etag(b"payload");
        assert!(if_none_match_matches(&headers_with(&etag), &etag));
        assert!(if_none_match_matches(
            &headers_with(&format!("W/{etag}")),
            &etag
        ));
        assert!(if_none_match_matches(&headers_with("*"), &etag));
        assert!(if_none_match_matches(
            &headers_with(&format!("\"other\", {etag}")),
            &etag
        ));
    }

    #[test]
    fn rejects_a_tag_from_different_content() {
        let etag = page_etag(b"payload");
        assert!(!if_none_match_matches(
            &headers_with(&etag),
            &page_etag(b"other")
        ));
        assert!(!if_none_match_matches(&HeaderMap::new(), &etag));
    }
}

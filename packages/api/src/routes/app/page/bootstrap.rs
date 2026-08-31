//! One authenticated read for the `/use` surface.
//!
//! A route is an Event selection. When the Event has a custom page, it pins both the board and
//! optionally a board version that own it. Keeping those reads together prevents a client from
//! accidentally combining a fresh Event with a page from a different board snapshot.

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        events::db::{
            filter_event_list_execution, filter_event_secrets, get_event_with_fallback_opt,
            get_events_for_app, get_events_with_fallback, is_listed_event_type,
            is_user_facing_event,
        },
        page::get_page::{if_none_match_matches, load_event_bound_page, page_etag},
    },
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use flow_like::a2ui::widget::Page;
use flow_like::flow::compiled::prerun::{
    decorate_page_actions, redact_page_execution_routes,
};
use flow_like::flow::event::Event;
use flow_like_types::anyhow;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

const BOOTSTRAP_CACHE: &str = "private, no-cache";

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapQuery {
    /// A `/use` route path. When present, it takes precedence over `eventId`.
    pub route: Option<String>,
    /// Direct Event target for links that do not name a route.
    #[serde(alias = "event_id")]
    pub event_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    /// The resolved route after `/use`-compatible normalization. Direct Event targets without a
    /// route leave this empty rather than pretending they are the app default.
    pub canonical_route: Option<String>,
    /// True when a non-root requested route was not mapped and the default route was selected.
    pub route_miss: bool,
    #[schema(value_type = Object)]
    pub event: Event,
    /// The exact Event-bound custom page. Runnable Events without a custom page return `null`.
    #[schema(value_type = Object)]
    pub page: Option<Page>,
    /// BLAKE3 of the encoded page payload, or `null` when the Event has no custom page. This is
    /// for a surface cache that is independent of Event metadata and HTTP freshness.
    pub revision: Option<String>,
    /// Revision of the Page execution authority map. Clients return this with
    /// lifecycle and static action invocations.
    pub execution_revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteResolution {
    event_index: Option<usize>,
    route_miss: bool,
}

/// This is deliberately the same canonical form as the `/use` client resolver. Case remains
/// significant, while links may omit a leading slash, contain a query/fragment, or retain a
/// trailing slash from an older saved route.
fn normalize_route_path(path: &str) -> String {
    let raw = path.trim();
    if raw.is_empty() {
        return "/".to_string();
    }
    let without_fragment = raw.split('#').next().unwrap_or_default();
    let without_query = without_fragment.split('?').next().unwrap_or_default();
    let with_leading_slash = if without_query.starts_with('/') {
        without_query.to_string()
    } else {
        format!("/{without_query}")
    };
    let without_trailing_slashes = with_leading_slash.trim_end_matches('/');
    if without_trailing_slashes.is_empty() {
        "/".to_string()
    } else {
        without_trailing_slashes.to_string()
    }
}

fn canonical_event_route(event: &Event) -> Option<String> {
    event
        .route
        .as_deref()
        .filter(|route| !route.trim().is_empty())
        .map(normalize_route_path)
        .or_else(|| event.is_default.then(|| "/".to_string()))
}

/// Preserve the client resolver's precedence rules: an explicit root mapping wins over a
/// synthesized default mapping, and the first Event wins when legacy data contains duplicates.
fn resolve_route(events: &[Event], requested_route: &str) -> RouteResolution {
    let requested_route = normalize_route_path(requested_route);
    let default = events
        .iter()
        .position(|event| {
            event
                .route
                .as_deref()
                .is_some_and(|route| normalize_route_path(route) == "/")
        })
        .or_else(|| {
            events.iter().position(|event| {
                event.is_default
                    && event
                        .route
                        .as_deref()
                        .is_none_or(|route| route.trim().is_empty())
            })
        });

    if requested_route == "/" {
        return RouteResolution {
            event_index: default,
            route_miss: false,
        };
    }

    let matched = events.iter().position(|event| {
        event
            .route
            .as_deref()
            .is_some_and(|route| normalize_route_path(route) == requested_route)
    });

    RouteResolution {
        event_index: matched.or(default),
        route_miss: matched.is_none(),
    }
}

fn bootstrap_response(
    event: Event,
    page: Option<Page>,
    execution_revision: Option<String>,
    canonical_route: Option<String>,
    route_miss: bool,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let revision = page
        .as_ref()
        .map(|page| {
            flow_like_types::json::to_vec(page)
                .map(|page_bytes| blake3::hash(&page_bytes).to_hex().to_string())
                .map_err(|error| {
                    ApiError::internal_error(anyhow!("failed to encode page: {error}"))
                })
        })
        .transpose()?;
    let body = flow_like_types::json::to_vec(&BootstrapResponse {
        canonical_route,
        route_miss,
        event,
        page,
        revision,
        execution_revision,
    })
    .map_err(|error| ApiError::internal_error(anyhow!("failed to encode bootstrap: {error}")))?;
    // The HTTP validator covers the entire sanitized document. A permission-safe Event change,
    // route correction, or page rewrite can therefore never receive a stale 304 response.
    let etag = page_etag(&body);

    if if_none_match_matches(headers, &etag) {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, BOOTSTRAP_CACHE),
                (header::VARY, header::AUTHORIZATION.as_str()),
            ],
        )
            .into_response());
    }

    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ETAG, etag.as_str()),
            (header::CACHE_CONTROL, BOOTSTRAP_CACHE),
            (header::VARY, header::AUTHORIZATION.as_str()),
        ],
        body,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/pages/bootstrap",
    tag = "pages",
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = [])),
    description = "Resolve a /use route or direct Event and return its active, sanitized Event and optional exact bound custom page.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        BootstrapQuery
    ),
    responses(
        (status = 200, description = "Resolved Event bootstrap, with a page when the Event owns one", body = BootstrapResponse),
        (status = 304, description = "The cached bootstrap document is still current"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No active route or Event was found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/pages/bootstrap",
    skip(state, user, params, headers)
)]
pub async fn bootstrap(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<BootstrapQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let can_read_events = permission.has_permission(RolePermissions::ReadEvents);
    let can_use_direct_board = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards);
    let principal_id = permission.identifier();
    let app = state.master_app(&principal_id, &app_id, &state).await?;
    let is_visible_runtime_event = |event: &Event| {
        event.active
            && is_listed_event_type(&event.event_type)
            && (can_read_events || is_user_facing_event(event))
    };

    let (event, route_miss) = if let Some(route) = params.route.as_deref() {
        // The mirror is already the route index. Its only bucket fallback occurs when the entire
        // mirror is absent, never by scanning boards or unrelated pages.
        let mut events = get_events_for_app(&state.db, &app_id).await?;
        if events.is_empty() {
            events = get_events_with_fallback(&state.db, &app).await?;
        }
        events.retain(&is_visible_runtime_event);
        let resolution = resolve_route(&events, route);
        let event = resolution
            .event_index
            .and_then(|index| events.into_iter().nth(index))
            .ok_or(ApiError::NOT_FOUND)?;
        (event, resolution.route_miss)
    } else if let Some(event_id) = params
        .event_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    {
        let event = get_event_with_fallback_opt(&state.db, &app, event_id)
            .await?
            .filter(&is_visible_runtime_event)
            .ok_or(ApiError::NOT_FOUND)?;
        (event, false)
    } else {
        let mut events = get_events_for_app(&state.db, &app_id).await?;
        if events.is_empty() {
            events = get_events_with_fallback(&state.db, &app).await?;
        }
        events.retain(&is_visible_runtime_event);
        let resolution = resolve_route(&events, "/");
        let event = resolution
            .event_index
            .and_then(|index| events.into_iter().nth(index))
            .ok_or(ApiError::NOT_FOUND)?;
        (event, false)
    };

    let canonical_route = canonical_event_route(&event);
    // An Event without `default_page_id` is still a valid bootstrap target for generic forms,
    // chats, and other runnable surfaces. When it declares a custom page, however, that page is
    // part of the contract and a missing version-bound artifact remains a uniform 404.
    let (page, execution_revision) = if event.default_page_id.is_some() {
        if event.board_version.is_none() {
            return Err(ApiError::bad_request(
                "Governed Page Events must pin an immutable board version",
            ));
        }
        let page = load_event_bound_page(&app, &event).await?;
        let board = app
            .open_board(event.board_id.clone(), None, event.board_version)
            .await
            .map_err(|_| ApiError::NOT_FOUND)?;
        let board = board.lock().await;
        if board.id != event.board_id || Some(board.version) != event.board_version {
            return Err(ApiError::bad_request(
                "The Page Event configuration is invalid",
            ));
        }
        let (page_execution, _, execution_revision) =
            crate::routes::app::events::page_trigger::compile_page_contract(&board, &page)?;
        let mut page = decorate_page_actions(&page, &page_execution, &execution_revision).map_err(
            |error| {
                ApiError::internal_error(anyhow!(
                    "failed to project governed Page actions: {error}"
                ))
            },
        )?;
        if !can_use_direct_board {
            page = redact_page_execution_routes(&page).map_err(ApiError::internal_error)?;
        }
        (Some(page), Some(execution_revision))
    } else {
        (None, None)
    };
    let mut event = filter_event_list_execution(filter_event_secrets(event));
    if !can_use_direct_board && event.default_page_id.is_some() {
        event.node_id.clear();
    }
    bootstrap_response(
        event,
        page,
        execution_revision,
        canonical_route,
        route_miss,
        &headers,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn event(id: &str, route: Option<&str>, is_default: bool) -> Event {
        Event {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            board_id: "board".to_string(),
            board_version: None,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            priority: 0,
            event_type: "generic_form".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: Some("page".to_string()),
            inputs: Vec::new(),
            route: route.map(ToString::to_string),
            is_default,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    #[test]
    fn route_resolution_matches_use_normalization_and_reports_a_miss() {
        let events = vec![
            event("settings", Some("/settings/"), false),
            event("home", Some("/"), true),
        ];
        assert_eq!(normalize_route_path("settings/?tab=api#keys"), "/settings");
        assert_eq!(
            resolve_route(&events, "settings/?tab=api#keys"),
            RouteResolution {
                event_index: Some(0),
                route_miss: false
            }
        );
        assert_eq!(
            resolve_route(&events, "/missing"),
            RouteResolution {
                event_index: Some(1),
                route_miss: true
            }
        );
    }

    #[test]
    fn explicit_root_beats_a_synthesized_default_route() {
        let events = vec![
            event("root", Some("/"), false),
            event("default", None, true),
        ];
        assert_eq!(
            resolve_route(&events, "/"),
            RouteResolution {
                event_index: Some(0),
                route_miss: false
            }
        );
    }

    #[test]
    fn bootstrap_etag_covers_event_metadata_but_revision_only_covers_page_content() {
        let page = Page::new("page", "Page", "/");
        let headers = HeaderMap::new();
        let response = bootstrap_response(
            event("event", Some("/"), true),
            Some(page.clone()),
            Some("per1_test".to_string()),
            Some("/".to_string()),
            false,
            &headers,
        )
        .unwrap();
        let etag = response.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(response.headers()[header::CACHE_CONTROL], BOOTSTRAP_CACHE);
        assert_eq!(
            response.headers()[header::VARY],
            header::AUTHORIZATION.as_str()
        );

        let mut conditional = HeaderMap::new();
        conditional.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        let unchanged = bootstrap_response(
            event("event", Some("/"), true),
            Some(page.clone()),
            Some("per1_test".to_string()),
            Some("/".to_string()),
            false,
            &conditional,
        )
        .unwrap();
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);

        let not_modified = bootstrap_response(
            event("renamed", Some("/"), true),
            Some(page),
            Some("per1_test".to_string()),
            Some("/".to_string()),
            false,
            &conditional,
        )
        .unwrap();
        assert_eq!(not_modified.status(), StatusCode::OK);
    }

    #[test]
    fn non_page_events_keep_their_event_bootstrap_without_a_surface_revision() {
        let response = BootstrapResponse {
            canonical_route: Some("/chat".to_string()),
            route_miss: false,
            event: event("chat", Some("/chat"), false),
            page: None,
            revision: None,
            execution_revision: None,
        };
        let value = flow_like_types::json::to_value(response).unwrap();
        assert!(value["event"].is_object());
        assert!(value["page"].is_null());
        assert!(value["revision"].is_null());
    }
}

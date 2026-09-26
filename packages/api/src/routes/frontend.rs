//! Runtime API for statically hosted apps. A hosted app is addressed as
//! `/a/{app_id}/{route}`; the route picks the chat, form or custom page whose
//! Event owns it, exactly like `/use`. Every request resolves the route and
//! rechecks that Event's publication. A static asset grants no access to other
//! events, boards, or the app's private resources.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use flow_like::{
    a2ui::widget::Page,
    flow::{
        compiled::prerun::{decorate_page_actions, redact_page_execution_routes},
        event::{Event, EventExecutionMode},
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::{
    entity::event,
    error::ApiError,
    execution::variant::{self, SplitKey},
    middleware::jwt::{AppUser, OpenIDUser, viewer_authorization},
    routes::app::{
        data::{
            batch::validate_batch,
            download_files::{DOWNLOAD_URL_TTL, sign_downloads},
            paths,
        },
        events::{
            db::db_model_to_event,
            invoke_event::{
                InvokeEventQuery, InvokeEventRequest, hosted_subject, invoke_hosted_event,
            },
            page_trigger::{resolve_authorized_page_trigger, resolve_page_contract_for_bootstrap},
            prerun_event::{PrerunEventResponse, PrerunPageEventRequest, build_response},
        },
        page::bootstrap::{
            BootstrapResponse, bootstrap_event, canonical_route, normalize_route_path,
        },
        prerun_shared::{PrerunPayload, load_prerun_manifest},
    },
    state::AppState,
    utils::event_alias,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrontendHosting {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auth_proxy: bool,
    #[serde(default)]
    pub allow_anonymous: bool,
}

impl FrontendHosting {
    fn requires_auth(self) -> bool {
        self.auth_proxy || !self.allow_anonymous
    }
}

#[derive(Default, Deserialize)]
struct FrontendQuery {
    #[serde(rename = "__variant")]
    variant: Option<String>,
}

/// The app route a runtime call is made for; absent means `/`.
#[derive(Default, Deserialize)]
struct RouteQuery {
    route: Option<String>,
}

impl RouteQuery {
    fn path(&self) -> String {
        normalize_route_path(self.route.as_deref().unwrap_or("/"))
    }
}

#[derive(Serialize)]
struct FrontendResponse {
    app_id: String,
    kind: &'static str,
    auth_proxy: bool,
    bootstrap: BootstrapResponse,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/a/{app_id}", get(bootstrap))
        .route("/a/{app_id}/invoke", post(invoke))
        .route("/a/{app_id}/prerun", get(prerun).post(prerun_page))
        .route("/a/{app_id}/routes", get(published_routes))
        .route("/a/{app_id}/widgets/{widget_id}", get(widget))
        .route("/a/{app_id}/assets", post(signed_assets))
}

pub fn link_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/a/{app_id}",
            get(
                |State(state): State<AppState>,
                 Path(app_id): Path<String>,
                 RawQuery(query): RawQuery| async move {
                    frontend_link(state, app_id, "/", query).await
                },
            ),
        )
        .route(
            "/a/{app_id}/{*route}",
            get(
                |State(state): State<AppState>,
                 Path((app_id, route)): Path<(String, String)>,
                 RawQuery(query): RawQuery| async move {
                    frontend_link(state, app_id, &route, query).await
                },
            ),
        )
}

fn frontend_location(
    base: &str,
    app_id: &str,
    route: &str,
    query: Option<&str>,
) -> Result<String, ApiError> {
    let mut url = reqwest::Url::parse(base).map_err(|_| {
        ApiError::service_unavailable(
            "FRONTEND_BASE_URL or FRONTEND_URL must be an absolute HTTP URL",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::service_unavailable(
            "Invalid FRONTEND_BASE_URL or FRONTEND_URL",
        ));
    }
    url.path_segments_mut()
        .map_err(|_| ApiError::service_unavailable("Invalid FRONTEND_BASE_URL or FRONTEND_URL"))?
        .pop_if_empty()
        .push("a")
        .push(app_id)
        .extend(route.split('/').filter(|segment| !segment.is_empty()));
    url.set_query(query.filter(|query| !query.is_empty()));
    Ok(url.to_string())
}

fn frontend_base_url<'a>(
    hosted: Option<&'a str>,
    web: Option<&'a str>,
) -> Result<&'a str, ApiError> {
    hosted
        .filter(|value| !value.trim().is_empty())
        .or_else(|| web.filter(|value| !value.trim().is_empty()))
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "Configure FRONTEND_BASE_URL or FRONTEND_URL to the web app origin to serve hosted frontends",
            )
        })
}

async fn frontend_link(
    state: AppState,
    app_id: String,
    route: &str,
    query: Option<String>,
) -> Result<Response, ApiError> {
    let route = normalize_route_path(route);
    load_route(&state, &app_id, &route).await?;
    let hosted_base = std::env::var("FRONTEND_BASE_URL").ok();
    let web_base = std::env::var("FRONTEND_URL").ok();
    let base = frontend_base_url(hosted_base.as_deref(), web_base.as_deref())?;
    let location = frontend_location(base, &app_id, &route, query.as_deref())?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Redirect::temporary(&location),
    )
        .into_response())
}

/// The hosted interface an Event renders as: `u` for a custom page, `c` for a
/// chat, `f` for a form. Other triggers have nothing to host.
fn hosted_kind(event: &Event) -> Option<&'static str> {
    if event.default_page_id.is_some() {
        return Some("u");
    }
    match event_alias::alias_event_type(&event.event_type, None) {
        "simple_chat" => Some("c"),
        "generic_form" => Some("f"),
        _ => None,
    }
}

fn hosting(event: &Event) -> Result<(&'static str, FrontendHosting), ApiError> {
    let kind = hosted_kind(event).ok_or(ApiError::NOT_FOUND)?;
    if !event.active
        || event.exposure.is_internal()
        || event.execution_mode != EventExecutionMode::Remote
        || event.event_type == "ontology_action"
    {
        return Err(ApiError::NOT_FOUND);
    }
    let config: serde_json::Value = serde_json::from_slice(&event.config).unwrap_or_default();
    let hosting = config
        .get("frontend_hosting")
        .cloned()
        .and_then(|value| serde_json::from_value::<FrontendHosting>(value).ok())
        .filter(|hosting| hosting.enabled)
        .ok_or(ApiError::NOT_FOUND)?;
    Ok((kind, hosting))
}

/// Index of the Event that owns `route`. An explicit mapping wins over the app
/// default standing in for `/`, matching the `/use` resolver; a miss is a miss.
fn route_owner<'a>(
    mappings: impl IntoIterator<Item = (Option<&'a str>, bool)>,
    route: &str,
) -> Option<usize> {
    let mut default = None;
    for (index, (path, is_default)) in mappings.into_iter().enumerate() {
        match path.filter(|path| !path.trim().is_empty()) {
            Some(path) if normalize_route_path(path) == route => return Some(index),
            None if is_default && route == "/" && default.is_none() => default = Some(index),
            _ => {}
        }
    }
    default
}

struct HostedRoute {
    event: Event,
    kind: &'static str,
    config: FrontendHosting,
}

async fn active_app_events(state: &AppState, app_id: &str) -> Result<Vec<event::Model>, ApiError> {
    Ok(event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Active.eq(true))
        .all(&state.db)
        .await?)
}

async fn load_route(state: &AppState, app_id: &str, route: &str) -> Result<HostedRoute, ApiError> {
    let mut rows = active_app_events(state, app_id).await?;
    let index = route_owner(
        rows.iter()
            .map(|row| (row.route.as_deref(), row.is_default)),
        route,
    )
    .ok_or(ApiError::NOT_FOUND)?;
    let event = db_model_to_event(rows.swap_remove(index)).map_err(ApiError::internal_error)?;
    let (kind, config) = hosting(&event)?;
    Ok(HostedRoute {
        event,
        kind,
        config,
    })
}

async fn caller(
    state: &AppState,
    headers: &HeaderMap,
    hosting: FrontendHosting,
) -> Result<Option<AppUser>, ApiError> {
    // Public anonymous hosting does not borrow a viewer's ambient login.
    if !hosting.requires_auth() {
        return Ok(None);
    }
    let token = viewer_authorization(headers)
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| ApiError::unauthorized("Sign in to open this frontend"))?;
    let validated = state
        .validate_token(token)
        .await
        .map_err(|_| ApiError::unauthorized("Invalid frontend access token"))?;
    let sub = validated
        .claims
        .get("sub")
        .and_then(serde_json::Value::as_str)
        .filter(|sub| !sub.is_empty())
        .ok_or_else(|| ApiError::unauthorized("Missing frontend user identity"))?;
    Ok(Some(AppUser::OpenID(OpenIDUser {
        sub: sub.to_string(),
        access_token: token.to_string(),
    })))
}

fn session_id(headers: &HeaderMap) -> Result<&str, ApiError> {
    let session = headers
        .get("x-flow-like-session")
        .and_then(|value| value.to_str().ok())
        .filter(|session| (16..=128).contains(&session.len()))
        .filter(|session| {
            session
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        })
        .ok_or_else(|| ApiError::bad_request("A valid X-Flow-Like-Session header is required"))?;
    Ok(session)
}

fn private_json(value: impl Serialize) -> Response {
    (
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::VARY, "Authorization, X-Flow-Like-Session"),
        ],
        Json(value),
    )
        .into_response()
}

async fn bootstrap(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let route = route.path();
    let HostedRoute {
        event,
        kind,
        config,
    } = load_route(&state, &app_id, &route).await?;
    let caller = caller(&state, &headers, config).await?;
    let session = session_id(&headers)?;
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session,
    )?;
    let pin = variant::pin_from_request(&headers, query.variant);
    let target = variant::resolve_page_target(
        &event,
        pin.as_deref(),
        &SplitKey {
            source: variant::SPLIT_SOURCE_SUBJECT,
            value: subject.clone(),
        },
    )
    .map_err(ApiError::bad_request)?;
    let served_variant = target.variant_name.clone();
    let event = variant::apply_target(event, &target);
    let (page, execution_revision, element_demand) = if event.default_page_id.is_some() {
        let contract = resolve_page_contract_for_bootstrap(&state, &app_id, &event).await?;
        let page = decorate_page_actions(
            &contract.page,
            &contract.page_execution,
            &contract.manifest_revision,
        )
        .and_then(|page| redact_page_execution_routes(&page))
        .map_err(ApiError::internal_error)?;
        (
            Some(page),
            Some(contract.manifest_revision),
            Some(contract.element_demand),
        )
    } else {
        (None, None, None)
    };
    let revision = page
        .as_ref()
        .map(|page| serde_json::to_vec(page).map(|bytes| blake3::hash(&bytes).to_hex().to_string()))
        .transpose()
        .map_err(ApiError::from)?;
    let app = state.master_app(&subject, &app_id, &state).await?;
    let canonical_route = canonical_route(event.route.as_deref(), event.is_default);
    let mut event = bootstrap_event(event);
    event.board_id.clear();
    event.board_version = None;
    event.node_id.clear();
    // A custom page can belong to a trigger with private integration settings.
    if event.default_page_id.is_some() {
        event.config = serde_json::to_vec(&serde_json::json!({"frontend_hosting": config}))?;
    }
    Ok(private_json(FrontendResponse {
        app_id,
        kind,
        auth_proxy: config.requires_auth(),
        bootstrap: BootstrapResponse {
            canonical_route,
            route_miss: false,
            event,
            page,
            revision,
            execution_revision,
            element_demand,
            app_custom_css: app
                .frontend
                .as_ref()
                .and_then(|frontend| frontend.custom_css.clone()),
            served_variant,
        },
    }))
}

async fn invoke(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    Query(mut query): Query<InvokeEventQuery>,
    headers: HeaderMap,
    Json(body): Json<InvokeEventRequest>,
) -> Result<Response, ApiError> {
    let HostedRoute { event, config, .. } = load_route(&state, &app_id, &route.path()).await?;
    let caller = caller(&state, &headers, config).await?;
    let session = session_id(&headers)?;
    query.variant = variant::pin_from_request(&headers, query.variant.take());
    invoke_hosted_event(state, app_id, event, caller, session, query, body).await
}

async fn prerun(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let HostedRoute { event, config, .. } = load_route(&state, &app_id, &route.path()).await?;
    let caller = caller(&state, &headers, config).await?;
    session_id(&headers)?;
    if event.default_page_id.is_some() {
        return Err(ApiError::bad_request(
            "Page prerun requires POST with page_trigger",
        ));
    }
    let pin = variant::pin_from_request(&headers, query.variant);
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session_id(&headers)?,
    )?;
    let target = variant::resolve_invoke_target(
        &event,
        pin.as_deref(),
        None,
        &variant::SplitKeyRequest {
            pinned_variant: pin.as_deref(),
            idempotency_key: None,
            parent_run_id: None,
            trace_id: None,
            caller_subject: Some(&subject),
            run_id: &subject,
        },
    )
    .map_err(ApiError::bad_request)?;
    let manifest =
        load_prerun_manifest(&state, &app_id, &target.board_id, target.board_version).await?;
    Ok(prerun_response(build_response(
        String::new(),
        PrerunPayload::from(&*manifest),
        event.execution_mode,
        false,
        None,
        None,
    )))
}

async fn prerun_page(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
    Json(body): Json<PrerunPageEventRequest>,
) -> Result<Response, ApiError> {
    let HostedRoute { event, config, .. } = load_route(&state, &app_id, &route.path()).await?;
    let caller = caller(&state, &headers, config).await?;
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session_id(&headers)?,
    )?;
    let pin = variant::pin_from_request(&headers, query.variant);
    let resolved = resolve_authorized_page_trigger(
        &state,
        &app_id,
        &event,
        &body.page_trigger,
        pin.as_deref(),
        &subject,
        None,
    )
    .await?;
    Ok(prerun_response(build_response(
        String::new(),
        resolved.prerun,
        event.execution_mode,
        false,
        Some(resolved.page_id),
        Some(resolved.manifest_revision),
    )))
}

fn prerun_response(mut response: PrerunEventResponse) -> Response {
    // Hosted execution uses the publisher's server-side integration settings.
    response.runtime_variables.clear();
    response.oauth_requirements.clear();
    private_json(response)
}

#[derive(Serialize)]
struct PublishedRoute {
    path: String,
    event_id: String,
    kind: &'static str,
    is_default: bool,
}

#[derive(Deserialize)]
struct WidgetQuery {
    version: Option<String>,
    #[serde(rename = "__variant")]
    variant: Option<String>,
}

async fn widget(
    State(state): State<AppState>,
    Path((app_id, widget_id)): Path<(String, String)>,
    Query(route): Query<RouteQuery>,
    Query(query): Query<WidgetQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let HostedRoute { event, config, .. } = load_route(&state, &app_id, &route.path()).await?;
    let caller = caller(&state, &headers, config).await?;
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session_id(&headers)?,
    )?;
    let page = hosted_page(&state, &app_id, event, &headers, query.variant, &subject).await?;
    let version = query
        .version
        .as_deref()
        .map(|version| {
            crate::routes::app::events::parse_version_tuple(version)
                .ok_or_else(|| ApiError::bad_request("Invalid widget version"))
        })
        .transpose()?;
    let widget =
        super::frontend_widgets::read_widget(&state, &app_id, &subject, &page, &widget_id, version)
            .await?;
    // External widget definitions may still contain direct board selectors.
    let mut projection = flow_like::a2ui::widget::Page::new("hosted-widget", "", "");
    projection.widget_refs.insert("widget".to_string(), widget);
    let mut projection =
        redact_page_execution_routes(&projection).map_err(ApiError::internal_error)?;
    Ok(private_json(
        projection
            .widget_refs
            .remove("widget")
            .ok_or(ApiError::NOT_FOUND)?,
    ))
}

/// The Page exactly as bootstrap serves it to this subject.
async fn hosted_page(
    state: &AppState,
    app_id: &str,
    event: Event,
    headers: &HeaderMap,
    variant: Option<String>,
    subject: &str,
) -> Result<Page, ApiError> {
    let pin = variant::pin_from_request(headers, variant);
    let target = variant::resolve_page_target(
        &event,
        pin.as_deref(),
        &SplitKey {
            source: variant::SPLIT_SOURCE_SUBJECT,
            value: subject.to_string(),
        },
    )
    .map_err(ApiError::bad_request)?;
    let event = variant::apply_target(event, &target);
    let contract = resolve_page_contract_for_bootstrap(state, app_id, &event).await?;
    decorate_page_actions(
        &contract.page,
        &contract.page_execution,
        &contract.manifest_revision,
    )
    .and_then(|page| redact_page_execution_routes(&page))
    .map_err(ApiError::internal_error)
}

/// Component fields the a2ui renderers sign, plus the canvas background.
const PAGE_ASSET_FIELDS: &[&str] = &[
    "src",
    "url",
    "fallback",
    "poster",
    "image",
    "mapImage",
    "hdriUrl",
    "backgroundImage",
];

/// Chat appearance settings the chat interface signs.
const CONFIG_ASSET_FIELDS: &[&str] = &["background_image", "placeholder_image"];

/// Mirrors `isStorageAssetPath` + `normalizeStorageAssetPath` in
/// packages/ui/lib/asset-url-cache.ts: app-relative, no scheme, not rooted.
fn storage_asset_path(value: &str) -> Option<String> {
    const DIRECT: &[&str] = &[
        "http://", "https://", "data:", "blob:", "asset://", "tauri://", "file://",
    ];
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    if DIRECT.iter().any(|prefix| value.starts_with(prefix))
        || value.starts_with('/')
        || windows_drive
    {
        return None;
    }
    let path = value.strip_prefix("storage://").unwrap_or(value);
    (!path.is_empty()).then(|| path.to_string())
}

fn literal_string(value: &serde_json::Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .get("literalString")
            .and_then(serde_json::Value::as_str)
    })
}

fn page_asset_paths(page: &serde_json::Value) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut pending = vec![page];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, value) in fields {
                    if PAGE_ASSET_FIELDS.contains(&key.as_str())
                        && let Some(path) = literal_string(value).and_then(storage_asset_path)
                    {
                        paths.insert(path);
                    }
                    pending.push(value);
                }
            }
            serde_json::Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    paths
}

fn config_asset_paths(config: &[u8]) -> HashSet<String> {
    let config: serde_json::Value = serde_json::from_slice(config).unwrap_or_default();
    CONFIG_ASSET_FIELDS
        .iter()
        .filter_map(|field| config.get(*field).and_then(serde_json::Value::as_str))
        .filter_map(|value| storage_asset_path(value.trim()))
        .collect()
}

#[derive(Deserialize)]
struct SignedAssetsRequest {
    prefixes: Vec<String>,
}

/// Signs only storage paths the served interface names in an asset field, so
/// a hosted link never becomes a read grant on the rest of the app's files.
async fn signed_assets(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
    Json(body): Json<SignedAssetsRequest>,
) -> Result<Response, ApiError> {
    validate_batch(&body.prefixes)?;
    let HostedRoute { event, config, .. } = load_route(&state, &app_id, &route.path()).await?;
    let caller = caller(&state, &headers, config).await?;
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session_id(&headers)?,
    )?;
    let published = if event.default_page_id.is_some() {
        let page = hosted_page(&state, &app_id, event, &headers, query.variant, &subject).await?;
        page_asset_paths(&serde_json::to_value(&page)?)
    } else {
        config_asset_paths(&event.config)
    };
    let mut entries = Vec::new();
    let mut refused = Vec::new();
    for prefix in body.prefixes {
        match storage_asset_path(&prefix).filter(|path| published.contains(path)) {
            Some(path) => {
                let key = paths::resolve_app_upload(&app_id, &path);
                entries.push((prefix, key));
            }
            None => refused.push(prefix),
        }
    }
    let mut signed = Vec::new();
    if !entries.is_empty() {
        let credentials = state.master_credentials().await?;
        let ttl = credentials.signing_ttl(DOWNLOAD_URL_TTL);
        let store = credentials.to_store(false).await?;
        signed = sign_downloads(&store, entries, ttl, &subject, &app_id).await;
    }
    signed.extend(refused.into_iter().map(|prefix| {
        serde_json::json!({
            "prefix": prefix,
            "error": "This file is not part of the published interface",
        })
    }));
    Ok(private_json(signed))
}

/// The app's hosted routes that share the current route's sign-in mode.
async fn published_routes(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let events = active_app_events(&state, &app_id)
        .await?
        .into_iter()
        .map(db_model_to_event)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::internal_error)?;
    let current = route_owner(
        events
            .iter()
            .map(|event| (event.route.as_deref(), event.is_default)),
        &route.path(),
    )
    .ok_or(ApiError::NOT_FOUND)?;
    let (_, config) = hosting(&events[current])?;
    caller(&state, &headers, config).await?;
    let routes: Vec<PublishedRoute> = events
        .into_iter()
        .filter_map(|event| {
            let (kind, target) = hosting(&event).ok()?;
            if target.requires_auth() != config.requires_auth() {
                return None;
            }
            Some(PublishedRoute {
                path: canonical_route(event.route.as_deref(), event.is_default)?,
                event_id: event.id,
                kind,
                is_default: event.is_default,
            })
        })
        .collect();
    Ok(private_json(routes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::event::EventExposure;
    use serde_json::json;

    #[test]
    fn shortlinks_use_an_explicit_hosted_origin_then_the_existing_web_origin() {
        let web = Some("https://app.example");
        assert_eq!(frontend_base_url(None, web).unwrap(), "https://app.example");
        assert_eq!(
            frontend_base_url(Some(" \t"), web).unwrap(),
            "https://app.example"
        );
        assert_eq!(
            frontend_base_url(Some("https://pages.example"), web).unwrap(),
            "https://pages.example"
        );
        assert!(frontend_base_url(None, None).is_err());
        assert!(frontend_base_url(Some(""), Some(" ")).is_err());
        let invalid_override = frontend_base_url(Some("not-a-url"), web).unwrap();
        assert!(frontend_location(invalid_override, "app", "/support", None).is_err());
    }

    fn event(kind: &str) -> Event {
        Event {
            id: "event".into(),
            name: "Example".into(),
            description: String::new(),
            board_id: "private-board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "node".into(),
            variables: Default::default(),
            config: serde_json::to_vec(&json!({"frontend_hosting": {"enabled":true}})).unwrap(),
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: kind.into(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::UNIX_EPOCH,
            updated_at: std::time::UNIX_EPOCH,
            default_page_id: None,
            inputs: vec![],
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::Remote,
            exposure: EventExposure::Public,
            correlation_mappings: None,
        }
    }

    #[test]
    fn hosting_requires_explicit_valid_publication() {
        let published = event("simple_chat");
        assert!(hosting(&published).is_ok());
        let mut unpublished = published.clone();
        unpublished.config = b"{}".to_vec();
        assert!(hosting(&unpublished).is_err());
        for config in [
            json!({}),
            json!({"enabled": false}),
            json!({"enabled": true, "auth_proxy":"false"}),
            json!({"enabled": true, "allow_anonymous":"true"}),
            json!({"enabled": true, "allow_anonymous":1}),
            json!({"enabled": true, "allow_anonymous":null}),
            json!({"enabled": "true"}),
        ] {
            unpublished.config = serde_json::to_vec(&json!({"frontend_hosting":config})).unwrap();
            assert!(hosting(&unpublished).is_err());
        }
        let mut hidden = published.clone();
        hidden.active = false;
        assert!(hosting(&hidden).is_err());
        hidden.active = true;
        hidden.exposure = EventExposure::Internal;
        assert!(hosting(&hidden).is_err());
        hidden.exposure = EventExposure::Public;
        hidden.execution_mode = EventExecutionMode::Local;
        assert!(hosting(&hidden).is_err());
    }

    #[test]
    fn anonymous_hosting_requires_a_separate_explicit_opt_in() {
        assert!(FrontendHosting::default().requires_auth());
        for config in [
            json!({"enabled": true}),
            json!({"enabled": true, "auth_proxy":false}),
            json!({"enabled": true, "allow_anonymous":false}),
            json!({"enabled": true, "auth_proxy":false, "allow_anonymous":false}),
            json!({"enabled": true, "auth_proxy":true, "allow_anonymous":true}),
        ] {
            let mut published = event("simple_chat");
            published.config = serde_json::to_vec(&json!({"frontend_hosting":config})).unwrap();
            assert!(hosting(&published).unwrap().1.requires_auth());
        }
        for config in [
            json!({"enabled": true, "allow_anonymous":true}),
            json!({"enabled": true, "auth_proxy":false, "allow_anonymous":true}),
        ] {
            let mut published = event("simple_chat");
            published.config = serde_json::to_vec(&json!({"frontend_hosting":config})).unwrap();
            assert!(!hosting(&published).unwrap().1.requires_auth());
        }
        let mut unpublished = event("simple_chat");
        unpublished.config = serde_json::to_vec(&json!({
            "frontend_hosting": {"allow_anonymous": true}
        }))
        .unwrap();
        assert!(hosting(&unpublished).is_err());
    }

    #[test]
    fn the_event_decides_which_interface_a_route_serves() {
        assert_eq!(hosting(&event("simple_chat")).unwrap().0, "c");
        assert_eq!(hosting(&event("generic_form")).unwrap().0, "f");
        assert_eq!(hosting(&event("quick_action")).unwrap().0, "f");
        assert!(hosting(&event("rest")).is_err());
        assert!(hosting(&event("cron")).is_err());
        let mut page = event("generic_form");
        page.default_page_id = Some("page".into());
        assert_eq!(hosting(&page).unwrap().0, "u");
        page.event_type = "rest".into();
        assert_eq!(hosting(&page).unwrap().0, "u");
        page.event_type = "ontology_action".into();
        assert!(hosting(&page).is_err());
    }

    #[test]
    fn routes_resolve_like_use_without_falling_back() {
        let mappings = [
            (Some("/config/"), false),
            (None, true),
            (Some("topic"), false),
            (Some(" "), false),
        ];
        assert_eq!(route_owner(mappings, "/config"), Some(0));
        assert_eq!(route_owner(mappings, "/topic"), Some(2));
        assert_eq!(route_owner(mappings, "/"), Some(1));
        assert_eq!(route_owner(mappings, "/missing"), None);
        let explicit_root = [(None, true), (Some("/"), false)];
        assert_eq!(route_owner(explicit_root, "/"), Some(1));
        assert_eq!(route_owner([(Some("/Config"), false)], "/config"), None);
    }

    #[test]
    fn frontend_redirect_uses_only_the_configured_host() {
        let location = frontend_location(
            "https://static.example/",
            "app-1",
            "/support/demo",
            Some("__variant=beta&topic=a%20b"),
        )
        .unwrap();
        assert_eq!(
            location,
            "https://static.example/a/app-1/support/demo?__variant=beta&topic=a%20b"
        );
        assert_eq!(
            frontend_location("https://static.example/web/", "app-1", "/", Some("")).unwrap(),
            "https://static.example/web/a/app-1"
        );
        let location = frontend_location(
            "https://static.example/",
            "app-1",
            "//evil.example/path",
            None,
        )
        .unwrap();
        assert_eq!(
            reqwest::Url::parse(&location).unwrap().host_str(),
            Some("static.example")
        );
        assert_eq!(
            frontend_location("https://static.example/", "app-1", "/a b/../x", None).unwrap(),
            "https://static.example/a/app-1/a%20b/x"
        );
        for base in [
            "//evil.example",
            "javascript:alert(1)",
            "https://user:secret@static.example",
            "https://static.example/?redirect=bad",
        ] {
            assert!(frontend_location(base, "app-1", "/test", None).is_err());
        }
    }

    #[test]
    fn only_asset_fields_of_the_served_page_are_signable() {
        let page = json!({
            "components": [
                {"id": "logo", "component": {"type": "image", "src": {"literalString": "assets/mopo.jpg"}}},
                {"id": "clip", "component": {"type": "video", "src": {"literalString": "storage://media/clip.mp4"}, "poster": {"path": "/poster"}}},
                {"id": "remote", "component": {"type": "image", "src": {"literalString": "https://cdn.example/a.png"}}},
                {"id": "note", "component": {"type": "text", "text": {"literalString": "private/contracts.pdf"}}},
            ],
            "widgetRefs": {"w": {"components": [
                {"id": "nested", "component": {"type": "avatar", "src": {"literalString": "people/ada.webp"}}}
            ]}},
            "canvasSettings": {"backgroundImage": "backgrounds/hero.jpg"},
        });
        let paths = page_asset_paths(&page);
        let expected: HashSet<String> = [
            "assets/mopo.jpg",
            "media/clip.mp4",
            "people/ada.webp",
            "backgrounds/hero.jpg",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn storage_asset_paths_match_the_client_rules() {
        assert_eq!(
            storage_asset_path("storage://a/b.png").as_deref(),
            Some("a/b.png")
        );
        assert_eq!(storage_asset_path("a/b.png").as_deref(), Some("a/b.png"));
        for direct in [
            "",
            "storage://",
            "/images/x.png",
            "C:\\x.png",
            "data:image/png;base64,AA",
            "blob:https://x/1",
            "asset://localhost/x",
            "https://x/y.png",
        ] {
            assert_eq!(storage_asset_path(direct), None, "{direct}");
        }
        let config = serde_json::to_vec(&json!({
            "background_image": " bg/chat.jpg ",
            "placeholder_image": "https://cdn.example/p.png",
            "knowledge_base": "private/docs.pdf",
        }))
        .unwrap();
        assert_eq!(
            config_asset_paths(&config),
            HashSet::from(["bg/chat.jpg".to_string()])
        );
    }

    #[test]
    fn session_ids_are_bounded_and_path_safe() {
        let mut headers = HeaderMap::new();
        assert!(session_id(&headers).is_err());
        for invalid in [
            "short",
            "../../../../../../../../secret",
            "invalid session value",
        ] {
            headers.insert("x-flow-like-session", invalid.parse().unwrap());
            assert!(session_id(&headers).is_err());
        }
        headers.insert(
            "x-flow-like-session",
            "12345678-abcd-1234-abcd-123456789012".parse().unwrap(),
        );
        assert!(session_id(&headers).is_ok());
    }
}

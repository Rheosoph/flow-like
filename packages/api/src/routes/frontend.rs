//! Runtime API for the statically hosted chat, form and custom page surfaces.
//! Each request rechecks publication. A static asset or an alias grants no
//! access to other events, boards, or the app's private resources.

use axum::{
    Json, Router,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use flow_like::flow::{
    compiled::prerun::{decorate_page_actions, redact_page_execution_routes},
    event::{Event, EventExecutionMode},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::{
    entity::event,
    error::ApiError,
    execution::variant::{self, SplitKey},
    middleware::jwt::{AppUser, OpenIDUser, viewer_authorization},
    routes::app::{
        events::{
            db::{db_model_to_event, filter_event_list_execution, filter_event_secrets},
            invoke_event::{
                InvokeEventQuery, InvokeEventRequest, hosted_subject, invoke_hosted_event,
            },
            page_trigger::{resolve_authorized_page_trigger, resolve_page_contract_for_bootstrap},
            prerun_event::{PrerunEventResponse, PrerunPageEventRequest, build_response},
        },
        page::bootstrap::BootstrapResponse,
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

#[derive(Serialize)]
struct FrontendResponse {
    app_id: String,
    auth_proxy: bool,
    bootstrap: BootstrapResponse,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{kind}/{slug_or_id}", get(bootstrap))
        .route("/{kind}/{slug_or_id}/invoke", axum::routing::post(invoke))
        .route("/{kind}/{slug_or_id}/prerun", get(prerun).post(prerun_page))
        .route("/{kind}/{slug_or_id}/routes", get(published_routes))
        .route("/{kind}/{slug_or_id}/widgets/{widget_id}", get(widget))
}

pub fn link_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/c/{slug}",
            get(
                |State(state): State<AppState>,
                 Path(slug): Path<String>,
                 RawQuery(query): RawQuery| async move {
                    frontend_link(state, "c", slug, query).await
                },
            ),
        )
        .route(
            "/f/{slug}",
            get(
                |State(state): State<AppState>,
                 Path(slug): Path<String>,
                 RawQuery(query): RawQuery| async move {
                    frontend_link(state, "f", slug, query).await
                },
            ),
        )
        .route(
            "/u/{slug}",
            get(
                |State(state): State<AppState>,
                 Path(slug): Path<String>,
                 RawQuery(query): RawQuery| async move {
                    frontend_link(state, "u", slug, query).await
                },
            ),
        )
}

fn frontend_location(
    base: &str,
    kind: &str,
    slug: &str,
    query: Option<&str>,
) -> Result<String, ApiError> {
    namespace(kind)?;
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
        .push(kind)
        .push(slug);
    url.set_query(query);
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
    kind: &str,
    slug: String,
    query: Option<String>,
) -> Result<Response, ApiError> {
    load_event(&state, kind, &slug).await?;
    let hosted_base = std::env::var("FRONTEND_BASE_URL").ok();
    let web_base = std::env::var("FRONTEND_URL").ok();
    let base = frontend_base_url(hosted_base.as_deref(), web_base.as_deref())?;
    let location = frontend_location(base, kind, &slug, query.as_deref())?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Redirect::temporary(&location),
    )
        .into_response())
}

fn namespace(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "c" => Ok("simple_chat"),
        "f" => Ok("generic_form"),
        "u" => Ok("page"),
        _ => Err(ApiError::NOT_FOUND),
    }
}

fn hosting(event: &Event, kind: &str) -> Result<FrontendHosting, ApiError> {
    let interface = if event.default_page_id.is_some() {
        "page"
    } else {
        event_alias::alias_event_type(&event.event_type, None)
    };
    if !event.active
        || event.exposure.is_internal()
        || event.execution_mode != EventExecutionMode::Remote
        || event.event_type == "ontology_action"
        || interface != namespace(kind)?
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
    Ok(hosting)
}

async fn load_event(
    state: &AppState,
    kind: &str,
    slug: &str,
) -> Result<(String, Event, FrontendHosting), ApiError> {
    let resolved =
        event_alias::resolve_for_event_type(&state.db, slug, None, namespace(kind)?).await?;
    let row = match resolved.event {
        Some(row) => row,
        None => event::Entity::find_by_id(&resolved.event_id)
            .one(&state.db)
            .await?
            .filter(|row| row.app_id == resolved.app_id)
            .ok_or(ApiError::NOT_FOUND)?,
    };
    let event = db_model_to_event(row).map_err(ApiError::internal_error)?;
    let config = hosting(&event, kind)?;
    Ok((resolved.app_id, event, config))
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
    Path((kind, slug)): Path<(String, String)>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (app_id, event, config) = load_event(&state, &kind, &slug).await?;
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
    let canonical_route = event.route.clone();
    let mut event = filter_event_list_execution(filter_event_secrets(event));
    event.board_id.clear();
    event.board_version = None;
    event.node_id.clear();
    event.correlation_mappings = None;
    // A custom page can belong to a trigger with private integration settings.
    if event.default_page_id.is_some() {
        event.config = serde_json::to_vec(&serde_json::json!({"frontend_hosting": config}))?;
    }
    Ok(private_json(FrontendResponse {
        app_id,
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
    Path((kind, slug)): Path<(String, String)>,
    Query(mut query): Query<InvokeEventQuery>,
    headers: HeaderMap,
    Json(body): Json<InvokeEventRequest>,
) -> Result<Response, ApiError> {
    let (app_id, event, config) = load_event(&state, &kind, &slug).await?;
    let caller = caller(&state, &headers, config).await?;
    let session = session_id(&headers)?;
    query.variant = variant::pin_from_request(&headers, query.variant.take());
    invoke_hosted_event(state, app_id, event, caller, session, query, body).await
}

async fn prerun(
    State(state): State<AppState>,
    Path((kind, slug)): Path<(String, String)>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (app_id, event, config) = load_event(&state, &kind, &slug).await?;
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
    Path((kind, slug)): Path<(String, String)>,
    Query(query): Query<FrontendQuery>,
    headers: HeaderMap,
    Json(body): Json<PrerunPageEventRequest>,
) -> Result<Response, ApiError> {
    let (app_id, event, config) = load_event(&state, &kind, &slug).await?;
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
    Path((kind, slug, widget_id)): Path<(String, String, String)>,
    Query(query): Query<WidgetQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (app_id, event, config) = load_event(&state, &kind, &slug).await?;
    let caller = caller(&state, &headers, config).await?;
    let subject = hosted_subject(
        &event.id,
        caller.as_ref().unwrap_or(&AppUser::Unauthorized),
        session_id(&headers)?,
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
    let event = variant::apply_target(event, &target);
    let contract = resolve_page_contract_for_bootstrap(&state, &app_id, &event).await?;
    let page = decorate_page_actions(
        &contract.page,
        &contract.page_execution,
        &contract.manifest_revision,
    )
    .and_then(|page| redact_page_execution_routes(&page))
    .map_err(ApiError::internal_error)?;
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

async fn published_routes(
    State(state): State<AppState>,
    Path((kind, slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (app_id, _, config) = load_event(&state, &kind, &slug).await?;
    caller(&state, &headers, config).await?;
    let events = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Active.eq(true))
        .all(&state.db)
        .await?;
    let mut routes = Vec::new();
    for row in events {
        let event = db_model_to_event(row).map_err(ApiError::internal_error)?;
        let kind = if event.default_page_id.is_some() {
            "u"
        } else if event.event_type == "simple_chat" {
            "c"
        } else {
            "f"
        };
        let Ok(target_config) = hosting(&event, kind) else {
            continue;
        };
        if target_config.requires_auth() != config.requires_auth() {
            continue;
        }
        let Some(path) = event
            .route
            .or_else(|| event.is_default.then(|| "/".to_string()))
        else {
            continue;
        };
        routes.push(PublishedRoute {
            path,
            event_id: event.id,
            kind,
            is_default: event.is_default,
        });
    }
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
        assert!(frontend_location(invalid_override, "c", "support", None).is_err());
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
        assert!(hosting(&published, "c").is_ok());
        let mut unpublished = published.clone();
        unpublished.config = b"{}".to_vec();
        assert!(hosting(&unpublished, "c").is_err());
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
            assert!(hosting(&unpublished, "c").is_err());
        }
        let mut hidden = published.clone();
        hidden.active = false;
        assert!(hosting(&hidden, "c").is_err());
        hidden.active = true;
        hidden.exposure = EventExposure::Internal;
        assert!(hosting(&hidden, "c").is_err());
        hidden.exposure = EventExposure::Public;
        hidden.execution_mode = EventExecutionMode::Local;
        assert!(hosting(&hidden, "c").is_err());
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
            assert!(hosting(&published, "c").unwrap().requires_auth());
        }
        for config in [
            json!({"enabled": true, "allow_anonymous":true}),
            json!({"enabled": true, "auth_proxy":false, "allow_anonymous":true}),
        ] {
            let mut published = event("simple_chat");
            published.config = serde_json::to_vec(&json!({"frontend_hosting":config})).unwrap();
            assert!(!hosting(&published, "c").unwrap().requires_auth());
        }
        let mut unpublished = event("simple_chat");
        unpublished.config = serde_json::to_vec(&json!({
            "frontend_hosting": {"allow_anonymous": true}
        }))
        .unwrap();
        assert!(hosting(&unpublished, "c").is_err());
    }

    #[test]
    fn interfaces_cannot_be_cross_addressed() {
        assert!(hosting(&event("simple_chat"), "f").is_err());
        assert!(hosting(&event("generic_form"), "c").is_err());
        assert!(hosting(&event("quick_action"), "f").is_ok());
        let mut page = event("generic_form");
        page.default_page_id = Some("page".into());
        assert!(hosting(&page, "u").is_ok());
        assert!(hosting(&page, "f").is_err());
        page.event_type = "rest".into();
        assert!(hosting(&page, "u").is_ok());
        page.event_type = "ontology_action".into();
        assert!(hosting(&page, "u").is_err());
    }

    #[test]
    fn frontend_redirect_uses_only_the_configured_host() {
        let location = frontend_location(
            "https://static.example/",
            "c",
            "support-demo",
            Some("__variant=beta"),
        )
        .unwrap();
        assert_eq!(
            location,
            "https://static.example/c/support-demo?__variant=beta"
        );
        let location =
            frontend_location("https://static.example/", "u", "//evil.example/path", None).unwrap();
        assert_eq!(
            reqwest::Url::parse(&location).unwrap().host_str(),
            Some("static.example")
        );
        for base in [
            "//evil.example",
            "javascript:alert(1)",
            "https://user:secret@static.example",
            "https://static.example/?redirect=bad",
        ] {
            assert!(frontend_location(base, "c", "test", None).is_err());
        }
        assert!(frontend_location("https://static.example", "admin", "test", None).is_err());
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

//! Inbound REST + MCP routers.
//!
//! These routers translate untrusted external HTTP traffic into event
//! dispatches. They live outside `/api/v1/apps/...` because they don't use
//! the standard JWT middleware — authentication is per-registration and
//! enforced from `EventRemoteAuth` config (locked decision: inbound REST
//! does NOT require the caller to hold the `ExecuteEvents` permission).
//!
//! ```text
//! ANY  /r/{slug_or_id}           inbound REST — root path ("/")
//! ANY  /r/{slug_or_id}/{*path}   inbound REST — function/file/openapi
//! ANY  /m/{slug_or_id}           inbound MCP streamable HTTP
//! ```
//!
//! The inbound routers are wrapped with CORS, decompression, compression,
//! body-size and error-reporting layers in `lib.rs` (without the JWT
//! middleware).

use std::{collections::HashMap, convert::Infallible, str::FromStr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::any,
};
use flow_like::flow::{
    board::Board,
    node::Node,
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use jsonwebtoken::{
    Algorithm,
    jwk::{AlgorithmParameters, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use crate::{
    correlation::CorrelationContext,
    credentials::validate_path_component,
    entity::{
        event, event_remote_auth, event_remote_registration, event_sink, execution_run,
        prelude::EventRemoteAuth,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        DispatchRequest, ExecutionBackend, ExecutionJwtParams, TokenType, collect_generic_result,
        collect_generic_result_bytes, is_jwt_configured, resolve_wasm_packages, sign_execution_jwt,
    },
    routes::{
        app::events::db::{db_model_to_event, decrypt_token},
        sink::trigger::{maybe_refresh_oauth_tokens, resolve_sink_pat_user_id},
    },
    state::AppState,
    utils::event_alias as alias_util,
};

/// Identity of the run that reached this event through the app-connection
/// proxy. Threaded into the dispatched run so cross-app REST/MCP hops stay
/// part of the caller's process case (chain, parent run, correlation) instead
/// of showing up as unrelated root runs. Public inbound traffic uses
/// `ProxyCallerContext::default()` — those runs are genuine roots.
#[derive(Clone, Default)]
pub(crate) struct ProxyCallerContext {
    pub app_chain: Option<Vec<String>>,
    pub parent_run_id: Option<String>,
    pub correlation: Option<CorrelationContext>,
}

const INBOUND_RESULT_TIMEOUT: Duration = Duration::from_secs(120);
const MCP_DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const MCP_WELL_KNOWN_OAUTH_PATH: &str = "/.well-known/oauth-protected-resource";
const MCP_BROWSER_INSPECTOR_TEMPLATE: &str = include_str!("../../../../assets/mcp-inspector.html");
/// The API bearer token occupies `Authorization` on app-connection proxy
/// requests. Callers put registration-level Basic/Bearer/OAuth credentials in
/// this header instead; the proxy restores them to `Authorization` only for
/// the event registration auth check. Single definition shared with the
/// catalog nodes that send it (any rename here changes both sides at once).
/// NOTE: the MCP CORS `Access-Control-Allow-Headers` list in
/// `mcp_options_response` must include this header name verbatim.
pub(crate) use flow_like_types::PROXY_EVENT_AUTHORIZATION_HEADER;

pub fn rest_routes() -> Router<AppState> {
    Router::new()
        .route("/{slug_or_id}", any(inbound_rest_root))
        .route("/{slug_or_id}/{*path}", any(inbound_rest))
}

pub fn mcp_routes() -> Router<AppState> {
    Router::new()
        .route("/{slug_or_id}", any(inbound_mcp_root))
        .route("/{slug_or_id}/{*path}", any(inbound_mcp))
}

#[tracing::instrument(name = "INBOUND /r/{slug}", skip(state, headers, body))]
async fn inbound_rest_root(
    State(state): State<AppState>,
    Path(slug_or_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    match dispatch_inbound_rest(
        &state,
        &slug_or_id,
        "",
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
    )
    .await
    {
        Ok(resp) => resp,
        Err(api_err) => api_err.into_response(),
    }
}

#[tracing::instrument(name = "INBOUND /r/{slug}/{path}", skip(state, headers, body))]
async fn inbound_rest(
    State(state): State<AppState>,
    Path((slug_or_id, path)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    match dispatch_inbound_rest(
        &state,
        &slug_or_id,
        &path,
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
    )
    .await
    {
        Ok(resp) => resp,
        Err(api_err) => api_err.into_response(),
    }
}

#[tracing::instrument(name = "INBOUND /m/{slug}", skip(state, headers, body))]
async fn inbound_mcp_root(
    State(state): State<AppState>,
    Path(slug_or_id): Path<String>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    match dispatch_inbound_mcp(
        &state,
        &slug_or_id,
        "",
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
    )
    .await
    {
        Ok(resp) => resp,
        Err(api_err) => api_err.into_response(),
    }
}

#[tracing::instrument(name = "INBOUND /m/{slug}/{path}", skip(state, headers, body))]
async fn inbound_mcp(
    State(state): State<AppState>,
    Path((slug_or_id, path)): Path<(String, String)>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    match dispatch_inbound_mcp(
        &state,
        &slug_or_id,
        &path,
        raw_query.as_deref().unwrap_or(""),
        &headers,
        &method,
        &body,
    )
    .await
    {
        Ok(resp) => resp,
        Err(api_err) => api_err.into_response(),
    }
}

/// Enforces the event's exposure against the surface it was reached on.
/// `is_public_surface` is true for the public inbound routers and false for
/// the authenticated app-connection proxy.
///
/// - A `PUBLIC` event on the proxy → 403 (call it via its public endpoint).
/// - An `INTERNAL` event on the public router → 404 (never publicly exposed;
///   404 rather than 403 so its existence is not revealed).
fn enforce_exposure(event_row: &event::Model, is_public_surface: bool) -> Result<(), Response> {
    let internal = flow_like::flow::event::EventExposure::parse(&event_row.exposure).is_internal();
    if is_public_surface && internal {
        return Err((StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response());
    }
    if !is_public_surface && !internal {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "This event is public; call it through its public endpoint, not the app-connection proxy"
            })),
        )
            .into_response());
    }
    Ok(())
}

/// Headers to use for the registration auth check and downstream forwarding.
/// On the public surface this is the request's own header map (borrowed, no
/// allocation on the hot path); on the internal proxy surface the forwarded
/// registration credential is restored to `Authorization` in an owned copy.
fn registration_auth_headers(
    headers: &HeaderMap,
    is_public_surface: bool,
) -> std::borrow::Cow<'_, HeaderMap> {
    if is_public_surface {
        return std::borrow::Cow::Borrowed(headers);
    }
    let mut auth_headers = headers.clone();
    let registration_authorization = auth_headers.remove(PROXY_EVENT_AUTHORIZATION_HEADER);
    auth_headers.remove(axum::http::header::AUTHORIZATION);
    if let Some(value) = registration_authorization {
        auth_headers.insert(axum::http::header::AUTHORIZATION, value);
    }
    std::borrow::Cow::Owned(auth_headers)
}

async fn dispatch_inbound_rest(
    state: &AppState,
    slug_or_id: &str,
    path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    method: &axum::http::Method,
    body: &Bytes,
) -> Result<Response, ApiError> {
    let resolved = alias_util::resolve_for_event_type(&state.db, slug_or_id, None, "rest").await?;

    // Load event row to get last_setup_version + node_id (entry).
    let event_row = event::Entity::find_by_id(&resolved.event_id)
        .filter(event::Column::AppId.eq(&resolved.app_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found("event not found"))?;

    dispatch_rest_for_event(
        state,
        &event_row,
        slug_or_id,
        path,
        raw_query,
        headers,
        method,
        body,
        true,
        None,
        &ProxyCallerContext::default(),
    )
    .await
}

/// Matches and executes a REST registration of an event. Used by the public
/// inbound router and by the authenticated app-connection proxy. Both surfaces
/// enforce configured per-registration auth. The surface flag only controls
/// PUBLIC/INTERNAL exposure; `injected_auth` is emitted separately as
/// `_client.proxy` for caller attribution.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_rest_for_event(
    state: &AppState,
    event_row: &event::Model,
    public_slug: &str,
    path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    method: &axum::http::Method,
    body: &Bytes,
    is_public_surface: bool,
    injected_auth: Option<Value>,
    caller: &ProxyCallerContext,
) -> Result<Response, ApiError> {
    if !event_row.active || event_row.event_type != "rest" {
        return Err(ApiError::not_found("REST event not found or inactive"));
    }

    if let Err(response) = enforce_exposure(&event_row, is_public_surface) {
        return Ok(response);
    }

    let resolved = alias_util::ResolvedAlias {
        event_id: event_row.id.clone(),
        app_id: event_row.app_id.clone(),
    };
    let slug_or_id = public_slug;

    let version = event_row.last_setup_version.clone().ok_or_else(|| {
        ApiError::not_found("event has no completed setup; call POST /setup first")
    })?;

    // Normalize path so `/foo` and `foo` match the same row.
    let normalized = normalize_inbound_path(path);

    // Auto-handle CORS pre-flight: respond with whatever methods the user
    // has registered for this path.
    if method == axum::http::Method::OPTIONS {
        return Ok(build_options_response(state, &resolved, &version, &normalized).await);
    }

    // Look up matching registration. Exact match wins over templated.
    let (registration, path_params) = match_registration(
        state,
        &resolved.app_id,
        &resolved.event_id,
        &version,
        method,
        &normalized,
    )
    .await?
    .ok_or_else(|| {
        ApiError::not_found(format!("no registration matches {} {}", method, normalized))
    })?;
    let registration_headers = registration_auth_headers(headers, is_public_surface);

    // A connection role authorizes access to the proxy, but does not replace
    // auth explicitly configured on the matched event registration.
    let auth_claims = if let Some(auth_id) = &registration.auth_id {
        let auth = EventRemoteAuth::find_by_id(auth_id)
            .one(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
            .ok_or_else(|| {
                ApiError::internal_error(flow_like_types::anyhow!("dangling auth_id"))
            })?;
        verify_inbound_auth(
            state,
            &auth,
            &registration_headers,
            method,
            &normalized,
            body,
        )
        .await?
    } else {
        None
    };
    let client = client_metadata("rest", &registration_headers, auth_claims, injected_auth);

    match registration.kind.as_str() {
        "rest_fn" => {
            let result = dispatch_rest_fn(
                state,
                event_row,
                &registration,
                &normalized,
                raw_query,
                &path_params,
                &registration_headers,
                method,
                body,
                client,
                caller,
            )
            .await?;
            Ok(materialize_response(result))
        }
        "rest_openapi" => {
            // `extras_json` is populated at setup-time with the actual
            // OpenAPI spec (see `expand_rest_config`). Older rows that
            // pre-date the spec generation fall back to a minimal stub.
            let spec = registration
                .extras_json
                .clone()
                .and_then(|v| v.get("spec").cloned())
                .unwrap_or_else(|| json!({"openapi": "3.1.0", "info": {"title": "Flow Like REST", "version": "0.0.0"}, "paths": {}}));
            let spec = with_inbound_openapi_server(spec, slug_or_id);
            let mut resp = Json(spec).into_response();
            resp.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static(
                    "application/vnd.oai.openapi+json; charset=utf-8",
                ),
            );
            Ok(resp)
        }
        "rest_openapi_ui" => {
            let spec_path = registration
                .extras_json
                .as_ref()
                .and_then(|v| v.get("spec_path"))
                .and_then(|v| v.as_str())
                .map(normalize_inbound_path)
                .unwrap_or_else(|| "/openapi.json".to_string());
            let spec_url = format!("{}{}", inbound_base_path(slug_or_id), spec_path);
            let mut resp = axum::response::Html(openapi_ui_html(&spec_url)).into_response();
            resp.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            Ok(resp)
        }
        "rest_file" => dispatch_rest_file(state, &registration, &path_params, method).await,
        other => Ok((
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": format!("inbound dispatch for kind '{other}' not implemented")
            })),
        )
            .into_response()),
    }
}

async fn dispatch_inbound_mcp(
    state: &AppState,
    slug_or_id: &str,
    path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    method: &axum::http::Method,
    body: &Bytes,
) -> Result<Response, ApiError> {
    let resolved = alias_util::resolve_for_event_type(&state.db, slug_or_id, None, "mcp").await?;
    let event_row = event::Entity::find_by_id(&resolved.event_id)
        .filter(event::Column::AppId.eq(&resolved.app_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found("event not found"))?;

    dispatch_mcp_for_event(
        state,
        &event_row,
        slug_or_id,
        path,
        raw_query,
        headers,
        method,
        body,
        true,
        None,
        &ProxyCallerContext::default(),
    )
    .await
}

/// Serves the MCP surface of an event. Both the public router and authenticated
/// app-connection proxy enforce configured per-registration auth. The surface
/// flag only controls PUBLIC/INTERNAL exposure.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_mcp_for_event(
    state: &AppState,
    event_row: &event::Model,
    public_slug: &str,
    path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    method: &axum::http::Method,
    body: &Bytes,
    is_public_surface: bool,
    injected_auth: Option<Value>,
    caller: &ProxyCallerContext,
) -> Result<Response, ApiError> {
    if !event_row.active || event_row.event_type != "mcp" {
        return Err(ApiError::not_found("MCP event not found or inactive"));
    }

    if let Err(response) = enforce_exposure(&event_row, is_public_surface) {
        return Ok(response);
    }

    let slug_or_id = public_slug;

    let version = event_row.last_setup_version.clone().ok_or_else(|| {
        ApiError::not_found("event has no completed setup; call POST /setup first")
    })?;

    let normalized = normalize_inbound_path(path);
    let registration_headers = registration_auth_headers(headers, is_public_surface);

    if method == axum::http::Method::OPTIONS {
        return Ok(mcp_options_response(&registration_headers));
    }

    let registration = event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(&event_row.app_id))
        .filter(event_remote_registration::Column::EventId.eq(&event_row.id))
        .filter(event_remote_registration::Column::EventVersion.eq(&version))
        .filter(event_remote_registration::Column::Kind.eq("mcp_raw"))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found("no MCP registration found"))?;
    let config = registration
        .extras_json
        .clone()
        .unwrap_or_else(|| json!({}));

    let endpoint_path = if is_public_surface {
        format!("/m/{}", urlencoding::encode(slug_or_id))
    } else {
        format!(
            "/api/v1/apps/{}/events/{}/mcp",
            urlencoding::encode(&event_row.app_id),
            urlencoding::encode(&event_row.id)
        )
    };
    let resource_url = mcp_resource_url(&registration_headers, &endpoint_path);

    if normalized == MCP_WELL_KNOWN_OAUTH_PATH && method == axum::http::Method::GET {
        return Ok(mcp_oauth_metadata_response(
            &registration_headers,
            &config,
            &resource_url,
        ));
    }

    if normalized != "/" {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "MCP endpoint not found"})),
        )
            .into_response());
    }

    let auth_claims = if let Some(auth_id) = &registration.auth_id {
        let auth = EventRemoteAuth::find_by_id(auth_id)
            .one(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
            .ok_or_else(|| {
                ApiError::internal_error(flow_like_types::anyhow!("dangling auth_id"))
            })?;
        verify_inbound_auth(
            state,
            &auth,
            &registration_headers,
            method,
            &normalized,
            body,
        )
        .await?
    } else {
        None
    };
    let client = client_metadata("mcp", &registration_headers, auth_claims, injected_auth);

    if method == axum::http::Method::POST {
        mcp_handle_post(
            state,
            event_row,
            &config,
            raw_query,
            &registration_headers,
            body,
            client,
            caller,
        )
        .await
    } else if method == axum::http::Method::DELETE {
        Ok(mcp_handle_delete(raw_query, &registration_headers).await)
    } else if method == axum::http::Method::GET {
        Ok(mcp_handle_get(event_row, &endpoint_path, raw_query, &registration_headers).await)
    } else {
        let mut resp = (
            StatusCode::METHOD_NOT_ALLOWED,
            Json(json!({"error": "Method Not Allowed"})),
        )
            .into_response();
        resp.headers_mut().insert(
            axum::http::header::ALLOW,
            axum::http::HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        apply_mcp_cors(resp.headers_mut(), &registration_headers);
        Ok(resp)
    }
}

fn normalize_inbound_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{path}")
    }
}

fn inbound_base_path(slug_or_id: &str) -> String {
    format!("/r/{}", urlencoding::encode(slug_or_id))
}

fn with_inbound_openapi_server(mut spec: Value, slug_or_id: &str) -> Value {
    let Some(obj) = spec.as_object_mut() else {
        return spec;
    };

    obj.insert(
        "servers".to_string(),
        json!([{
            "url": inbound_base_path(slug_or_id),
            "description": "Flow-Like inbound REST endpoint"
        }]),
    );
    spec
}

fn openapi_ui_html(spec_path: &str) -> String {
    let spec_url =
        serde_json::to_string(spec_path).unwrap_or_else(|_| "\"/openapi.json\"".to_string());
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Flow Like REST API</title>
  <link rel="icon" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/favicon-32x32.png">
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
  <style>
    html, body {{
      margin: 0;
      min-height: 100%;
      background: #ffffff;
    }}
    #swagger-ui .topbar {{
      background-color: #111827;
    }}
    #swagger-ui .topbar .download-url-wrapper .select-label {{
      color: #e5e7eb;
    }}
  </style>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js" crossorigin></script>
  <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-standalone-preset.js" crossorigin></script>
  <script>
    window.addEventListener("load", function () {{
      window.ui = SwaggerUIBundle({{
        url: {spec_url},
        dom_id: "#swagger-ui",
        deepLinking: true,
        displayRequestDuration: true,
        filter: true,
        persistAuthorization: true,
        showCommonExtensions: true,
        showExtensions: true,
        tryItOutEnabled: true,
        presets: [
          SwaggerUIBundle.presets.apis,
          SwaggerUIStandalonePreset
        ],
        plugins: [
          SwaggerUIBundle.plugins.DownloadUrl
        ],
        layout: "StandaloneLayout"
      }});
    }});
  </script>
</body>
</html>"##
    )
}

/// Build a CORS pre-flight response describing the methods the user has
/// registered for `normalized_path` on this event. Always permissive on
/// origin so browser-side callers don't need extra setup.
async fn build_options_response(
    state: &AppState,
    resolved: &alias_util::ResolvedAlias,
    version: &str,
    normalized_path: &str,
) -> Response {
    let rows = match event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(&resolved.app_id))
        .filter(event_remote_registration::Column::EventId.eq(&resolved.event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(version))
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return StatusCode::NO_CONTENT.into_response(),
    };

    let mut methods: Vec<String> = rows
        .iter()
        .filter(|r| r.kind == "rest_fn" && registration_matches_path(r, normalized_path))
        .filter_map(|r| r.method.clone())
        .flat_map(|m| {
            if m.eq_ignore_ascii_case("ANY") {
                vec![
                    "GET".to_string(),
                    "POST".to_string(),
                    "PUT".to_string(),
                    "PATCH".to_string(),
                    "DELETE".to_string(),
                    "HEAD".to_string(),
                ]
            } else {
                vec![m.to_uppercase()]
            }
        })
        .collect();
    if rows
        .iter()
        .any(|r| r.kind == "rest_file" && rest_file_registration_matches_path(r, normalized_path))
    {
        methods.push("GET".to_string());
        methods.push("HEAD".to_string());
    }
    if rows
        .iter()
        .any(|r| openapi_registration_matches_get_path(r, normalized_path))
    {
        methods.push("GET".to_string());
    }
    methods.sort();
    methods.dedup();
    methods.push("OPTIONS".to_string());

    let allow = methods.join(", ");
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let h = resp.headers_mut();
    if let Ok(v) = axum::http::HeaderValue::from_str(&allow) {
        h.insert(axum::http::header::ALLOW, v.clone());
        h.insert("access-control-allow-methods", v);
    }
    h.insert(
        "access-control-allow-origin",
        axum::http::HeaderValue::from_static("*"),
    );
    h.insert(
        "access-control-allow-headers",
        axum::http::HeaderValue::from_static("*"),
    );
    h.insert(
        "access-control-max-age",
        axum::http::HeaderValue::from_static("600"),
    );
    resp
}

fn openapi_registration_matches_get_path(
    r: &event_remote_registration::Model,
    normalized_path: &str,
) -> bool {
    match r.kind.as_str() {
        "rest_openapi" => {
            r.path == normalized_path
                || openapi_ui_path_from_registration(r).as_deref() == Some(normalized_path)
        }
        "rest_openapi_ui" => r.path == normalized_path,
        _ => false,
    }
}

fn openapi_ui_path_from_registration(r: &event_remote_registration::Model) -> Option<String> {
    r.extras_json
        .as_ref()
        .and_then(|extras| {
            extras
                .get("ui_path")
                .or_else(|| extras.get("route").and_then(|route| route.get("ui_path")))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(normalize_inbound_path)
}

fn openapi_ui_extras(r: &event_remote_registration::Model) -> Value {
    let mut extras = r.extras_json.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = extras.as_object_mut() {
        obj.insert("spec_path".to_string(), Value::String(r.path.clone()));
        extras
    } else {
        json!({ "spec_path": r.path.clone() })
    }
}

/// Match a normalized path against either an exact registration path or a
/// `{name}` template. Returns true only if the segment counts match.
fn registration_matches_path(r: &event_remote_registration::Model, path: &str) -> bool {
    if r.path == path {
        return true;
    }
    match_template(&r.path, path).is_some()
}

/// Match a `{name}` template against a concrete path. Returns the
/// captured params (empty when there are none, `None` when the template
/// does not match).
fn match_template(template: &str, path: &str) -> Option<HashMap<String, String>> {
    let t_parts: Vec<&str> = template
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let p_parts: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if t_parts.len() != p_parts.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (tseg, pseg) in t_parts.iter().zip(p_parts.iter()) {
        if let Some(name) = tseg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let value = urlencoding::decode(pseg)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| (*pseg).to_string());
            params.insert(name.to_string(), value);
        } else if tseg != pseg {
            return None;
        }
    }
    Some(params)
}

fn rest_file_route_prefix(path: &str) -> Option<String> {
    let path = normalize_inbound_path(path);
    let prefix = path.strip_suffix("/{filename}")?;
    Some(if prefix.is_empty() {
        "/".to_string()
    } else {
        prefix.to_string()
    })
}

fn normalize_rest_file_mount_path(path: String) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn rest_file_mount_path(path: &str) -> String {
    normalize_rest_file_mount_path(
        rest_file_route_prefix(path).unwrap_or_else(|| normalize_inbound_path(path)),
    )
}

fn rest_file_is_directory_registration(r: &event_remote_registration::Model) -> bool {
    r.extras_json
        .as_ref()
        .and_then(|extras| extras.get("directory"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || rest_file_route_prefix(&r.path).is_some()
}

fn rest_file_subpath_for_request(route_path: &str, normalized_path: &str) -> Option<String> {
    let mount = rest_file_mount_path(route_path);
    let subpath = if mount == "/" {
        normalized_path.strip_prefix('/')?
    } else if normalized_path == mount {
        ""
    } else {
        normalized_path.strip_prefix(&format!("{}/", mount.trim_end_matches('/')))?
    };
    Some(subpath.to_string())
}

fn rest_file_registration_matches_path(
    r: &event_remote_registration::Model,
    normalized_path: &str,
) -> bool {
    if !rest_file_is_directory_registration(r) {
        return r.path == normalized_path;
    }
    rest_file_subpath_for_request(&r.path, normalized_path).is_some()
}

fn app_scoped_content_path(
    app_id: &str,
    flow_path: &str,
    subpath: Option<&str>,
) -> Result<flow_like_storage::Path, ApiError> {
    validate_path_component(app_id, "app_id")
        .map_err(|_| ApiError::internal("event has invalid app id"))?;

    let app_prefix = format!("apps/{app_id}");
    let mut relative = flow_path
        .trim()
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();
    if relative == app_prefix {
        relative.clear();
    } else if let Some(rest) = relative.strip_prefix(&format!("{app_prefix}/")) {
        relative = rest.to_string();
    }

    let mut path = flow_like_storage::Path::from("apps").child(app_id);
    if !relative.is_empty() {
        path = append_object_path_segments(path, relative.as_str());
    }
    if let Some(subpath) = subpath {
        let subpath = subpath.trim_start_matches('/');
        if !subpath.is_empty() {
            path = append_object_path_segments(path, subpath);
        }
    }

    Ok(path)
}

fn append_object_path_segments(
    mut path: flow_like_storage::Path,
    value: &str,
) -> flow_like_storage::Path {
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        path = path.child(segment);
    }
    path
}

async fn match_registration(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    version: &str,
    method: &axum::http::Method,
    normalized_path: &str,
) -> Result<Option<(event_remote_registration::Model, HashMap<String, String>)>, ApiError> {
    let rows = event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(app_id))
        .filter(event_remote_registration::Column::EventId.eq(event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(version))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let method_str = method.as_str().to_uppercase();
    let method_ok = |r: &event_remote_registration::Model| -> bool {
        r.method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case(&method_str) || m.eq_ignore_ascii_case("ANY"))
            .unwrap_or(true)
    };

    // 1. rest_fn — exact path + method match (most specific).
    if let Some(hit) = rows
        .iter()
        .find(|r| r.kind == "rest_fn" && r.path == normalized_path && method_ok(r))
    {
        return Ok(Some((hit.clone(), HashMap::new())));
    }

    // 2. rest_fn — templated path + method match.
    for r in rows.iter().filter(|r| r.kind == "rest_fn" && method_ok(r)) {
        if let Some(params) = match_template(&r.path, normalized_path) {
            return Ok(Some((r.clone(), params)));
        }
    }

    // 3. rest_openapi — exact spec path or UI path, GET only.
    if method == axum::http::Method::GET
        && let Some(hit) = rows.iter().find(|r| {
            matches!(r.kind.as_str(), "rest_openapi" | "rest_openapi_ui")
                && r.path == normalized_path
        })
    {
        return Ok(Some((hit.clone(), HashMap::new())));
    }
    if method == axum::http::Method::GET {
        for r in rows.iter().filter(|r| r.kind == "rest_openapi") {
            if openapi_ui_path_from_registration(r).as_deref() == Some(normalized_path) {
                let mut hit = r.clone();
                hit.kind = "rest_openapi_ui".to_string();
                hit.path = normalized_path.to_string();
                hit.extras_json = Some(openapi_ui_extras(r));
                return Ok(Some((hit, HashMap::new())));
            }
        }
    }

    // 4. rest_file — exact file routes, then directory mount routes.
    // Directory mounts accept both `/static` and `/static/{filename}` config
    // shapes but are matched by their mount path, mirroring the local runtime.
    if matches!(method, &axum::http::Method::GET | &axum::http::Method::HEAD) {
        // 4a. Exact single-file route.
        if let Some(hit) = rows.iter().find(|r| {
            r.kind == "rest_file"
                && !rest_file_is_directory_registration(r)
                && r.path == normalized_path
        }) {
            return Ok(Some((hit.clone(), HashMap::new())));
        }

        // 4b. Longest directory mount prefix.
        let mut best: Option<&event_remote_registration::Model> = None;
        for r in rows.iter().filter(|r| r.kind == "rest_file") {
            if !rest_file_is_directory_registration(r)
                || rest_file_subpath_for_request(&r.path, normalized_path).is_none()
            {
                continue;
            }
            let mount_len = rest_file_mount_path(&r.path).len();
            let best_mount_len = best
                .map(|b| rest_file_mount_path(&b.path).len())
                .unwrap_or(0);
            if mount_len > best_mount_len {
                best = Some(r);
            }
        }
        if let Some(hit) = best {
            let mut params = HashMap::new();
            params.insert(
                "__subpath".to_string(),
                rest_file_subpath_for_request(&hit.path, normalized_path).unwrap_or_default(),
            );
            return Ok(Some((hit.clone(), params)));
        }
    }

    Ok(None)
}

/// Verify inbound auth. All variants supported.
async fn verify_inbound_auth(
    state: &AppState,
    auth: &event_remote_auth::Model,
    headers: &HeaderMap,
    method: &axum::http::Method,
    normalized_path: &str,
    body: &Bytes,
) -> Result<Option<Value>, ApiError> {
    let cfg = &auth.config_json;
    let kind = cfg
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match canonical_auth_kind(&kind) {
        "none" | "" => Ok(None),
        "api_key" => {
            let header = cfg
                .get("header")
                .and_then(|v| v.as_str())
                .unwrap_or("x-api-key");
            let expected = secret_config_value(cfg, "key", &state.encryption_key)?;
            let provided = headers
                .get(header)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if provided.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(ApiError::unauthorized("invalid api key"));
            }
            Ok(None)
        }
        "bearer_token" => {
            let expected = secret_config_value(cfg, "token", &state.encryption_key)?;
            let provided = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .unwrap_or("");
            if provided.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(ApiError::unauthorized("invalid bearer token"));
            }
            Ok(None)
        }
        "basic_auth" => {
            let user = cfg.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let pass = secret_config_value(cfg, "password", &state.encryption_key)?;
            let expected = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("{user}:{pass}"),
            );
            let provided = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Basic "))
                .unwrap_or("");
            if provided.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(ApiError::unauthorized("invalid basic auth"));
            }
            Ok(None)
        }
        "hmac_sha256" => {
            verify_hmac_sha256(
                cfg,
                headers,
                method.as_str(),
                normalized_path,
                body,
                &state.encryption_key,
            )?;
            Ok(None)
        }
        "oauth_bearer" => verify_oauth_bearer(state, cfg, headers).await.map(Some),
        other => Err(ApiError::internal(format!(
            "unknown inbound auth kind '{other}'"
        ))),
    }
}

fn canonical_auth_kind(kind: &str) -> &str {
    match kind {
        "o_auth_bearer" | "oauth_bearer" => "oauth_bearer",
        other => other,
    }
}

fn secret_config_value(
    cfg: &Value,
    field: &str,
    encryption_key: &[u8; 32],
) -> Result<String, ApiError> {
    let encrypted_field = format!("{field}_encrypted");
    if let Some(encrypted) = cfg.get(&encrypted_field).and_then(|v| v.as_str()) {
        return decrypt_token(encrypted, encryption_key).ok_or_else(|| {
            ApiError::internal(format!("failed to decrypt auth config field '{field}'"))
        });
    }
    Ok(cfg
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// Constant-time byte comparison. Returns false for unequal lengths.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// HMAC-SHA256 verification.
///
/// Canonical signed string is `<timestamp>.<raw-body>` (matches Stripe /
/// Slack-style schemes). The provided signature header may be `hex` or
/// `base64`; both are accepted (constant-time compared).
fn verify_hmac_sha256(
    cfg: &Value,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &Bytes,
    encryption_key: &[u8; 32],
) -> Result<(), ApiError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let secret = secret_config_value(cfg, "secret", encryption_key)?;
    if secret.is_empty() {
        return Err(ApiError::internal("hmac config missing 'secret'"));
    }
    let sig_header = cfg
        .get("signature_header")
        .and_then(|v| v.as_str())
        .unwrap_or("x-signature");
    let ts_header = cfg
        .get("timestamp_header")
        .and_then(|v| v.as_str())
        .unwrap_or("x-timestamp");
    let max_skew = cfg
        .get("max_skew_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    let ts_str = headers
        .get(ts_header)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing hmac timestamp header"))?;
    let provided_sig = headers
        .get(sig_header)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing hmac signature header"))?
        // Allow `sha256=<value>` prefix common in github-style webhooks.
        .trim_start_matches("sha256=")
        .trim();

    let ts: i64 = ts_str
        .parse()
        .map_err(|_| ApiError::unauthorized("invalid hmac timestamp"))?;
    if max_skew > 0 {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).unsigned_abs() > max_skew {
            return Err(ApiError::unauthorized("hmac timestamp outside skew window"));
        }
    }

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|e| ApiError::internal(format!("hmac key init failed: {e}")))?;
    let canonical = format!(
        "{}\n{}\n{}\n{}",
        method.to_uppercase(),
        path,
        ts_str.trim(),
        sha256_hex(body)
    );
    mac.update(canonical.as_bytes());
    let expected = mac.finalize().into_bytes();

    let expected_hex = hex_encode(&expected);

    if constant_time_eq(provided_sig.as_bytes(), expected_hex.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::unauthorized("hmac signature mismatch"))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex_encode(&sha2::Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Cached JWKS by URL with a short TTL. Tiny in-process cache; on first
/// miss a single HTTP GET runs and the result lives for 5 minutes. This
/// avoids hammering the IdP on every inbound request without inviting a
/// distributed cache.
static JWKS_CACHE: std::sync::LazyLock<
    flow_like_types::tokio::sync::Mutex<
        HashMap<String, (std::time::Instant, jsonwebtoken::jwk::JwkSet)>,
    >,
> = std::sync::LazyLock::new(|| flow_like_types::tokio::sync::Mutex::new(HashMap::new()));

const JWKS_TTL: Duration = Duration::from_secs(300);

async fn get_jwks(jwks_url: &str) -> Result<jsonwebtoken::jwk::JwkSet, ApiError> {
    {
        let cache = JWKS_CACHE.lock().await;
        if let Some((fetched_at, jwks)) = cache.get(jwks_url) {
            if fetched_at.elapsed() < JWKS_TTL {
                return Ok(jwks.clone());
            }
        }
    }
    let resp = reqwest::Client::new()
        .get(jwks_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("jwks fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::internal(format!(
            "jwks fetch returned status {}",
            resp.status()
        )));
    }
    let jwks: jsonwebtoken::jwk::JwkSet = resp
        .json()
        .await
        .map_err(|e| ApiError::internal(format!("jwks parse failed: {e}")))?;
    let mut cache = JWKS_CACHE.lock().await;
    cache.insert(
        jwks_url.to_string(),
        (std::time::Instant::now(), jwks.clone()),
    );
    Ok(jwks)
}

/// Resolve the JWKS URL: prefer explicit `jwks_url`, otherwise discover
/// it from `oidc_discovery_url` (`/.well-known/openid-configuration`).
async fn resolve_jwks_url(cfg: &Value) -> Result<String, ApiError> {
    if let Some(url) = cfg.get("jwks_url").and_then(|v| v.as_str()) {
        return Ok(url.to_string());
    }
    if let Some(disco) = cfg.get("oidc_discovery_url").and_then(|v| v.as_str()) {
        let resp = reqwest::Client::new()
            .get(disco)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("oidc discovery fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(ApiError::internal(format!(
                "oidc discovery returned status {}",
                resp.status()
            )));
        }
        let doc: Value = resp
            .json()
            .await
            .map_err(|e| ApiError::internal(format!("oidc discovery parse failed: {e}")))?;
        if let Some(url) = doc.get("jwks_uri").and_then(|v| v.as_str()) {
            return Ok(url.to_string());
        }
        return Err(ApiError::internal(
            "oidc discovery document has no `jwks_uri`",
        ));
    }
    Err(ApiError::internal(
        "oauth_bearer auth requires `jwks_url`, `jwks_flow_path`, or `oidc_discovery_url`",
    ))
}

async fn oauth_jwks(state: &AppState, cfg: &Value) -> Result<jsonwebtoken::jwk::JwkSet, ApiError> {
    if let Some(inline) = cfg.get("jwks_json") {
        return serde_json::from_value(inline.clone())
            .map_err(|e| ApiError::internal(format!("jwks_json parse failed: {e}")));
    }

    if let Some(flow_path) = cfg.get("jwks_flow_path").filter(|v| !v.is_null()) {
        let object_path = flow_path
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::internal("jwks_flow_path missing path"))?;
        let credentials = state
            .master_credentials()
            .await
            .map_err(ApiError::internal_error)?;
        let store = credentials
            .to_store(false)
            .await
            .map_err(ApiError::internal_error)?;
        let file = store
            .as_generic()
            .get(&flow_like_storage::Path::from(object_path.to_string()))
            .await
            .map_err(|e| ApiError::internal(format!("jwks_flow_path fetch failed: {e}")))?;
        let bytes = file
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("jwks_flow_path read failed: {e}")))?;
        return serde_json::from_slice::<jsonwebtoken::jwk::JwkSet>(&bytes)
            .map_err(|e| ApiError::internal(format!("jwks_flow_path parse failed: {e}")));
    }

    let jwks_url = resolve_jwks_url(cfg).await?;
    get_jwks(&jwks_url).await
}

async fn verify_oauth_bearer(
    state: &AppState,
    cfg: &Value,
    headers: &HeaderMap,
) -> Result<Value, ApiError> {
    use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};

    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("missing bearer token"))?
        .trim();
    if token.is_empty() {
        return Err(ApiError::unauthorized("empty bearer token"));
    }

    let jwks = oauth_jwks(state, cfg).await?;

    let header = decode_header(token)
        .map_err(|e| ApiError::unauthorized(format!("invalid jwt header: {e}")))?;
    let alg = header.alg;
    if !is_asymmetric_jwt_algorithm(alg) {
        return Err(ApiError::unauthorized("unsupported oauth token algorithm"));
    }

    let candidates = candidate_jwks(&jwks, header.kid.as_deref());
    if candidates.is_empty() {
        return Err(ApiError::unauthorized("no matching jwk for token kid"));
    }

    let mut validation = Validation::new(alg);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    if let Some(iss) = cfg.get("issuer").and_then(|v| v.as_str()) {
        validation.set_issuer(&[iss]);
    }
    if let Some(aud) = cfg.get("audience").and_then(|v| v.as_str()) {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }
    validation.algorithms = vec![alg];

    let mut last_error = None;
    for jwk in candidates {
        if !jwk_matches_oauth_header(jwk, alg) {
            continue;
        }
        let key = match DecodingKey::from_jwk(jwk) {
            Ok(key) => key,
            Err(error) => {
                last_error = Some(format!("jwk decode failed: {error}"));
                continue;
            }
        };
        let data = match decode::<Value>(token, &key, &validation) {
            Ok(data) => data,
            Err(error) => {
                last_error = Some(format!("jwt verification failed: {error}"));
                continue;
            }
        };

        // Scope check.
        if let Some(required) = cfg.get("required_scopes").and_then(|v| v.as_array()) {
            let claim_scopes: Vec<String> = data
                .claims
                .get("scope")
                .and_then(|v| v.as_str())
                .map(|s| s.split_whitespace().map(String::from).collect())
                .or_else(|| {
                    data.claims.get("scp").and_then(|v| v.as_array()).map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect()
                    })
                })
                .unwrap_or_default();
            for needed in required {
                let needed_str = needed.as_str().unwrap_or("");
                if !claim_scopes.iter().any(|s| s == needed_str) {
                    return Err(ApiError::forbidden(format!(
                        "missing required scope: {needed_str}"
                    )));
                }
            }
        }

        return Ok(data.claims);
    }

    Err(ApiError::unauthorized(last_error.unwrap_or_else(|| {
        "no usable jwk matched token algorithm".to_string()
    })))
}

fn candidate_jwks<'a>(jwks: &'a JwkSet, kid: Option<&str>) -> Vec<&'a Jwk> {
    if let Some(kid) = kid {
        jwks.find(kid).into_iter().collect()
    } else {
        jwks.keys.iter().collect()
    }
}

fn jwk_matches_oauth_header(jwk: &Jwk, alg: Algorithm) -> bool {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &PublicKeyUse::Signature)
    {
        return false;
    }
    if jwk.common.key_operations.as_ref().is_some_and(|ops| {
        !ops.iter()
            .any(|op| matches!(op, KeyOperations::Verify | KeyOperations::Sign))
    }) {
        return false;
    }
    if let Some(key_alg) = &jwk.common.key_algorithm
        && !key_algorithm_matches(key_alg, alg)
    {
        return false;
    }
    jwk_key_family_matches(jwk, alg)
}

fn key_algorithm_matches(key_alg: &KeyAlgorithm, alg: Algorithm) -> bool {
    Algorithm::from_str(&key_alg.to_string()).is_ok_and(|jwk_alg| jwk_alg == alg)
}

fn jwk_key_family_matches(jwk: &Jwk, alg: Algorithm) -> bool {
    match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => matches!(
            alg,
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512
        ),
        AlgorithmParameters::EllipticCurve(_) => {
            matches!(alg, Algorithm::ES256 | Algorithm::ES384)
        }
        AlgorithmParameters::OctetKeyPair(_) => alg == Algorithm::EdDSA,
        AlgorithmParameters::OctetKey(_) => false,
    }
}

fn is_asymmetric_jwt_algorithm(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

/// Dispatch a static-file (`rest_file`) inbound request by issuing a
/// 307 redirect to a short-lived signed URL. This keeps the file payload
/// off the API process — the client downloads directly from the object
/// store (S3/GCS/Azure/etc.).
async fn dispatch_rest_file(
    state: &AppState,
    registration: &event_remote_registration::Model,
    path_params: &HashMap<String, String>,
    method: &axum::http::Method,
) -> Result<Response, ApiError> {
    let extras = registration
        .extras_json
        .as_ref()
        .ok_or_else(|| ApiError::internal("rest_file row has no extras_json"))?;

    let flow_path = extras
        .get("flow_path")
        .ok_or_else(|| ApiError::internal("rest_file extras missing flow_path"))?;
    let base_object_path = flow_path
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::internal("rest_file flow_path missing path"))?
        .trim_end_matches('/')
        .to_string();

    let is_directory = extras
        .get("directory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || rest_file_route_prefix(&registration.path).is_some()
        || path_params.contains_key("__subpath");

    // Resolve the requested subpath under the event app's content prefix.
    // `app_scoped_content_path` always starts from `apps/{app_id}`; object
    // store Path handling is responsible for rejecting traversal components.
    let object_path = if is_directory {
        let raw_sub = path_params
            .get("__subpath")
            .cloned()
            .or_else(|| path_params.get("filename").cloned())
            .unwrap_or_default();
        let sub = urlencoding::decode(&raw_sub)
            .map(|c| c.into_owned())
            .unwrap_or(raw_sub);
        if sub.is_empty() {
            return Err(ApiError::not_found("missing filename"));
        }
        app_scoped_content_path(&registration.app_id, &base_object_path, Some(&sub))?
    } else {
        app_scoped_content_path(&registration.app_id, &base_object_path, None)?
    };

    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?;

    let signed = store
        .sign(method.as_str(), &object_path, Duration::from_secs(60 * 5))
        .await
        .map_err(ApiError::internal_error)?;

    // HEAD requests get the same redirect; the client follows it and
    // re-issues HEAD against the signed URL.
    let mut resp = Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .body(axum::body::Body::empty())
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    let headers_mut = resp.headers_mut();
    if let Ok(loc) = axum::http::HeaderValue::from_str(signed.as_str()) {
        headers_mut.insert(axum::http::header::LOCATION, loc);
    }
    headers_mut.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=60"),
    );
    if let Some(ct) = extras.get("content_type").and_then(|v| v.as_str())
        && let Ok(v) = axum::http::HeaderValue::from_str(ct)
    {
        headers_mut.insert(axum::http::header::CONTENT_TYPE, v);
    }
    Ok(resp)
}

async fn dispatch_rest_fn(
    state: &AppState,
    event_row: &event::Model,
    registration: &event_remote_registration::Model,
    request_path: &str,
    raw_query: &str,
    path_params: &HashMap<String, String>,
    headers: &HeaderMap,
    method: &axum::http::Method,
    body: &Bytes,
    client: Value,
    caller: &ProxyCallerContext,
) -> Result<Value, ApiError> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body_value = parse_rest_body_value(body, &content_type)?;
    let headers_single = header_map_to_json(headers);
    let query_single = parse_query_single(raw_query);
    let mut args = rest_args_from_body_and_query(&body_value, &query_single);

    let request_id = headers
        .get("traceparent")
        .or_else(|| headers.get("x-request-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(flow_like_types::create_id);

    // The target node id is what the registration carries: setup-time we
    // resolve `function_refs[0]` and persist it there, so this points at
    // the actual handler — NOT the REST server config node.
    let target_node_id = registration
        .node_id
        .clone()
        .ok_or_else(|| ApiError::internal("registration has no target node_id"))?;

    let payload_with_client = payload_with_client(body_value.clone(), &client);
    let request_value = json!({
        "method": method.as_str(),
        "path": request_path,
        "query": query_single.clone(),
        "headers": headers_single,
        "body": body_value.clone(),
        "body_text": String::from_utf8(body.to_vec()).ok(),
        "body_bytes": body.to_vec(),
        "_client": client.clone(),
    });

    args.insert("payload".to_string(), payload_with_client);
    args.insert("request".to_string(), request_value);
    args.insert("method".to_string(), json!(method.as_str()));
    args.insert("path".to_string(), json!(request_path));
    args.insert("query".to_string(), json!(query_single));
    args.insert("headers".to_string(), json!(header_map_to_json(headers)));
    args.insert("body".to_string(), body_value.clone());
    args.insert(
        "body_text".to_string(),
        json!(String::from_utf8(body.to_vec()).ok()),
    );
    args.insert("body_bytes".to_string(), json!(body.to_vec()));
    args.insert("_client".to_string(), client.clone());
    args.insert(
        "__inbound_rest".to_string(),
        json!({
            "method": method.as_str(),
            "path": request_path,
            "registration_path": registration.path,
            "target_node_id": target_node_id,
            "path_params": path_params,
            "request_id": request_id,
            "content_type": content_type,
        }),
    );

    dispatch_event_collect(
        state,
        event_row,
        target_node_id,
        Some(Value::Object(args)),
        caller,
    )
    .await
}

async fn dispatch_event_collect(
    state: &AppState,
    event_row: &event::Model,
    target_node_id: String,
    payload: Option<Value>,
    caller: &ProxyCallerContext,
) -> Result<Value, ApiError> {
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }

    let core_event = db_model_to_event(event_row.clone()).map_err(ApiError::internal_error)?;
    let board_id = core_event.board_id.clone();
    if board_id.is_empty() {
        return Err(ApiError::internal("event has no associated board_id"));
    }

    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(&event_row.app_id))
        .filter(event_sink::Column::EventId.eq(&event_row.id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    if let Some(sink) = sink.as_ref()
        && !sink.active
    {
        return Err(ApiError::not_found("event sink is inactive"));
    }

    let token = sink
        .as_ref()
        .and_then(|sink| sink.pat_encrypted.as_ref())
        .and_then(|encrypted| decrypt_token(encrypted, &state.encryption_key));
    let actor_user_id = if let Some(sink) = sink.as_ref() {
        resolve_sink_pat_user_id(state, sink, token.as_deref()).await?
    } else {
        None
    };
    let executor_subject = actor_user_id.clone().unwrap_or_else(|| {
        sink.as_ref()
            .map(|sink| format!("sink:{}", sink.id))
            .unwrap_or_else(|| format!("inbound:{}", event_row.id))
    });

    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &event_row.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await?;
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .as_ref()
        .and_then(|sink| sink.oauth_tokens_encrypted.as_ref())
        .and_then(|encrypted| decrypt_token(encrypted, &state.encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());
    let oauth_tokens = match (sink.as_ref(), oauth_tokens) {
        (Some(sink), Some(tokens)) => {
            Some(maybe_refresh_oauth_tokens(state, &sink.id, tokens).await)
        }
        _ => None,
    };

    let run_id = flow_like_types::create_id();
    // Tie the run into the caller's process case: proxied calls inherit the
    // caller's trace/keys; public inbound traffic roots a fresh case.
    let parent_run_id = caller.parent_run_id.clone();
    let mut correlation = caller.correlation.clone().unwrap_or_default();
    if correlation.trace_id.is_none() {
        correlation.trace_id = parent_run_id.clone().or_else(|| Some(run_id.clone()));
    }
    // Auto-extract business keys via the event's correlation mappings.
    if let (Some(mappings), Some(payload_value)) = (
        core_event
            .correlation_mappings
            .as_ref()
            .filter(|mappings| !mappings.is_empty()),
        payload.as_ref(),
    ) {
        let extracted = crate::correlation::extract_mapped_keys(payload_value, mappings);
        if !extracted.is_empty() {
            correlation = correlation.with_keys(&extracted);
        }
    }
    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: event_row.app_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_row.id.clone()),
        app_chain: caller.app_chain.clone(),
        correlation: correlation.clone().into_option(),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })
    .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let input_payload_len = payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let event_json = serde_json::to_string(&core_event)
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    let wasm_packages = resolve_wasm_packages(state, &event_row.app_id).await;

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: event_row.app_id.clone(),
        board_id: board_id.clone(),
        board_version: core_event.board_version,
        node_id: target_node_id,
        event_json: Some(event_json),
        payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token,
        oauth_tokens,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Event),
        runtime_variables: None,
        user_context: None,
        profile: {
            let mut profile = sink.as_ref().and_then(|sink| sink.profile_json.clone());
            if let Some(profile_json) = profile.as_mut() {
                crate::execution::hydrate_profile_custom_bit_secrets(state, profile_json).await;
            }
            profile
        },
        wasm_packages,
    };

    let now = chrono::Utc::now().naive_utc();
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(board_id),
        version: Set(None),
        event_id: Set(Some(event_row.id.clone())),
        node_id: Set(Some(event_row.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(now + chrono::Duration::hours(24))),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(caller.app_chain.clone()),
        trace_id: Set(correlation.trace_id.clone()),
        parent_run_id: Set(parent_run_id.clone()),
        correlation_keys: Set(correlation.keys_json()),
        app_id: Set(event_row.app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    run.insert(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let db = Some(Arc::new(state.db.clone()));
    match state.dispatcher.backend() {
        ExecutionBackend::LambdaStream => {
            let (_dispatch_response, byte_stream) = state
                .dispatcher
                .dispatch_streaming(request)
                .await
                .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
            collect_generic_result_bytes(byte_stream, run_id, db, INBOUND_RESULT_TIMEOUT)
                .await
                .ok_or_else(|| {
                    ApiError::gateway_timeout(
                        "flow did not return a result within the allotted time",
                    )
                })
        }
        _ => {
            let (_dispatch_response, executor_response) = state
                .dispatcher
                .dispatch_http_sse(request)
                .await
                .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
            collect_generic_result(executor_response, run_id, db, INBOUND_RESULT_TIMEOUT)
                .await
                .ok_or_else(|| {
                    ApiError::gateway_timeout(
                        "flow did not return a result within the allotted time",
                    )
                })
        }
    }
}

fn parse_rest_body_value(body: &Bytes, content_type: &str) -> Result<Value, ApiError> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ct == "application/json" || ct.ends_with("+json") {
        return serde_json::from_slice(body)
            .map_err(|e| ApiError::bad_request(format!("Invalid JSON request body: {e}")));
    }
    Ok(match String::from_utf8(body.to_vec()) {
        Ok(text) => Value::String(text),
        Err(_) => Value::Array(body.iter().map(|byte| Value::from(*byte)).collect()),
    })
}

fn parse_query_single(raw: &str) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    if raw.is_empty() {
        return out;
    }
    for pair in raw.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = urlencoding::decode(&k.replace('+', " "))
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| k.to_string());
        let value = urlencoding::decode(&v.replace('+', " "))
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| v.to_string());
        out.insert(key, value);
    }
    out
}

fn rest_args_from_body_and_query(
    body: &Value,
    query: &HashMap<String, String>,
) -> serde_json::Map<String, Value> {
    let mut args = serde_json::Map::new();

    for (key, value) in query {
        if !is_rest_internal_arg_key(key) {
            args.insert(key.clone(), Value::String(value.clone()));
        }
    }

    if let Some(body_object) = body.as_object() {
        for (key, value) in body_object {
            args.insert(key.clone(), value.clone());
        }
    }

    args
}

fn is_rest_internal_arg_key(name: &str) -> bool {
    matches!(
        name,
        "_client"
            | "request"
            | "method"
            | "path"
            | "query"
            | "headers"
            | "body"
            | "body_text"
            | "body_bytes"
            | "payload"
            | "__inbound_rest"
    )
}

fn header_map_to_json(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(value) = value.to_str() {
            out.insert(name.as_str().to_ascii_lowercase(), value.to_string());
        }
    }
    out
}

fn inbound_remote_addr(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

fn client_metadata(
    protocol: &str,
    headers: &HeaderMap,
    oauth_claims: Option<Value>,
    proxy_identity: Option<Value>,
) -> Value {
    let mut client = serde_json::Map::new();
    client.insert(
        "remote_addr".to_string(),
        json!(inbound_remote_addr(headers)),
    );
    client.insert("protocol".to_string(), json!(protocol));

    if let Some(claims) = oauth_claims {
        if let Some(value) = claims.get("sub").cloned() {
            client.insert("sub".to_string(), value);
        }
        if let Some(value) = claims.get("iss").cloned() {
            client.insert("issuer".to_string(), value);
        }
        if let Some(value) = claims.get("aud").cloned() {
            client.insert("audience".to_string(), value);
        }
        if let Some(value) = claims
            .get("client_id")
            .or_else(|| claims.get("azp"))
            .cloned()
        {
            client.insert("client_id".to_string(), value);
        }
        if let Some(value) = claims.get("email").cloned() {
            client.insert("email".to_string(), value);
        }
        let scopes = scopes_from_claims(&claims);
        if !scopes.is_empty() {
            client.insert(
                "scopes".to_string(),
                Value::Array(scopes.into_iter().map(Value::String).collect()),
            );
        }
        client.insert(
            "auth".to_string(),
            json!({
                "type": "oauth_bearer",
                "claims": claims
            }),
        );
    }

    if let Some(proxy_identity) = proxy_identity {
        client.insert("proxy".to_string(), proxy_identity);
    }

    Value::Object(client)
}

fn scopes_from_claims(claims: &Value) -> Vec<String> {
    let mut scopes = Vec::new();
    if let Some(scope) = claims.get("scope").and_then(|value| value.as_str()) {
        scopes.extend(scope.split_whitespace().map(ToString::to_string));
    }
    if let Some(scope) = claims.get("scp").and_then(|value| value.as_str()) {
        scopes.extend(scope.split_whitespace().map(ToString::to_string));
    }
    for key in ["scp", "permissions"] {
        if let Some(values) = claims.get(key).and_then(|value| value.as_array()) {
            scopes.extend(
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string)),
            );
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
}

fn payload_with_client(payload: Value, client: &Value) -> Value {
    match payload {
        Value::Object(mut object) => {
            object.insert("_client".to_string(), client.clone());
            Value::Object(object)
        }
        other => json!({
            "payload": other,
            "_client": client
        }),
    }
}

#[derive(Clone, Debug)]
struct InboundMcpSession {
    event_id: String,
    protocol_version: String,
    initialized: bool,
    created_at: std::time::Instant,
    sse_tx: flow_like_types::tokio::sync::broadcast::Sender<String>,
}

static MCP_SESSIONS: std::sync::LazyLock<
    flow_like_types::tokio::sync::Mutex<HashMap<String, InboundMcpSession>>,
> = std::sync::LazyLock::new(|| flow_like_types::tokio::sync::Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
struct McpToolEntry {
    name: String,
    description: Option<String>,
    schema: Value,
    node_id: String,
    argument_aliases: HashMap<String, String>,
}

fn mcp_options_response(headers: &HeaderMap) -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    {
        let h = resp.headers_mut();
        h.insert(
            axum::http::header::ALLOW,
            axum::http::HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        h.insert(
            "access-control-allow-methods",
            axum::http::HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        h.insert(
            "access-control-allow-headers",
            headers
                .get("access-control-request-headers")
                .cloned()
                .unwrap_or_else(|| {
                    axum::http::HeaderValue::from_static(
                        "Content-Type, Authorization, X-Flow-Like-Event-Authorization, Mcp-Session-Id, MCP-Protocol-Version, Last-Event-ID, Accept",
                    )
                }),
        );
        h.insert(
            "access-control-max-age",
            axum::http::HeaderValue::from_static("86400"),
        );
    }
    apply_mcp_cors(resp.headers_mut(), headers);
    resp
}

fn apply_mcp_cors(response_headers: &mut HeaderMap, request_headers: &HeaderMap) {
    let origin = request_headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let origin_value = origin.unwrap_or("*");
    if let Ok(value) = axum::http::HeaderValue::from_str(origin_value) {
        response_headers.insert("access-control-allow-origin", value);
    }
    response_headers.insert(
        "access-control-expose-headers",
        axum::http::HeaderValue::from_static(
            "Mcp-Session-Id, MCP-Protocol-Version, WWW-Authenticate",
        ),
    );
    if origin.is_some() {
        response_headers.insert(
            "access-control-allow-credentials",
            axum::http::HeaderValue::from_static("true"),
        );
        response_headers.insert(
            axum::http::header::VARY,
            axum::http::HeaderValue::from_static("Origin"),
        );
    }
}

fn mcp_oauth_metadata_response(headers: &HeaderMap, config: &Value, resource: &str) -> Response {
    let auth = config.get("auth").cloned().unwrap_or(Value::Null);
    let body = if auth
        .get("type")
        .and_then(|v| v.as_str())
        .is_some_and(|v| canonical_auth_kind(v) == "oauth_bearer")
    {
        let mut body = json!({
            "resource": resource,
            "bearer_methods_supported": ["header"],
            "resource_documentation": "https://modelcontextprotocol.io",
        });
        if let Some(map) = body.as_object_mut() {
            if let Some(issuer) = auth
                .get("issuer")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                map.insert(
                    "authorization_servers".to_string(),
                    json!([issuer.to_string()]),
                );
            }
            if let Some(scopes) = auth.get("required_scopes").and_then(|v| v.as_array())
                && !scopes.is_empty()
            {
                map.insert("scopes_supported".to_string(), Value::Array(scopes.clone()));
            }
        }
        body
    } else {
        json!({
            "resource": resource,
            "bearer_methods_supported": [],
        })
    };
    let mut resp = Json(body).into_response();
    apply_mcp_cors(resp.headers_mut(), headers);
    resp
}

fn mcp_resource_url(headers: &HeaderMap, endpoint_path: &str) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let host = headers
                .get(axum::http::header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if host.starts_with("localhost")
                || host.starts_with("127.")
                || host.starts_with("[::1]")
                || host.starts_with("::1")
            {
                "http".to_string()
            } else {
                "https".to_string()
            }
        });
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{scheme}://{host}{endpoint_path}")
}

fn mcp_session_id(raw_query: &str, headers: &HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| parse_query_single(raw_query).remove("sessionId"))
}

fn negotiate_mcp_protocol_version(requested: Option<&str>) -> String {
    match requested {
        Some(v) if MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v.to_string(),
        _ => MCP_DEFAULT_PROTOCOL_VERSION.to_string(),
    }
}

fn mcp_protocol_version_from_headers(headers: &HeaderMap) -> String {
    let requested = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok());
    negotiate_mcp_protocol_version(requested)
}

fn new_mcp_session(
    event_id: &str,
    protocol_version: String,
    initialized: bool,
) -> (
    InboundMcpSession,
    flow_like_types::tokio::sync::broadcast::Receiver<String>,
) {
    let (sse_tx, rx) = flow_like_types::tokio::sync::broadcast::channel::<String>(64);
    (
        InboundMcpSession {
            event_id: event_id.to_string(),
            protocol_version,
            initialized,
            created_at: std::time::Instant::now(),
            sse_tx,
        },
        rx,
    )
}

async fn prune_expired_mcp_sessions() {
    const TTL: Duration = Duration::from_secs(60 * 60 * 6);
    let mut sessions = MCP_SESSIONS.lock().await;
    sessions.retain(|_, session| session.created_at.elapsed() < TTL);
}

async fn mcp_handle_post(
    state: &AppState,
    event_row: &event::Model,
    config: &Value,
    raw_query: &str,
    headers: &HeaderMap,
    body: &Bytes,
    client: Value,
    caller: &ProxyCallerContext,
) -> Result<Response, ApiError> {
    let payload: Value = serde_json::from_slice(body).map_err(|_| {
        ApiError::bad_request("MCP POST body must be a valid JSON-RPC object or array")
    })?;
    let (items, is_batch) = if let Some(items) = payload.as_array() {
        (items.clone(), true)
    } else {
        (vec![payload], false)
    };
    let has_initialize = items
        .iter()
        .any(|item| item.get("method").and_then(|v| v.as_str()) == Some("initialize"));
    let supplied_session_id = mcp_session_id(raw_query, headers);
    let is_legacy_sse_post = !headers.contains_key("mcp-session-id")
        && parse_query_single(raw_query).contains_key("sessionId");
    let mut assigned_session_id = supplied_session_id.clone();
    prune_expired_mcp_sessions().await;

    if !has_initialize && supplied_session_id.is_none() {
        return Ok(mcp_json_response(
            StatusCode::BAD_REQUEST,
            json_rpc_error(None, -32600, "Missing Mcp-Session-Id header"),
            None,
            headers,
        ));
    }

    if let Some(session_id) = supplied_session_id.as_ref()
        && !has_initialize
    {
        let client_protocol = headers
            .get("mcp-protocol-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let mut sessions = MCP_SESSIONS.lock().await;
        if let Some(session) = sessions.get(session_id) {
            if session.event_id != event_row.id {
                return Ok(mcp_json_response(
                    StatusCode::NOT_FOUND,
                    json_rpc_error(None, -32001, "Session not found"),
                    None,
                    headers,
                ));
            }
            if let Some(client_protocol) = client_protocol.as_deref()
                && client_protocol != session.protocol_version
                && !MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&client_protocol)
            {
                return Ok(mcp_json_response(
                    StatusCode::BAD_REQUEST,
                    json_rpc_error(
                        None,
                        -32600,
                        &format!("Unsupported MCP-Protocol-Version: {client_protocol}"),
                    ),
                    None,
                    headers,
                ));
            }
        } else {
            if let Some(client_protocol) = client_protocol.as_deref()
                && !MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&client_protocol)
            {
                return Ok(mcp_json_response(
                    StatusCode::BAD_REQUEST,
                    json_rpc_error(
                        None,
                        -32600,
                        &format!("Unsupported MCP-Protocol-Version: {client_protocol}"),
                    ),
                    None,
                    headers,
                ));
            }
            let (session, _) = new_mcp_session(
                &event_row.id,
                mcp_protocol_version_from_headers(headers),
                true,
            );
            tracing::warn!(
                session_id = %session_id,
                event_id = %event_row.id,
                "MCP session not found locally; recreated session for stateless request"
            );
            sessions.insert(session_id.clone(), session);
        }
    }

    let mut responses = Vec::new();
    for item in &items {
        let method_name = item.get("method").and_then(|v| v.as_str()).unwrap_or("");
        if method_name == "initialize" {
            let (response, session_id) =
                handle_mcp_initialize(event_row, item, assigned_session_id.clone()).await;
            assigned_session_id = Some(session_id);
            if let Some(response) = response {
                responses.push(response);
            }
            continue;
        }

        if method_name == "notifications/initialized" {
            if let Some(session_id) = assigned_session_id.as_ref() {
                if let Some(session) = MCP_SESSIONS.lock().await.get_mut(session_id) {
                    session.initialized = true;
                }
            }
            continue;
        }

        if let Some(response) =
            dispatch_mcp_json_rpc(state, event_row, config, item, &client, caller).await?
        {
            responses.push(response);
        }
    }

    if responses.is_empty() {
        return Ok(mcp_empty_response(
            StatusCode::ACCEPTED,
            assigned_session_id,
            headers,
        ));
    }

    if is_legacy_sse_post {
        if let Some(session_id) = assigned_session_id.as_ref() {
            let tx = {
                let sessions = MCP_SESSIONS.lock().await;
                sessions
                    .get(session_id)
                    .map(|session| session.sse_tx.clone())
            };
            if let Some(tx) = tx {
                for response in &responses {
                    let data = serde_json::to_string(response).unwrap_or_else(|_| "{}".to_string());
                    let _ = tx.send(data);
                }
                return Ok(mcp_empty_response(
                    StatusCode::ACCEPTED,
                    assigned_session_id,
                    headers,
                ));
            }
        }
    }

    let body_value = if is_batch {
        Value::Array(responses)
    } else {
        responses.into_iter().next().unwrap_or_else(|| json!({}))
    };
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (wants_json, wants_sse) = parse_accept_types(accept);
    if wants_sse {
        Ok(mcp_sse_response(body_value, assigned_session_id, headers))
    } else if wants_json {
        Ok(mcp_json_response(
            StatusCode::OK,
            body_value,
            assigned_session_id,
            headers,
        ))
    } else {
        Ok(mcp_text_response(
            StatusCode::NOT_ACCEPTABLE,
            "Not Acceptable: client must accept application/json or text/event-stream",
            headers,
        ))
    }
}

async fn handle_mcp_initialize(
    event_row: &event::Model,
    payload: &Value,
    existing_session_id: Option<String>,
) -> (Option<Value>, String) {
    let id = payload.get("id").cloned();
    let requested = payload
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str());
    let protocol_version = negotiate_mcp_protocol_version(requested);
    let session_id =
        existing_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let mut sessions = MCP_SESSIONS.lock().await;
    let sse_tx = sessions
        .get(&session_id)
        .map(|session| session.sse_tx.clone())
        .unwrap_or_else(|| {
            let (tx, _rx) = flow_like_types::tokio::sync::broadcast::channel::<String>(64);
            tx
        });
    let session = InboundMcpSession {
        event_id: event_row.id.clone(),
        protocol_version: protocol_version.clone(),
        initialized: false,
        created_at: std::time::Instant::now(),
        sse_tx,
    };
    sessions.insert(session_id.clone(), session);
    let result = json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false},
            "prompts": {"listChanged": false},
            "logging": {},
            "completions": {}
        },
        "serverInfo": {
            "name": "Flow Like MCP Server",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Flow Like MCP server exposing flow functions as tools."
    });
    let response = id
        .as_ref()
        .map(|_| json!({"jsonrpc": "2.0", "id": id, "result": result}));
    (response, session_id)
}

async fn mcp_handle_delete(raw_query: &str, headers: &HeaderMap) -> Response {
    let Some(session_id) = mcp_session_id(raw_query, headers) else {
        return mcp_text_response(
            StatusCode::BAD_REQUEST,
            "Missing Mcp-Session-Id header",
            headers,
        );
    };
    let removed = MCP_SESSIONS.lock().await.remove(&session_id).is_some();
    if removed {
        mcp_empty_response(StatusCode::NO_CONTENT, None, headers)
    } else {
        mcp_text_response(StatusCode::NOT_FOUND, "Session not found", headers)
    }
}

async fn mcp_handle_get(
    event_row: &event::Model,
    endpoint_path: &str,
    raw_query: &str,
    headers: &HeaderMap,
) -> Response {
    let accept = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if accepts_html(accept) {
        return mcp_browser_inspector_response(endpoint_path, headers);
    }

    let (_, wants_sse) = parse_accept_types(accept);
    if !wants_sse {
        return mcp_text_response(
            StatusCode::NOT_ACCEPTABLE,
            "Not Acceptable: GET requires Accept: text/event-stream",
            headers,
        );
    }

    prune_expired_mcp_sessions().await;
    let supplied_session_id = mcp_session_id(raw_query, headers);
    let (session_id, mut rx, legacy_endpoint) = {
        let mut sessions = MCP_SESSIONS.lock().await;
        if let Some(session_id) = supplied_session_id {
            if let Some(session) = sessions.get(&session_id) {
                if session.event_id != event_row.id {
                    return mcp_text_response(StatusCode::NOT_FOUND, "Session not found", headers);
                }
                (session_id, session.sse_tx.subscribe(), None)
            } else {
                let (session, rx) = new_mcp_session(
                    &event_row.id,
                    mcp_protocol_version_from_headers(headers),
                    true,
                );
                tracing::warn!(
                    session_id = %session_id,
                    event_id = %event_row.id,
                    "MCP SSE session not found locally; recreated session"
                );
                sessions.insert(session_id.clone(), session);
                (session_id, rx, None)
            }
        } else {
            let session_id = uuid::Uuid::new_v4().simple().to_string();
            let (session, rx) = new_mcp_session(
                &event_row.id,
                MCP_DEFAULT_PROTOCOL_VERSION.to_string(),
                false,
            );
            sessions.insert(session_id.clone(), session);
            let endpoint = format!(
                "{}?sessionId={}",
                mcp_resource_url(headers, endpoint_path),
                session_id
            );
            (session_id, rx, Some(endpoint))
        }
    };

    let stream = async_stream::stream! {
        if let Some(endpoint) = legacy_endpoint {
            yield Ok::<SseEvent, Infallible>(SseEvent::default().event("endpoint").data(endpoint));
        }
        loop {
            match rx.recv().await {
                Ok(data) => {
                    let event_id = uuid::Uuid::new_v4().simple().to_string();
                    yield Ok::<SseEvent, Infallible>(
                        SseEvent::default()
                            .id(event_id)
                            .event("message")
                            .data(data),
                    );
                }
                Err(flow_like_types::tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(flow_like_types::tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    let mut resp = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&session_id) {
        resp.headers_mut().insert("mcp-session-id", value);
    }
    apply_mcp_cors(resp.headers_mut(), headers);
    resp
}

fn mcp_browser_inspector_response(endpoint_path: &str, headers: &HeaderMap) -> Response {
    let endpoint_path_json =
        serde_json::to_string(&endpoint_path).unwrap_or_else(|_| "\"/m\"".to_string());
    let html =
        MCP_BROWSER_INSPECTOR_TEMPLATE.replace("__ENDPOINT_PATH_JSON__", &endpoint_path_json);
    let mut resp = axum::response::Html(html).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    apply_mcp_cors(resp.headers_mut(), headers);
    resp
}

async fn dispatch_mcp_json_rpc(
    state: &AppState,
    event_row: &event::Model,
    config: &Value,
    payload: &Value,
    client: &Value,
    caller: &ProxyCallerContext,
) -> Result<Option<Value>, ApiError> {
    let id = payload.get("id").cloned();
    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    let notification = id.is_none();

    let result = match method {
        "notifications/initialized"
        | "notifications/cancelled"
        | "notifications/roots/list_changed"
        | "notifications/progress" => return Ok(None),
        "ping" => json!({}),
        "tools/list" => {
            let tools = mcp_tools_for_event(state, event_row, config).await?;
            json!({
                "tools": tools.into_iter().map(|tool| {
                    let mut item = serde_json::Map::new();
                    item.insert("name".to_string(), json!(tool.name));
                    if let Some(description) = tool.description {
                        item.insert("description".to_string(), json!(description));
                    }
                    item.insert("inputSchema".to_string(), tool.schema);
                    Value::Object(item)
                }).collect::<Vec<_>>()
            })
        }
        "tools/call" => {
            return mcp_tool_call_response(state, event_row, config, id, params, client, caller)
                .await;
        }
        "resources/list" => json!({
            "resources": mcp_resources(config).into_iter().map(|resource| {
                let mut item = serde_json::Map::new();
                item.insert("uri".to_string(), resource.get("uri").cloned().unwrap_or(Value::Null));
                item.insert("name".to_string(), resource.get("name").cloned().unwrap_or(Value::Null));
                if let Some(description) = resource.get("description").cloned().filter(|v| !v.is_null()) {
                    item.insert("description".to_string(), description);
                }
                if let Some(mime) = resource.get("mime_type").or_else(|| resource.get("mimeType")).cloned().filter(|v| !v.is_null()) {
                    item.insert("mimeType".to_string(), mime);
                }
                Value::Object(item)
            }).collect::<Vec<_>>()
        }),
        "resources/templates/list" => json!({"resourceTemplates": []}),
        "resources/read" => {
            match mcp_resource_read(state, &event_row.app_id, config, &params).await {
                Ok(value) => value,
                Err((code, message)) => return Ok(Some(json_rpc_error(id, code, &message))),
            }
        }
        "resources/subscribe" | "resources/unsubscribe" => json!({}),
        "prompts/list" => json!({
            "prompts": mcp_prompts(config).into_iter().map(|prompt| {
                let mut item = serde_json::Map::new();
                item.insert("name".to_string(), prompt.get("name").cloned().unwrap_or(Value::Null));
                if let Some(description) = prompt.get("description").cloned().filter(|v| !v.is_null()) {
                    item.insert("description".to_string(), description);
                }
                let template = prompt.get("template").and_then(|v| v.as_str()).unwrap_or("");
                item.insert("arguments".to_string(), json!(prompt_argument_specs(template)));
                Value::Object(item)
            }).collect::<Vec<_>>()
        }),
        "prompts/get" => match mcp_prompt_get(config, &params) {
            Ok(value) => value,
            Err((code, message)) => return Ok(Some(json_rpc_error(id, code, &message))),
        },
        "logging/setLevel" => json!({}),
        "completion/complete" => json!({
            "completion": {
                "values": [],
                "total": 0,
                "hasMore": false
            }
        }),
        _ => {
            if notification {
                return Ok(None);
            }
            return Ok(Some(json_rpc_error(id, -32601, "Method not found")));
        }
    };

    if notification {
        Ok(None)
    } else {
        Ok(Some(json!({"jsonrpc": "2.0", "id": id, "result": result})))
    }
}

async fn mcp_tool_call_response(
    state: &AppState,
    event_row: &event::Model,
    config: &Value,
    id: Option<Value>,
    params: Value,
    client: &Value,
    caller: &ProxyCallerContext,
) -> Result<Option<Value>, ApiError> {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tools = mcp_tools_for_event(state, event_row, config).await?;
    let Some(tool) = tools.into_iter().find(|tool| tool.name == name) else {
        return Ok(Some(json_rpc_error(
            id,
            -32602,
            &format!("Unknown tool: {name}"),
        )));
    };

    let normalized_arguments = normalize_tool_arguments(arguments, &tool);
    let mut args = normalized_arguments
        .as_object()
        .cloned()
        .unwrap_or_default();
    args.insert(
        "payload".to_string(),
        payload_with_client(normalized_arguments, client),
    );
    args.insert("_client".to_string(), client.clone());
    match dispatch_event_collect(
        state,
        event_row,
        tool.node_id,
        Some(Value::Object(args)),
        caller,
    )
    .await
    {
        Ok(value) => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": result_text(&value)}],
                "isError": false
            }
        }))),
        Err(err) => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": err.to_string()}],
                "isError": true
            }
        }))),
    }
}

async fn mcp_tools_for_event(
    state: &AppState,
    event_row: &event::Model,
    config: &Value,
) -> Result<Vec<McpToolEntry>, ApiError> {
    let function_refs: Vec<String> = config
        .get("function_refs")
        .and_then(|v| v.as_array())
        .map(|refs| {
            refs.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    if function_refs.is_empty() {
        return Ok(Vec::new());
    }

    let core_event = db_model_to_event(event_row.clone()).map_err(ApiError::internal_error)?;
    let board = state
        .master_board(
            "inbound",
            &event_row.app_id,
            &core_event.board_id,
            state,
            core_event.board_version,
        )
        .await
        .map_err(ApiError::internal_error)?;
    Ok(mcp_tool_entries(&board, &function_refs))
}

fn mcp_tool_entries(board: &Board, function_refs: &[String]) -> Vec<McpToolEntry> {
    let mut out = Vec::new();
    let mut used_names = std::collections::HashSet::new();
    for node_id in function_refs {
        let Some(node) = board.nodes.get(node_id) else {
            continue;
        };
        let (base_name, description, schema, argument_aliases) = tool_metadata(node, &board.refs);
        let mut name = base_name.clone();
        let mut suffix = 2u32;
        while used_names.contains(&name) {
            name = format!("{}_{}", base_name, suffix);
            suffix += 1;
        }
        used_names.insert(name.clone());
        out.push(McpToolEntry {
            name,
            description,
            schema,
            node_id: node_id.clone(),
            argument_aliases,
        });
    }
    out
}

fn tool_metadata(
    node: &Node,
    board_refs: &HashMap<String, String>,
) -> (String, Option<String>, Value, HashMap<String, String>) {
    let name_source = if node.friendly_name.trim().is_empty() {
        node.name.as_str()
    } else {
        node.friendly_name.as_str()
    };
    let name = sanitize_identifier(name_source);
    let description = resolved_mcp_description(&node.description, board_refs);
    let has_non_payload_data_pin = node.pins.values().any(|pin| {
        pin.pin_type == PinType::Output
            && pin.data_type != VariableType::Execution
            && pin.name != "payload"
            && pin.name != "_client"
    });

    let mut properties = serde_json::Map::new();
    let mut argument_aliases = HashMap::new();
    let mut used_argument_names = std::collections::HashSet::new();
    for pin in node.pins.values() {
        if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
            continue;
        }
        if pin.name == "_client" || (pin.name == "payload" && has_non_payload_data_pin) {
            continue;
        }
        let argument_name = unique_tool_argument_name(pin, &used_argument_names);
        used_argument_names.insert(argument_name.clone());
        register_tool_argument_aliases(&mut argument_aliases, &argument_name, pin);
        let schema = pin_schema(
            &pin.data_type,
            &pin.value_type,
            pin.schema
                .as_deref()
                .map(|schema| resolve_mcp_text_ref(schema, board_refs))
                .as_deref(),
            resolved_mcp_description(&pin.description, board_refs)
                .unwrap_or_default()
                .as_str(),
        );
        properties.insert(argument_name, schema);
    }

    (
        name,
        description,
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": true
        }),
        argument_aliases,
    )
}

fn sanitize_identifier(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            output.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_string();
    if output.is_empty() {
        "function".to_string()
    } else {
        output
    }
}

fn resolve_mcp_text_ref(value: &str, board_refs: &HashMap<String, String>) -> String {
    let trimmed = value.trim();
    if trimmed == "16248035215404677707" {
        return String::new();
    }
    board_refs
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

fn resolved_mcp_description(value: &str, board_refs: &HashMap<String, String>) -> Option<String> {
    let resolved = resolve_mcp_text_ref(value, board_refs);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn unique_tool_argument_name(pin: &Pin, used: &std::collections::HashSet<String>) -> String {
    let friendly = sanitize_identifier(pin.friendly_name.trim());
    let raw = sanitize_identifier(pin.name.trim());
    for candidate in [&friendly, &raw] {
        if !candidate.is_empty() && !used.contains(candidate) {
            return candidate.clone();
        }
    }
    let base = if !friendly.is_empty() {
        friendly
    } else if !raw.is_empty() {
        raw
    } else {
        "arg".to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    while used.contains(&candidate) {
        candidate = format!("{}_{}", base, suffix);
        suffix += 1;
    }
    candidate
}

fn register_tool_argument_aliases(
    aliases: &mut HashMap<String, String>,
    public_name: &str,
    pin: &Pin,
) {
    register_tool_argument_alias(aliases, public_name, &pin.name);
    register_tool_argument_alias(aliases, &pin.name, &pin.name);
    register_tool_argument_alias(aliases, &sanitize_identifier(&pin.name), &pin.name);
    register_tool_argument_alias(aliases, &pin.friendly_name, &pin.name);
    register_tool_argument_alias(aliases, &sanitize_identifier(&pin.friendly_name), &pin.name);
}

fn register_tool_argument_alias(
    aliases: &mut HashMap<String, String>,
    alias: &str,
    pin_name: &str,
) {
    let trimmed = alias.trim();
    if !trimmed.is_empty() {
        aliases
            .entry(trimmed.to_string())
            .or_insert_with(|| pin_name.to_string());
    }
}

fn pin_schema(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    description: &str,
) -> Value {
    let mut base = match data_type {
        VariableType::String | VariableType::PathBuf | VariableType::Date => {
            json!({"type": "string"})
        }
        VariableType::Integer | VariableType::Byte => json!({"type": "integer"}),
        VariableType::Float => json!({"type": "number"}),
        VariableType::Boolean => json!({"type": "boolean"}),
        VariableType::Struct | VariableType::Generic => schema
            .and_then(|schema| serde_json::from_str::<Value>(schema).ok())
            .unwrap_or_else(|| json!({"type": "object"})),
        VariableType::Execution => json!({"type": "null"}),
    };
    if let Some(obj) = base.as_object_mut()
        && !description.is_empty()
    {
        obj.insert("description".to_string(), json!(description));
    }
    match value_type {
        ValueType::Array | ValueType::HashSet => json!({"type": "array", "items": base}),
        ValueType::HashMap => json!({"type": "object", "additionalProperties": base}),
        ValueType::Normal => base,
    }
}

fn normalize_tool_arguments(arguments: Value, tool: &McpToolEntry) -> Value {
    let Some(args) = arguments.as_object() else {
        return arguments;
    };
    let mut normalized = serde_json::Map::new();
    for (key, value) in args {
        let target_key = tool
            .argument_aliases
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.clone());
        if target_key != *key && normalized.contains_key(&target_key) {
            continue;
        }
        normalized.insert(target_key, value.clone());
    }
    Value::Object(normalized)
}

fn mcp_resources(config: &Value) -> Vec<Value> {
    config
        .get("resources")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

async fn mcp_resource_read(
    state: &AppState,
    app_id: &str,
    config: &Value,
    params: &Value,
) -> Result<Value, (i64, String)> {
    let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
    if uri.is_empty() {
        return Err((-32602, "Missing required parameter: uri".to_string()));
    }
    let Some(resource) = mcp_resources(config)
        .into_iter()
        .find(|resource| resource.get("uri").and_then(|v| v.as_str()) == Some(uri))
    else {
        return Err((-32002, format!("Resource not found: {uri}")));
    };
    let flow_path = resource
        .get("flow_path")
        .ok_or_else(|| (-32002, format!("Resource has no flow_path: {uri}")))?;
    let path = flow_path
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (-32002, format!("Resource has invalid flow_path: {uri}")))?;
    let object_path = app_scoped_content_path(app_id, path, None)
        .map_err(|e| (-32002, format!("Resource path is outside app scope: {e}")))?;
    let bytes = read_content_file(state, &object_path)
        .await
        .map_err(|e| (-32002, format!("Failed to read resource {uri}: {e}")))?;
    let mime_type = resource
        .get("mime_type")
        .or_else(|| resource.get("mimeType"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            flow_like_types::mime_guess::from_path(object_path.as_ref())
                .first_or_octet_stream()
                .to_string()
        });
    let content = match std::str::from_utf8(&bytes) {
        Ok(text) => json!({
            "uri": uri,
            "mimeType": mime_type,
            "text": text
        }),
        Err(_) => json!({
            "uri": uri,
            "mimeType": mime_type,
            "blob": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        }),
    };
    Ok(json!({"contents": [content]}))
}

async fn read_content_file(
    state: &AppState,
    path: &flow_like_storage::Path,
) -> Result<Vec<u8>, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?;
    let file = store
        .as_generic()
        .get(path)
        .await
        .map_err(|e| ApiError::internal(format!("object fetch failed: {e}")))?;
    let bytes = file
        .bytes()
        .await
        .map_err(|e| ApiError::internal(format!("object read failed: {e}")))?;
    Ok(bytes.to_vec())
}

fn mcp_prompts(config: &Value) -> Vec<Value> {
    config
        .get("prompts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn prompt_argument_specs(template: &str) -> Vec<Value> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end) = template[start..].find("}}") {
                let name = template[start..start + end].trim();
                if !name.is_empty() && seen.insert(name.to_string()) {
                    out.push(json!({
                        "name": name,
                        "required": true
                    }));
                }
                i = start + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn mcp_prompt_get(config: &Value, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() {
        return Err((-32602, "Missing required parameter: name".to_string()));
    }
    let Some(prompt) = mcp_prompts(config)
        .into_iter()
        .find(|prompt| prompt.get("name").and_then(|v| v.as_str()) == Some(name))
    else {
        return Err((-32602, format!("Prompt not found: {name}")));
    };
    let template = prompt
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(json!({
        "description": prompt.get("description").cloned().unwrap_or(Value::Null),
        "messages": [{
            "role": "user",
            "content": {
                "type": "text",
                "text": substitute_prompt_template(template, &arguments)
            }
        }]
    }))
}

fn substitute_prompt_template(template: &str, arguments: &Value) -> String {
    let map = arguments.as_object().cloned().unwrap_or_default();
    let mut output = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(rel_end) = template[start..].find("}}") {
                let name = template[start..start + rel_end].trim();
                if let Some(value) = map.get(name) {
                    match value {
                        Value::String(s) => output.push_str(s),
                        other => output.push_str(&serde_json::to_string(other).unwrap_or_default()),
                    }
                } else {
                    output.push_str(&template[i..start + rel_end + 2]);
                }
                i = start + rel_end + 2;
                continue;
            }
        }
        let Some(ch) = template[i..].chars().next() else {
            break;
        };
        output.push(ch);
        i += ch.len_utf8();
    }
    output
}

fn result_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn json_rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn parse_accept_types(accept: &str) -> (bool, bool) {
    let trimmed = accept.trim();
    if trimmed.is_empty() || trimmed.contains("*/*") {
        return (true, true);
    }
    let mut wants_json = false;
    let mut wants_sse = false;
    for piece in trimmed.split(',') {
        let mime = piece
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime == "application/json" || mime == "application/*" {
            wants_json = true;
        }
        if mime == "text/event-stream" || mime == "text/*" {
            wants_sse = true;
        }
    }
    (wants_json, wants_sse)
}

fn accepts_html(accept: &str) -> bool {
    accept.split(',').any(|piece| {
        let mime = piece
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        mime == "text/html" || mime == "application/xhtml+xml"
    })
}

fn mcp_json_response(
    status: StatusCode,
    body: Value,
    session_id: Option<String>,
    request_headers: &HeaderMap,
) -> Response {
    let mut resp = Json(body).into_response();
    *resp.status_mut() = status;
    if let Some(session_id) = session_id {
        if let Ok(value) = axum::http::HeaderValue::from_str(&session_id) {
            resp.headers_mut().insert("mcp-session-id", value);
        }
    }
    apply_mcp_cors(resp.headers_mut(), request_headers);
    resp
}

fn mcp_sse_response(
    body: Value,
    session_id: Option<String>,
    request_headers: &HeaderMap,
) -> Response {
    let data = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    let event_id = uuid::Uuid::new_v4().simple().to_string();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(format!("id: {event_id}\n").as_bytes());
    bytes.extend_from_slice(b"event: message\n");
    for line in data.split('\n') {
        bytes.extend_from_slice(format!("data: {line}\n").as_bytes());
    }
    bytes.extend_from_slice(b"\n");
    let mut resp = Response::new(axum::body::Body::from(bytes));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-transform"),
    );
    if let Some(session_id) = session_id {
        if let Ok(value) = axum::http::HeaderValue::from_str(&session_id) {
            resp.headers_mut().insert("mcp-session-id", value);
        }
    }
    apply_mcp_cors(resp.headers_mut(), request_headers);
    resp
}

fn mcp_empty_response(
    status: StatusCode,
    session_id: Option<String>,
    request_headers: &HeaderMap,
) -> Response {
    let mut resp = Response::new(axum::body::Body::empty());
    *resp.status_mut() = status;
    if let Some(session_id) = session_id {
        if let Ok(value) = axum::http::HeaderValue::from_str(&session_id) {
            resp.headers_mut().insert("mcp-session-id", value);
        }
    }
    apply_mcp_cors(resp.headers_mut(), request_headers);
    resp
}

fn mcp_text_response(status: StatusCode, text: &str, request_headers: &HeaderMap) -> Response {
    let mut resp = Response::new(axum::body::Body::from(text.to_string()));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    apply_mcp_cors(resp.headers_mut(), request_headers);
    resp
}

/// Materialize a workflow result into an HTTP response. If the result is
/// shaped like `{ "__inbound_response": { status, headers, content_type,
/// body } }` we honor it precisely; otherwise we fall back to a
/// `200 application/json` body containing the raw value (legacy contract).
///
/// `body` rules inside the envelope:
///   * a JSON value (object/array/number/bool) → JSON-encoded body
///   * a string → raw bytes (matched against `content_type`)
///   * `{ "__b64": "..." }` → base64-decoded raw bytes
///   * missing/null → empty body
fn materialize_response(result: Value) -> Response {
    let envelope = result
        .as_object()
        .and_then(|o| o.get("__inbound_response"))
        .cloned();

    let Some(env) = envelope.or_else(|| local_response_envelope(&result)) else {
        return body_response(StatusCode::OK, HeaderMap::new(), result);
    };

    let status = env
        .get("status")
        .and_then(|v| v.as_u64())
        .and_then(|c| u16::try_from(c).ok())
        .and_then(|c| StatusCode::from_u16(c).ok())
        .unwrap_or(StatusCode::OK);

    let content_type = env
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("application/json")
        .to_string();

    let body_value = env.get("body").cloned().unwrap_or(Value::Null);
    let body_bytes: Vec<u8> = match &body_value {
        Value::Null => Vec::new(),
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Object(o) if o.contains_key("__b64") => {
            match o.get("__b64").and_then(|v| v.as_str()) {
                Some(s) => base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
                    .unwrap_or_default(),
                None => Vec::new(),
            }
        }
        other => serde_json::to_vec(other).unwrap_or_default(),
    };

    let mut resp = Response::new(axum::body::Body::from(body_bytes));
    *resp.status_mut() = status;

    let headers_mut = resp.headers_mut();
    if let Ok(ct) = axum::http::HeaderValue::from_str(&content_type) {
        headers_mut.insert(axum::http::header::CONTENT_TYPE, ct);
    }

    if let Some(map) = env.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in map {
            let name = match axum::http::HeaderName::try_from(k.as_str()) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let value_str = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            if let Ok(hv) = axum::http::HeaderValue::from_str(&value_str) {
                headers_mut.append(name, hv);
            }
        }
    }

    resp
}

fn local_response_envelope(result: &Value) -> Option<Value> {
    let obj = result.as_object()?;
    if !(obj.contains_key("body")
        || obj.contains_key("status")
        || obj.contains_key("status_code")
        || obj.contains_key("headers"))
    {
        return None;
    }

    let status = obj
        .get("status_code")
        .or_else(|| obj.get("status"))
        .cloned()
        .unwrap_or_else(|| json!(200));
    let mut env = serde_json::Map::new();
    env.insert("status".to_string(), status);
    if let Some(headers) = obj.get("headers").cloned() {
        env.insert("headers".to_string(), headers);
    }
    if let Some(body) = obj.get("body").cloned() {
        if body.is_string() {
            env.insert(
                "content_type".to_string(),
                json!("text/plain; charset=utf-8"),
            );
        }
        env.insert("body".to_string(), body);
    } else {
        env.insert("body".to_string(), Value::Null);
    }
    Some(Value::Object(env))
}

fn body_response(status: StatusCode, mut headers: HeaderMap, body: Value) -> Response {
    let body_bytes = match body {
        Value::Null => Vec::new(),
        Value::String(text) => {
            headers.entry(axum::http::header::CONTENT_TYPE).or_insert(
                axum::http::HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            text.into_bytes()
        }
        other => {
            headers.entry(axum::http::header::CONTENT_TYPE).or_insert(
                axum::http::HeaderValue::from_static("application/json; charset=utf-8"),
            );
            serde_json::to_vec(&other).unwrap_or_default()
        }
    };

    let mut resp = Response::new(axum::body::Body::from(body_bytes));
    *resp.status_mut() = status;
    *resp.headers_mut() = headers;
    resp
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, jwk::Jwk};
    use serde_json::json;

    use super::{
        PROXY_EVENT_AUTHORIZATION_HEADER, canonical_auth_kind, client_metadata, inbound_base_path,
        is_asymmetric_jwt_algorithm, jwk_matches_oauth_header, mcp_resource_url,
        parse_query_single, registration_auth_headers, rest_args_from_body_and_query,
        with_inbound_openapi_server,
    };

    #[test]
    fn inbound_base_path_encodes_the_route_key() {
        assert_eq!(inbound_base_path("event/id alias"), "/r/event%2Fid%20alias");
    }

    #[test]
    fn oauth_auth_kind_accepts_legacy_serialization() {
        assert_eq!(canonical_auth_kind("oauth_bearer"), "oauth_bearer");
        assert_eq!(canonical_auth_kind("o_auth_bearer"), "oauth_bearer");
    }

    #[test]
    fn oauth_jwk_filter_rejects_mismatched_or_symmetric_algorithms() {
        let rsa_jwk: Jwk = serde_json::from_value(json!({
            "kty": "RSA",
            "kid": "rsa",
            "use": "sig",
            "alg": "RS256",
            "n": "sXch",
            "e": "AQAB"
        }))
        .unwrap();
        let hmac_jwk: Jwk = serde_json::from_value(json!({
            "kty": "oct",
            "kid": "hmac",
            "alg": "HS256",
            "k": "YWJjZGVmZ2hpamtsbW5vcA"
        }))
        .unwrap();

        assert!(jwk_matches_oauth_header(&rsa_jwk, Algorithm::RS256));
        assert!(!jwk_matches_oauth_header(&rsa_jwk, Algorithm::ES256));
        assert!(!is_asymmetric_jwt_algorithm(Algorithm::HS256));
        assert!(!jwk_matches_oauth_header(&hmac_jwk, Algorithm::HS256));
    }

    #[test]
    fn openapi_spec_server_points_to_current_inbound_route() {
        let spec = json!({
            "openapi": "3.1.0",
            "info": {"title": "Test", "version": "1.0.0"},
            "paths": {"/hi": {"get": {}}}
        });

        let spec = with_inbound_openapi_server(spec, "public-alias");

        assert_eq!(spec["servers"][0]["url"], json!("/r/public-alias"));
        assert_eq!(spec["paths"]["/hi"], json!({"get": {}}));
    }

    #[test]
    fn rest_query_params_fill_named_args_without_overwriting_body_or_internal_args() {
        let query = parse_query_single(
            "name=from-query&limit=10&payload=ignored&body=ignored&encoded=hello+world",
        );
        let body = json!({
            "name": "from-body",
            "active": true
        });

        let args = rest_args_from_body_and_query(&body, &query);

        assert_eq!(args.get("name").cloned(), Some(json!("from-body")));
        assert_eq!(args.get("limit").cloned(), Some(json!("10")));
        assert_eq!(args.get("active").cloned(), Some(json!(true)));
        assert_eq!(args.get("encoded").cloned(), Some(json!("hello world")));
        assert!(!args.contains_key("payload"));
        assert!(!args.contains_key("body"));
    }

    #[test]
    fn mcp_resource_url_uses_http_for_localhost_without_forwarded_proto() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            axum::http::HeaderValue::from_static("localhost:8080"),
        );

        assert_eq!(
            mcp_resource_url(&headers, "/m/mcp-event"),
            "http://localhost:8080/m/mcp-event"
        );
        assert_eq!(
            mcp_resource_url(&headers, "/api/v1/apps/target/events/mcp-event/mcp"),
            "http://localhost:8080/api/v1/apps/target/events/mcp-event/mcp"
        );
    }

    #[test]
    fn proxy_registration_auth_replaces_only_the_api_authorization_for_auth_checks() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer app-connection-token"),
        );
        headers.insert(
            PROXY_EVENT_AUTHORIZATION_HEADER,
            axum::http::HeaderValue::from_static("Bearer registration-token"),
        );

        let auth_headers = registration_auth_headers(&headers, false);

        assert_eq!(
            auth_headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer registration-token")
        );
        assert!(!auth_headers.contains_key(PROXY_EVENT_AUTHORIZATION_HEADER));
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer app-connection-token")
        );
    }

    #[test]
    fn public_registration_auth_ignores_the_proxy_only_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer public-token"),
        );
        headers.insert(
            PROXY_EVENT_AUTHORIZATION_HEADER,
            axum::http::HeaderValue::from_static("Bearer proxy-token"),
        );

        let auth_headers = registration_auth_headers(&headers, true);

        assert_eq!(
            auth_headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer public-token")
        );
    }

    #[test]
    fn proxy_without_registration_authorization_does_not_forward_connection_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer app-connection-token"),
        );

        let auth_headers = registration_auth_headers(&headers, false);

        assert!(!auth_headers.contains_key(axum::http::header::AUTHORIZATION));
        assert!(!auth_headers.contains_key(PROXY_EVENT_AUTHORIZATION_HEADER));
    }

    #[test]
    fn registration_oauth_and_proxy_identity_have_separate_client_metadata() {
        let client = client_metadata(
            "rest",
            &axum::http::HeaderMap::new(),
            Some(json!({
                "sub": "registered-user",
                "iss": "https://issuer.example"
            })),
            Some(json!({
                "via": "app_connection",
                "origin_app_id": "source-app"
            })),
        );

        assert_eq!(client["sub"], json!("registered-user"));
        assert_eq!(client["auth"]["type"], json!("oauth_bearer"));
        assert_eq!(client["proxy"]["via"], json!("app_connection"));
        assert_eq!(client["proxy"]["origin_app_id"], json!("source-app"));
    }
}

//! Widget sandbox route: the host wrapper, widget entry documents and bundle
//! files, each served under the policy the viewer approved.
//!
//! - `frame/{widget_id}/{0|grant}[~{runtime}]` returns the wrapper. It frames
//!   exactly one document, `widgets/{widget_id}/index.{0|grant}[~{runtime}].html`,
//!   and pins that URL in its own `frame-src`.
//! - `widgets/{widget_id}/index.{0|grant}[~{runtime}].html` returns the
//!   declared entry with the document CSP in the header and in an injected
//!   `<meta>`.
//! - Anything else is a bundle file under the locked asset CSP.
//!
//! The wrapper and the document only load as iframes. A bundle file a browser
//! would render as a scriptable document (HTML, SVG, XML) is refused to any
//! navigation, so a stripped or overwritten CSP header never lets publisher
//! markup run as a page on the serving origin.
//!
//! A grant widens a response only after it verifies, is bound to this exact
//! package, version, bundle hash and widget, and the policy re-derived from the
//! stored contract still has the digest it carries. A version 2 grant widens
//! further only when the runtime component of the URL re-derives, against this
//! request's reserved hosts, origins and engine, to exactly the runtime and
//! effective digests it carries; otherwise it serves the declared policy.
//! Every other outcome serves the baseline. Bundle sources are the exact route
//! prefix on each serving origin and never `'self'`, since the API origin also
//! hosts endpoints a publisher can reach.

use super::widget_asset::{
    AuthorizedWidgetVersion, authorize_widget_version, content_type_for, is_active_document_type,
    is_safe_asset_path, read_widget_entry,
};
use super::widget_grant_jwt::{WidgetGrantCheck, WidgetGrantClaims, verify_widget_grant};
use super::widget_policy::{
    WidgetRequestContext, WidgetTarget, load_runtime_descriptor, load_widget_policy_descriptor,
};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use flow_like::hub::hub_origin;
use flow_like_wasm_schema::widget_bundle::widget_entry_path;
use flow_like_wasm_schema::widget_frame::{
    BASELINE_GRANT_SEGMENT, WIDGET_ASSET_CSP, decode_runtime_component, inject_document_csp_meta,
    is_bundle_source, is_frame_widget_id, is_valid_package_id, is_web_grant_token,
    split_grant_segment, widget_child_document_urls, widget_document_csp, widget_frame_csp,
    widget_frame_document, widget_frame_meta_csp,
};
use flow_like_wasm_schema::widget_policy::{
    EngineGate, MAX_WIDGET_DOCUMENT_CSP_BYTES, WidgetPolicy, WidgetPolicyDescriptor,
    WidgetRuntimeSourceRequest, WidgetRuntimeStatus, narrow_policy_for_engine,
};

pub const WIDGET_SANDBOX_ROUTE: &str = "widget-sandbox";
pub const WIDGET_ASSET_ROUTE: &str = "widget-asset";

const API_BASE_PATH: &str = "/api/v1";
const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const NO_STORE: &str = "no-store";
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const SEC_FETCH_DEST: &str = "sec-fetch-dest";
/// `Sec-Fetch-Dest` values of Fetch navigation requests, whose response the
/// browser renders as a document.
const NAVIGATION_DESTINATIONS: [&str; 6] = [
    "document",
    "embed",
    "fencedframe",
    "frame",
    "iframe",
    "object",
];
const X_FORWARDED_HOST: &str = "x-forwarded-host";
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

/// Whether `value` is a plain `host[:port]` authority.
pub fn is_valid_authority(value: &str) -> bool {
    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (value, None),
    };
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    let port_ok = port.is_none_or(|port| {
        (1..=5).contains(&port.len())
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port != 0)
    });
    host_ok && port_ok
}

fn header_values<'a>(headers: &'a HeaderMap, name: &str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
}

/// The authority this request was addressed to: the first valid
/// `X-Forwarded-Host` value, else a valid `Host`, lowercased.
pub fn request_authority(headers: &HeaderMap) -> Option<String> {
    header_values(headers, X_FORWARDED_HOST)
        .find(|value| is_valid_authority(value))
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| is_valid_authority(value))
        })
        .map(str::to_ascii_lowercase)
}

fn request_scheme(headers: &HeaderMap, secure: bool) -> &'static str {
    match header_values(headers, X_FORWARDED_PROTO)
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("https") => "https",
        Some("http") => "http",
        _ if secure => "https",
        _ => "http",
    }
}

/// `scheme://host[:port]` for `http`/`https` origins, lowercased, else `None`.
pub fn normalize_origin(origin: &str) -> Option<String> {
    let (scheme, authority) = origin.trim().trim_end_matches('/').split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    (matches!(scheme.as_str(), "http" | "https") && is_valid_authority(authority))
        .then(|| format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

/// Origins widget files are served from: the hub origin and the request's
/// own origin, deduplicated. Anything that fails validation is left out, so
/// a wrong derivation blocks the widget instead of widening it.
pub fn serving_origins(hub_domain: &str, secure: bool, headers: &HeaderMap) -> Vec<String> {
    let hub = hub_origin(hub_domain, secure).and_then(|origin| normalize_origin(&origin));
    let request = request_authority(headers).and_then(|authority| {
        normalize_origin(&format!(
            "{}://{authority}",
            request_scheme(headers, secure)
        ))
    });
    let mut origins: Vec<String> = Vec::with_capacity(2);
    for origin in [hub, request].into_iter().flatten() {
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

/// The widget route prefix of one package version on every origin:
/// `{origin}/api/v1/registry/package/{pkg}/{route}/{version}/`.
pub fn bundle_sources(
    origins: &[String],
    route: &str,
    package_id: &str,
    version: &str,
) -> Vec<String> {
    if !is_valid_package_id(package_id) {
        return Vec::new();
    }
    let prefix = format!(
        "{API_BASE_PATH}/registry/package/{}/{route}/{}/",
        urlencoding::encode(package_id),
        urlencoding::encode(version)
    );
    origins
        .iter()
        .map(|origin| format!("{origin}{prefix}"))
        .filter(|source| is_bundle_source(source))
        .collect()
}

fn is_grant_path_segment(segment: &str) -> bool {
    split_grant_segment(segment).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPath<'a> {
    /// `frame/{widget_id}` (no grant segment, served as baseline) or
    /// `frame/{widget_id}/{0|grant}[~{runtime}]`.
    Frame {
        widget_id: &'a str,
        grant: Option<&'a str>,
    },
    /// `widgets/{widget_id}/index.{0|grant}[~{runtime}].html`, matched before
    /// any file.
    Document {
        widget_id: &'a str,
        grant: &'a str,
    },
    Asset(&'a str),
}

pub fn parse_sandbox_path(path: &str) -> Option<SandboxPath<'_>> {
    if let Some(rest) = path.strip_prefix("frame/") {
        let (widget_id, grant) = match rest.split_once('/') {
            Some((widget_id, grant)) => (widget_id, Some(grant)),
            None => (rest, None),
        };
        return (is_frame_widget_id(widget_id) && grant.is_none_or(is_grant_path_segment))
            .then_some(SandboxPath::Frame { widget_id, grant });
    }
    if let Some((widget_id, file)) = path
        .strip_prefix("widgets/")
        .and_then(|rest| rest.split_once('/'))
        && let Some(grant) = file
            .strip_prefix("index.")
            .and_then(|name| name.strip_suffix(".html"))
            .filter(|grant| is_grant_path_segment(grant))
    {
        return is_frame_widget_id(widget_id).then_some(SandboxPath::Document { widget_id, grant });
    }
    is_safe_asset_path(path).then_some(SandboxPath::Asset(path))
}

/// A navigation that is not an iframe load (for example a top-level visit).
/// Requests without `Sec-Fetch-Dest` are allowed.
pub fn is_non_iframe_navigation(headers: &HeaderMap) -> bool {
    headers
        .get(SEC_FETCH_DEST)
        .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"iframe"))
}

/// A navigation of any kind (top-level, frame, embed or object), which renders
/// the response as a document. Requests without `Sec-Fetch-Dest` are allowed.
pub fn is_document_navigation(headers: &HeaderMap) -> bool {
    headers.get(SEC_FETCH_DEST).is_some_and(|value| {
        NAVIGATION_DESTINATIONS.iter().any(|destination| {
            value
                .as_bytes()
                .eq_ignore_ascii_case(destination.as_bytes())
        })
    })
}

/// A navigation to a bundle file that would render as a scriptable document.
pub fn is_active_asset_navigation(path: &str, headers: &HeaderMap) -> bool {
    is_active_document_type(content_type_for(path)) && is_document_navigation(headers)
}

/// Whether a sandbox request must be refused before anything is served: a
/// wrapper or document outside an iframe, or an active bundle file navigated to.
pub fn is_refused_sandbox_navigation(route: SandboxPath<'_>, headers: &HeaderMap) -> bool {
    match route {
        SandboxPath::Frame { .. } | SandboxPath::Document { .. } => {
            is_non_iframe_navigation(headers)
        }
        SandboxPath::Asset(path) => is_active_asset_navigation(path, headers),
    }
}

fn html_headers(csp: String) -> [(HeaderName, String); 5] {
    [
        (header::CONTENT_TYPE, HTML_CONTENT_TYPE.to_string()),
        (header::CONTENT_SECURITY_POLICY, csp),
        (header::CACHE_CONTROL, NO_STORE.to_string()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (header::REFERRER_POLICY, "no-referrer".to_string()),
    ]
}

fn asset_headers(path: &str) -> [(HeaderName, &'static str); 4] {
    [
        (header::CONTENT_TYPE, content_type_for(path)),
        (header::CONTENT_SECURITY_POLICY, WIDGET_ASSET_CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CACHE_CONTROL, IMMUTABLE_CACHE),
    ]
}

pub fn non_iframe_refusal() -> Response {
    (StatusCode::FORBIDDEN, [(header::CACHE_CONTROL, NO_STORE)]).into_response()
}

/// Wrapper framing `{child_prefix}widgets/{widget_id}/index.{segment}.html`,
/// with `frame-src` pinned to that exact document, runtime component
/// included, under every sandbox source.
pub fn frame_response(
    child_prefix: &str,
    sandbox_sources: &[String],
    widget_id: &str,
    segment: &str,
    allow_downloads: bool,
) -> Result<Response, ApiError> {
    let (grant, runtime) = split_grant_segment(segment)
        .ok_or_else(|| ApiError::not_found("Widget frame not found"))?;
    let body = widget_frame_document(child_prefix, widget_id, grant, runtime, allow_downloads)
        .ok_or_else(|| ApiError::not_found("Widget frame not found"))?;
    let child_urls = widget_child_document_urls(sandbox_sources, widget_id, grant, runtime);
    let body = inject_document_csp_meta(body.as_bytes(), &widget_frame_meta_csp(&child_urls));
    Ok((
        html_headers(widget_frame_csp(&child_urls, allow_downloads)),
        body,
    )
        .into_response())
}

/// Baseline wrapper for the legacy `widget-asset/{version}/frame/{widget_id}`
/// path, framing the sandbox route's `index.0.html`.
pub fn legacy_frame_response(
    origins: &[String],
    package_id: &str,
    version: &str,
    widget_id: &str,
) -> Result<Response, ApiError> {
    let child_prefix = format!(
        "../../../{WIDGET_SANDBOX_ROUTE}/{}/",
        urlencoding::encode(version)
    );
    frame_response(
        &child_prefix,
        &bundle_sources(origins, WIDGET_SANDBOX_ROUTE, package_id, version),
        widget_id,
        BASELINE_GRANT_SEGMENT,
        false,
    )
}

/// A widget entry document under `policy`. `local_media` comes from the
/// request's engine gate, so the response varies on `User-Agent`.
pub fn document_response(
    sources: &[String],
    policy: &WidgetPolicy,
    entry: &[u8],
    local_media: bool,
) -> Response {
    let body = inject_document_csp_meta(
        entry,
        &widget_document_csp(sources, policy, local_media, false),
    );
    let csp = widget_document_csp(sources, policy, local_media, true);
    let mut response = (html_headers(csp), body).into_response();
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("user-agent"));
    response
}

/// A bundle file. Active document types vary on `Sec-Fetch-Dest`, so a cached
/// subresource load is never replayed to a navigation the route refuses.
pub fn asset_response(path: &str, bytes: Vec<u8>) -> Response {
    let mut response = (asset_headers(path), bytes).into_response();
    if is_active_document_type(content_type_for(path)) {
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static(SEC_FETCH_DEST));
    }
    response
}

/// Whether a verified grant still matches the re-derived declared descriptor:
/// version 1 by its policy digest and without a runtime component, version 2
/// by its declared digest.
pub fn declared_grant_matches(
    claims: &WidgetGrantClaims,
    declared: &WidgetPolicyDescriptor,
    runtime_component: Option<&str>,
) -> bool {
    let bound = declared.package_version.as_deref().is_some_and(|version| {
        claims.is_bound_to(
            &declared.package_id,
            version,
            &declared.bundle_hash,
            &declared.widget_id,
        )
    });
    let approved_declared = if claims.approves_runtime() {
        claims.declared_digest.as_deref()
    } else if runtime_component.is_none() {
        Some(claims.policy_digest.as_str())
    } else {
        None
    };
    bound
        && declared.is_ok()
        && claims.preview == declared.preview
        && approved_declared == Some(declared.policy_digest.as_str())
}

/// Whether a descriptor re-derived from a URL runtime component is exactly
/// what a version 2 grant approved: accepted without rejections, with the
/// runtime and effective digests the grant carries, and derived from a
/// component that lists exactly the accepted sources (no extra entries that
/// derivation dropped as already declared).
pub fn runtime_grant_matches(
    claims: &WidgetGrantClaims,
    declared: &WidgetPolicyDescriptor,
    derived: &WidgetPolicyDescriptor,
    component: &str,
) -> bool {
    let Some(runtime) = derived.runtime.as_ref() else {
        return false;
    };
    let exact_component = derived.runtime_sources().is_some_and(|accepted| {
        decode_runtime_component(component).is_ok_and(|slots| slots == accepted.slots)
    });
    claims.approves_runtime()
        && derived.is_ok()
        && runtime.status == WidgetRuntimeStatus::Ok
        && runtime.rejected.is_empty()
        && runtime.runtime_digest.is_some()
        && runtime.runtime_digest == claims.runtime_digest
        && runtime.declared_digest == declared.policy_digest
        && derived.policy_digest == claims.policy_digest
        && exact_component
}

/// The document policy a verified grant unlocks, narrowed for the engine:
/// the re-derived effective policy when `derived` matches a version 2 grant
/// and fits the header budget, else the declared policy when the declared
/// part still matches, else `None` (baseline).
pub fn document_grant_policy(
    claims: &WidgetGrantClaims,
    declared: &WidgetPolicyDescriptor,
    runtime_component: Option<&str>,
    derived: Option<&WidgetPolicyDescriptor>,
    gate: &EngineGate,
    bundle_sources: &[String],
) -> Option<WidgetPolicy> {
    if !declared_grant_matches(claims, declared, runtime_component) {
        return None;
    }
    let runtime = derived
        .zip(runtime_component)
        .filter(|(derived, component)| runtime_grant_matches(claims, declared, derived, component))
        .map(|(derived, _)| narrow_policy_for_engine(&derived.policy, &declared.policy, gate))
        .filter(|effective| {
            widget_document_csp(bundle_sources, effective, gate.local_media, true).len()
                <= MAX_WIDGET_DOCUMENT_CSP_BYTES
        });
    Some(
        runtime
            .unwrap_or_else(|| narrow_policy_for_engine(&declared.policy, &declared.policy, gate)),
    )
}

fn verified_claims(
    token: &str,
    check: WidgetGrantCheck,
    authorized: &AuthorizedWidgetVersion,
    target: WidgetTarget<'_>,
) -> Option<WidgetGrantClaims> {
    if !is_web_grant_token(token) {
        return None;
    }
    match verify_widget_grant(token, check) {
        Ok(claims)
            if claims.is_bound_to(
                target.package_id,
                target.version,
                &authorized.bundle_hash,
                target.widget_id,
            ) =>
        {
            Some(claims)
        }
        Ok(_) => None,
        Err(error) => {
            tracing::debug!(
                package_id = target.package_id,
                version = target.version,
                widget_id = target.widget_id,
                %error,
                "Widget grant rejected; serving baseline"
            );
            None
        }
    }
}

/// Whether the wrapper frames the granted document, and its `downloads`
/// flag from the declared policy. `None` frames the baseline document.
async fn resolve_frame_grant(
    state: &AppState,
    context: &WidgetRequestContext,
    authorized: &AuthorizedWidgetVersion,
    target: WidgetTarget<'_>,
    segment: &str,
) -> Result<Option<bool>, ApiError> {
    let Some((token, runtime)) = split_grant_segment(segment) else {
        return Ok(None);
    };
    let Some(claims) = verified_claims(token, WidgetGrantCheck::Strict, authorized, target) else {
        return Ok(None);
    };
    let declared = load_widget_policy_descriptor(state, context, target, claims.preview).await?;
    Ok(declared_grant_matches(&claims, &declared, runtime).then_some(declared.policy.downloads))
}

/// Re-derives the runtime component of a version 2 document URL. Any failure
/// is `None`, which serves the declared policy.
async fn derive_runtime_component(
    state: &AppState,
    context: &WidgetRequestContext,
    target: WidgetTarget<'_>,
    claims: &WidgetGrantClaims,
    component: &str,
) -> Option<WidgetPolicyDescriptor> {
    let slots = match decode_runtime_component(component) {
        Ok(slots) => slots,
        Err(error) => {
            tracing::debug!(
                package_id = target.package_id,
                widget_id = target.widget_id,
                code = error.code(),
                "Widget runtime component rejected; serving the declared policy"
            );
            return None;
        }
    };
    let request = WidgetRuntimeSourceRequest::from_slots(&slots);
    match load_runtime_descriptor(
        state,
        context,
        target,
        claims.preview,
        claims.app_id.as_deref(),
        &request,
    )
    .await
    {
        Ok((descriptor, _)) => Some(descriptor),
        Err(error) => {
            tracing::warn!(
                package_id = target.package_id,
                widget_id = target.widget_id,
                error = ?error,
                "Widget runtime sources could not be re-derived; serving the declared policy"
            );
            None
        }
    }
}

async fn resolve_document_grant(
    state: &AppState,
    context: &WidgetRequestContext,
    authorized: &AuthorizedWidgetVersion,
    target: WidgetTarget<'_>,
    segment: &str,
) -> Result<Option<WidgetPolicy>, ApiError> {
    let Some((token, runtime)) = split_grant_segment(segment) else {
        return Ok(None);
    };
    let Some(claims) = verified_claims(token, WidgetGrantCheck::Document, authorized, target)
    else {
        return Ok(None);
    };
    let declared = load_widget_policy_descriptor(state, context, target, claims.preview).await?;
    if !declared_grant_matches(&claims, &declared, runtime) {
        return Ok(None);
    }
    let derived = match runtime.filter(|_| claims.approves_runtime()) {
        Some(component) => {
            derive_runtime_component(state, context, target, &claims, component).await
        }
        None => None,
    };
    Ok(document_grant_policy(
        &claims,
        &declared,
        runtime,
        derived.as_ref(),
        &context.engine,
        &context.bundle_sources,
    ))
}

/// GET /registry/package/{package_id}/widget-sandbox/{version}/{path}
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/widget-sandbox/{version}/{path}",
    tag = "registry",
    description = "Serve a package widget inside its sandbox: the host wrapper (frame/{widget_id}/{grant}), the widget document (widgets/{widget_id}/index.{grant}.html) or a bundle file. Use grant 0 to run the widget without extra permissions, or a grant from the widget-grant endpoint to run it with the permissions the viewer approved. A grant with approved runtime addresses is written as {grant}~{runtime}.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("version" = String, Path, description = "Package version"),
        ("path" = String, Path, description = "frame/{widget_id}/{grant}[~{runtime}], widgets/{widget_id}/index.{grant}[~{runtime}].html, or a file path inside the widget bundle")
    ),
    responses(
        (status = 200, description = "Wrapper or widget document (no-store; documents vary on User-Agent), or a bundle file (immutable-cached)"),
        (status = 403, description = "No access to this package, a wrapper or document requested outside an iframe, or an HTML, SVG or XML file opened as a page"),
        (status = 404, description = "Package, version, widget, or file not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_widget_sandbox(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Path((package_id, version, path)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    if !is_valid_package_id(&package_id) {
        return Err(ApiError::not_found(format!(
            "Package '{}' not found",
            package_id
        )));
    }
    let route =
        parse_sandbox_path(&path).ok_or_else(|| ApiError::not_found("Widget asset not found"))?;

    let authorized = authorize_widget_version(&state, &user, &package_id, &version).await?;

    if is_refused_sandbox_navigation(route, &headers) {
        return Ok(non_iframe_refusal());
    }

    if let SandboxPath::Asset(asset_path) = route {
        let bytes =
            read_widget_entry(&authorized.registry, &package_id, &version, asset_path).await?;
        return Ok(asset_response(asset_path, bytes));
    }

    let context = WidgetRequestContext::from_request(&state, &headers, &package_id, &version);
    let sources = &context.bundle_sources;

    match route {
        SandboxPath::Frame {
            widget_id,
            grant: None,
        } => frame_response("../", sources, widget_id, BASELINE_GRANT_SEGMENT, false),
        SandboxPath::Frame {
            widget_id,
            grant: Some(grant),
        } => {
            let target = WidgetTarget {
                package_id: &package_id,
                version: &version,
                widget_id,
            };
            match resolve_frame_grant(&state, &context, &authorized, target, grant).await? {
                Some(downloads) => frame_response("../../", sources, widget_id, grant, downloads),
                None => frame_response("../../", sources, widget_id, BASELINE_GRANT_SEGMENT, false),
            }
        }
        SandboxPath::Document { widget_id, grant } => {
            let target = WidgetTarget {
                package_id: &package_id,
                version: &version,
                widget_id,
            };
            let policy = resolve_document_grant(&state, &context, &authorized, target, grant)
                .await?
                .unwrap_or_default();
            let entry = read_widget_entry(
                &authorized.registry,
                &package_id,
                &version,
                &widget_entry_path(widget_id),
            )
            .await?;
            Ok(document_response(
                sources,
                &policy,
                &entry,
                context.engine.local_media,
            ))
        }
        SandboxPath::Asset(_) => Err(ApiError::not_found("Widget asset not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_jwt;
    use crate::routes::registry::widget_grant_jwt::{WidgetGrantParams, sign_widget_grant};
    use crate::routes::registry::widget_policy::{
        describe_stored_widget, grant_runtime, widget_reserved_hosts,
    };
    use axum::http::HeaderValue;
    use flow_like_wasm_schema::widget::{
        ContractInput, ContractInputType, WidgetCapabilities, WidgetContract,
    };
    use flow_like_wasm_schema::widget_frame::encode_runtime_component;
    use flow_like_wasm_schema::widget_policy::{
        CspDirective, PlatformStorageScope, WIDGET_POLICY_SOURCE_HUB, WidgetCspPurpose,
        WidgetNetworkInput, WidgetPolicySubject, WidgetRuntimeContext,
    };
    use std::collections::BTreeMap;

    const PKG: &str = "com.example.maps";
    const VERSION: &str = "1.2.0";
    const WIDGET: &str = "live-map";
    const HASH: &str = "4f1c0a3b2d5e6f708192a3b4c5d6e7f80112233445566778899aabbccddeeff0";
    const JWT: &str = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl-_x";
    const RUNTIME: &str = "eyJ0aWxlVXJsIjpbImh0dHBzOi8vYS5jb20iXX0";
    const APP: &str = "app_01";
    const TILE_HOST: &str = "https://a.tiles.customer-maps.com";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn header(response: &Response, name: HeaderName) -> &str {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }

    async fn body(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn contract(hosts: &[&str]) -> WidgetContract {
        let mut contract = WidgetContract::new(WIDGET);
        contract.capabilities = Some(WidgetCapabilities {
            workers: Some(true),
            downloads: Some(true),
            media: Some(true),
            ..Default::default()
        });
        if hosts.is_empty() {
            return contract;
        }
        contract.with_csp(vec![WidgetCspPurpose {
            reason: "Loads vector map tiles".into(),
            connect_src: hosts.iter().map(|host| host.to_string()).collect(),
            ..Default::default()
        }])
    }

    fn runtime_contract(hosts: &[&str]) -> WidgetContract {
        let mut contract = contract(hosts);
        contract.inputs.insert(
            "tileUrl".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: None,
                default: None,
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: true,
            },
        );
        let mut purposes = contract.csp.clone().unwrap_or_default();
        purposes.push(WidgetCspPurpose {
            reason: "Loads map tiles from tile servers given to it at runtime".into(),
            inputs: vec![WidgetNetworkInput {
                path: "tileUrl".into(),
                directives: vec![CspDirective::ConnectSrc, CspDirective::ImgSrc],
                template: None,
            }],
            ..Default::default()
        });
        contract.with_csp(purposes)
    }

    fn subject(preview: bool) -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: WIDGET_POLICY_SOURCE_HUB.to_string(),
            package_id: PKG.to_string(),
            package_version: Some(VERSION.to_string()),
            bundle_hash: HASH.to_string(),
            widget_id: WIDGET.to_string(),
            preview,
        }
    }

    fn sandbox_sources() -> Vec<String> {
        bundle_sources(
            &["https://api.flow-like.com".to_string()],
            WIDGET_SANDBOX_ROUTE,
            PKG,
            VERSION,
        )
    }

    /// Deployment and request facts of one describe, mint or document request.
    struct Request {
        reserved: Vec<String>,
        storage: Vec<PlatformStorageScope>,
        sources: Vec<String>,
        gate: EngineGate,
    }

    impl Request {
        fn new(reserved: &[&str]) -> Self {
            let configured: Vec<Option<&str>> = reserved.iter().copied().map(Some).collect();
            Self {
                reserved: widget_reserved_hosts(&configured, None),
                storage: Vec::new(),
                sources: sandbox_sources(),
                gate: EngineGate::OPEN,
            }
        }

        fn derive(
            &self,
            contract: &WidgetContract,
            request: &[WidgetRuntimeSourceRequest],
            app_id: Option<&str>,
        ) -> WidgetPolicyDescriptor {
            describe_stored_widget(
                subject(false),
                &Ok(contract.clone()),
                request,
                &WidgetRuntimeContext {
                    reserved_hosts: &self.reserved,
                    platform_storage: &self.storage,
                    app_id,
                    bundle_sources: &self.sources,
                    engine: self.gate,
                },
            )
        }

        /// The document route after token verification: the declared
        /// descriptor, the runtime component re-derived with this request's
        /// facts and the claims' app id, and the resulting policy.
        fn document(
            &self,
            contract: &WidgetContract,
            claims: &WidgetGrantClaims,
            component: Option<&str>,
        ) -> Option<WidgetPolicy> {
            let declared = self.derive(contract, &[], None);
            let derived = component
                .filter(|_| claims.approves_runtime())
                .and_then(|component| decode_runtime_component(component).ok())
                .map(|slots| {
                    self.derive(
                        contract,
                        &WidgetRuntimeSourceRequest::from_slots(&slots),
                        claims.app_id.as_deref(),
                    )
                });
            document_grant_policy(
                claims,
                &declared,
                component,
                derived.as_ref(),
                &self.gate,
                &self.sources,
            )
        }
    }

    fn descriptor(contract: &WidgetContract, preview: bool) -> WidgetPolicyDescriptor {
        WidgetPolicyDescriptor::describe(
            subject(preview),
            contract,
            &["api.flow-like.com".to_string()],
        )
    }

    fn tile_request(sources: &[&str]) -> Vec<WidgetRuntimeSourceRequest> {
        vec![WidgetRuntimeSourceRequest {
            slot: "tileUrl".into(),
            sources: sources.iter().map(|source| source.to_string()).collect(),
        }]
    }

    /// Mints exactly as the grant endpoint does: version 2 with the runtime
    /// component when the descriptor accepted runtime sources.
    fn mint(descriptor: &WidgetPolicyDescriptor) -> (WidgetGrantClaims, Option<String>) {
        backend_jwt::init_for_tests();
        let (runtime, component) = grant_runtime(descriptor).unwrap();
        let token = sign_widget_grant(WidgetGrantParams {
            package_id: descriptor.package_id.clone(),
            version: descriptor.package_version.clone().unwrap(),
            bundle_hash: descriptor.bundle_hash.clone(),
            widget_id: descriptor.widget_id.clone(),
            preview: descriptor.preview,
            policy_digest: descriptor.policy_digest.clone(),
            runtime,
            ttl_seconds: None,
        })
        .unwrap()
        .token;
        (
            verify_widget_grant(&token, WidgetGrantCheck::Strict).unwrap(),
            component,
        )
    }

    fn claims_for(descriptor: &WidgetPolicyDescriptor) -> WidgetGrantClaims {
        mint(descriptor).0
    }

    #[test]
    fn widget_sandbox_parses_frames_documents_and_assets() {
        assert_eq!(
            parse_sandbox_path("frame/live-map/0"),
            Some(SandboxPath::Frame {
                widget_id: WIDGET,
                grant: Some("0")
            })
        );
        assert_eq!(
            parse_sandbox_path("frame/live-map"),
            Some(SandboxPath::Frame {
                widget_id: WIDGET,
                grant: None
            })
        );
        assert_eq!(
            parse_sandbox_path(&format!("frame/live-map/{JWT}")),
            Some(SandboxPath::Frame {
                widget_id: WIDGET,
                grant: Some(JWT)
            })
        );
        assert_eq!(
            parse_sandbox_path(&format!("widgets/live-map/index.{JWT}.html")),
            Some(SandboxPath::Document {
                widget_id: WIDGET,
                grant: JWT
            })
        );
        assert_eq!(
            parse_sandbox_path("widgets/live-map/index.0.html"),
            Some(SandboxPath::Document {
                widget_id: WIDGET,
                grant: "0"
            })
        );
        let hex = "a".repeat(64);
        assert_eq!(
            parse_sandbox_path(&format!("widgets/live-map/index.{hex}.html")),
            Some(SandboxPath::Document {
                widget_id: WIDGET,
                grant: &hex
            })
        );
        let with_runtime = format!("{JWT}~{RUNTIME}");
        assert_eq!(
            parse_sandbox_path(&format!("frame/live-map/{with_runtime}")),
            Some(SandboxPath::Frame {
                widget_id: WIDGET,
                grant: Some(&with_runtime)
            })
        );
        assert_eq!(
            parse_sandbox_path(&format!("widgets/live-map/index.{with_runtime}.html")),
            Some(SandboxPath::Document {
                widget_id: WIDGET,
                grant: &with_runtime
            })
        );
        for rejected in [
            format!("frame/live-map/0~{RUNTIME}"),
            format!("frame/live-map/{JWT}~"),
            format!("frame/live-map/{JWT}~{RUNTIME}~{RUNTIME}"),
            format!("frame/live-map/{JWT}~a+b"),
        ] {
            assert_eq!(parse_sandbox_path(&rejected), None, "{rejected}");
        }
        assert_eq!(
            parse_sandbox_path(&format!("widgets/live-map/index.0~{RUNTIME}.html")),
            Some(SandboxPath::Asset(&format!(
                "widgets/live-map/index.0~{RUNTIME}.html"
            ))),
            "a baseline document never carries a runtime component"
        );
        assert_eq!(
            parse_sandbox_path("widgets/live-map/index.html"),
            Some(SandboxPath::Asset("widgets/live-map/index.html"))
        );
        assert_eq!(
            parse_sandbox_path("widgets/live-map/index.worker.html"),
            Some(SandboxPath::Asset("widgets/live-map/index.worker.html"))
        );
        assert_eq!(
            parse_sandbox_path("shared/pin.svg"),
            Some(SandboxPath::Asset("shared/pin.svg"))
        );

        for rejected in [
            "frame/live-map/1",
            "frame/live-map/0/extra",
            "frame/../0",
            "frame/",
            "frame/live map/0",
            "widgets/../index.0.html",
            "../bundle.json",
            "",
        ] {
            assert_eq!(parse_sandbox_path(rejected), None, "{rejected}");
        }
        assert_eq!(
            parse_sandbox_path("widgets/live map/index.0.html"),
            None,
            "an invalid widget id never falls through to a raw file"
        );
    }

    #[test]
    fn widget_sandbox_request_authority_prefers_first_valid_forwarded_host() {
        assert_eq!(
            request_authority(&headers(&[
                ("x-forwarded-host", "bad host, App.Flow-Like.com:8443"),
                ("host", "internal:8080"),
            ])),
            Some("app.flow-like.com:8443".to_string())
        );
        assert_eq!(
            request_authority(&headers(&[
                ("x-forwarded-host", "evil.com/path"),
                ("host", "api.flow-like.com"),
            ])),
            Some("api.flow-like.com".to_string())
        );
        assert_eq!(
            request_authority(&headers(&[("host", "api.flow-like.com:0")])),
            None
        );
        assert_eq!(request_authority(&headers(&[("host", "a@b.com")])), None);
        assert_eq!(request_authority(&headers(&[("host", "[::1]:8080")])), None);
        assert_eq!(request_authority(&HeaderMap::new()), None);
    }

    #[test]
    fn widget_sandbox_origins_dedup_hub_and_request_authority() {
        assert_eq!(
            serving_origins(
                "api.flow-like.com",
                true,
                &headers(&[("host", "API.flow-like.com")])
            ),
            vec!["https://api.flow-like.com".to_string()]
        );
        assert_eq!(
            serving_origins(
                "api.flow-like.com",
                true,
                &headers(&[
                    ("x-forwarded-host", "app.flow-like.com"),
                    ("x-forwarded-proto", "https"),
                ])
            ),
            vec![
                "https://api.flow-like.com".to_string(),
                "https://app.flow-like.com".to_string()
            ]
        );
        assert_eq!(
            serving_origins(
                "localhost:8080",
                false,
                &headers(&[("host", "localhost:8080")])
            ),
            vec!["http://localhost:8080".to_string()]
        );
        assert_eq!(
            serving_origins(
                "api.flow-like.com",
                true,
                &headers(&[
                    ("host", "api.flow-like.com"),
                    ("x-forwarded-proto", "gopher")
                ])
            ),
            vec!["https://api.flow-like.com".to_string()]
        );
        assert_eq!(
            serving_origins("", true, &headers(&[("host", "bad host")])),
            Vec::<String>::new()
        );
        assert_eq!(normalize_origin("javascript://x.com"), None);
        assert_eq!(normalize_origin("https://x.com/path"), None);
    }

    #[test]
    fn widget_sandbox_bundle_sources_encode_segments_and_reject_bad_ids() {
        let origins = vec![
            "https://api.flow-like.com".to_string(),
            "http://localhost:8080".to_string(),
        ];
        assert_eq!(
            bundle_sources(&origins, WIDGET_SANDBOX_ROUTE, PKG, "1.2.0+build.7"),
            vec![
                "https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0%2Bbuild.7/".to_string(),
                "http://localhost:8080/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0%2Bbuild.7/".to_string(),
            ]
        );
        assert!(bundle_sources(&origins, WIDGET_SANDBOX_ROUTE, "bad;id", VERSION).is_empty());
        assert!(bundle_sources(&origins, WIDGET_SANDBOX_ROUTE, PKG, "..").is_empty());
        assert!(bundle_sources(&origins, WIDGET_SANDBOX_ROUTE, PKG, "a/b").is_empty());
    }

    #[test]
    fn widget_sandbox_refuses_non_iframe_navigations() {
        assert!(is_non_iframe_navigation(&headers(&[(
            "sec-fetch-dest",
            "document"
        )])));
        assert!(is_non_iframe_navigation(&headers(&[(
            "sec-fetch-dest",
            "empty"
        )])));
        assert!(!is_non_iframe_navigation(&headers(&[(
            "sec-fetch-dest",
            "iframe"
        )])));
        assert!(!is_non_iframe_navigation(&HeaderMap::new()));

        let response = non_iframe_refusal();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(header(&response, header::CONTENT_TYPE), "");
        assert_eq!(header(&response, header::CACHE_CONTROL), NO_STORE);
    }

    #[test]
    fn widget_sandbox_refuses_active_bundle_files_to_navigations() {
        let route = |path: &'static str| parse_sandbox_path(path).unwrap();
        let dest = |value: &str| headers(&[("sec-fetch-dest", value)]);

        for destination in [
            "document",
            "iframe",
            "frame",
            "embed",
            "object",
            "fencedframe",
            "Document",
        ] {
            let headers = dest(destination);
            assert!(is_document_navigation(&headers), "{destination}");
            for path in [
                "shared/pin.svg",
                "widgets/live-map/index.html",
                "widgets/live-map/index.worker.html",
            ] {
                assert!(
                    is_refused_sandbox_navigation(route(path), &headers),
                    "{path} as {destination}"
                );
            }
        }

        for (path, destination) in [
            ("shared/app.js", "script"),
            ("shared/app.css", "style"),
            ("shared/pin.svg", "image"),
            ("shared/pin.svg", "empty"),
            ("shared/inter.woff2", "font"),
            ("shared/engine.wasm", "empty"),
            ("shared/app.js", "document"),
            ("shared/photo.png", "iframe"),
        ] {
            assert!(
                !is_refused_sandbox_navigation(route(path), &dest(destination)),
                "{path} as {destination}"
            );
        }
        for destination in ["script", "style", "image", "font", "empty", "worker"] {
            assert!(!is_document_navigation(&dest(destination)), "{destination}");
        }
        assert!(!is_document_navigation(&HeaderMap::new()));
        assert!(!is_refused_sandbox_navigation(
            route("shared/pin.svg"),
            &HeaderMap::new()
        ));

        for framed in [
            route("widgets/live-map/index.0.html"),
            route("frame/live-map/0"),
        ] {
            assert!(!is_refused_sandbox_navigation(framed, &dest("iframe")));
            assert!(is_refused_sandbox_navigation(framed, &dest("document")));
            assert!(is_refused_sandbox_navigation(framed, &dest("empty")));
            assert!(!is_refused_sandbox_navigation(framed, &HeaderMap::new()));
        }
    }

    #[tokio::test]
    async fn widget_sandbox_frame_pins_the_exact_child_document() {
        let sources = bundle_sources(
            &["https://api.flow-like.com".to_string()],
            WIDGET_SANDBOX_ROUTE,
            PKG,
            VERSION,
        );
        let response = frame_response("../../", &sources, WIDGET, JWT, true).unwrap();

        assert_eq!(header(&response, header::CACHE_CONTROL), "no-store");
        assert_eq!(header(&response, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(header(&response, header::REFERRER_POLICY), "no-referrer");
        let csp = header(&response, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(csp.contains(&format!(
            "frame-src https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.{JWT}.html;"
        )));
        assert!(csp.ends_with("sandbox allow-scripts allow-downloads"));
        assert!(!csp.contains("'self'"));

        let html = body(response).await;
        assert!(html.contains(&format!("'../../widgets/live-map/index.{JWT}.html'")));
        assert!(html.contains("attachShadow({mode:'closed'})"));
        let meta_pin = format!(
            "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.{JWT}.html; base-uri 'none'; form-action 'none'\">"
        );
        assert!(html.starts_with(&meta_pin), "{html}");

        let baseline = frame_response("../", &sources, WIDGET, "0", false).unwrap();
        let csp = header(&baseline, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(csp.contains("/widgets/live-map/index.0.html;"));
        assert!(csp.ends_with("sandbox allow-scripts"));
        assert!(
            body(baseline)
                .await
                .contains("'../widgets/live-map/index.0.html'")
        );

        let segment = format!("{JWT}~{RUNTIME}");
        let runtime = frame_response("../../", &sources, WIDGET, &segment, false).unwrap();
        let csp = header(&runtime, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(
            csp.contains(&format!(
                "frame-src https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.{segment}.html;"
            )),
            "the pin carries the runtime component: {csp}"
        );
        assert!(!csp.contains(&format!("index.{JWT}.html")));
        assert!(
            body(runtime)
                .await
                .contains(&format!("'../../widgets/live-map/index.{segment}.html'"))
        );
        assert!(
            frame_response("../../", &sources, WIDGET, &format!("0~{RUNTIME}"), false).is_err()
        );
    }

    #[tokio::test]
    async fn widget_sandbox_legacy_frame_is_baseline_and_targets_the_sandbox_route() {
        let response = legacy_frame_response(
            &["https://api.flow-like.com".to_string()],
            PKG,
            VERSION,
            WIDGET,
        )
        .unwrap();
        let csp = header(&response, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(csp.contains(
            "frame-src https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.0.html;"
        ));
        assert!(csp.ends_with("sandbox allow-scripts"));
        assert!(
            body(response)
                .await
                .contains("'../../../widget-sandbox/1.2.0/widgets/live-map/index.0.html'")
        );

        let invalid_package = legacy_frame_response(
            &["https://api.flow-like.com".to_string()],
            "bad;id",
            VERSION,
            WIDGET,
        )
        .unwrap();
        assert!(
            header(&invalid_package, header::CONTENT_SECURITY_POLICY).contains("frame-src 'none'")
        );
    }

    #[tokio::test]
    async fn widget_sandbox_document_injects_meta_and_sends_header_csp() {
        let sources = bundle_sources(
            &["https://api.flow-like.com".to_string()],
            WIDGET_SANDBOX_ROUTE,
            PKG,
            VERSION,
        );
        let policy = descriptor(&contract(&["https://tiles.example-maps.com"]), false).policy;
        let entry = b"<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"></head><body></body></html>";
        let response = document_response(&sources, &policy, entry, true);

        assert_eq!(header(&response, header::CONTENT_TYPE), HTML_CONTENT_TYPE);
        assert_eq!(header(&response, header::CACHE_CONTROL), "no-store");
        assert_eq!(header(&response, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(header(&response, header::REFERRER_POLICY), "no-referrer");
        assert!(header(&response, header::VARY).eq_ignore_ascii_case("user-agent"));
        let csp = header(&response, header::CONTENT_SECURITY_POLICY).to_string();
        assert_eq!(csp, widget_document_csp(&sources, &policy, true, true));
        assert!(csp.contains("https://tiles.example-maps.com"));
        assert!(csp.contains("media-src data: blob: "));
        assert!(csp.ends_with("sandbox allow-scripts allow-downloads"));

        let html = body(response).await;
        assert!(html.starts_with("<!doctype html><meta http-equiv=\"Content-Security-Policy\""));
        assert!(!html.contains("default-src *"));
        assert!(!html.contains("sandbox allow-scripts"));

        let baseline = document_response(&sources, &WidgetPolicy::default(), entry, true);
        let csp = header(&baseline, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(csp.contains("connect-src data: blob:;"));
        assert!(csp.contains("media-src data: blob:;"));
        assert!(csp.ends_with("sandbox allow-scripts"));
        assert!(header(&baseline, header::VARY).eq_ignore_ascii_case("user-agent"));

        let gated = document_response(&sources, &WidgetPolicy::default(), entry, false);
        let csp = header(&gated, header::CONTENT_SECURITY_POLICY).to_string();
        assert!(csp.contains("media-src 'none';"), "{csp}");
        assert!(csp.contains("connect-src data: blob:;"));
    }

    #[test]
    fn widget_sandbox_assets_carry_the_locked_asset_csp() {
        let response = asset_response("shared/pin.svg", b"<svg/>".to_vec());
        assert_eq!(header(&response, header::CONTENT_TYPE), "image/svg+xml");
        assert_eq!(
            header(&response, header::CONTENT_SECURITY_POLICY),
            WIDGET_ASSET_CSP
        );
        assert_eq!(header(&response, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(header(&response, header::CACHE_CONTROL), IMMUTABLE_CACHE);

        let html = asset_response("widgets/live-map/extra.html", b"<script>".to_vec());
        assert_eq!(
            header(&html, header::CONTENT_SECURITY_POLICY),
            WIDGET_ASSET_CSP
        );
    }

    #[test]
    fn widget_sandbox_active_assets_vary_on_fetch_destination() {
        for path in ["shared/pin.svg", "widgets/live-map/extra.html"] {
            let response = asset_response(path, b"<svg/>".to_vec());
            assert!(
                header(&response, header::VARY).eq_ignore_ascii_case("sec-fetch-dest"),
                "{path}"
            );
        }
        for path in ["shared/app.js", "shared/app.css", "shared/photo.png"] {
            let response = asset_response(path, Vec::new());
            assert_eq!(header(&response, header::VARY), "", "{path}");
        }
    }

    fn declared_policy(
        claims: &WidgetGrantClaims,
        declared: &WidgetPolicyDescriptor,
    ) -> Option<WidgetPolicy> {
        document_grant_policy(
            claims,
            declared,
            None,
            None,
            &EngineGate::OPEN,
            &sandbox_sources(),
        )
    }

    #[test]
    fn widget_sandbox_grant_unlocks_only_the_matching_policy() {
        let granted = descriptor(&contract(&["https://tiles.example-maps.com"]), false);
        let claims = claims_for(&granted);
        assert!(!claims.approves_runtime());
        assert!(declared_grant_matches(&claims, &granted, None));
        let policy = declared_policy(&claims, &granted).expect("matching grant unlocks");
        assert!(policy.workers && policy.downloads);
        assert_eq!(policy.csp.connect_src, ["https://tiles.example-maps.com"]);

        let widened = descriptor(
            &contract(&[
                "https://tiles.example-maps.com",
                "https://z.example-maps.com",
            ]),
            false,
        );
        assert_eq!(
            declared_policy(&claims, &widened),
            None,
            "digest mismatch after re-derivation"
        );

        let mut other_widget = granted.clone();
        other_widget.widget_id = "other-map".to_string();
        assert_eq!(declared_policy(&claims, &other_widget), None);

        let mut other_version = granted.clone();
        other_version.package_version = Some("1.2.1".to_string());
        assert_eq!(declared_policy(&claims, &other_version), None);

        let mut other_hash = granted.clone();
        other_hash.bundle_hash = "0".repeat(64);
        assert_eq!(declared_policy(&claims, &other_hash), None);

        let preview = descriptor(&contract(&["https://tiles.example-maps.com"]), true);
        assert_eq!(declared_policy(&claims, &preview), None);

        let reserved = descriptor(&contract(&["https://cdn.api.flow-like.com"]), false);
        assert!(!reserved.is_ok());
        assert_eq!(declared_policy(&claims_for(&reserved), &reserved), None);

        assert!(
            !declared_grant_matches(&claims, &granted, Some(RUNTIME)),
            "a version 1 grant never carries a runtime component"
        );
        assert_eq!(
            document_grant_policy(
                &claims,
                &granted,
                Some(RUNTIME),
                None,
                &EngineGate::OPEN,
                &sandbox_sources()
            ),
            None
        );
    }

    #[test]
    fn widget_sandbox_version_two_document_widens_to_the_rederived_runtime_policy() {
        let contract = runtime_contract(&["https://tiles.example-maps.com"]);
        let mint_request = Request::new(&["api.flow-like.com"]);
        let declared = mint_request.derive(&contract, &[], None);
        let approved = mint_request.derive(&contract, &tile_request(&[TILE_HOST]), Some(APP));
        assert_ne!(approved.policy_digest, declared.policy_digest);

        let (claims, component) = mint(&approved);
        let component = component.expect("accepted runtime sources are carried in the URL");
        assert!(claims.approves_runtime());
        assert_eq!(
            claims.declared_digest.as_deref(),
            Some(declared.policy_digest.as_str())
        );
        assert_eq!(claims.app_id.as_deref(), Some(APP));
        assert!(declared_grant_matches(&claims, &declared, Some(&component)));
        assert!(
            declared_grant_matches(&claims, &declared, None),
            "the wrapper accepts a version 2 grant by its declared digest"
        );

        let document_request = Request::new(&["api.flow-like.com"]);
        let policy = document_request
            .document(&contract, &claims, Some(&component))
            .expect("valid version 2 grant");
        assert_eq!(policy, approved.policy);
        assert!(policy.csp.img_src.contains(&TILE_HOST.to_string()));
        let csp = widget_document_csp(&document_request.sources, &policy, true, true);
        assert!(csp.contains(&format!(
            "img-src data: blob: {}",
            document_request.sources[0]
        )));
        assert!(csp.contains(TILE_HOST));

        let declared_only = document_request
            .document(&contract, &claims, None)
            .expect("a missing runtime component serves the declared policy");
        assert_eq!(declared_only, declared.policy);
    }

    #[test]
    fn widget_sandbox_version_two_document_falls_back_on_any_mismatch() {
        let contract = runtime_contract(&["https://tiles.example-maps.com"]);
        let request = Request::new(&["api.flow-like.com"]);
        let declared = request.derive(&contract, &[], None);
        let approved = request.derive(
            &contract,
            &tile_request(&[TILE_HOST, "https://b.tiles.customer-maps.com"]),
            Some(APP),
        );
        let (claims, component) = mint(&approved);
        let component = component.unwrap();

        let tampered = encode_runtime_component(&BTreeMap::from([(
            "tileUrl".to_string(),
            vec![
                TILE_HOST.to_string(),
                "https://b.tiles.customer-maps.com".to_string(),
                "https://collector.example.org".to_string(),
            ],
        )]))
        .unwrap();
        assert_eq!(
            request.document(&contract, &claims, Some(&tampered)),
            Some(declared.policy.clone()),
            "a swapped runtime component does not match the runtime digest"
        );
        assert_eq!(
            request.document(&contract, &claims, Some(RUNTIME)),
            Some(declared.policy.clone())
        );
        assert_eq!(
            request.document(&contract, &claims, Some("AAAA")),
            Some(declared.policy.clone()),
            "an undecodable component serves the declared policy"
        );

        let mut other_app = claims.clone();
        other_app.app_id = Some("other_app".into());
        assert_eq!(
            request.document(&contract, &other_app, Some(&component)),
            Some(declared.policy.clone()),
            "the runtime digest binds the app id"
        );
        let mut anonymous = claims.clone();
        anonymous.app_id = None;
        assert_eq!(
            request.document(&contract, &anonymous, Some(&component)),
            Some(declared.policy.clone())
        );

        let mut wrong_effective = claims.clone();
        wrong_effective.policy_digest = declared.policy_digest.clone();
        assert_eq!(
            request.document(&contract, &wrong_effective, Some(&component)),
            Some(declared.policy.clone())
        );

        let drifted = Request::new(&["api.flow-like.com", "customer-maps.com"]);
        assert_eq!(
            drifted.document(&contract, &claims, Some(&component)),
            Some(declared.policy.clone()),
            "a runtime host that became reserved is rejected on re-derivation"
        );

        let republished = runtime_contract(&[
            "https://tiles.example-maps.com",
            "https://z.example-maps.com",
        ]);
        assert_eq!(
            request.document(&republished, &claims, Some(&component)),
            None,
            "a changed declared policy serves the baseline"
        );
        let mut wrong_declared = claims.clone();
        wrong_declared.declared_digest = Some(approved.policy_digest.clone());
        assert_eq!(
            request.document(&contract, &wrong_declared, Some(&component)),
            None
        );

        let padded = |pad: usize| vec![format!("https://api.flow-like.com/{}/", "a".repeat(pad))];
        let declared_len =
            |pad: usize| widget_document_csp(&padded(pad), &declared.policy, true, true).len();
        let per_char = declared_len(2) - declared_len(1);
        let pad = 1 + (MAX_WIDGET_DOCUMENT_CSP_BYTES - declared_len(1)) / per_char;
        assert!(declared_len(pad) <= MAX_WIDGET_DOCUMENT_CSP_BYTES);
        assert!(
            widget_document_csp(&padded(pad), &approved.policy, true, true).len()
                > MAX_WIDGET_DOCUMENT_CSP_BYTES
        );
        let mut long_origin = Request::new(&["api.flow-like.com"]);
        long_origin.sources = padded(pad);
        assert_eq!(
            long_origin.document(&contract, &claims, Some(&component)),
            Some(declared.policy.clone()),
            "an effective header over budget on this request's origins serves the declared policy"
        );
    }

    #[test]
    fn widget_sandbox_version_two_document_needs_the_exact_runtime_component() {
        let declared_host = "https://tiles.example-maps.com";
        let contract = runtime_contract(&[]);
        let mut purposes = contract.csp.clone().unwrap();
        purposes.insert(
            0,
            WidgetCspPurpose {
                reason: "Loads vector map tiles".into(),
                connect_src: vec![declared_host.into()],
                img_src: vec![declared_host.into()],
                ..Default::default()
            },
        );
        let contract = contract.with_csp(purposes);
        let request = Request::new(&["api.flow-like.com"]);
        let declared = request.derive(&contract, &[], None);
        let approved = request.derive(&contract, &tile_request(&[TILE_HOST]), Some(APP));
        let (claims, component) = mint(&approved);
        let component = component.unwrap();
        assert_eq!(
            request.document(&contract, &claims, Some(&component)),
            Some(approved.policy.clone())
        );

        let padded_slots = BTreeMap::from([(
            "tileUrl".to_string(),
            vec![TILE_HOST.to_string(), declared_host.to_string()],
        )]);
        let padded = encode_runtime_component(&padded_slots).unwrap();
        let rederived = request.derive(
            &contract,
            &WidgetRuntimeSourceRequest::from_slots(&padded_slots),
            Some(APP),
        );
        assert_eq!(
            rederived.runtime.as_ref().unwrap().runtime_digest,
            approved.runtime.as_ref().unwrap().runtime_digest,
            "derivation drops the source every directive already declares"
        );
        assert_eq!(
            request.document(&contract, &claims, Some(&padded)),
            Some(declared.policy.clone()),
            "a component with extra declared sources is not the approved one"
        );
    }

    #[test]
    fn widget_sandbox_engine_gate_narrows_the_served_policy() {
        let contract = runtime_contract(&[
            "https://*.customer-maps.com",
            "https://tiles.example-maps.com",
        ]);
        let request = Request::new(&["api.flow-like.com"]);
        let approved = request.derive(
            &contract,
            &tile_request(&["https://tiles.other-maps.com"]),
            Some(APP),
        );
        let (claims, component) = mint(&approved);
        let component = component.unwrap();
        let open = request
            .document(&contract, &claims, Some(&component))
            .unwrap();
        assert!(
            open.csp
                .connect_src
                .contains(&"https://*.customer-maps.com".to_string())
        );
        assert!(
            open.csp
                .connect_src
                .contains(&"https://tiles.other-maps.com".to_string())
        );

        let mut no_runtime = Request::new(&["api.flow-like.com"]);
        no_runtime.gate = EngineGate {
            runtime_sources: false,
            ..EngineGate::OPEN
        };
        let narrowed = no_runtime
            .document(&contract, &claims, Some(&component))
            .unwrap();
        assert!(
            !narrowed
                .csp
                .connect_src
                .contains(&"https://tiles.other-maps.com".to_string())
        );
        assert!(
            narrowed
                .csp
                .connect_src
                .contains(&"https://*.customer-maps.com".to_string())
        );

        let mut no_wildcards = Request::new(&["api.flow-like.com"]);
        no_wildcards.gate = EngineGate {
            wildcard_sources: false,
            ..EngineGate::OPEN
        };
        let narrowed = no_wildcards
            .document(&contract, &claims, Some(&component))
            .unwrap();
        assert!(
            !narrowed
                .csp
                .connect_src
                .contains(&"https://*.customer-maps.com".to_string())
        );
        assert!(
            narrowed
                .csp
                .connect_src
                .contains(&"https://tiles.other-maps.com".to_string())
        );
        assert!(
            narrowed
                .csp
                .connect_src
                .contains(&"https://tiles.example-maps.com".to_string())
        );

        let mut no_declared = Request::new(&["api.flow-like.com"]);
        no_declared.gate = EngineGate {
            declared_sources: false,
            ..EngineGate::OPEN
        };
        let declared = request.derive(&contract, &[], None);
        let declared_claims = claims_for(&declared);
        let narrowed = no_declared
            .document(&contract, &declared_claims, None)
            .unwrap();
        assert!(narrowed.csp.is_empty());
        assert!(narrowed.workers && narrowed.downloads);
    }
}

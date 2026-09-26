//! Widget bundle access shared by every widget route, plus the legacy
//! `widget-asset` route.
//!
//! Widget files are read from the objects unpacked under `widget-assets/` at
//! publish time, falling back to the stored `.flwb` while that unpack is still
//! in flight. Access control mirrors `GET /registry/package/{id}` so store
//! previews work pre-install.
//!
//! The legacy route predates per-widget grants and never widens a widget
//! beyond the baseline: `frame/{widget_id}` returns the baseline wrapper pinned
//! to the `widget-sandbox` route's `index.0.html`, entry documents carry the
//! baseline document CSP, and every other file the asset CSP. Wrappers and
//! entries only load as iframes, and files that render as scriptable documents
//! are refused to navigations, exactly as on the `widget-sandbox` route.

use super::ServerRegistry;
use super::widget_policy::request_engine_gate;
use super::widget_sandbox::{
    WIDGET_ASSET_ROUTE, asset_response, bundle_sources, document_response,
    is_active_asset_navigation, is_non_iframe_navigation, legacy_frame_response,
    non_iframe_refusal, serving_origins,
};
use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility};
use crate::entity::{wasm_package, wasm_package_version};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use flow_like_wasm_schema::widget_bundle::{WidgetBundleReader, widget_entry_path};
use flow_like_wasm_schema::widget_frame::is_frame_widget_id;
use flow_like_wasm_schema::widget_policy::WidgetPolicy;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use std::sync::Arc;

/// A package version the caller may load widgets from.
pub struct AuthorizedWidgetVersion {
    pub registry: Arc<ServerRegistry>,
    pub sub: Option<String>,
    pub bundle_hash: String,
}

/// Access checks shared by every widget route. They mirror
/// `GET /registry/package/{id}` plus version-level visibility, and require the
/// version to ship a widget bundle. A widget grant never substitutes for them.
pub async fn authorize_widget_version(
    state: &AppState,
    user: &AppUser,
    package_id: &str,
    version: &str,
) -> Result<AuthorizedWidgetVersion, ApiError> {
    let sub = user.sub().ok();

    if !state.platform_config.features.unauthorized_read && sub.is_none() {
        return Err(ApiError::FORBIDDEN);
    }

    let registry = state
        .wasm_registry
        .clone()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(package_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_id)))?;

    if package.status != WasmPackageStatus::Active {
        if let Some(ref uid) = sub {
            let access = crate::check_wasm_access!(state, uid, package_id);
            if access.is_none() {
                return Err(ApiError::not_found(format!(
                    "Package '{}' not found",
                    package_id
                )));
            }
        } else {
            return Err(ApiError::not_found(format!(
                "Package '{}' not found",
                package_id
            )));
        }
    }

    if package.visibility == WasmPackageVisibility::Private {
        let uid = sub.clone().ok_or(ApiError::FORBIDDEN)?;
        let access = crate::check_wasm_access!(state, &uid, package_id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    let sees_all_versions = package.visibility == WasmPackageVisibility::Private
        || super::viewer_can_manage(state, sub.as_deref(), package_id).await?;
    let bundle_hash = visible_bundle_hash(state, package_id, version, sees_all_versions).await?;

    Ok(AuthorizedWidgetVersion {
        registry,
        sub,
        bundle_hash,
    })
}

/// Widget bundle hash of one package version, under the version visibility of
/// `GET /registry/package/{id}`: a version that is not `Active` exists only for
/// a viewer who sees all versions.
///
/// A version row never changes its bundle hash, so it is memoized. Approval
/// changes the status, so only a viewer who sees all versions is answered from
/// the memo; everyone else reads the row.
async fn visible_bundle_hash(
    state: &AppState,
    package_id: &str,
    version: &str,
    sees_all_versions: bool,
) -> Result<String, ApiError> {
    let cache_key = format!(
        "widget_bundle_hash:{}:{}:{}",
        package_id.len(),
        package_id,
        version
    );
    if sees_all_versions && let Some(bundle_hash) = state.get_cache::<String>(&cache_key) {
        return Ok(bundle_hash);
    }

    let (bundle_hash, status) = wasm_package_version::Entity::find()
        .select_only()
        .column(wasm_package_version::Column::WidgetBundleHash)
        .column(wasm_package_version::Column::Status)
        .filter(wasm_package_version::Column::PackageId.eq(package_id))
        .filter(wasm_package_version::Column::Version.eq(version))
        .into_tuple::<(Option<String>, WasmPackageStatus)>()
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Version '{}' not found", version)))?;

    if !sees_all_versions && status != WasmPackageStatus::Active {
        return Err(ApiError::not_found(format!(
            "Version '{}' not found",
            version
        )));
    }

    let bundle_hash = bundle_hash
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| ApiError::not_found("This package version ships no widgets"))?;
    state.set_cache(cache_key, &bundle_hash);

    Ok(bundle_hash)
}

/// Read one bundle file of a package version: the unpacked object when it
/// exists, otherwise the entry of the stored bundle (which verifies its hash).
pub async fn read_widget_entry(
    registry: &ServerRegistry,
    package_id: &str,
    version: &str,
    path: &str,
) -> Result<Vec<u8>, ApiError> {
    if let Ok(Some(bytes)) = registry
        .get_widget_asset_object(package_id, version, path)
        .await
    {
        return Ok(bytes);
    }

    let bundle_bytes = registry
        .get_widget_bundle_bytes(package_id, version)
        .await
        .map_err(|_| ApiError::not_found("Widget bundle not found for this version"))?;
    let entry_path = path.to_string();
    flow_like_types::tokio::task::spawn_blocking(move || -> flow_like_types::Result<Vec<u8>> {
        let mut reader = WidgetBundleReader::from_bytes(bundle_bytes)?;
        reader.read_entry(&entry_path)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Failed to read widget asset: {}", e)))?
    .map_err(|_| ApiError::not_found(format!("Widget asset not found: {}", path)))
}

pub fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "map" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Whether a browser renders `content_type` as a document that can run
/// script: HTML, or any XML type (XML, XSL, SVG, XHTML and every `+xml`).
pub fn is_active_document_type(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "text/html" | "text/xml" | "application/xml" | "text/xsl"
    ) || essence.ends_with("+xml")
}

pub fn is_safe_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyAssetPath<'a> {
    Frame(&'a str),
    Entry(&'a str),
    File,
    Invalid,
}

fn parse_legacy_asset_path(path: &str) -> LegacyAssetPath<'_> {
    if let Some(widget_id) = path.strip_prefix("frame/") {
        return if is_frame_widget_id(widget_id) {
            LegacyAssetPath::Frame(widget_id)
        } else {
            LegacyAssetPath::Invalid
        };
    }
    if let Some(widget_id) = path
        .strip_prefix("widgets/")
        .and_then(|rest| rest.strip_suffix("/index.html"))
        .filter(|widget_id| is_frame_widget_id(widget_id))
    {
        return LegacyAssetPath::Entry(widget_id);
    }
    if is_safe_asset_path(path) {
        LegacyAssetPath::File
    } else {
        LegacyAssetPath::Invalid
    }
}

fn is_refused_legacy_navigation(
    route: LegacyAssetPath<'_>,
    path: &str,
    headers: &HeaderMap,
) -> bool {
    match route {
        LegacyAssetPath::Frame(_) | LegacyAssetPath::Entry(_) => is_non_iframe_navigation(headers),
        LegacyAssetPath::File => is_active_asset_navigation(path, headers),
        LegacyAssetPath::Invalid => false,
    }
}

/// GET /registry/package/{package_id}/widget-asset/{version}/{path}
/// Serve one widget file of a published package version at baseline.
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/widget-asset/{version}/{path}",
    tag = "registry",
    description = "Serve a single widget file of a package version without any approved widget permissions. Superseded by the widget-sandbox route; kept for hosts that predate it.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("version" = String, Path, description = "Package version"),
        ("path" = String, Path, description = "Entry path inside the widget bundle, e.g. widgets/{widget_id}/index.html, or frame/{widget_id} for the baseline host wrapper")
    ),
    responses(
        (status = 200, description = "The widget file; documents are no-store, other files immutable-cached"),
        (status = 403, description = "No access to this package, a wrapper or widget document requested outside an iframe, or an HTML, SVG or XML file opened as a page"),
        (status = 404, description = "Package, version, or asset not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_widget_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Path((package_id, version, path)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let route = parse_legacy_asset_path(&path);
    if route == LegacyAssetPath::Invalid {
        return Err(ApiError::not_found("Widget asset not found"));
    }

    let authorized = authorize_widget_version(&state, &user, &package_id, &version).await?;
    if is_refused_legacy_navigation(route, &path, &headers) {
        return Ok(non_iframe_refusal());
    }

    let origins = serving_origins(
        &state.platform_config.domain,
        state.platform_config.secure,
        &headers,
    );

    match route {
        LegacyAssetPath::Frame(widget_id) => {
            legacy_frame_response(&origins, &package_id, &version, widget_id)
        }
        LegacyAssetPath::Entry(widget_id) => {
            let entry = read_widget_entry(
                &authorized.registry,
                &package_id,
                &version,
                &widget_entry_path(widget_id),
            )
            .await?;
            let sources = bundle_sources(&origins, WIDGET_ASSET_ROUTE, &package_id, &version);
            Ok(document_response(
                &sources,
                &WidgetPolicy::default(),
                &entry,
                request_engine_gate(&headers).local_media,
            ))
        }
        LegacyAssetPath::File => {
            let bytes =
                read_widget_entry(&authorized.registry, &package_id, &version, &path).await?;
            Ok(asset_response(&path, bytes))
        }
        LegacyAssetPath::Invalid => Err(ApiError::not_found("Widget asset not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, StatusCode};

    const MAPPED_EXTENSIONS: [&str; 26] = [
        "html", "js", "mjs", "css", "json", "map", "txt", "webp", "png", "jpg", "jpeg", "gif",
        "avif", "ico", "svg", "woff2", "woff", "ttf", "otf", "mp3", "wav", "ogg", "mp4", "webm",
        "wasm", "bin",
    ];

    fn fetch_dest(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-dest", HeaderValue::from_str(value).unwrap());
        headers
    }

    fn refused(path: &str, headers: &HeaderMap) -> bool {
        is_refused_legacy_navigation(parse_legacy_asset_path(path), path, headers)
    }

    #[test]
    fn widget_asset_active_document_types_cover_html_and_xml() {
        for active in [
            "text/html; charset=utf-8",
            " Text/HTML ",
            "image/svg+xml",
            "application/xhtml+xml",
            "text/xml",
            "application/xml; charset=utf-8",
            "text/xsl",
            "application/rss+xml",
        ] {
            assert!(is_active_document_type(active), "{active}");
        }
        for passive in [
            "text/javascript; charset=utf-8",
            "text/css; charset=utf-8",
            "application/json",
            "text/plain; charset=utf-8",
            "image/png",
            "font/woff2",
            "application/wasm",
            "application/octet-stream",
            "",
        ] {
            assert!(!is_active_document_type(passive), "{passive}");
        }

        let active: Vec<&str> = MAPPED_EXTENSIONS
            .into_iter()
            .filter(|ext| is_active_document_type(content_type_for(&format!("f.{ext}"))))
            .collect();
        assert_eq!(active, ["html", "svg"]);
    }

    #[test]
    fn widget_asset_legacy_route_refuses_document_navigations() {
        let document = fetch_dest("document");
        for path in [
            "widgets/x/index.html",
            "frame/x",
            "shared/a.svg",
            "widgets/x/extra.html",
            "widgets/x/index.0.html",
        ] {
            assert!(refused(path, &document), "{path}");
        }
        assert_eq!(non_iframe_refusal().status(), StatusCode::FORBIDDEN);

        for destination in [
            "iframe",
            "frame",
            "embed",
            "object",
            "fencedframe",
            "IFRAME",
        ] {
            let headers = fetch_dest(destination);
            assert!(refused("shared/a.svg", &headers), "{destination}");
            assert!(refused("widgets/x/extra.html", &headers), "{destination}");
        }

        let iframe = fetch_dest("iframe");
        assert!(!refused("frame/x", &iframe));
        assert!(!refused("widgets/x/index.html", &iframe));
        for destination in ["empty", "script", "style", "image"] {
            let headers = fetch_dest(destination);
            assert!(refused("frame/x", &headers), "{destination}");
            assert!(refused("widgets/x/index.html", &headers), "{destination}");
        }

        for (path, destination) in [
            ("shared/a.js", "script"),
            ("shared/a.mjs", "script"),
            ("shared/a.css", "style"),
            ("shared/a.svg", "image"),
            ("shared/a.svg", "empty"),
            ("shared/a.png", "image"),
            ("shared/a.woff2", "font"),
            ("shared/engine.wasm", "empty"),
            ("widgets/x/contract.json", "empty"),
            ("shared/a.js", "document"),
            ("shared/a.png", "document"),
        ] {
            assert!(
                !refused(path, &fetch_dest(destination)),
                "{path} as {destination}"
            );
        }
        let served = asset_response("shared/a.js", b"export {}".to_vec());
        assert_eq!(served.status(), StatusCode::OK);

        for path in [
            "frame/x",
            "widgets/x/index.html",
            "shared/a.svg",
            "shared/a.js",
        ] {
            assert!(
                !refused(path, &HeaderMap::new()),
                "{path} without Sec-Fetch-Dest"
            );
        }
    }

    #[test]
    fn test_content_types() {
        assert_eq!(
            content_type_for("widgets/sales-chart/index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type_for("shared/react-abc.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type_for("bundle.json"), "application/json");
        assert_eq!(content_type_for("widgets/x/thumbnail.webp"), "image/webp");
        assert_eq!(content_type_for("font.woff2"), "font/woff2");
        assert_eq!(content_type_for("shared/engine.wasm"), "application/wasm");
        assert_eq!(content_type_for("widgets/x/photo.jpg"), "image/jpeg");
        assert_eq!(content_type_for("shared/inter.woff"), "font/woff");
        assert_eq!(content_type_for("unknown.bin"), "application/octet-stream");
    }

    #[test]
    fn test_safe_asset_paths() {
        assert!(is_safe_asset_path("widgets/sales-chart/index.html"));
        assert!(is_safe_asset_path("bundle.json"));
        assert!(!is_safe_asset_path("../secrets"));
        assert!(!is_safe_asset_path("widgets/../../etc/passwd"));
        assert!(!is_safe_asset_path("/absolute"));
        assert!(!is_safe_asset_path("widgets//double"));
        assert!(!is_safe_asset_path("windows\\path"));
        assert!(!is_safe_asset_path(""));
    }

    #[test]
    fn widget_asset_legacy_paths_parse_frames_entries_and_files() {
        assert_eq!(
            parse_legacy_asset_path("frame/sales-chart"),
            LegacyAssetPath::Frame("sales-chart")
        );
        assert_eq!(
            parse_legacy_asset_path("frame/sales-chart/0"),
            LegacyAssetPath::Invalid
        );
        assert_eq!(parse_legacy_asset_path("frame/"), LegacyAssetPath::Invalid);
        assert_eq!(
            parse_legacy_asset_path("widgets/sales-chart/index.html"),
            LegacyAssetPath::Entry("sales-chart")
        );
        assert_eq!(
            parse_legacy_asset_path("widgets/sales-chart/index.0.html"),
            LegacyAssetPath::File
        );
        assert_eq!(
            parse_legacy_asset_path("widgets/sales-chart/contract.json"),
            LegacyAssetPath::File
        );
        assert_eq!(
            parse_legacy_asset_path("shared/chart.svg"),
            LegacyAssetPath::File
        );
        assert_eq!(
            parse_legacy_asset_path("widgets/../index.html"),
            LegacyAssetPath::Invalid
        );
    }
}

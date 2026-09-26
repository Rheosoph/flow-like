//! `flow-widget://` custom protocol: serves unpacked widget-bundle assets from
//! the content-addressed widget store (`<registry cache_dir>/widgets/{package_id}/{bundle_hash}`).
//!
//! URL contract (built by the frontend):
//! - macOS/Linux/iOS: `flow-widget://localhost/{package_id}/{bundle_hash}/{bundle-internal-path}`
//! - Windows/Android: `http://flow-widget.localhost/{package_id}/{bundle_hash}/{bundle-internal-path}`
//!
//! Both forms carry the same path component, so parsing ignores the host
//! entirely. CSP sources name only the current platform's form.
//!
//! Host-authored routes are matched before any file lookup: the wrapper
//! `frame/{widget_id}/{grant|0}` and the entry document
//! `widgets/{widget_id}/index.{grant|0}.html`. Both carry the policy of a
//! grant bound to exactly this package, bundle and widget, else the baseline.
//! Grant ids never carry a runtime component: the grant registry holds the
//! effective policy. Documents are served that policy narrowed by this
//! webview's engine gate. Every other file gets the locked asset CSP.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use flow_like_wasm::widget::is_valid_widget_id;
use flow_like_wasm::widget_bundle::{
    BUNDLE_MANIFEST_PATH, WidgetBundleManifest, widget_entry_path, widget_store_dir,
};
use flow_like_wasm::widget_frame::{
    BASELINE_GRANT_SEGMENT, WIDGET_ASSET_CSP, inject_document_csp_meta, is_desktop_grant_id,
    is_valid_bundle_hash, is_valid_package_id, widget_child_document_urls, widget_document_csp,
    widget_frame_csp, widget_frame_document, widget_frame_meta_csp,
};
use flow_like_wasm::widget_policy::{EngineGate, WidgetPolicy};
use flow_like_wasm::widget_sources::warm_widget_source_data;
use tauri::{AppHandle, Manager, UriSchemeResponder, http};

use crate::state::TauriSettingsState;
use crate::widget_grants::{WIDGET_ENGINE_GATE, WIDGET_GRANTS, WidgetGrantRegistry};

pub const WIDGET_PROTOCOL_SCHEME: &str = "flow-widget";

pub fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    warm_widget_policy_data();
    builder.register_asynchronous_uri_scheme_protocol(
        WIDGET_PROTOCOL_SCHEME,
        |ctx, request, responder| {
            let app_handle = ctx.app_handle().clone();
            let uri_path = request.uri().path().to_string();
            tauri::async_runtime::spawn(async move {
                respond(&app_handle, uri_path, responder).await;
            });
        },
    )
}

/// Parses the source classification data and evaluates the engine gate off
/// the main thread, so the first describe and document never pay for either.
/// Reading the webview version can block until a window exists (Android).
fn warm_widget_policy_data() {
    let spawned = std::thread::Builder::new()
        .name("widget-policy-warm".into())
        .spawn(|| {
            if let Err(error) = warm_widget_source_data() {
                tracing::error!(%error, "Widget source classification data failed to load");
            }
            LazyLock::force(&WIDGET_ENGINE_GATE);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "Failed to start the widget policy warm-up thread");
    }
}

async fn respond(app_handle: &AppHandle, uri_path: String, responder: UriSchemeResponder) {
    let cache_dir = match widget_cache_dir(app_handle).await {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!(%error, path = %uri_path, "flow-widget protocol: failed to resolve widget cache dir");
            responder.respond(status_response(http::StatusCode::INTERNAL_SERVER_ERROR));
            return;
        }
    };

    let response = tauri::async_runtime::spawn_blocking(move || {
        serve_from_store(&cache_dir, &uri_path, &WIDGET_GRANTS, &WIDGET_ENGINE_GATE)
    })
    .await
    .unwrap_or_else(|error| {
        tracing::error!(%error, "flow-widget protocol: serve task failed");
        status_response(http::StatusCode::INTERNAL_SERVER_ERROR)
    });

    responder.respond(response);
}

/// The widget store lives under the same cache dir as the WASM registry cache
/// (`RegistryConfig.cache_dir` built in `registry_init`); derived through the
/// shared helper so the two can never diverge.
async fn widget_cache_dir(app_handle: &AppHandle) -> anyhow::Result<PathBuf> {
    let settings = app_handle
        .try_state::<TauriSettingsState>()
        .ok_or_else(|| anyhow::anyhow!("Settings State not found"))?;
    let project_dir = settings.0.lock().await.project_dir.clone();
    Ok(crate::functions::registry::wasm_registry_cache_dir(
        &project_dir,
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WidgetAssetPath {
    pub package_id: String,
    pub bundle_hash: String,
    pub rest: String,
}

/// Parses `/{package_id}/{bundle_hash}/{rest}` from a request path component.
/// Returns `None` for anything that is not a well-formed, traversal-safe widget
/// asset path.
pub(crate) fn parse_widget_asset_path(uri_path: &str) -> Option<WidgetAssetPath> {
    let decoded = urlencoding::decode(uri_path).ok()?;
    let decoded = decoded.as_ref();
    if decoded.contains('\\') || decoded.contains('\0') {
        return None;
    }

    let trimmed = decoded.strip_prefix('/').unwrap_or(decoded);
    let mut segments = trimmed.split('/');
    let package_id = segments.next()?;
    let bundle_hash = segments.next()?;
    let rest: Vec<&str> = segments.collect();

    if !is_valid_package_id(package_id) || !is_valid_bundle_hash(bundle_hash) {
        return None;
    }
    if rest.is_empty() || !rest.iter().all(|segment| is_safe_path_segment(segment)) {
        return None;
    }

    Some(WidgetAssetPath {
        package_id: package_id.to_string(),
        bundle_hash: bundle_hash.to_string(),
        rest: rest.join("/"),
    })
}

fn is_safe_path_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".."
}

/// `B`: the bundle prefix in the serving form of the current platform only.
/// Naming the other form would let a baseline widget reach a real loopback
/// listener on `flow-widget.localhost`.
pub(crate) fn bundle_source(package_id: &str, bundle_hash: &str) -> String {
    if cfg!(any(target_os = "windows", target_os = "android")) {
        format!("http://{WIDGET_PROTOCOL_SCHEME}.localhost/{package_id}/{bundle_hash}/")
    } else {
        format!("{WIDGET_PROTOCOL_SCHEME}://localhost/{package_id}/{bundle_hash}/")
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WidgetRoute<'a> {
    /// `frame/{widget_id}/{grant}`; `grant` is `None` on the legacy
    /// `frame/{widget_id}` shape.
    Frame {
        widget_id: &'a str,
        grant: Option<&'a str>,
    },
    Document {
        widget_id: &'a str,
        grant: &'a str,
    },
    LegacyDocument {
        widget_id: &'a str,
    },
    Asset,
    Invalid,
}

fn is_desktop_grant_segment(segment: &str) -> bool {
    segment == BASELINE_GRANT_SEGMENT || is_desktop_grant_id(segment)
}

fn widget_route(rest: &str) -> WidgetRoute<'_> {
    let segments: Vec<&str> = rest.split('/').collect();
    match segments[..] {
        ["frame", widget_id] if is_valid_widget_id(widget_id) => WidgetRoute::Frame {
            widget_id,
            grant: None,
        },
        ["frame", widget_id, grant]
            if is_valid_widget_id(widget_id) && is_desktop_grant_segment(grant) =>
        {
            WidgetRoute::Frame {
                widget_id,
                grant: Some(grant),
            }
        }
        ["frame", ..] => WidgetRoute::Invalid,
        ["widgets", widget_id, "index.html"] if is_valid_widget_id(widget_id) => {
            WidgetRoute::LegacyDocument { widget_id }
        }
        ["widgets", widget_id, file] if is_valid_widget_id(widget_id) => file
            .strip_prefix("index.")
            .and_then(|name| name.strip_suffix(".html"))
            .filter(|grant| is_desktop_grant_segment(grant))
            .map_or(WidgetRoute::Asset, |grant| WidgetRoute::Document {
                widget_id,
                grant,
            }),
        _ => WidgetRoute::Asset,
    }
}

fn content_type_for(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "webp" => "image/webp",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Resolves and serves a widget request from the unpacked widget store.
/// Pure given `cache_dir`, `grants` and `gate`; no Tauri runtime required.
pub(crate) fn serve_from_store(
    cache_dir: &Path,
    uri_path: &str,
    grants: &WidgetGrantRegistry,
    gate: &EngineGate,
) -> http::Response<Vec<u8>> {
    let uri_path = uri_path.split_once('?').map_or(uri_path, |(path, _)| path);
    let Some(asset) = parse_widget_asset_path(uri_path) else {
        return status_response(http::StatusCode::BAD_REQUEST);
    };
    let route = widget_route(&asset.rest);
    if route == WidgetRoute::Invalid {
        return status_response(http::StatusCode::BAD_REQUEST);
    }

    let store_dir = widget_store_dir(cache_dir, &asset.package_id, &asset.bundle_hash);
    let Ok(store_dir) = store_dir.canonicalize() else {
        return status_response(http::StatusCode::NOT_FOUND);
    };
    let bundle = bundle_source(&asset.package_id, &asset.bundle_hash);
    let resolve = |widget_id: &str, grant: &str| {
        grants.resolve(grant, &asset.package_id, &asset.bundle_hash, widget_id)
    };

    match route {
        WidgetRoute::Frame { widget_id, grant } => {
            let granted = grant.and_then(|grant| {
                resolve(widget_id, grant).map(|granted| (grant, granted.effective.downloads))
            });
            let (child_grant, allow_downloads) = granted.unwrap_or((BASELINE_GRANT_SEGMENT, false));
            frame_response(
                &bundle,
                widget_id,
                child_grant,
                grant.is_none(),
                allow_downloads,
            )
        }
        WidgetRoute::Document { widget_id, grant } => {
            let policy = resolve(widget_id, grant)
                .map_or_else(WidgetPolicy::default, |granted| granted.served(gate));
            match read_declared_entry(&store_dir, widget_id) {
                Some(body) => document_response(body, &bundle, &policy, gate),
                None => status_response(http::StatusCode::NOT_FOUND),
            }
        }
        WidgetRoute::LegacyDocument { widget_id } => {
            match read_declared_entry(&store_dir, widget_id) {
                Some(body) => document_response(body, &bundle, &WidgetPolicy::default(), gate),
                None => asset_response(&store_dir, &asset.rest),
            }
        }
        WidgetRoute::Asset | WidgetRoute::Invalid => asset_response(&store_dir, &asset.rest),
    }
}

fn frame_response(
    bundle: &str,
    widget_id: &str,
    child_grant: &str,
    legacy: bool,
    allow_downloads: bool,
) -> http::Response<Vec<u8>> {
    let relative = if legacy { "../" } else { "../../" };
    let child_urls =
        widget_child_document_urls(&[bundle.to_string()], widget_id, child_grant, None);
    let body = widget_frame_document(relative, widget_id, child_grant, None, allow_downloads);
    let Some(body) = body.filter(|_| !child_urls.is_empty()) else {
        return status_response(http::StatusCode::BAD_REQUEST);
    };
    let body = inject_document_csp_meta(body.as_bytes(), &widget_frame_meta_csp(&child_urls));
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            http::header::CONTENT_SECURITY_POLICY,
            widget_frame_csp(&child_urls, allow_downloads),
        )
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(http::header::REFERRER_POLICY, "no-referrer")
        .body(body)
        .unwrap_or_else(|_| status_response(http::StatusCode::INTERNAL_SERVER_ERROR))
}

/// The declared entry document of `widget_id`, if `bundle.json` lists the
/// widget with its canonical entry path.
fn read_declared_entry(store_dir: &Path, widget_id: &str) -> Option<Vec<u8>> {
    let manifest_bytes = std::fs::read(store_dir.join(BUNDLE_MANIFEST_PATH)).ok()?;
    let manifest: WidgetBundleManifest = serde_json::from_slice(&manifest_bytes).ok()?;
    let entry = widget_entry_path(widget_id);
    manifest
        .widgets
        .iter()
        .any(|widget| widget.id == widget_id && widget.entry == entry)
        .then(|| read_store_file(store_dir, &entry))
        .flatten()
}

fn read_store_file(store_dir: &Path, rest: &str) -> Option<Vec<u8>> {
    let file_path = store_dir.join(rest).canonicalize().ok()?;
    if !file_path.starts_with(store_dir) || !file_path.is_file() {
        return None;
    }
    std::fs::read(&file_path).ok()
}

fn document_response(
    body: Vec<u8>,
    bundle: &str,
    policy: &WidgetPolicy,
    gate: &EngineGate,
) -> http::Response<Vec<u8>> {
    let bundle_sources = [bundle.to_string()];
    let csp = |include_sandbox| {
        widget_document_csp(&bundle_sources, policy, gate.local_media, include_sandbox)
    };
    let body = inject_document_csp_meta(&body, &csp(false));
    cross_origin_headers(http::Response::builder())
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(http::header::CONTENT_SECURITY_POLICY, csp(true))
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::REFERRER_POLICY, "no-referrer")
        .body(body)
        .unwrap_or_else(|_| status_response(http::StatusCode::INTERNAL_SERVER_ERROR))
}

fn asset_response(store_dir: &Path, rest: &str) -> http::Response<Vec<u8>> {
    let Some(body) = read_store_file(store_dir, rest) else {
        return status_response(http::StatusCode::NOT_FOUND);
    };
    cross_origin_headers(http::Response::builder())
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type_for(rest))
        .header(http::header::CONTENT_SECURITY_POLICY, WIDGET_ASSET_CSP)
        .header(
            http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        )
        .body(body)
        .unwrap_or_else(|_| status_response(http::StatusCode::INTERNAL_SERVER_ERROR))
}

/// Widget iframes deliberately omit `allow-same-origin`, so their subresource
/// requests carry the opaque `Origin: null`. Authorize that serialized origin
/// only; never pair this with credentialed CORS because all sandboxed opaque
/// origins serialize identically.
fn cross_origin_headers(builder: http::response::Builder) -> http::response::Builder {
    builder
        .header(http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "null")
        .header("Cross-Origin-Resource-Policy", "cross-origin")
}

fn status_response(status: http::StatusCode) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("empty status response is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget_grants::{GrantedWidgetPolicy, WIDGET_GRANT_TTL, WidgetGrantBinding};
    use flow_like_wasm::widget_policy::{WidgetCsp, narrow_policy_for_engine};
    use flow_like_wasm::{BuilderWidget, WidgetBundleBuilder, WidgetBundleReader, WidgetContract};

    const HASH: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    const PACKAGE: &str = "com.example.sales";
    const ENTRY: &str = "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src *; connect-src https://evil.example.org\"></head><body>chart</body></html>";

    fn parse(path: &str) -> Option<WidgetAssetPath> {
        parse_widget_asset_path(path)
    }

    fn grants() -> WidgetGrantRegistry {
        WidgetGrantRegistry::new(WIDGET_GRANT_TTL, 64)
    }

    fn header<'a>(response: &'a http::Response<Vec<u8>>, name: &str) -> Option<&'a str> {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().unwrap())
    }

    fn csp(response: &http::Response<Vec<u8>>) -> &str {
        header(response, "content-security-policy").expect("CSP header")
    }

    fn body(response: &http::Response<Vec<u8>>) -> String {
        String::from_utf8(response.body().clone()).unwrap()
    }

    fn assert_platform_sources_only(csp: &str) {
        assert!(!csp.contains("'self'"), "'self' in {csp}");
        let other_form = if cfg!(any(target_os = "windows", target_os = "android")) {
            "flow-widget://"
        } else {
            "http://flow-widget.localhost"
        };
        assert!(!csp.contains(other_form), "{other_form} in {csp}");
        assert!(
            !csp.contains("flow-widget: "),
            "scheme-wide source in {csp}"
        );
    }

    #[test]
    fn parses_widget_entry_path() {
        let parsed = parse(&format!(
            "/com.example.sales/{HASH}/widgets/sales-chart/index.html"
        ))
        .expect("valid path must parse");
        assert_eq!(parsed.package_id, "com.example.sales");
        assert_eq!(parsed.bundle_hash, HASH);
        assert_eq!(parsed.rest, "widgets/sales-chart/index.html");
    }

    #[test]
    fn parses_shared_chunk_path() {
        let parsed = parse(&format!("/com.example.sales/{HASH}/shared/react-a1b2c3.js"))
            .expect("shared chunk path must parse");
        assert_eq!(parsed.rest, "shared/react-a1b2c3.js");
    }

    #[test]
    fn parses_percent_encoded_segments() {
        let parsed = parse(&format!(
            "/com.example.sales/{HASH}/widgets/kpi%20card/index.html"
        ))
        .expect("decoded path must parse");
        assert_eq!(parsed.rest, "widgets/kpi card/index.html");
    }

    #[test]
    fn rejects_traversal_and_malformed_paths() {
        for path in [
            // missing parts
            "",
            "/",
            "/com.example.sales",
            &format!("/com.example.sales/{HASH}"),
            &format!("/com.example.sales/{HASH}/"),
            // traversal in rest, plain and encoded
            &format!("/com.example.sales/{HASH}/../secret"),
            &format!("/com.example.sales/{HASH}/widgets/../../secret"),
            &format!("/com.example.sales/{HASH}/%2e%2e/secret"),
            &format!("/com.example.sales/{HASH}/widgets%2f..%2f..%2fsecret"),
            // empty / dot segments, backslashes, NUL
            &format!("/com.example.sales/{HASH}/widgets//index.html"),
            &format!("/com.example.sales/{HASH}/./index.html"),
            &format!("/com.example.sales/{HASH}/widgets\\index.html"),
            &format!("/com.example.sales/{HASH}/widgets%5cindex.html"),
            &format!("/com.example.sales/{HASH}/index%00.html"),
            // bad package ids
            &format!("/../{HASH}/widgets/x/index.html"),
            &format!("/./{HASH}/widgets/x/index.html"),
            &format!("/com*example/{HASH}/widgets/x/index.html"),
            &format!("/com%20example/{HASH}/widgets/x/index.html"),
            &format!("/com;example/{HASH}/widgets/x/index.html"),
            &format!("//{HASH}/widgets/x/index.html"),
            // bad hashes: wrong length, uppercase, non-hex
            "/com.example.sales/abc123/widgets/x/index.html",
            &format!(
                "/com.example.sales/{}/widgets/x/index.html",
                HASH.to_uppercase()
            ),
            &format!("/com.example.sales/{}Z/widgets/x/index.html", &HASH[..63]),
        ] {
            assert!(parse(path).is_none(), "must reject {path:?}");
        }
    }

    #[test]
    fn widget_routes_are_matched_before_file_lookup() {
        let gid = "f".repeat(64);
        assert_eq!(
            widget_route("frame/sales-chart"),
            WidgetRoute::Frame {
                widget_id: "sales-chart",
                grant: None
            }
        );
        assert_eq!(
            widget_route("frame/sales-chart/0"),
            WidgetRoute::Frame {
                widget_id: "sales-chart",
                grant: Some("0")
            }
        );
        assert_eq!(
            widget_route(&format!("frame/sales-chart/{gid}")),
            WidgetRoute::Frame {
                widget_id: "sales-chart",
                grant: Some(gid.as_str())
            }
        );
        assert_eq!(
            widget_route("widgets/sales-chart/index.0.html"),
            WidgetRoute::Document {
                widget_id: "sales-chart",
                grant: "0"
            }
        );
        assert_eq!(
            widget_route(&format!("widgets/sales-chart/index.{gid}.html")),
            WidgetRoute::Document {
                widget_id: "sales-chart",
                grant: gid.as_str()
            }
        );
        assert_eq!(
            widget_route("widgets/sales-chart/index.html"),
            WidgetRoute::LegacyDocument {
                widget_id: "sales-chart"
            }
        );
        for rest in [
            "frame",
            "frame/sales-chart/1",
            "frame/sales-chart/0/extra",
            "frame/Sales_Chart/0",
            "frame/sales-chart/eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln",
            &format!("frame/sales-chart/{}", gid.to_uppercase()),
            "frame/x\"onload=\"alert(1)",
        ] {
            assert_eq!(widget_route(rest), WidgetRoute::Invalid, "{rest}");
        }
        for rest in [
            "widgets/sales-chart/index.1.html",
            "widgets/sales-chart/INDEX.0.HTML",
            "widgets/sales-chart/index.0.htm",
            "widgets/sales-chart/contract.json",
            "widgets/kpi card/index.0.html",
            "widgets/sales-chart/nested/index.0.html",
            "shared/index.0.html",
            "bundle.json",
        ] {
            assert_eq!(widget_route(rest), WidgetRoute::Asset, "{rest}");
        }
        assert_eq!(
            widget_route(&format!("frame/sales-chart/{gid}~e30")),
            WidgetRoute::Invalid,
            "desktop grants never carry a runtime component"
        );
        assert_eq!(
            widget_route(&format!("widgets/sales-chart/index.{gid}~e30.html")),
            WidgetRoute::Asset
        );
        assert_eq!(
            widget_route("frame/sales-chart/0~e30"),
            WidgetRoute::Invalid
        );
    }

    #[test]
    fn content_types_by_extension() {
        assert_eq!(
            content_type_for("widgets/a/index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type_for("shared/react-a1b2c3.js"),
            "text/javascript"
        );
        assert_eq!(content_type_for("shared/entry.mjs"), "text/javascript");
        assert_eq!(content_type_for("widgets/a/style.css"), "text/css");
        assert_eq!(content_type_for("bundle.json"), "application/json");
        assert_eq!(content_type_for("shared/engine.wasm"), "application/wasm");
        assert_eq!(content_type_for("widgets/a/thumbnail.webp"), "image/webp");
        assert_eq!(content_type_for("img.png"), "image/png");
        assert_eq!(content_type_for("icon.svg"), "image/svg+xml");
        assert_eq!(content_type_for("photo.jpg"), "image/jpeg");
        assert_eq!(content_type_for("photo.jpeg"), "image/jpeg");
        assert_eq!(content_type_for("font.woff2"), "font/woff2");
        assert_eq!(content_type_for("data.bin"), "application/octet-stream");
        assert_eq!(content_type_for("noextension"), "application/octet-stream");
    }

    #[test]
    fn widget_bundle_source_uses_only_the_platform_form() {
        let bundle = bundle_source(PACKAGE, HASH);
        if cfg!(any(target_os = "windows", target_os = "android")) {
            assert_eq!(
                bundle,
                format!("http://flow-widget.localhost/{PACKAGE}/{HASH}/")
            );
        } else {
            assert_eq!(bundle, format!("flow-widget://localhost/{PACKAGE}/{HASH}/"));
        }
        assert!(flow_like_wasm::widget_frame::is_bundle_source(&bundle));
    }

    struct TempStore(PathBuf);
    impl TempStore {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "flow-widget-protocol-test-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp store");
            Self(dir)
        }

        /// Installs a bundle with `sales-chart` and `kpi-card` and returns its hash.
        fn install(&self) -> String {
            let widget = |id: &str, entry: &str| BuilderWidget {
                id: id.into(),
                name: id.into(),
                description: String::new(),
                framework: None,
                entry_html: entry.as_bytes().to_vec(),
                contract: WidgetContract::new(id),
                assets: vec!["shared/chunk.js".into()],
                thumbnail: None,
            };
            let (bytes, bundle_hash) = WidgetBundleBuilder::new(PACKAGE, "1.0.0")
                .created_at("2026-09-17T00:00:00Z")
                .add_shared_chunk("chunk.js", b"export {}".to_vec())
                .add_widget(widget("sales-chart", ENTRY))
                .add_widget(widget("kpi-card", "<p>kpi</p>"))
                .build()
                .expect("build bundle");
            WidgetBundleReader::from_bytes(bytes)
                .expect("open bundle")
                .unpack(&self.bundle_dir(&bundle_hash))
                .expect("unpack bundle");
            bundle_hash
        }

        fn bundle_dir(&self, bundle_hash: &str) -> PathBuf {
            widget_store_dir(&self.0, PACKAGE, bundle_hash)
        }

        fn get(&self, grants: &WidgetGrantRegistry, path: &str) -> http::Response<Vec<u8>> {
            self.get_on(grants, &EngineGate::OPEN, path)
        }

        fn get_on(
            &self,
            grants: &WidgetGrantRegistry,
            gate: &EngineGate,
            path: &str,
        ) -> http::Response<Vec<u8>> {
            serve_from_store(&self.0, path, grants, gate)
        }
    }
    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn network_policy() -> WidgetPolicy {
        WidgetPolicy {
            workers: true,
            downloads: true,
            csp: WidgetCsp {
                connect_src: vec!["https://api.maptiler.com".into()],
                ..WidgetCsp::default()
            },
            ..WidgetPolicy::default()
        }
    }

    fn mint_granted(
        grants: &WidgetGrantRegistry,
        bundle_hash: &str,
        widget_id: &str,
        granted: GrantedWidgetPolicy,
    ) -> String {
        grants
            .mint(
                WidgetGrantBinding {
                    package_id: PACKAGE.into(),
                    bundle_hash: bundle_hash.into(),
                    widget_id: widget_id.into(),
                    preview: false,
                },
                granted,
            )
            .unwrap()
            .id
    }

    fn mint(
        grants: &WidgetGrantRegistry,
        bundle_hash: &str,
        widget_id: &str,
        policy: WidgetPolicy,
    ) -> String {
        let granted = GrantedWidgetPolicy {
            effective: policy.clone(),
            declared: policy,
        };
        mint_granted(grants, bundle_hash, widget_id, granted)
    }

    type Sources<'a> = &'a [&'a str];

    /// Source list of one directive, rejecting duplicate directives and tokens.
    fn directive_sources<'a>(csp: &'a str, name: &str) -> Vec<&'a str> {
        let mut matches = csp
            .split(';')
            .map(|directive| directive.split_ascii_whitespace().collect::<Vec<_>>())
            .filter(|tokens| tokens.first() == Some(&name));
        let tokens = matches.next().unwrap_or_else(|| panic!("{name} in {csp}"));
        assert!(matches.next().is_none(), "{name} twice in {csp}");
        let sources = tokens[1..].to_vec();
        for (index, source) in sources.iter().enumerate() {
            assert!(
                !sources[..index].contains(source),
                "{source} twice in {name}: {csp}"
            );
        }
        sources
    }

    fn assert_baseline_document(response: &http::Response<Vec<u8>>, bundle_hash: &str) {
        let bundle = [bundle_source(PACKAGE, bundle_hash)];
        let baseline = WidgetPolicy::default();
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            csp(response),
            widget_document_csp(&bundle, &baseline, true, true)
        );
        assert_eq!(
            directive_sources(csp(response), "connect-src"),
            ["data:", "blob:"]
        );
        assert_eq!(
            directive_sources(csp(response), "media-src"),
            ["data:", "blob:"]
        );
        assert_eq!(directive_sources(csp(response), "worker-src"), ["'none'"]);
        let meta = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
            widget_document_csp(&bundle, &baseline, true, false)
        );
        assert_eq!(
            body(response),
            format!("<!doctype html>{meta}<html><head></head><body>chart</body></html>")
        );
        assert_platform_sources_only(csp(response));
    }

    #[test]
    fn widget_entry_documents_get_the_baseline_csp_and_injected_meta() {
        let store = TempStore::new("document");
        let bundle_hash = store.install();
        let grants = grants();

        let document = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.0.html"),
        );
        assert_baseline_document(&document, &bundle_hash);
        assert_eq!(
            header(&document, "content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(header(&document, "cache-control"), Some("no-store"));
        assert_eq!(header(&document, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&document, "referrer-policy"), Some("no-referrer"));
        assert_eq!(
            header(&document, "access-control-allow-origin"),
            Some("null")
        );
        assert_eq!(
            header(&document, "cross-origin-resource-policy"),
            Some("cross-origin")
        );
        assert!(csp(&document).ends_with("; sandbox allow-scripts"));
        assert!(csp(&document).contains("; connect-src data: blob:; "));
        assert!(csp(&document).contains("; media-src data: blob:; "));
        assert!(!body(&document).contains("evil.example.org"));

        let legacy = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.html"),
        );
        assert_baseline_document(&legacy, &bundle_hash);
        assert_eq!(header(&legacy, "cache-control"), Some("no-store"));

        let undeclared = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/ghost/index.0.html"),
        );
        assert_eq!(undeclared.status(), http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn widget_contract_json_is_not_read_at_serve_time() {
        let store = TempStore::new("contract");
        let bundle_hash = store.install();
        let contract_path = store
            .bundle_dir(&bundle_hash)
            .join("widgets/sales-chart/contract.json");
        std::fs::write(
            &contract_path,
            br#"{"contractVersion":1,"id":"sales-chart","capabilities":{"workers":true,"media":true,"wasm":true,"downloads":true}}"#,
        )
        .unwrap();
        let grants = grants();
        for path in ["index.0.html", "index.html"] {
            let document = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/{path}"),
            );
            assert_baseline_document(&document, &bundle_hash);
            assert!(csp(&document).contains("; worker-src 'none'; "));
            assert!(csp(&document).contains("; media-src data: blob:; "));
            assert!(!csp(&document).contains("wasm-unsafe-eval"));
        }
        let frame = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/0"),
        );
        assert!(!csp(&frame).contains("allow-downloads"));
    }

    #[test]
    fn widget_bundle_files_named_like_grant_documents_are_never_served_raw() {
        let store = TempStore::new("raw");
        let bundle_hash = store.install();
        let gid = "e".repeat(64);
        let widget_dir = store.bundle_dir(&bundle_hash).join("widgets/sales-chart");
        std::fs::write(widget_dir.join("index.0.html"), b"<script>raw()</script>").unwrap();
        std::fs::write(
            widget_dir.join(format!("index.{gid}.html")),
            b"<script>raw()</script>",
        )
        .unwrap();
        let grants = grants();

        for grant in ["0", gid.as_str()] {
            let document = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{grant}.html"),
            );
            assert_baseline_document(&document, &bundle_hash);
            assert!(!body(&document).contains("raw()"));
        }
    }

    #[test]
    fn widget_frame_routes_pin_the_exact_baseline_child() {
        let store = TempStore::new("frame");
        let bundle_hash = store.install();
        let bundle = bundle_source(PACKAGE, &bundle_hash);
        let grants = grants();
        let child = format!("{bundle}widgets/sales-chart/index.0.html");
        let expected_csp = widget_frame_csp(std::slice::from_ref(&child), false);

        for (path, relative) in [
            ("frame/sales-chart/0", "../../"),
            ("frame/sales-chart", "../"),
        ] {
            let frame = store.get(&grants, &format!("/{PACKAGE}/{bundle_hash}/{path}"));
            assert_eq!(frame.status(), http::StatusCode::OK, "{path}");
            let wrapper = widget_frame_document(relative, "sales-chart", "0", None, false).unwrap();
            assert_eq!(
                body(&frame),
                String::from_utf8(inject_document_csp_meta(
                    wrapper.as_bytes(),
                    &widget_frame_meta_csp(std::slice::from_ref(&child)),
                ))
                .unwrap(),
                "{path}"
            );
            assert!(
                body(&frame)
                    .starts_with("<!doctype html><meta http-equiv=\"Content-Security-Policy\"")
            );
            assert!(body(&frame).contains(&format!(
                "frame.setAttribute('src','{relative}widgets/sales-chart/index.0.html');"
            )));
            assert!(body(&frame).contains("frame.setAttribute('sandbox','allow-scripts');"));
            assert_eq!(csp(&frame), expected_csp);
            assert!(csp(&frame).contains(&format!("frame-src {child};")));
            assert_platform_sources_only(csp(&frame));
            assert_eq!(header(&frame, "cache-control"), Some("no-store"));
            assert_eq!(header(&frame, "x-content-type-options"), Some("nosniff"));
            assert_eq!(header(&frame, "referrer-policy"), Some("no-referrer"));
            assert_eq!(
                header(&frame, "content-type"),
                Some("text/html; charset=utf-8")
            );
        }

        for path in [
            "frame/x%22onload%3D%22alert(1)",
            "frame/sales-chart/1",
            "frame/sales-chart/0/extra",
            "frame",
        ] {
            let response = store.get(&grants, &format!("/{PACKAGE}/{bundle_hash}/{path}"));
            assert_eq!(response.status(), http::StatusCode::BAD_REQUEST, "{path}");
        }
        let missing = store.get(
            &grants,
            &format!("/com.example.other/{bundle_hash}/frame/sales-chart/0"),
        );
        assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn widget_grant_routes_apply_only_bound_grants() {
        let store = TempStore::new("grant");
        let bundle_hash = store.install();
        let bundle = bundle_source(PACKAGE, &bundle_hash);
        let grants = grants();
        let gid = mint(&grants, &bundle_hash, "sales-chart", network_policy());

        let frame = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/{gid}"),
        );
        assert_eq!(frame.status(), http::StatusCode::OK);
        let child = format!("{bundle}widgets/sales-chart/index.{gid}.html");
        assert_eq!(
            body(&frame),
            String::from_utf8(inject_document_csp_meta(
                widget_frame_document("../../", "sales-chart", &gid, None, true)
                    .unwrap()
                    .as_bytes(),
                &widget_frame_meta_csp(std::slice::from_ref(&child)),
            ))
            .unwrap()
        );
        assert!(body(&frame).contains(&format!(
            "frame.setAttribute('src','../../widgets/sales-chart/index.{gid}.html');"
        )));
        assert_eq!(csp(&frame), widget_frame_csp(&[child], true));
        assert_eq!(header(&frame, "cache-control"), Some("no-store"));

        let document = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{gid}.html"),
        );
        assert_eq!(document.status(), http::StatusCode::OK);
        let bundle_sources = [bundle.clone()];
        assert_eq!(
            csp(&document),
            widget_document_csp(&bundle_sources, &network_policy(), true, true)
        );
        assert_eq!(
            directive_sources(csp(&document), "connect-src"),
            [
                "data:",
                "blob:",
                bundle.as_str(),
                "https://api.maptiler.com"
            ]
        );
        assert_eq!(
            directive_sources(csp(&document), "worker-src"),
            ["blob:", bundle.as_str()]
        );
        assert_eq!(
            directive_sources(csp(&document), "media-src"),
            ["data:", "blob:"]
        );
        assert!(csp(&document).ends_with("sandbox allow-scripts allow-downloads"));
        assert!(body(&document).contains(&widget_document_csp(
            &bundle_sources,
            &network_policy(),
            true,
            false
        )));
        assert_eq!(header(&document, "cache-control"), Some("no-store"));
        assert_platform_sources_only(csp(&document));

        let foreign = mint(&grants, &bundle_hash, "kpi-card", network_policy());
        let other_bundle = mint(&grants, HASH, "sales-chart", network_policy());
        let unknown = "f".repeat(64);
        for gid in [&foreign, &other_bundle, &unknown] {
            let frame = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/{gid}"),
            );
            let baseline_child = format!("{bundle}widgets/sales-chart/index.0.html");
            assert_eq!(
                body(&frame),
                String::from_utf8(inject_document_csp_meta(
                    widget_frame_document("../../", "sales-chart", "0", None, false)
                        .unwrap()
                        .as_bytes(),
                    &widget_frame_meta_csp(std::slice::from_ref(&baseline_child)),
                ))
                .unwrap()
            );
            assert_eq!(
                csp(&frame),
                widget_frame_csp(
                    &[format!("{bundle}widgets/sales-chart/index.0.html")],
                    false
                )
            );
            let document = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{gid}.html"),
            );
            assert_baseline_document(&document, &bundle_hash);
        }

        grants.revoke(PACKAGE, Some("sales-chart"));
        let revoked = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{gid}.html"),
        );
        assert_baseline_document(&revoked, &bundle_hash);
    }

    #[test]
    fn widget_documents_carry_exact_local_source_lists() {
        let store = TempStore::new("local-sources");
        let bundle_hash = store.install();
        let bundle = bundle_source(PACKAGE, &bundle_hash);
        let b = bundle.as_str();
        let grants = grants();

        let baseline = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.0.html"),
        );
        assert_eq!(
            csp(&baseline),
            format!(
                "default-src 'none'; script-src 'unsafe-inline' {b}; style-src 'unsafe-inline' {b}; \
img-src data: blob: {b}; font-src data: {b}; connect-src data: blob:; worker-src 'none'; \
media-src data: blob:; frame-src 'none'; child-src 'none'; object-src 'none'; \
manifest-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts"
            )
        );

        let hosts = WidgetCsp {
            connect_src: vec!["https://api.cesium.com".into()],
            media_src: vec!["https://media.example.org".into()],
            ..WidgetCsp::default()
        };
        let cases: [(WidgetPolicy, Sources, Sources, Sources); 4] = [
            (
                WidgetPolicy {
                    workers: true,
                    ..WidgetPolicy::default()
                },
                &["data:", "blob:", b],
                &["blob:", b],
                &["data:", "blob:"],
            ),
            (
                WidgetPolicy {
                    media: true,
                    ..WidgetPolicy::default()
                },
                &["data:", "blob:"],
                &["'none'"],
                &["data:", "blob:", b],
            ),
            (
                WidgetPolicy {
                    csp: hosts.clone(),
                    ..WidgetPolicy::default()
                },
                &["data:", "blob:", "https://api.cesium.com"],
                &["'none'"],
                &["data:", "blob:", "https://media.example.org"],
            ),
            (
                WidgetPolicy {
                    workers: true,
                    media: true,
                    csp: hosts,
                    ..WidgetPolicy::default()
                },
                &["data:", "blob:", b, "https://api.cesium.com"],
                &["blob:", b],
                &["data:", "blob:", b, "https://media.example.org"],
            ),
        ];
        for (policy, connect, worker, media) in cases {
            let gid = mint(&grants, &bundle_hash, "sales-chart", policy.clone());
            let document = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{gid}.html"),
            );
            let meta = widget_document_csp(std::slice::from_ref(&bundle), &policy, true, false);
            for header in [csp(&document), meta.as_str()] {
                assert_eq!(
                    directive_sources(header, "connect-src"),
                    connect,
                    "{policy:?}"
                );
                assert_eq!(
                    directive_sources(header, "worker-src"),
                    worker,
                    "{policy:?}"
                );
                assert_eq!(directive_sources(header, "media-src"), media, "{policy:?}");
                assert_eq!(directive_sources(header, "img-src"), ["data:", "blob:", b]);
                assert_eq!(directive_sources(header, "font-src"), ["data:", b]);
                assert_platform_sources_only(header);
            }
            assert!(body(&document).contains(&meta));
        }
    }

    #[test]
    fn widget_documents_are_narrowed_by_the_engine_gate() {
        let store = TempStore::new("engine");
        let bundle_hash = store.install();
        let bundle = bundle_source(PACKAGE, &bundle_hash);
        let b = bundle.as_str();
        let grants = grants();
        const WILDCARD: &str = "https://*.tiles.customer-maps.com";
        const DECLARED: &str = "https://api.maptiler.com";
        const RUNTIME: &str = "https://a.tiles.other-maps.com";

        let declared = WidgetPolicy {
            media: true,
            downloads: true,
            csp: WidgetCsp {
                connect_src: vec![WILDCARD.into(), DECLARED.into()],
                img_src: vec![WILDCARD.into()],
                ..WidgetCsp::default()
            },
            ..WidgetPolicy::default()
        };
        let mut effective = declared.clone();
        effective.csp.connect_src.push(RUNTIME.into());
        effective.csp.img_src.push(RUNTIME.into());
        effective.csp.canonicalize();
        let gid = mint_granted(
            &grants,
            &bundle_hash,
            "sales-chart",
            GrantedWidgetPolicy {
                effective: effective.clone(),
                declared: declared.clone(),
            },
        );
        let path = format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.{gid}.html");
        let bundle_sources = [bundle.clone()];

        let open = store.get(&grants, &path);
        assert_eq!(
            csp(&open),
            widget_document_csp(&bundle_sources, &effective, true, true)
        );
        assert_eq!(
            directive_sources(csp(&open), "connect-src"),
            ["data:", "blob:", WILDCARD, RUNTIME, DECLARED]
        );
        assert_eq!(
            directive_sources(csp(&open), "media-src"),
            ["data:", "blob:", b]
        );

        let closed = EngineGate {
            runtime_sources: false,
            wildcard_sources: false,
            local_media: false,
            declared_sources: true,
        };
        let narrowed = store.get_on(&grants, &closed, &path);
        assert_eq!(narrowed.status(), http::StatusCode::OK);
        assert_eq!(
            directive_sources(csp(&narrowed), "connect-src"),
            ["data:", "blob:", DECLARED]
        );
        assert_eq!(
            directive_sources(csp(&narrowed), "img-src"),
            ["data:", "blob:", b]
        );
        assert_eq!(directive_sources(csp(&narrowed), "media-src"), [b]);
        assert!(csp(&narrowed).ends_with("sandbox allow-scripts allow-downloads"));
        let narrowed_policy = narrow_policy_for_engine(&effective, &declared, &closed);
        assert_eq!(
            csp(&narrowed),
            widget_document_csp(&bundle_sources, &narrowed_policy, false, true)
        );
        assert!(body(&narrowed).contains(&widget_document_csp(
            &bundle_sources,
            &narrowed_policy,
            false,
            false
        )));
        assert!(!body(&narrowed).contains("other-maps.com"));
        assert!(!body(&narrowed).contains("customer-maps.com"));

        let runtime_only = EngineGate {
            runtime_sources: false,
            ..EngineGate::OPEN
        };
        assert_eq!(
            directive_sources(
                csp(&store.get_on(&grants, &runtime_only, &path)),
                "connect-src"
            ),
            ["data:", "blob:", WILDCARD, DECLARED]
        );
        let wildcard_only = EngineGate {
            wildcard_sources: false,
            ..EngineGate::OPEN
        };
        assert_eq!(
            directive_sources(
                csp(&store.get_on(&grants, &wildcard_only, &path)),
                "connect-src"
            ),
            ["data:", "blob:", RUNTIME, DECLARED]
        );
        let declared_off = EngineGate {
            declared_sources: false,
            ..EngineGate::OPEN
        };
        assert_eq!(
            directive_sources(
                csp(&store.get_on(&grants, &declared_off, &path)),
                "connect-src"
            ),
            ["data:", "blob:", RUNTIME]
        );

        let baseline = store.get_on(
            &grants,
            &closed,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.0.html"),
        );
        assert_eq!(directive_sources(csp(&baseline), "media-src"), ["'none'"]);
        assert_eq!(
            directive_sources(csp(&baseline), "connect-src"),
            ["data:", "blob:"]
        );

        let frame = store.get_on(
            &grants,
            &closed,
            &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/{gid}"),
        );
        assert_eq!(
            csp(&frame),
            widget_frame_csp(
                &[format!("{bundle}widgets/sales-chart/index.{gid}.html")],
                true
            )
        );
    }

    #[test]
    fn widget_download_flag_in_the_query_string_is_ignored() {
        let store = TempStore::new("query");
        let bundle_hash = store.install();
        let grants = grants();
        let plain = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/0"),
        );
        for query in ["?downloads=1", "?downloads=1&x=y", "?x=1&downloads=1"] {
            for path in ["frame/sales-chart/0", "frame/sales-chart"] {
                let response =
                    store.get(&grants, &format!("/{PACKAGE}/{bundle_hash}/{path}{query}"));
                assert_eq!(response.status(), http::StatusCode::OK);
                assert!(!body(&response).contains("allow-downloads"));
                assert!(!csp(&response).contains("allow-downloads"));
                assert_eq!(csp(&response), csp(&plain));
            }
            let document = store.get(
                &grants,
                &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.0.html{query}"),
            );
            assert_baseline_document(&document, &bundle_hash);
        }

        let no_downloads = WidgetPolicy {
            downloads: false,
            ..network_policy()
        };
        let gid = mint(&grants, &bundle_hash, "sales-chart", no_downloads);
        let frame = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/frame/sales-chart/{gid}?downloads=1"),
        );
        assert!(!body(&frame).contains("allow-downloads"));
        assert!(!csp(&frame).contains("allow-downloads"));
    }

    #[test]
    fn widget_assets_carry_the_locked_asset_csp() {
        let store = TempStore::new("assets");
        let bundle_hash = store.install();
        let bundle_dir = store.bundle_dir(&bundle_hash);
        std::fs::write(
            bundle_dir.join("shared/icon.svg"),
            b"<svg><script>x()</script></svg>",
        )
        .unwrap();
        std::fs::write(bundle_dir.join("shared/engine.wasm"), b"\0asm").unwrap();
        std::fs::write(
            bundle_dir.join("widgets/sales-chart/extra.html"),
            b"<script>x()</script>",
        )
        .unwrap();
        std::fs::create_dir_all(bundle_dir.join("widgets/ghost")).unwrap();
        std::fs::write(
            bundle_dir.join("widgets/ghost/index.html"),
            b"<script>x()</script>",
        )
        .unwrap();
        let grants = grants();

        for (path, content_type) in [
            ("shared/icon.svg", "image/svg+xml"),
            ("shared/engine.wasm", "application/wasm"),
            ("shared/chunk.js", "text/javascript"),
            ("widgets/sales-chart/extra.html", "text/html; charset=utf-8"),
            ("widgets/ghost/index.html", "text/html; charset=utf-8"),
            ("widgets/sales-chart/contract.json", "application/json"),
            ("bundle.json", "application/json"),
        ] {
            let response = store.get(&grants, &format!("/{PACKAGE}/{bundle_hash}/{path}"));
            assert_eq!(response.status(), http::StatusCode::OK, "{path}");
            assert_eq!(csp(&response), WIDGET_ASSET_CSP, "{path}");
            assert_eq!(
                header(&response, "content-type"),
                Some(content_type),
                "{path}"
            );
            assert_eq!(header(&response, "x-content-type-options"), Some("nosniff"));
            assert_eq!(
                header(&response, "access-control-allow-origin"),
                Some("null")
            );
            assert_eq!(
                header(&response, "cross-origin-resource-policy"),
                Some("cross-origin")
            );
            assert_eq!(
                header(&response, "cache-control"),
                Some("public, max-age=31536000, immutable")
            );
        }
        let chunk = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/shared/chunk.js"),
        );
        assert_eq!(chunk.body().as_slice(), b"export {}");
    }

    #[test]
    fn missing_file_and_directory_yield_404() {
        let store = TempStore::new("missing");
        let bundle_hash = store.install();
        let grants = grants();

        let missing = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/missing.js"),
        );
        assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
        assert!(missing.body().is_empty());

        let directory = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart"),
        );
        assert_eq!(directory.status(), http::StatusCode::NOT_FOUND);

        for path in [
            "widgets/sales-chart/index.html",
            "widgets/sales-chart/index.0.html",
            "frame/sales-chart/0",
        ] {
            let unknown_bundle =
                store.get(&grants, &format!("/com.example.other/{bundle_hash}/{path}"));
            assert_eq!(
                unknown_bundle.status(),
                http::StatusCode::NOT_FOUND,
                "{path}"
            );
        }

        std::fs::remove_file(
            store
                .bundle_dir(&bundle_hash)
                .join("widgets/sales-chart/index.html"),
        )
        .unwrap();
        let deleted_entry = store.get(
            &grants,
            &format!("/{PACKAGE}/{bundle_hash}/widgets/sales-chart/index.0.html"),
        );
        assert_eq!(deleted_entry.status(), http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn escape_attempts_never_leave_the_store_dir() {
        let store = TempStore::new("escape");
        let cache_dir = &store.0;
        let bundle_dir = cache_dir
            .join("widgets")
            .join("com.example.sales")
            .join(HASH);
        std::fs::create_dir_all(&bundle_dir).expect("create bundle dir");
        std::fs::write(cache_dir.join("secret.txt"), b"secret").expect("write sibling secret");
        let grants = grants();

        for path in [
            format!("/com.example.sales/{HASH}/../../secret.txt"),
            format!("/com.example.sales/{HASH}/%2e%2e/%2e%2e/secret.txt"),
            format!("/com.example.sales/{HASH}/..%2f..%2fsecret.txt"),
        ] {
            let response = serve_from_store(cache_dir, &path, &grants, &EngineGate::OPEN);
            assert_eq!(
                response.status(),
                http::StatusCode::BAD_REQUEST,
                "traversal must be rejected before touching the fs: {path:?}"
            );
            assert!(response.body().is_empty());
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(cache_dir.join("secret.txt"), bundle_dir.join("link.txt"))
                .expect("create symlink");
            let response = serve_from_store(
                cache_dir,
                &format!("/com.example.sales/{HASH}/link.txt"),
                &grants,
                &EngineGate::OPEN,
            );
            assert_eq!(
                response.status(),
                http::StatusCode::NOT_FOUND,
                "symlink escaping the store dir must 404"
            );
        }
    }

    #[test]
    fn invalid_request_is_bad_request() {
        let store = TempStore::new("invalid");
        let response = serve_from_store(
            &store.0,
            "/not-enough-segments",
            &grants(),
            &EngineGate::OPEN,
        );
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        assert!(response.body().is_empty());
    }
}

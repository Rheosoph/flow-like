//! `flow-widget://` custom protocol: serves unpacked widget-bundle assets from
//! the content-addressed widget store (`<registry cache_dir>/widgets/{package_id}/{bundle_hash}`).
//!
//! URL contract (frozen, built by the frontend):
//! - macOS/Linux/iOS: `flow-widget://localhost/{package_id}/{bundle_hash}/{bundle-internal-path}`
//! - Windows/Android: `http://flow-widget.localhost/{package_id}/{bundle_hash}/{bundle-internal-path}`
//!
//! Both forms carry the same path component, so parsing ignores the host entirely.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, UriSchemeResponder, http};

use crate::state::TauriSettingsState;

pub const WIDGET_PROTOCOL_SCHEME: &str = "flow-widget";

/// Belt-and-braces CSP for widget documents (the bundle also carries a
/// `<meta http-equiv>` copy injected at pack time). Only set on `.html` responses.
const WIDGET_HTML_CSP: &str = "default-src 'none'; script-src 'unsafe-inline' flow-widget: http://flow-widget.localhost; style-src 'unsafe-inline' flow-widget: http://flow-widget.localhost; img-src data: blob: flow-widget: http://flow-widget.localhost; font-src data: flow-widget: http://flow-widget.localhost";

pub fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
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

async fn respond(app_handle: &AppHandle, uri_path: String, responder: UriSchemeResponder) {
    let cache_dir = match widget_cache_dir(app_handle).await {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!(%error, path = %uri_path, "flow-widget protocol: failed to resolve widget cache dir");
            responder.respond(status_response(http::StatusCode::INTERNAL_SERVER_ERROR));
            return;
        }
    };

    let response =
        tauri::async_runtime::spawn_blocking(move || serve_from_store(&cache_dir, &uri_path))
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

fn is_valid_package_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn is_valid_bundle_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_safe_path_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".."
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
        "webp" => "image/webp",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Resolves and serves a widget asset from the unpacked widget store.
/// Pure given `cache_dir` — no Tauri runtime required (unit-testable).
pub(crate) fn serve_from_store(cache_dir: &Path, uri_path: &str) -> http::Response<Vec<u8>> {
    let Some(asset) = parse_widget_asset_path(uri_path) else {
        return status_response(http::StatusCode::BAD_REQUEST);
    };

    let store_dir =
        flow_like_wasm::widget_store_dir(cache_dir, &asset.package_id, &asset.bundle_hash);
    let Ok(store_dir) = store_dir.canonicalize() else {
        return status_response(http::StatusCode::NOT_FOUND);
    };
    let Ok(file_path) = store_dir.join(&asset.rest).canonicalize() else {
        return status_response(http::StatusCode::NOT_FOUND);
    };
    if !file_path.starts_with(&store_dir) || !file_path.is_file() {
        return status_response(http::StatusCode::NOT_FOUND);
    }
    let Ok(body) = std::fs::read(&file_path) else {
        return status_response(http::StatusCode::NOT_FOUND);
    };

    let mut response = http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type_for(&asset.rest))
        // Widget iframes deliberately omit `allow-same-origin`, so their
        // subresource requests carry the opaque `Origin: null`. Authorize
        // that serialized origin only; never pair this with credentialed
        // CORS because all sandboxed opaque origins serialize identically.
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "null")
        .header("Cross-Origin-Resource-Policy", "cross-origin")
        .header(
            http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        );
    if asset.rest.to_ascii_lowercase().ends_with(".html") {
        response = response.header(http::header::CONTENT_SECURITY_POLICY, WIDGET_HTML_CSP);
    }
    response
        .body(body)
        .unwrap_or_else(|_| status_response(http::StatusCode::INTERNAL_SERVER_ERROR))
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

    const HASH: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn parse(path: &str) -> Option<WidgetAssetPath> {
        parse_widget_asset_path(path)
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
        assert_eq!(content_type_for("widgets/a/thumbnail.webp"), "image/webp");
        assert_eq!(content_type_for("img.png"), "image/png");
        assert_eq!(content_type_for("icon.svg"), "image/svg+xml");
        assert_eq!(content_type_for("photo.jpg"), "image/jpeg");
        assert_eq!(content_type_for("photo.jpeg"), "image/jpeg");
        assert_eq!(content_type_for("font.woff2"), "font/woff2");
        assert_eq!(content_type_for("data.bin"), "application/octet-stream");
        assert_eq!(content_type_for("noextension"), "application/octet-stream");
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
    }
    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn serves_existing_file_with_headers() {
        let store = TempStore::new("serve");
        let cache_dir = &store.0;
        let widget_dir = cache_dir
            .join("widgets")
            .join("com.example.sales")
            .join(HASH)
            .join("widgets")
            .join("sales-chart");
        std::fs::create_dir_all(&widget_dir).expect("create widget dir");
        std::fs::write(widget_dir.join("index.html"), b"<h1>hi</h1>").expect("write entry");
        std::fs::write(widget_dir.join("chunk.js"), b"export {}").expect("write chunk");

        let html = serve_from_store(
            cache_dir,
            &format!("/com.example.sales/{HASH}/widgets/sales-chart/index.html"),
        );
        assert_eq!(html.status(), http::StatusCode::OK);
        assert_eq!(html.body().as_slice(), b"<h1>hi</h1>");
        assert_eq!(
            html.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            html.headers()
                .get(http::header::CONTENT_SECURITY_POLICY)
                .unwrap(),
            WIDGET_HTML_CSP
        );
        assert_eq!(
            html.headers()
                .get(http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "null"
        );
        assert_eq!(
            html.headers().get("Cross-Origin-Resource-Policy").unwrap(),
            "cross-origin"
        );
        assert_eq!(
            html.headers().get(http::header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );

        let js = serve_from_store(
            cache_dir,
            &format!("/com.example.sales/{HASH}/widgets/sales-chart/chunk.js"),
        );
        assert_eq!(js.status(), http::StatusCode::OK);
        assert_eq!(
            js.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/javascript"
        );
        assert_eq!(
            js.headers()
                .get(http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "null"
        );
        assert!(
            js.headers()
                .get(http::header::CONTENT_SECURITY_POLICY)
                .is_none(),
            "CSP header must only be set on .html responses"
        );
    }

    #[test]
    fn missing_file_and_directory_yield_404() {
        let store = TempStore::new("missing");
        let cache_dir = &store.0;
        let widget_dir = cache_dir
            .join("widgets")
            .join("com.example.sales")
            .join(HASH)
            .join("widgets")
            .join("sales-chart");
        std::fs::create_dir_all(&widget_dir).expect("create widget dir");

        let missing = serve_from_store(
            cache_dir,
            &format!("/com.example.sales/{HASH}/widgets/sales-chart/index.html"),
        );
        assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
        assert!(missing.body().is_empty());

        let directory = serve_from_store(
            cache_dir,
            &format!("/com.example.sales/{HASH}/widgets/sales-chart"),
        );
        assert_eq!(directory.status(), http::StatusCode::NOT_FOUND);

        let unknown_bundle = serve_from_store(
            cache_dir,
            &format!("/com.example.other/{HASH}/widgets/sales-chart/index.html"),
        );
        assert_eq!(unknown_bundle.status(), http::StatusCode::NOT_FOUND);
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

        for path in [
            format!("/com.example.sales/{HASH}/../../secret.txt"),
            format!("/com.example.sales/{HASH}/%2e%2e/%2e%2e/secret.txt"),
            format!("/com.example.sales/{HASH}/..%2f..%2fsecret.txt"),
        ] {
            let response = serve_from_store(cache_dir, &path);
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
            let response =
                serve_from_store(cache_dir, &format!("/com.example.sales/{HASH}/link.txt"));
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
        let response = serve_from_store(&store.0, "/not-enough-segments");
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        assert!(response.body().is_empty());
    }
}

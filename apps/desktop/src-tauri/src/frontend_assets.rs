#[cfg(any(target_os = "ios", test))]
use std::path::Path;
use std::{borrow::Cow, collections::HashSet};
#[cfg(target_os = "ios")]
use tauri::Manager;
use tauri::{
    Builder, Context, Runtime, Url,
    http::{Response, StatusCode, header},
    utils::{
        assets::AssetKey,
        config::{HeaderAddition, HeaderConfig, WebviewUrl},
    },
};

#[cfg(any(target_os = "ios", test))]
#[path = "../frontend_payload.rs"]
mod frontend_payload;

const USE_PAGE: &str = "/use.html";

pub(crate) fn register<R: Runtime>(builder: Builder<R>, context: &Context<R>) -> Builder<R> {
    // Development uses Next's server, including Tauri's mobile proxy to it.
    if tauri::is_dev() {
        return builder;
    }
    let exported: HashSet<String> = context
        .assets()
        .iter()
        .map(|(path, _)| path.into_owned())
        .collect();
    #[cfg(target_os = "ios")]
    let bundled: HashSet<String> = serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/frontend-bundle-manifest.json"
    )))
    .expect("valid bundled frontend manifest");
    #[cfg(target_os = "ios")]
    let exported = exported
        .into_iter()
        .chain(bundled.iter().map(|path| format!("/{path}")))
        .collect();
    builder.register_uri_scheme_protocol("tauri", move |ctx, request| {
        let app = ctx.app_handle();
        let window_config = app
            .config()
            .app
            .windows
            .iter()
            .find(|window| window.label == ctx.webview_label());
        let use_https = request.uri().scheme_str() == Some("https")
            || window_config.is_some_and(|window| window.use_https_scheme);
        let request_url = Url::parse(&request.uri().to_string()).ok();
        // A native URL query can synchronously dispatch to the UI thread.
        // Use the initial configuration and request transport in this callback.
        let configured_url = window_config.and_then(|window| match &window.url {
            WebviewUrl::External(url) | WebviewUrl::CustomProtocol(url) => Some(url),
            _ => None,
        });
        let origin = protocol_origin(
            configured_url.or(request_url.as_ref()),
            use_https,
            cfg!(any(windows, target_os = "android")),
        );
        let path = frontend_asset_path(request.uri().path(), &exported);
        #[cfg(target_os = "ios")]
        if let Some(resource) = bundled_path(&path, &bundled) {
            let headers = app.config().app.security.headers.as_ref();
            return match resource {
                Ok((relative_path, mime_type)) => match app.path().resource_dir() {
                    Ok(resources) => {
                        bundled_response(&resources, &relative_path, mime_type, &origin, headers)
                    }
                    Err(_) => {
                        asset_error_response(StatusCode::INTERNAL_SERVER_ERROR, &origin, headers)
                    }
                },
                Err(status) => asset_error_response(status, &origin, headers),
            };
        }
        // Tauri resolves the chosen document, retaining CSP hashes, nonces,
        // MIME detection, and ordinary exported-file fallbacks.
        match app
            .asset_resolver()
            .get_for_scheme(path.into_owned(), use_https)
        {
            Some(asset) => asset_response(
                asset.bytes,
                &asset.mime_type,
                asset.csp_header.as_deref(),
                &origin,
                app.config().app.security.headers.as_ref(),
            )
            .unwrap_or_else(|_| unavailable_response(&origin)),
            None => unavailable_response(&origin),
        }
    })
}

#[cfg(any(target_os = "ios", test))]
fn bundled_path(
    path: &str,
    manifest: &HashSet<String>,
) -> Option<Result<(String, &'static str), StatusCode>> {
    let valid_encoding = path.bytes().enumerate().all(|(index, byte)| {
        byte != b'%'
            || path
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
    });
    if !valid_encoding {
        return Some(Err(StatusCode::BAD_REQUEST));
    }
    let decoded = match urlencoding::decode(path) {
        Ok(decoded) => decoded,
        Err(_) => return Some(Err(StatusCode::BAD_REQUEST)),
    };
    // Tauri falls back to index.html, so a missing bundled file must stay a 404.
    let mime_type = frontend_payload::bundled_mime_type(decoded.trim_end_matches('/'))?;
    let Some(relative_path) = decoded.strip_prefix('/') else {
        return Some(Err(StatusCode::BAD_REQUEST));
    };
    if relative_path.contains('\\')
        || relative_path.chars().any(char::is_control)
        || relative_path
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
    {
        return Some(Err(StatusCode::BAD_REQUEST));
    }
    if !manifest.contains(relative_path) {
        return Some(Err(StatusCode::NOT_FOUND));
    }
    Some(Ok((relative_path.to_owned(), mime_type)))
}

#[cfg(any(target_os = "ios", test))]
fn bundled_response(
    resource_dir: &Path,
    relative_path: &str,
    mime_type: &str,
    origin: &str,
    headers: Option<&HeaderConfig>,
) -> Response<Vec<u8>> {
    let file = resource_dir
        .join(frontend_payload::RESOURCE_DIR)
        .join(relative_path);
    match std::fs::read(file) {
        Ok(bytes) => asset_response(bytes, mime_type, None, origin, headers).unwrap_or_else(|_| {
            asset_error_response(StatusCode::INTERNAL_SERVER_ERROR, origin, headers)
        }),
        Err(error) => {
            let status = if error.kind() == std::io::ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            asset_error_response(status, origin, headers)
        }
    }
}

#[cfg(any(target_os = "ios", test))]
fn asset_error_response(
    status: StatusCode,
    origin: &str,
    headers: Option<&HeaderConfig>,
) -> Response<Vec<u8>> {
    Response::builder()
        .add_configured_headers(headers)
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(b"Frontend asset unavailable".to_vec())
        .expect("valid frontend error response")
}

fn frontend_asset_path<'a>(path: &'a str, exported: &HashSet<String>) -> Cow<'a, str> {
    let decoded = urlencoding::decode(path).unwrap_or_else(|_| Cow::Borrowed(path));
    let key = AssetKey::from(decoded.trim_end_matches('/'));
    let normalized = key.as_ref();
    if normalized.starts_with("/use/")
        && !exported.contains(normalized)
        && !exported.contains(&format!("{normalized}.html"))
        && !exported.contains(&format!("{normalized}/index.html"))
    {
        Cow::Borrowed(USE_PAGE)
    } else {
        Cow::Borrowed(path)
    }
}

fn protocol_origin(url: Option<&Url>, use_https: bool, http_custom_protocol: bool) -> String {
    let Some(url) = url else { return "null".into() };
    if url.scheme() == "data" {
        return "null".into();
    }
    if http_custom_protocol && !matches!(url.scheme(), "http" | "https") {
        let scheme = if use_https { "https" } else { "http" };
        return format!("{scheme}://{}.localhost", url.scheme());
    }
    let Some(host) = url.host() else {
        return "null".into();
    };
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}", url.scheme())
}

fn asset_response(
    bytes: Vec<u8>,
    mime_type: &str,
    csp: Option<&str>,
    origin: &str,
    headers: Option<&HeaderConfig>,
) -> Result<Response<Vec<u8>>, tauri::http::Error> {
    let mut response = Response::builder()
        .add_configured_headers(headers)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::CONTENT_TYPE, mime_type);
    if let Some(csp) = csp {
        response = response.header(header::CONTENT_SECURITY_POLICY, csp);
    }
    response.body(bytes)
}

fn unavailable_response(origin: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(b"Frontend asset unavailable".to_vec())
        .expect("valid frontend error response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::config::HeaderSource;

    fn exported() -> HashSet<String> {
        [
            "/index.html",
            USE_PAGE,
            "/use.txt",
            "/use/__next._full.txt",
            "/use/exported.html",
            "/use/nested/index.html",
            "/use/logo.svg",
            "/use/café logo.svg",
            "/_next/static/app.js",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn bundled_payloads_require_an_exact_manifest_entry() {
        let manifest: HashSet<String> = [
            "viewer/engine.wasm",
            "viewer/engine.wasm.gz",
            "viewer/engine.wasm.gz.3",
            "viewer/café engine.wasm",
            "_next/static/media/inter.woff2",
            "craft/pdfcraft/licenses/sources/krilla.tar.gz",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for (path, expected_path, mime_type) in [
            (
                "/viewer/engine.wasm",
                "viewer/engine.wasm",
                "application/wasm",
            ),
            (
                "/viewer/engine.wasm.gz",
                "viewer/engine.wasm.gz",
                "application/octet-stream",
            ),
            (
                "/viewer/engine.wasm.gz.3",
                "viewer/engine.wasm.gz.3",
                "application/octet-stream",
            ),
            (
                "/viewer/caf%C3%A9%20engine.wasm",
                "viewer/café engine.wasm",
                "application/wasm",
            ),
            (
                "/_next/static/media/inter.woff2",
                "_next/static/media/inter.woff2",
                "font/woff2",
            ),
            (
                "/craft/pdfcraft/licenses/sources/krilla.tar.gz",
                "craft/pdfcraft/licenses/sources/krilla.tar.gz",
                "application/gzip",
            ),
        ] {
            assert_eq!(
                bundled_path(path, &manifest),
                Some(Ok((expected_path.into(), mime_type))),
                "{path}"
            );
        }
        for path in [
            "/viewer/missing.wasm",
            "/viewer/engine.wasm.gz.4",
            "/_next/static/media/missing.woff2",
            "/missing.png",
        ] {
            assert_eq!(
                bundled_path(path, &manifest),
                Some(Err(StatusCode::NOT_FOUND)),
                "{path}"
            );
        }
        for path in [
            "/use/orders/123",
            "/viewer/engine.wasm.js",
            "/viewer/engine.wasm.gz.manifest.json",
            "/_next/static/chunks/app.js",
            "/craft/pdfcraft/licenses/index.html",
            "/index.html",
        ] {
            assert!(bundled_path(path, &manifest).is_none(), "{path}");
        }
    }

    #[test]
    fn bundled_payloads_reject_traversal_and_malformed_paths() {
        for path in [
            "/../engine.wasm",
            "/viewer/../engine.wasm",
            "/viewer/%2E%2E/engine.wasm",
            "/viewer/./engine.wasm",
            "/viewer//engine.wasm",
            "//viewer/engine.wasm",
            "/viewer\\engine.wasm",
            "/viewer%5Cengine.wasm",
            "/viewer/%00engine.wasm",
            "/viewer/%0Aengine.wasm",
            "/viewer/%FFengine.%77asm",
            "/viewer/%GGengine.wasm",
            "/viewer/engine.wasm%",
            "/viewer/engine.wasm/",
            "viewer/engine.wasm",
        ] {
            // Even a malformed entry in the manifest must not authorize traversal.
            let manifest = [path.trim_start_matches('/').to_owned()].into();
            assert_eq!(
                bundled_path(path, &manifest),
                Some(Err(StatusCode::BAD_REQUEST)),
                "{path}"
            );
        }
    }

    #[test]
    fn bundled_payloads_preserve_binary_bodies_and_headers_and_do_not_fall_back() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let resource_dir = std::env::temp_dir().join(format!(
            "flow-like-frontend-bundle-{}-{unique}",
            std::process::id()
        ));
        let payload_root = resource_dir.join(frontend_payload::RESOURCE_DIR);
        std::fs::create_dir_all(payload_root.join("viewer")).unwrap();
        std::fs::create_dir_all(payload_root.join("_next/static/media")).unwrap();
        std::fs::write(payload_root.join("index.html"), b"fallback document").unwrap();
        let headers = HeaderConfig {
            x_content_type_options: Some(HeaderSource::Inline("nosniff".into())),
            ..Default::default()
        };
        for (relative_path, bytes, mime_type) in [
            (
                "viewer/engine.wasm",
                &b"\0asm\x01\0\0\0"[..],
                "application/wasm",
            ),
            (
                "viewer/engine.wasm.gz.0",
                &b"\x1f\x8b\x08\x00\xff\x80"[..],
                "application/octet-stream",
            ),
            (
                "_next/static/media/inter.woff2",
                &b"wOF2\0\x01\0\0"[..],
                "font/woff2",
            ),
        ] {
            std::fs::write(payload_root.join(relative_path), bytes).unwrap();
            let response = bundled_response(
                &resource_dir,
                relative_path,
                mime_type,
                "tauri://localhost",
                Some(&headers),
            );
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.body(), bytes);
            assert_eq!(response.headers()[header::CONTENT_TYPE], mime_type);
            assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "tauri://localhost"
            );
        }
        let missing = bundled_response(
            &resource_dir,
            "viewer/missing.wasm",
            "application/wasm",
            "tauri://localhost",
            Some(&headers),
        );
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()[header::CONTENT_TYPE], "text/plain");
        assert_eq!(missing.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(missing.body(), b"Frontend asset unavailable");
        std::fs::remove_dir_all(resource_dir).unwrap();
    }

    #[test]
    fn app_deep_links_select_html_even_for_file_like_route_names() {
        for path in [
            "/use/orders/123",
            "/use/orders/123/",
            "/use/caf%C3%A9%20sale/%E6%9D%B1%E4%BA%AC",
            "/use/report.svg",
            "/use/report.json",
            "/use/report.html",
            "/use/report.wasm",
            "/use/report.wasm.gz.0",
            "/use/.well-known",
        ] {
            assert_eq!(frontend_asset_path(path, &exported()), USE_PAGE, "{path}");
        }
    }

    #[test]
    fn bundled_exports_under_app_routes_keep_their_resource_urls() {
        let manifest: HashSet<String> = ["use/engine.wasm", "use/engine.wasm.gz.0"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let exported = exported()
            .into_iter()
            .chain(manifest.iter().map(|path| format!("/{path}")))
            .collect();
        for path in ["/use/engine.wasm", "/use/engine.wasm.gz.0"] {
            let resolved = frontend_asset_path(path, &exported);
            assert_eq!(resolved, path);
            assert!(bundled_path(&resolved, &manifest).unwrap().is_ok());
        }
        let app_route = frontend_asset_path("/use/report.wasm", &exported);
        assert_eq!(app_route, USE_PAGE);
        assert!(bundled_path(&app_route, &manifest).is_none());
    }

    #[test]
    fn existing_exports_and_non_app_routes_keep_tauris_resolution() {
        for path in [
            "/use",
            "/use/",
            USE_PAGE,
            "/use.txt",
            "/use/__next._full.txt",
            "/use/exported",
            "/use/exported.html",
            "/use/nested",
            "/use/nested/",
            "/use/logo.svg",
            "/use/caf%C3%A9%20logo.svg",
            "/_next/static/app.js",
            "/useful/orders",
            "/users/a",
            "/settings",
            "/",
        ] {
            assert_eq!(frontend_asset_path(path, &exported()), path, "{path}");
        }
    }

    #[test]
    fn resolved_document_body_csp_and_configured_headers_are_preserved() {
        let bytes = b"<!DOCTYPE html><script nonce=resolved>app</script>".to_vec();
        let csp = "script-src 'nonce-resolved' 'sha256-app'; style-src 'sha256-style'";
        let headers = HeaderConfig {
            x_content_type_options: Some(HeaderSource::Inline("nosniff".into())),
            ..Default::default()
        };
        let response = asset_response(
            bytes.clone(),
            "text/html",
            Some(csp),
            "tauri://localhost",
            Some(&headers),
        )
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), &bytes);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], csp);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "tauri://localhost"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }

    #[test]
    fn cors_origin_matches_each_tauri_transport() {
        let tauri = Url::parse("tauri://localhost/use/orders?id=app").unwrap();
        assert_eq!(
            protocol_origin(Some(&tauri), false, false),
            "tauri://localhost"
        );
        assert_eq!(
            protocol_origin(Some(&tauri), false, true),
            "http://tauri.localhost"
        );
        assert_eq!(
            protocol_origin(Some(&tauri), true, true),
            "https://tauri.localhost"
        );
        let https = Url::parse("https://tauri.localhost/use/orders").unwrap();
        assert_eq!(
            protocol_origin(Some(&https), true, true),
            "https://tauri.localhost"
        );
        let external = Url::parse("https://app.example:8443/use/orders").unwrap();
        assert_eq!(
            protocol_origin(Some(&external), true, false),
            "https://app.example:8443"
        );
        let data = Url::parse("data:text/html,hello").unwrap();
        assert_eq!(protocol_origin(Some(&data), false, false), "null");
    }
}

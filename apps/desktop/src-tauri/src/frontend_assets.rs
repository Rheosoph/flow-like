use std::{borrow::Cow, collections::HashSet};
use tauri::{
    Builder, Context, Runtime, Url,
    http::{Response, StatusCode, header},
    utils::{
        assets::AssetKey,
        config::{HeaderAddition, HeaderConfig, WebviewUrl},
    },
};

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
    fn app_deep_links_select_html_even_for_file_like_route_names() {
        for path in [
            "/use/orders/123",
            "/use/orders/123/",
            "/use/caf%C3%A9%20sale/%E6%9D%B1%E4%BA%AC",
            "/use/report.svg",
            "/use/report.json",
            "/use/report.html",
            "/use/.well-known",
        ] {
            assert_eq!(frontend_asset_path(path, &exported()), USE_PAGE, "{path}");
        }
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

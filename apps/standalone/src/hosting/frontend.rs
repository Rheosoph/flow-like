use anyhow::Context;
use axum::{
    extract::Request,
    http::{HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/standalone_frontend.rs"));

/// Streamed workflow output can name any URL, so every fetch stays on this origin except the
/// map, model and chart infrastructure the shared renderer fetches itself and the origins the
/// owner allowlisted. Map libraries and compressed models run blob workers and WebAssembly, and
/// PDF attachments preview in blob frames.
const DIRECTIVES: &[(&str, &str)] = &[
    ("default-src", "'self'"),
    ("script-src", "'self' 'unsafe-inline' 'wasm-unsafe-eval'"),
    ("style-src", "'self' 'unsafe-inline'"),
    (
        "img-src",
        "'self' data: blob: https://basemaps.cartocdn.com https://*.basemaps.cartocdn.com",
    ),
    ("font-src", "'self' data:"),
    ("media-src", "'self' data: blob:"),
    (
        "connect-src",
        "'self' data: blob: https://basemaps.cartocdn.com https://*.basemaps.cartocdn.com \
         https://fonts.openmaptiles.org https://www.gstatic.com/draco/ \
         https://raw.githack.com/pmndrs/drei-assets/ https://dl.polyhaven.org/file/ph-assets/ \
         https://cdn.plot.ly/un/",
    ),
    ("worker-src", "'self' blob:"),
    ("child-src", "'self' blob:"),
    (
        "frame-src",
        "'self' blob: https://www.youtube-nocookie.com https://www.youtube.com https://player.vimeo.com",
    ),
    ("form-action", "'self'"),
    ("base-uri", "'none'"),
    ("object-src", "'none'"),
    ("frame-ancestors", "'none'"),
];
const OWNER_EXTENSIBLE: [&str; 4] = ["img-src", "media-src", "connect-src", "frame-src"];

pub(super) fn content_security_policy(origins: &[String]) -> anyhow::Result<HeaderValue> {
    let policy = DIRECTIVES
        .iter()
        .map(|(name, sources)| {
            let extra = if OWNER_EXTENSIBLE.contains(name) {
                origins
            } else {
                &[]
            };
            [*name, *sources]
                .into_iter()
                .chain(extra.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("; ");
    HeaderValue::from_str(&policy).with_context(|| {
        format!("Service UI origins {origins:?} do not form a valid Content-Security-Policy")
    })
}

pub(super) fn asset(request: &Request, policy: &HeaderValue) -> Option<Response> {
    let path = request.uri().path();
    let name = match path {
        "/ui" | "/ui/" => "index.html",
        _ => path.strip_prefix("/ui/")?,
    };
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return Some(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    let Ok(index) = ASSETS.binary_search_by(|(asset, _)| asset.cmp(&name)) else {
        return Some(StatusCode::NOT_FOUND.into_response());
    };
    let bytes = ASSETS[index].1;
    let mime = match name.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    };
    let body = if request.method() == Method::HEAD {
        &[][..]
    } else {
        bytes
    };
    let mut response = ([("content-type", mime)], body).into_response();
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("content-security-policy", policy.clone());
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(policy: &HeaderValue, name: &str) -> Vec<String> {
        policy
            .to_str()
            .unwrap()
            .split(';')
            .map(str::trim)
            .find_map(|entry| entry.strip_prefix(name)?.strip_prefix(' '))
            .map(|sources| sources.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn allows(policy: &HeaderValue, name: &str, source: &str) -> bool {
        sources(policy, name).iter().any(|entry| entry == source)
    }

    #[test]
    fn service_ui_policy_confines_fetches_but_allows_workers_and_wasm() {
        let policy = content_security_policy(&[]).unwrap();
        assert_eq!(sources(&policy, "default-src"), ["'self'"]);
        for fetch in OWNER_EXTENSIBLE {
            let sources = sources(&policy, fetch);
            assert!(!sources.is_empty(), "{fetch} must be explicit");
            assert!(
                sources
                    .iter()
                    .all(|source| !matches!(source.as_str(), "*" | "https:" | "http:")),
                "{fetch} must not allow arbitrary origins: {sources:?}"
            );
        }
        assert!(allows(&policy, "script-src", "'wasm-unsafe-eval'"));
        assert!(!allows(&policy, "script-src", "'unsafe-eval'"));
        assert!(allows(&policy, "worker-src", "blob:"));
        assert!(allows(&policy, "frame-src", "blob:"));
        assert!(allows(&policy, "img-src", "data:"));
        assert_eq!(sources(&policy, "frame-ancestors"), ["'none'"]);
        let request = Request::builder()
            .uri("/ui/")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = asset(&request, &policy).unwrap();
        assert_eq!(response.headers()["content-security-policy"], policy);
    }

    #[test]
    fn owner_origins_extend_only_content_fetches() {
        let origin = "https://cdn.example.com/pages/";
        let policy = content_security_policy(&[origin.to_owned()]).unwrap();
        for (name, _) in DIRECTIVES {
            assert_eq!(
                allows(&policy, name, origin),
                OWNER_EXTENSIBLE.contains(name),
                "{name}"
            );
        }
        assert!(content_security_policy(&["https://cdn.example.com\n".to_owned()]).is_err());
    }
}

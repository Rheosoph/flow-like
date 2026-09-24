use axum::{
    extract::Request,
    http::{HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/standalone_frontend.rs"));

pub(super) fn asset(request: &Request) -> Option<Response> {
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
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("base-uri 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self' 'unsafe-inline'"));
    Some(response)
}

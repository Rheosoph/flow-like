use anyhow::Result;
use axum::extract::{FromRequest, Multipart};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    response::IntoResponse,
};
use flow_like::flow_like_storage::{
    Path as StorePath, files::store::FlowLikeStore, object_store::PutPayload,
};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::intercom::BufferedInterComHandler;
use flow_like_types::utils::constant_time_eq;
use flow_like_types::{Bytes, create_id};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};
use crate::utils::UiEmitTarget;
use flow_like_types::sync::Mutex;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpSink {
    pub path: String,
    pub method: String,
    pub auth_token: Option<String>,
    /// Where this sink should execute: "LOCAL", "REMOTE", or "HYBRID"
    #[serde(default)]
    pub sink_execution: Option<String>,
}

const HTTP_SINK_BODY_LIMIT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Default)]
struct ParsedHttpRequestPayload {
    payload: Option<flow_like_types::Value>,
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(normalize_authorization_token)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_authorization_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

fn http_auth_token_matches(headers: &HeaderMap, expected_token: &str) -> bool {
    let expected_token = normalize_authorization_token(expected_token);
    authorization_token_from_headers(headers).is_some_and(|provided_token| {
        constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes())
    })
}

fn is_multipart_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
        })
        .unwrap_or(false)
}

fn is_urlencoded_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value.split(';').next().is_some_and(|mime| {
                mime.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
        .unwrap_or(false)
}

fn decode_form_component(value: &str) -> String {
    let value = value.replace('+', " ");
    urlencoding::decode(&value)
        .unwrap_or(std::borrow::Cow::Borrowed(value.as_str()))
        .into_owned()
}

fn normalize_form_key(raw_key: &str, fallback: &str) -> (String, bool) {
    let decoded = decode_form_component(raw_key);
    let trimmed = decoded.trim();
    let key = if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    };

    if let Some(stripped) = key.strip_suffix("[]") {
        let stripped = stripped.trim();
        return (
            if stripped.is_empty() {
                fallback.to_string()
            } else {
                stripped.to_string()
            },
            true,
        );
    }

    (key, false)
}

fn insert_payload_value(
    map: &mut serde_json::Map<String, flow_like_types::Value>,
    key: String,
    value: flow_like_types::Value,
    force_array: bool,
) {
    match map.get_mut(&key) {
        Some(flow_like_types::Value::Array(values)) => values.push(value),
        Some(existing) => {
            let previous = std::mem::replace(existing, flow_like_types::Value::Null);
            *existing = flow_like_types::Value::Array(vec![previous, value]);
        }
        None if force_array => {
            map.insert(key, flow_like_types::Value::Array(vec![value]));
        }
        None => {
            map.insert(key, value);
        }
    }
}

fn parse_form_encoded_payload(input: &str) -> serde_json::Map<String, flow_like_types::Value> {
    let mut map = serde_json::Map::new();

    for pair in input.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let (key, force_array) = normalize_form_key(raw_key, "value");
        let value = flow_like_types::Value::String(decode_form_component(raw_value));
        insert_payload_value(&mut map, key, value, force_array);
    }

    map
}

fn merge_query_and_body(
    query: serde_json::Map<String, flow_like_types::Value>,
    body: Option<flow_like_types::Value>,
) -> Option<flow_like_types::Value> {
    match (query.is_empty(), body) {
        (true, None) => None,
        (false, None) => Some(flow_like_types::Value::Object(query)),
        (true, Some(body)) => Some(body),
        (false, Some(flow_like_types::Value::Object(mut body_map))) => {
            let mut merged = query;
            merged.append(&mut body_map);
            Some(flow_like_types::Value::Object(merged))
        }
        (false, Some(body)) => {
            let mut merged = query;
            merged.insert("_body".to_string(), body);
            Some(flow_like_types::Value::Object(merged))
        }
    }
}

fn sanitize_request_file_name(filename: Option<&str>, fallback_index: usize) -> String {
    let raw = filename
        .and_then(|name| name.rsplit(['/', '\\']).next())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("file");

    let mut sanitized = String::with_capacity(raw.len().min(120));
    for ch in raw.chars().take(120) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    let sanitized = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    if sanitized.is_empty() {
        format!("file-{fallback_index}")
    } else {
        sanitized.to_string()
    }
}

fn sanitize_store_path_segment(value: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(80));
    for ch in value.chars().take(80) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    let sanitized = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized.to_string()
    }
}

fn flow_path_value(path: &str) -> flow_like_types::Value {
    serde_json::json!({
        "path": path,
        "store_ref": REQUEST_FILES_STORE_REF,
        "cache_store_ref": null
    })
}

async fn parse_multipart_payload(
    request: Request<Body>,
    query: serde_json::Map<String, flow_like_types::Value>,
    body_limit: usize,
    file_store: FlowLikeStore,
    file_path_prefix: String,
) -> std::result::Result<ParsedHttpRequestPayload, (StatusCode, String)> {
    let mut multipart = Multipart::from_request(request, &()).await.map_err(|e| {
        eprintln!("[HTTP] Failed to parse multipart body: {}", e);
        (
            StatusCode::BAD_REQUEST,
            "Invalid multipart/form-data request".to_string(),
        )
    })?;

    let mut body_map = serde_json::Map::new();
    let mut total_bytes = 0usize;
    let mut file_count = 0usize;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        eprintln!("[HTTP] Failed to read multipart field: {}", e);
        (
            StatusCode::BAD_REQUEST,
            "Invalid multipart/form-data field".to_string(),
        )
    })? {
        let raw_name = field.name().unwrap_or("file").to_string();
        let file_name = field.file_name().map(ToOwned::to_owned);
        let (key, force_array) = normalize_form_key(&raw_name, "file");
        let bytes = field.bytes().await.map_err(|e| {
            eprintln!("[HTTP] Failed to read multipart field bytes: {}", e);
            (
                StatusCode::BAD_REQUEST,
                "Invalid multipart/form-data field".to_string(),
            )
        })?;

        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > body_limit {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds size limit".to_string(),
            ));
        }

        if file_name.is_some() {
            file_count += 1;
            let file_index = file_count;
            let sanitized_name = sanitize_request_file_name(file_name.as_deref(), file_index);
            let path = format!("{file_path_prefix}/{file_index:04}-{sanitized_name}");
            file_store
                .as_generic()
                .put(
                    &StorePath::from(path.clone()),
                    PutPayload::from_bytes(Bytes::copy_from_slice(&bytes)),
                )
                .await
                .map_err(|e| {
                    eprintln!("[HTTP] Failed to stage multipart file: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to stage multipart file".to_string(),
                    )
                })?;
            insert_payload_value(&mut body_map, key, flow_path_value(&path), force_array);
        } else {
            let value = flow_like_types::Value::String(String::from_utf8_lossy(&bytes).to_string());
            insert_payload_value(&mut body_map, key, value, force_array);
        }
    }

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, Some(flow_like_types::Value::Object(body_map))),
    })
}

async fn parse_http_request_payload(
    request: Request<Body>,
    body_limit: usize,
    file_store: Option<FlowLikeStore>,
    file_path_prefix: Option<String>,
) -> std::result::Result<ParsedHttpRequestPayload, (StatusCode, String)> {
    let (parts, body) = request.into_parts();
    let query = parts
        .uri
        .query()
        .map(parse_form_encoded_payload)
        .unwrap_or_default();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    if is_multipart_content_type(content_type.as_deref()) {
        let file_store = file_store.ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Temporary file store is not configured".to_string(),
            )
        })?;
        let file_path_prefix = file_path_prefix.ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Temporary file path is not configured".to_string(),
            )
        })?;
        return parse_multipart_payload(
            Request::from_parts(parts, body),
            query,
            body_limit,
            file_store,
            file_path_prefix,
        )
        .await;
    }

    let body_bytes = axum::body::to_bytes(body, body_limit).await.map_err(|e| {
        eprintln!("[HTTP] Failed to read request body: {}", e);
        (
            StatusCode::BAD_REQUEST,
            "Failed to read request body".to_string(),
        )
    })?;

    let body_payload = if body_bytes.is_empty() {
        None
    } else if is_urlencoded_content_type(content_type.as_deref()) {
        let body_str = std::str::from_utf8(&body_bytes)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid form body".to_string()))?;
        Some(flow_like_types::Value::Object(parse_form_encoded_payload(
            body_str,
        )))
    } else {
        match serde_json::from_slice::<flow_like_types::Value>(&body_bytes) {
            Ok(value) => Some(value),
            Err(_) => Some(flow_like_types::Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            )),
        }
    };

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, body_payload),
    })
}

impl HttpSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS http_routes (
                event_id TEXT PRIMARY KEY,
                app_id TEXT NOT NULL,
                path TEXT NOT NULL,
                method TEXT NOT NULL,
                auth_token TEXT,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_http_routes_unique
             ON http_routes(app_id, path, method)",
            [],
        )?;

        Ok(())
    }

    fn add_route(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &HttpSink,
    ) -> Result<()> {
        let method = config.method.trim().to_ascii_uppercase();
        let auth_token = config
            .auth_token
            .as_deref()
            .map(normalize_authorization_token)
            .filter(|token| !token.is_empty())
            .map(ToOwned::to_owned);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let conn = db.lock().unwrap();

        let existing = conn
            .query_row(
                "SELECT event_id FROM http_routes
                 WHERE app_id = ?1 AND path = ?2 AND method = ?3",
                params![registration.app_id, config.path, method],
                |row| row.get::<_, String>(0),
            )
            .ok();

        if let Some(existing_event_id) = existing
            && existing_event_id != registration.event_id
        {
            tracing::warn!(
                "Route conflict: {} {} {} already registered to event {}. Overwriting with event {}",
                registration.app_id,
                method,
                config.path,
                existing_event_id,
                registration.event_id
            );

            conn.execute(
                "DELETE FROM http_routes WHERE event_id = ?1",
                params![existing_event_id],
            )?;
        }

        conn.execute(
            "INSERT INTO http_routes
             (event_id, app_id, path, method, auth_token, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(event_id) DO UPDATE SET
                 app_id = excluded.app_id,
                 path = excluded.path,
                 method = excluded.method,
                 auth_token = excluded.auth_token",
            params![
                registration.event_id,
                registration.app_id,
                config.path,
                method,
                auth_token,
                now,
            ],
        )?;

        Ok(())
    }

    fn remove_route(db: &DbConnection, event_id: &str) -> Result<()> {
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM http_routes WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }

    fn list_routes(db: &DbConnection) -> Result<Vec<(String, String, String, String)>> {
        let conn = db.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT app_id, method, path, event_id FROM http_routes ORDER BY app_id, path, method",
        )?;

        let routes = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(routes)
    }

    async fn health_check() -> impl IntoResponse {
        (StatusCode::OK, "OK")
    }

    async fn handle_request(
        State(state): State<Arc<HttpServerState>>,
        AxumPath((app_id, path)): AxumPath<(String, String)>,
        request: Request<Body>,
    ) -> impl IntoResponse {
        use crate::state::TauriEventSinkManagerState;

        let method = request.method().clone();
        let headers = request.headers().clone();
        let method_str = method.as_str();
        let full_path = format!("/{}", path);
        let path_without_app_id = full_path
            .strip_prefix(&format!("/{}", app_id))
            .unwrap_or(&full_path);

        println!(
            "[HTTP] Received {} request for /{}{}, path without app_id: {}",
            method_str, app_id, full_path, path_without_app_id
        );

        let app_handle = state.app_handle.clone();

        // Query database and release lock immediately to prevent deadlock
        let (event_id, auth_token): (String, Option<String>) = {
            let conn = state.db.lock().unwrap();

            let mut route_stmt = match conn.prepare(
                "SELECT event_id, auth_token FROM http_routes
                     WHERE app_id = ?1 AND path = ?2 AND method = ?3",
            ) {
                Ok(stmt) => stmt,
                Err(e) => {
                    eprintln!("[HTTP] Database error preparing route statement: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
                }
            };

            println!(
                "[HTTP] Querying route for app_id: {}, path: {}, method: {}",
                app_id, path_without_app_id, method_str
            );

            let route_result = route_stmt
                .query_row(params![app_id, path_without_app_id, method_str], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                });

            match route_result {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("[HTTP] Error querying route: {}", e);
                    return (StatusCode::NOT_FOUND, "Route not found").into_response();
                }
            }
            // Lock is released here when conn goes out of scope
        };

        println!("[HTTP] Route found: event_id: {}", event_id);

        if let Some(auth_token) = auth_token {
            if !http_auth_token_matches(&headers, &auth_token) {
                return (StatusCode::UNAUTHORIZED, "Invalid auth token").into_response();
            }
        }

        println!("[HTTP] Authentication passed");

        let temporary_store = match app_handle.try_state::<crate::state::TauriFlowLikeState>() {
            Some(state) => {
                let config = state.0.config.read().await;
                config.stores.temporary_store.clone()
            }
            None => None,
        };
        let file_path_prefix = format!(
            "tmp/global/apps/{}/events/{}/requests/{}",
            sanitize_store_path_segment(&app_id, "app"),
            sanitize_store_path_segment(&event_id, "event"),
            create_id()
        );

        let parsed_payload = match parse_http_request_payload(
            request,
            HTTP_SINK_BODY_LIMIT_BYTES,
            temporary_store,
            Some(file_path_prefix),
        )
        .await
        {
            Ok(payload) => payload,
            Err((status, message)) => return (status, message).into_response(),
        };

        println!("[HTTP] Triggering event: {}", event_id);

        let response = Arc::new(Mutex::new(None));
        let (tx, rx) = flow_like_types::tokio::sync::oneshot::channel::<()>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let app_handle_clone = app_handle.clone();
        let response_clone = response.clone();
        let tx_clone = tx.clone();
        let callback = BufferedInterComHandler::new(
            Arc::new(move |events| {
                let app_handle = app_handle_clone.clone();
                let response = response_clone.clone();
                let tx = tx_clone.clone();
                Box::pin({
                    async move {
                        for event in &events {
                            if event.event_type == "generic_result" {
                                println!("[HTTP] Received generic_result event");
                                let mut resp_lock = response.lock().await;
                                *resp_lock = Some(event.payload.clone());

                                // Signal that we received a response
                                if let Some(sender) = tx.lock().await.take() {
                                    let _ = sender.send(());
                                }
                            }
                        }

                        let first_event = events.first();
                        if let Some(first_event) = first_event {
                            crate::utils::emit_event_batch_throttled(
                                &app_handle,
                                UiEmitTarget::All,
                                &first_event.event_type,
                                events.clone(),
                                std::time::Duration::from_millis(150),
                            );
                        }

                        Ok(())
                    }
                })
            }),
            Some(100),
            Some(400),
            Some(true),
        );

        if let Some(manager_state) = app_handle.try_state::<TauriEventSinkManagerState>() {
            let result = match manager_state.0.try_lock() {
                Ok(manager) => manager.fire_event(
                    &app_handle,
                    &event_id,
                    parsed_payload.payload,
                    Some(callback),
                ),
                Err(_) => {
                    tracing::warn!(
                        "EventSinkManager busy while handling HTTP event {}",
                        event_id
                    );
                    return (StatusCode::SERVICE_UNAVAILABLE, "Event manager busy, retry")
                        .into_response();
                }
            };

            println!("[HTTP] Event {} fired, awaiting result...", event_id);

            if let Err(e) = result {
                eprintln!(
                    "[HTTP] Failed to fire event '{}' for HTTP request: {}",
                    event_id, e
                );
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to trigger event")
                    .into_response();
            }
        } else {
            tracing::error!("EventSinkManager state not available for {}", event_id);
        }

        // Wait for the callback to receive the response (with timeout).
        // Kept aligned with the server's HTTP sink timeout so a caller sees
        // the same window regardless of which transport served the request.
        let timeout_result =
            flow_like_types::tokio::time::timeout(std::time::Duration::from_secs(120), rx).await;

        match timeout_result {
            Ok(Ok(())) => {
                // Response received
                if let Some(resp) = &*response.lock().await {
                    println!("[HTTP] Returning response for event {}", event_id);
                    return (StatusCode::OK, Json(resp.clone())).into_response();
                }
            }
            Ok(Err(_)) => {
                // Channel closed without sending (shouldn't happen)
                tracing::warn!(
                    "[HTTP] Response channel closed without response for event {}",
                    event_id
                );
            }
            Err(_) => {
                // Timeout
                tracing::warn!("[HTTP] Timeout waiting for response for event {}", event_id);
            }
        }

        (StatusCode::OK, "Event triggered").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "test-http-auth-token";

    fn authorization_headers(value: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn auth_accepts_valid_token() {
        let headers = authorization_headers(Some(TOKEN));

        assert!(http_auth_token_matches(&headers, TOKEN));
    }

    #[test]
    fn auth_rejects_invalid_or_missing_token() {
        let invalid = authorization_headers(Some("best-http-auth-token"));

        assert!(!http_auth_token_matches(&invalid, TOKEN));
        assert!(!http_auth_token_matches(&HeaderMap::new(), TOKEN));
    }

    #[test]
    fn auth_preserves_bearer_token_normalization() {
        let bearer = authorization_headers(Some("bEaReR   test-http-auth-token"));
        let raw = authorization_headers(Some(TOKEN));

        assert!(http_auth_token_matches(
            &bearer,
            "Bearer test-http-auth-token"
        ));
        assert!(http_auth_token_matches(&raw, "Bearer test-http-auth-token"));
    }
}

#[derive(Clone)]
struct HttpServerState {
    db: DbConnection,
    app_handle: AppHandle,
}

#[async_trait::async_trait]
impl EventSink for HttpSink {
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        tracing::info!("🌐 Starting HTTP event sink server...");

        let routes = Self::list_routes(&db)?;
        if !routes.is_empty() {
            tracing::info!("📋 Existing HTTP routes:");
            for (app_id, method, path, event_id) in routes {
                tracing::info!("   {} /{}{} -> {}", method, app_id, path, event_id);
            }
        }

        // Check if server is already running by trying to connect to it
        let server_check = flow_like_types::tokio::net::TcpStream::connect("127.0.0.1:9657").await;
        if server_check.is_ok() {
            tracing::info!("✅ HTTP server already running on port 9657, skipping server start");
            return Ok(());
        }

        let state = Arc::new(HttpServerState {
            db: db.clone(),
            app_handle: app_handle.clone(),
        });

        // Build router in a blocking context to avoid any async interference
        let app = flow_like_types::tokio::task::spawn_blocking(move || {
            Router::new()
                .route("/health", axum::routing::get(Self::health_check))
                .route(
                    "/{app_id}/{*rest}",
                    axum::routing::any(Self::handle_request),
                )
                .with_state(state)
        })
        .await
        .expect("Failed to build router");

        // Use a channel to wait for server to actually start before returning
        let (tx, rx) = flow_like_types::tokio::sync::oneshot::channel();

        flow_like_types::tokio::spawn(async move {
            let listener =
                match flow_like_types::tokio::net::TcpListener::bind("0.0.0.0:9657").await {
                    Ok(l) => {
                        tracing::info!("✅ HTTP server listening on http://0.0.0.0:9657");
                        tracing::info!("   Example: POST http://localhost:9657/my-app/webhook");
                        let _ = tx.send(());
                        l
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to bind HTTP server on 0.0.0.0:9657: {}", e);
                        let _ = tx.send(());
                        return;
                    }
                };

            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("❌ HTTP server error: {}", e);
            }
        });

        // Wait for server to start (with timeout)
        let result =
            flow_like_types::tokio::time::timeout(std::time::Duration::from_secs(5), rx).await;

        if result.is_err() {
            tracing::error!("❌ HTTP server failed to start within 5 seconds (timeout)");
        }

        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        // TODO: Shutdown Axum server if no more routes registered
        tracing::info!("HTTP sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        if !self.path.starts_with('/') {
            return Err(anyhow::anyhow!(
                "HTTP path must start with '/': {}",
                self.path
            ));
        }

        let method_upper = self.method.to_uppercase();
        if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
            .contains(&method_upper.as_str())
        {
            return Err(anyhow::anyhow!("Unsupported HTTP method: {}", self.method));
        }

        Self::add_route(&db, registration, self)?;

        tracing::info!(
            "✓ Registered HTTP route: {} /{}{} -> event {} (app: {})",
            self.method.to_uppercase(),
            registration.app_id,
            self.path,
            registration.event_id,
            registration.app_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::remove_route(&db, &registration.event_id)?;
        tracing::info!(
            "✗ Unregistered HTTP route: {} /{}{} (event: {})",
            self.method.to_uppercase(),
            registration.app_id,
            self.path,
            registration.event_id
        );
        Ok(())
    }
}

use anyhow::Result;
use axum::http::{HeaderMap, StatusCode};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    response::IntoResponse,
};
use flow_like_types::intercom::BufferedInterComHandler;
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let conn = db.lock().unwrap();

        let existing = conn
            .query_row(
                "SELECT event_id FROM http_routes
                 WHERE app_id = ?1 AND path = ?2 AND method = ?3",
                params![registration.app_id, config.path, config.method],
                |row| row.get::<_, String>(0),
            )
            .ok();

        if let Some(existing_event_id) = existing
            && existing_event_id != registration.event_id
        {
            tracing::warn!(
                "Route conflict: {} {} {} already registered to event {}. Overwriting with event {}",
                registration.app_id,
                config.method,
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
                config.method,
                config.auth_token,
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
        method: axum::http::Method,
        headers: HeaderMap,
        body: Body,
    ) -> impl IntoResponse {
        use crate::state::TauriEventSinkManagerState;

        // Extract body as string
        let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("[HTTP] Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
            }
        };

        let body_str = if !body_bytes.is_empty() {
            match String::from_utf8(body_bytes.to_vec()) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("[HTTP] Invalid UTF-8 in request body: {}", e);
                    return (StatusCode::BAD_REQUEST, "Invalid UTF-8 in request body")
                        .into_response();
                }
            }
        } else {
            None
        };

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

        println!(
            "[HTTP] Route found: event_id: {}, auth_token: {:?}",
            event_id, auth_token
        );

        if let Some(auth_token) = auth_token {
            let header_token = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if header_token != auth_token {
                return (StatusCode::UNAUTHORIZED, "Invalid auth token").into_response();
            }
        }

        println!("[HTTP] Authentication passed");

        // Extract body from request
        let body = if let Some(body_str) = body_str {
            if !body_str.is_empty() {
                match serde_json::from_str::<flow_like_types::Value>(&body_str) {
                    Ok(value) => Some(value),
                    Err(e) => {
                        eprintln!("[HTTP] Failed to parse JSON body: {}", e);
                        return (StatusCode::BAD_REQUEST, "Invalid JSON body").into_response();
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        println!(
            "[HTTP] Triggering event: {}, with body {:?}",
            event_id, body
        );

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
                                println!("[HTTP] Received generic_result event: {:?}", event);
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
                            crate::utils::emit_throttled(
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
                Ok(manager) => manager.fire_event(&app_handle, &event_id, body, Some(callback)),
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
                    println!(
                        "[HTTP] Returning response for event {}: {:?}",
                        event_id, resp
                    );
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

//! Shared helpers for nodes that work with connected apps: they exchange the
//! runtime token for a short-lived app-to-app token and call the hub API on
//! behalf of the current app.

use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{PROXY_EVENT_AUTHORIZATION_HEADER, Value, json::json, reqwest};
use std::sync::{Arc, OnceLock};

use flow_like::credentials::SharedCredentials;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Cacheable;

#[derive(Debug, flow_like_types::json::Deserialize)]
struct AppConnectionTokenResponse {
    token: String,
}

#[derive(Debug, flow_like_types::json::Deserialize)]
struct PresignProjectDbResponse {
    shared_credentials: Value,
}

#[derive(Clone)]
struct CachedRemoteProjectConnection {
    connection: Arc<flow_like_types::tokio::sync::OnceCell<Connection>>,
}

impl Cacheable for CachedRemoteProjectConnection {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Clone)]
struct CachedRemoteAppSession {
    session: Arc<flow_like_types::tokio::sync::OnceCell<RemoteAppSession>>,
}

impl Cacheable for CachedRemoteAppSession {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Clone)]
struct CachedRemoteOntologyAuthorization {
    authorized: Arc<flow_like_types::tokio::sync::OnceCell<()>>,
}

impl Cacheable for CachedRemoteOntologyAuthorization {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub(crate) fn api_base_url(hub: &str, secure: bool) -> Option<String> {
    let hub = hub.trim().trim_end_matches('/');
    if hub.is_empty() {
        return None;
    }

    let origin = if hub.starts_with("http://") || hub.starts_with("https://") {
        hub.to_string()
    } else {
        let protocol = if secure { "https" } else { "http" };
        format!("{protocol}://{hub}")
    };

    if origin.ends_with("/api/v1") {
        return Some(origin);
    }

    Some(format!("{origin}/api/v1"))
}

pub(crate) fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

/// Client for requests carrying app-connection or registration credentials.
/// Redirects are handled explicitly so custom auth headers cannot cross an
/// origin boundary (reqwest only strips a limited set of standard headers).
pub(crate) fn http_client_no_redirect() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("no-redirect HTTP client should build")
        })
        .clone()
}

/// Follow a GET redirect without copying any headers from the authenticated
/// request that produced it. Used for signed object-store downloads.
pub(crate) async fn follow_get_redirect_without_credentials(
    response: reqwest::Response,
) -> flow_like_types::Result<reqwest::Response> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            flow_like_types::anyhow!("Remote file redirect is missing a valid Location")
        })?;
    let redirect_url = response
        .url()
        .join(location)
        .map_err(|err| flow_like_types::anyhow!("Remote file redirect URL is invalid: {}", err))?;

    http_client()
        .get(redirect_url)
        .send()
        .await
        .map_err(|err| flow_like_types::anyhow!("Remote file download failed: {}", err))
}

/// Adds registration-level headers to a connected-app proxy request without
/// replacing the app-connection bearer token used by the API middleware.
/// Authorization-based registration auth is transported in a dedicated
/// header and restored by the proxy before the registration auth check.
pub(crate) fn with_event_registration_headers(
    mut request: reqwest::RequestBuilder,
    headers: &Value,
) -> reqwest::RequestBuilder {
    if let Some(header_obj) = headers.as_object() {
        // Header names are case-insensitive; dedup on the final name so two
        // spellings (e.g. `Authorization` and `authorization`, both remapped
        // to the proxy header) aren't sent twice — the proxy restores only one.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (name, value) in header_obj {
            if let Some(value) = value.as_str() {
                let name = if name.eq_ignore_ascii_case("authorization") {
                    PROXY_EVENT_AUTHORIZATION_HEADER
                } else {
                    name.as_str()
                };
                if !seen.insert(name.to_ascii_lowercase()) {
                    continue;
                }
                request = request.header(name, value);
            }
        }
    }
    request
}

pub(crate) async fn error_for_status(
    response: reqwest::Response,
    operation: &str,
) -> flow_like_types::Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(500).collect();
    Err(flow_like_types::anyhow!(
        "{} failed with status {}: {}",
        operation,
        status,
        body
    ))
}

/// Validates an id used as a URL path segment (app ids, event ids).
pub(crate) fn validate_path_id(value: &str, label: &str) -> flow_like_types::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(flow_like_types::anyhow!("No {} selected", label));
    }
    if !value
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(flow_like_types::anyhow!("Invalid {} '{}'", label, value));
    }
    Ok(value)
}

/// A short-lived session against a connected app: the app-to-app token plus
/// the API base url. Mint one per remote operation — tokens expire quickly.
#[derive(Clone)]
pub(crate) struct RemoteAppSession {
    pub token: String,
    pub base_url: String,
    pub target_app_id: String,
}

async fn remote_app_session(
    context: &ExecutionContext,
    target_app_id: &str,
) -> flow_like_types::Result<RemoteAppSession> {
    let target_app_id = validate_path_id(target_app_id, "remote project")?;
    let cache_key = format!("remote::session::{target_app_id}");
    let cached = {
        let existing = context.cache.read().await.get(&cache_key).cloned();
        if let Some(existing) = existing {
            existing
                .as_any()
                .downcast_ref::<CachedRemoteAppSession>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Remote app session cache entry has an unexpected type"
                    )
                })?
                .clone()
        } else {
            let mut cache = context.cache.write().await;
            if let Some(existing) = cache.get(&cache_key) {
                existing
                    .as_any()
                    .downcast_ref::<CachedRemoteAppSession>()
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Remote app session cache entry has an unexpected type"
                        )
                    })?
                    .clone()
            } else {
                let cached = CachedRemoteAppSession {
                    session: Arc::new(flow_like_types::tokio::sync::OnceCell::new()),
                };
                cache.insert(cache_key, Arc::new(cached.clone()));
                cached
            }
        }
    };
    Ok(cached
        .session
        .get_or_try_init(|| RemoteAppSession::open(context, &target_app_id))
        .await?
        .clone())
}

/// Rechecks the source project's live exposure decision once per ontology and
/// run. Installed snapshots remain stable contracts, but exposure revocation
/// takes effect for every new run without adding a request per node.
pub(crate) async fn ensure_remote_ontology_exposed(
    context: &ExecutionContext,
    target_app_id: &str,
    ontology_id: &str,
) -> flow_like_types::Result<()> {
    let target_app_id = validate_path_id(target_app_id, "remote project")?;
    let ontology_id = validate_path_id(ontology_id, "remote ontology")?;
    let cache_key = format!("remote::ontology-auth::{target_app_id}::{ontology_id}");
    let cached = {
        let existing = context.cache.read().await.get(&cache_key).cloned();
        if let Some(existing) = existing {
            existing
                .as_any()
                .downcast_ref::<CachedRemoteOntologyAuthorization>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Remote ontology authorization cache entry has an unexpected type"
                    )
                })?
                .clone()
        } else {
            let mut cache = context.cache.write().await;
            if let Some(existing) = cache.get(&cache_key) {
                existing
                    .as_any()
                    .downcast_ref::<CachedRemoteOntologyAuthorization>()
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Remote ontology authorization cache entry has an unexpected type"
                        )
                    })?
                    .clone()
            } else {
                let cached = CachedRemoteOntologyAuthorization {
                    authorized: Arc::new(flow_like_types::tokio::sync::OnceCell::new()),
                };
                cache.insert(cache_key, Arc::new(cached.clone()));
                cached
            }
        }
    };

    cached
        .authorized
        .get_or_try_init(|| async {
            let session = remote_app_session(context, &target_app_id).await?;
            let response = http_client_no_redirect()
                .get(session.url(&format!("graph/{ontology_id}")))
                .bearer_auth(&session.token)
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!("Failed to verify remote ontology exposure: {}", error)
                })?;
            error_for_status(response, "Remote ontology exposure check").await?;
            Ok::<(), flow_like_types::Error>(())
        })
        .await?;
    Ok(())
}

impl RemoteAppSession {
    pub async fn open(
        context: &ExecutionContext,
        target_app_id: &str,
    ) -> flow_like_types::Result<Self> {
        let context_cache = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let origin_app_id = context_cache.app_id.clone();

        if origin_app_id == target_app_id {
            return Err(flow_like_types::anyhow!(
                "The remote project is the current project"
            ));
        }

        let token = context
            .token
            .clone()
            .filter(|token| !token.trim().is_empty())
            .ok_or(flow_like_types::anyhow!(
                "Working with a connected app requires a connected session (no auth token available)"
            ))?;
        let base_url = api_base_url(&context.profile.hub, context.profile.secure).ok_or(
            flow_like_types::anyhow!("No hub URL configured on the execution profile"),
        )?;

        let token_url = format!(
            "{}/apps/{}/connections/{}/token",
            base_url, origin_app_id, target_app_id
        );
        let response = http_client()
            .post(&token_url)
            .bearer_auth(token.trim())
            // The run id ties the minted token — and every run it triggers
            // downstream — into this run's process case, even when the bearer
            // is a user token instead of an executor JWT.
            .json(&json!({ "run_id": context.run_id() }))
            .send()
            .await
            .map_err(|err| {
                flow_like_types::anyhow!("Failed to request app connection token: {}", err)
            })?;
        let response = error_for_status(response, "App connection token request").await?;
        let token_response: AppConnectionTokenResponse = response.json().await?;

        Ok(Self {
            token: token_response.token,
            base_url,
            target_app_id: target_app_id.to_string(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!(
            "{}/apps/{}/{}",
            self.base_url,
            self.target_app_id,
            path.trim_start_matches('/')
        )
    }
}

/// Opens a connected project's LanceDB through short-lived scoped credentials
/// and caches the raw connection for the current run. The credentials are
/// scoped to the project database root rather than one table, so ontology
/// reads for different object types can share this connection.
pub(crate) async fn open_remote_project_database(
    context: &ExecutionContext,
    target_app_id: &str,
    table: &str,
    write_access: bool,
) -> flow_like_types::Result<Connection> {
    let target_app_id = validate_path_id(target_app_id, "remote project")?;
    let table = table.trim();
    if table.is_empty() {
        return Err(flow_like_types::anyhow!(
            "The installed ontology object has no source table"
        ));
    }
    let access_mode = if write_access { "write" } else { "read" };
    let cache_key = format!("lance::remote::{target_app_id}::{access_mode}");

    let cached = {
        let existing = context.cache.read().await.get(&cache_key).cloned();
        if let Some(existing) = existing {
            existing
                .as_any()
                .downcast_ref::<CachedRemoteProjectConnection>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Remote project connection cache entry has an unexpected type"
                    )
                })?
                .clone()
        } else {
            let mut cache = context.cache.write().await;
            if let Some(existing) = cache.get(&cache_key) {
                existing
                    .as_any()
                    .downcast_ref::<CachedRemoteProjectConnection>()
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Remote project connection cache entry has an unexpected type"
                        )
                    })?
                    .clone()
            } else {
                let cached = CachedRemoteProjectConnection {
                    connection: Arc::new(flow_like_types::tokio::sync::OnceCell::new()),
                };
                cache.insert(cache_key, Arc::new(cached.clone()));
                cached
            }
        }
    };

    let connection = cached
        .connection
        .get_or_try_init(|| async {
            let session = remote_app_session(context, &target_app_id).await?;
            let response = http_client_no_redirect()
                .post(session.url("db/presign/project"))
                .bearer_auth(&session.token)
                .json(&json!({
                    "table_name": table,
                    "access_mode": access_mode,
                }))
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "Failed to request remote project database access: {}",
                        error
                    )
                })?;
            let response = error_for_status(response, "Remote project database presign").await?;
            let presigned: PresignProjectDbResponse = response.json().await?;
            let credentials: SharedCredentials =
                flow_like_types::json::from_value(presigned.shared_credentials)?;
            let database = credentials.to_db(&target_app_id).await?;
            let connection = context
                .app_state
                .with_lance_session(database)
                .execute()
                .await?;
            Ok::<Connection, flow_like_types::Error>(connection)
        })
        .await?;
    Ok(connection.clone())
}

#[cfg(test)]
mod tests {
    use super::{
        follow_get_redirect_without_credentials, http_client, http_client_no_redirect,
        with_event_registration_headers,
    };
    use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use flow_like_types::tokio::net::TcpListener;
    use flow_like_types::{json::json, reqwest};

    #[test]
    fn registration_authorization_does_not_replace_connection_bearer() {
        let request = http_client()
            .get("https://example.invalid")
            .bearer_auth("connection-token");
        let request = with_event_registration_headers(
            request,
            &json!({
                "Authorization": "Bearer registration-token",
                "x-api-key": "secret"
            }),
        )
        .build()
        .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get(flow_like_types::reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer connection-token")
        );
        assert_eq!(
            request
                .headers()
                .get("x-flow-like-event-authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer registration-token")
        );
        assert_eq!(
            request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("secret")
        );
    }

    #[flow_like_types::tokio::test]
    async fn authenticated_redirect_is_followed_without_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = flow_like_types::tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&request).to_ascii_lowercase());

                let response = if index == 0 {
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: /download\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });

        let response = http_client_no_redirect()
            .get(format!("http://{address}/proxy"))
            .bearer_auth("connection-token")
            .header(
                "x-flow-like-event-authorization",
                "Bearer registration-token",
            )
            .header("x-api-key", "secret")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);

        let response = follow_get_redirect_without_credentials(response)
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let requests = server.await.unwrap();
        assert!(requests[0].contains("authorization: bearer connection-token"));
        assert!(requests[0].contains("x-flow-like-event-authorization: bearer registration-token"));
        assert!(requests[0].contains("x-api-key: secret"));
        assert!(!requests[1].contains("authorization:"));
        assert!(!requests[1].contains("x-flow-like-event-authorization:"));
        assert!(!requests[1].contains("x-api-key:"));
    }
}

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Parses an MCP HTTP response that may be either `application/json` or an SSE
/// `text/event-stream` (`data: {json}` frames). Returns the first decodable
/// JSON object and any `Mcp-Session-Id` echoed back.
async fn parse_mcp_response(
    response: reqwest::Response,
) -> flow_like_types::Result<(Option<flow_like_types::Value>, Option<String>)> {
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let text = response.text().await?;
    if text.trim().is_empty() {
        return Ok((None, session_id));
    }

    if content_type.contains("text/event-stream") {
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:")
                && let Ok(value) =
                    flow_like_types::json::from_str::<flow_like_types::Value>(data.trim())
            {
                return Ok((Some(value), session_id));
            }
        }
        return Ok((None, session_id));
    }

    let value = flow_like_types::json::from_str::<flow_like_types::Value>(text.trim())?;
    Ok((Some(value), session_id))
}

impl RemoteAppSession {
    async fn mcp_post(
        &self,
        event_id: &str,
        session_id: Option<&str>,
        body: &flow_like_types::Value,
        registration_headers: &flow_like_types::Value,
    ) -> flow_like_types::Result<(Option<flow_like_types::Value>, Option<String>)> {
        let url = self.url(&format!("events/{}/mcp", event_id));
        let mut request = http_client_no_redirect()
            .post(&url)
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/json")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);
        request = with_event_registration_headers(request, registration_headers);
        if let Some(session_id) = session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let response = request
            .json(body)
            .send()
            .await
            .map_err(|err| flow_like_types::anyhow!("Failed to call remote MCP: {}", err))?;
        let response = error_for_status(response, "Remote MCP request").await?;
        parse_mcp_response(response).await
    }

    /// Runs a single MCP JSON-RPC method against a connected app's MCP event:
    /// initializes a session, issues the call, and returns the JSON-RPC
    /// `result` (erroring on a JSON-RPC error).
    pub async fn mcp_request(
        &self,
        event_id: &str,
        method: &str,
        params: flow_like_types::Value,
        registration_headers: &flow_like_types::Value,
    ) -> flow_like_types::Result<flow_like_types::Value> {
        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "Flow-Like", "version": "alpha" }
            }
        });
        let (_, session_id) = self
            .mcp_post(event_id, None, &init, registration_headers)
            .await?;

        let call = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": method,
            "params": params
        });
        let (response, _) = self
            .mcp_post(event_id, session_id.as_deref(), &call, registration_headers)
            .await?;

        let response = response
            .ok_or_else(|| flow_like_types::anyhow!("Empty MCP response for {}", method))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(flow_like_types::anyhow!(
                "MCP {} error: {}",
                method,
                message
            ));
        }
        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(flow_like_types::Value::Null))
    }
}

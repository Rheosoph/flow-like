//! Shared helpers for nodes that work with connected apps: they exchange the
//! runtime token for a short-lived app-to-app token and call the hub API on
//! behalf of the current app.

use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{PROXY_EVENT_AUTHORIZATION_HEADER, Value, async_trait, json::json, reqwest};
use std::sync::{Arc, OnceLock};

use flow_like::credentials::SharedCredentials;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Cacheable;

const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const CONTROL_PLANE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_SSE_FRAME_BYTES: usize = 1024 * 1024;
const MAX_SSE_DELIMITER_BYTES: usize = 4;
const MAX_MCP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, flow_like_types::json::Deserialize)]
struct AppConnectionTokenResponse {
    token: String,
    expires_at: i64,
}

#[derive(Debug, flow_like_types::json::Deserialize)]
struct PresignProjectDbResponse {
    shared_credentials: Value,
    expiration: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone)]
struct CachedRemoteProjectConnection {
    connection: Arc<flow_like_types::tokio::sync::Mutex<Option<RemoteProjectConnectionCacheEntry>>>,
}

#[derive(Clone)]
struct RemoteProjectConnectionCacheEntry {
    connection: Connection,
    refresh_at: std::time::Instant,
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
    session: Arc<flow_like_types::tokio::sync::Mutex<Option<RemoteAppSession>>>,
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
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .build()
                .expect("HTTP client should build")
        })
        .clone()
}

/// Client for requests carrying app-connection or registration credentials.
/// Redirects are handled explicitly so custom auth headers cannot cross an
/// origin boundary (reqwest only strips a limited set of standard headers).
pub(crate) fn http_client_no_redirect() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("no-redirect HTTP client should build")
        })
        .clone()
}

/// Bounded client for short control-plane operations performed while holding
/// run-scoped single-flight locks (token exchange, authorization, presign).
/// It deliberately rejects redirects so credentials never cross origins.
pub(crate) fn control_plane_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .timeout(CONTROL_PLANE_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("control-plane HTTP client should build")
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
pub fn validate_path_id(value: &str, label: &str) -> flow_like_types::Result<String> {
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
/// the API base URL and authoritative expiration. Sessions are shared until
/// shortly before expiry, then refreshed atomically for the next operation.
#[derive(Clone)]
pub struct RemoteAppSession {
    pub token: String,
    pub base_url: String,
    pub target_app_id: String,
    refresh_at: std::time::Instant,
    valid_until: std::time::Instant,
}

impl RemoteAppSession {
    fn is_fresh_for(&self, minimum_validity: std::time::Duration) -> bool {
        let now = std::time::Instant::now();
        self.refresh_at > now && self.valid_until.saturating_duration_since(now) > minimum_validity
    }
}

fn remote_session_deadlines(
    expires_at: i64,
    requested_at: std::time::Instant,
    requested_ttl: std::time::Duration,
) -> (std::time::Instant, std::time::Instant) {
    const REFRESH_MARGIN: std::time::Duration = std::time::Duration::from_secs(30);

    let now = std::time::Instant::now();
    let reported_ttl = std::time::Duration::from_secs(
        expires_at
            .saturating_sub(chrono::Utc::now().timestamp())
            .max(0) as u64,
    );
    let reported_valid_until = now + reported_ttl.min(requested_ttl);
    // Anchor the maximum lifetime at request start as well as response receipt:
    // a delayed response must not extend a token beyond the server-side TTL.
    let request_valid_until = requested_at + requested_ttl;
    let valid_until = reported_valid_until.min(request_valid_until);
    let refresh_at = valid_until
        .checked_sub(REFRESH_MARGIN)
        .unwrap_or(requested_at);
    (refresh_at, valid_until)
}

pub async fn remote_app_session(
    context: &ExecutionContext,
    target_app_id: &str,
) -> flow_like_types::Result<RemoteAppSession> {
    remote_app_session_cached(
        context,
        target_app_id,
        None,
        std::time::Duration::from_secs(30),
    )
    .await
}

/// Returns a session suitable for constructing a longer-lived remote MCP
/// transport. If the run cache only has a token near expiry, it is atomically
/// replaced with a token requesting the API's 15-minute maximum lifetime.
pub async fn remote_app_session_for_mcp(
    context: &ExecutionContext,
    target_app_id: &str,
) -> flow_like_types::Result<RemoteAppSession> {
    remote_app_session_cached(
        context,
        target_app_id,
        Some(15 * 60),
        std::time::Duration::from_secs(10 * 60),
    )
    .await
}

async fn remote_app_session_cached(
    context: &ExecutionContext,
    target_app_id: &str,
    requested_ttl_seconds: Option<u64>,
    minimum_validity: std::time::Duration,
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
                    session: Arc::new(flow_like_types::tokio::sync::Mutex::new(None)),
                };
                cache.insert(cache_key, Arc::new(cached.clone()));
                cached
            }
        }
    };

    let mut session = cached.session.lock().await;
    if let Some(session) = session.as_ref()
        && session.is_fresh_for(minimum_validity)
    {
        return Ok(session.clone());
    }

    let refreshed = RemoteAppSession::open(context, &target_app_id, requested_ttl_seconds).await?;
    if !refreshed.is_fresh_for(minimum_validity) {
        return Err(flow_like_types::anyhow!(
            "The connected-app session expires too soon for this operation (requires more than {} seconds remaining)",
            minimum_validity.as_secs()
        ));
    }
    *session = Some(refreshed.clone());
    Ok(refreshed)
}

/// Rechecks the source project's live exposure decision once per ontology and
/// run. Installed snapshots remain stable contracts, but exposure revocation
/// takes effect for every new run without adding a request per node.
fn remote_ontology_authorization_cache_key(
    target_app_id: &str,
    ontology_id: &str,
    expected_revision: Option<&str>,
) -> String {
    let revision_key = expected_revision
        .map(|revision| format!("revision:{revision}"))
        .unwrap_or_else(|| "producer-authoritative".to_string());
    format!("remote::ontology-auth::{target_app_id}::{ontology_id}::{revision_key}")
}

pub(crate) async fn ensure_remote_ontology_exposed(
    context: &ExecutionContext,
    target_app_id: &str,
    ontology_id: &str,
    expected_revision: Option<&str>,
) -> flow_like_types::Result<()> {
    let target_app_id = validate_path_id(target_app_id, "remote project")?;
    let ontology_id = validate_path_id(ontology_id, "remote ontology")?;
    let cache_key =
        remote_ontology_authorization_cache_key(&target_app_id, &ontology_id, expected_revision);
    let expected_revision = expected_revision.map(str::to_owned);
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
            let response = control_plane_http_client()
                .get(session.url(&format!("graph/{ontology_id}")))
                .bearer_auth(&session.token)
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!("Failed to verify remote ontology exposure: {}", error)
                })?;
            let response = error_for_status(response, "Remote ontology exposure check").await?;
            if let Some(expected_revision) = expected_revision.as_deref() {
                let live: Value = response.json().await.map_err(|error| {
                    flow_like_types::anyhow!(
                        "Failed to decode remote ontology revision: {}",
                        error
                    )
                })?;
                let live_revision = live
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Remote ontology exposure response omitted its revision"
                        )
                    })?;
                if live_revision != expected_revision {
                    return Err(flow_like_types::anyhow!(
                        "The remote ontology changed after this contract was installed. Refresh the installed ontology before reading it."
                    ));
                }
            }
            Ok::<(), flow_like_types::Error>(())
        })
        .await?;
    Ok(())
}

impl RemoteAppSession {
    async fn open(
        context: &ExecutionContext,
        target_app_id: &str,
        requested_ttl_seconds: Option<u64>,
    ) -> flow_like_types::Result<Self> {
        const DEFAULT_TTL_SECONDS: u64 = 10 * 60;
        const MAX_TTL_SECONDS: u64 = 15 * 60;

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
        let requested_at = std::time::Instant::now();
        let request_body = match requested_ttl_seconds {
            Some(ttl_seconds) => json!({
                "run_id": context.run_id(),
                "ttl_seconds": ttl_seconds.clamp(60, MAX_TTL_SECONDS),
            }),
            None => json!({ "run_id": context.run_id() }),
        };
        let response = control_plane_http_client()
            .post(&token_url)
            .bearer_auth(token.trim())
            // The run id ties the minted token — and every run it triggers
            // downstream — into this run's process case, even when the bearer
            // is a user token instead of an executor JWT.
            .json(&request_body)
            .send()
            .await
            .map_err(|err| {
                flow_like_types::anyhow!("Failed to request app connection token: {}", err)
            })?;
        let response = error_for_status(response, "App connection token request").await?;
        let token_response: AppConnectionTokenResponse = response.json().await?;

        let requested_ttl = std::time::Duration::from_secs(
            requested_ttl_seconds
                .unwrap_or(DEFAULT_TTL_SECONDS)
                .clamp(60, MAX_TTL_SECONDS),
        );
        let (refresh_at, valid_until) =
            remote_session_deadlines(token_response.expires_at, requested_at, requested_ttl);

        Ok(Self {
            token: token_response.token,
            base_url,
            target_app_id: target_app_id.to_string(),
            refresh_at,
            valid_until,
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

fn remote_connection_refresh_at(
    expiration: Option<chrono::DateTime<chrono::Utc>>,
    requested_at: std::time::Instant,
) -> std::time::Instant {
    const REFRESH_MARGIN: std::time::Duration = std::time::Duration::from_secs(60);
    // Every scoped provider credential currently has a one-hour maximum. The
    // cap also makes the cache safe when the API and executor clocks differ.
    const MAX_SCOPED_CREDENTIAL_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

    let now = std::time::Instant::now();
    match expiration {
        Some(expiration) => {
            let reported_refresh = now
                + (expiration - chrono::Utc::now())
                    .to_std()
                    .unwrap_or_default()
                    .min(MAX_SCOPED_CREDENTIAL_TTL)
                    .saturating_sub(REFRESH_MARGIN);
            let request_refresh =
                requested_at + MAX_SCOPED_CREDENTIAL_TTL.saturating_sub(REFRESH_MARGIN);
            reported_refresh.min(request_refresh)
        }
        // An omitted expiration must not silently turn temporary credentials
        // into an all-day cache entry. Current providers report one hour; use
        // the same conservative refresh window for future providers as well.
        None => requested_at + MAX_SCOPED_CREDENTIAL_TTL.saturating_sub(REFRESH_MARGIN),
    }
}

fn ensure_remote_connection_fresh(refresh_at: std::time::Instant) -> flow_like_types::Result<()> {
    if refresh_at <= std::time::Instant::now() {
        return Err(flow_like_types::anyhow!(
            "Remote database credentials are expired or too close to expiration"
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct RemoteProjectDatabaseLease {
    pub connection: Connection,
    pub refresh_at: std::time::Instant,
}

/// Opens a connected project's LanceDB through short-lived scoped credentials.
/// Connections remain cached while their credentials are fresh and are rebuilt
/// one minute before the API-reported expiration, avoiding both per-node setup
/// cost and stale credentials in long-running workflows.
pub(crate) async fn open_remote_project_database(
    context: &ExecutionContext,
    target_app_id: &str,
    table: &str,
    write_access: bool,
) -> flow_like_types::Result<Connection> {
    Ok(
        open_remote_project_database_lease(context, target_app_id, table, write_access)
            .await?
            .connection,
    )
}

pub(crate) async fn open_remote_project_database_lease(
    context: &ExecutionContext,
    target_app_id: &str,
    table: &str,
    write_access: bool,
) -> flow_like_types::Result<RemoteProjectDatabaseLease> {
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
                    connection: Arc::new(flow_like_types::tokio::sync::Mutex::new(None)),
                };
                cache.insert(cache_key, Arc::new(cached.clone()));
                cached
            }
        }
    };

    let mut connection = cached.connection.lock().await;
    if let Some(connection) = connection.as_ref()
        && connection.refresh_at > std::time::Instant::now()
    {
        return Ok(RemoteProjectDatabaseLease {
            connection: connection.connection.clone(),
            refresh_at: connection.refresh_at,
        });
    }

    // Hold this cache entry's mutex while refreshing so concurrent graph nodes
    // share one presign request instead of stampeding the connected app.
    let session = remote_app_session(context, &target_app_id).await?;
    let requested_at = std::time::Instant::now();
    let response = control_plane_http_client()
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
    let refresh_at = remote_connection_refresh_at(presigned.expiration, requested_at);
    ensure_remote_connection_fresh(refresh_at)?;
    let credentials: SharedCredentials =
        flow_like_types::json::from_value(presigned.shared_credentials)?;
    let database = credentials.to_db(&target_app_id).await?;
    let refreshed = context
        .app_state
        .with_lance_session(database)
        .execute()
        .await?;
    *connection = Some(RemoteProjectConnectionCacheEntry {
        connection: refreshed.clone(),
        refresh_at,
    });
    Ok(RemoteProjectDatabaseLease {
        connection: refreshed,
        refresh_at,
    })
}

// ---------------------------------------------------------------------------
// Invocation helpers (SSE collection)
//
// Shared by every node that invokes a connected project's event or governed
// ontology action and waits for the run result. The producer is authoritative:
// these helpers only transport the request and collect the streamed outcome.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct SseOutcome {
    pub(crate) run_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) generic_result: Option<Value>,
    pub(crate) chat_out: Option<Value>,
    pub(crate) chat_stream: Option<Value>,
    pub(crate) chat_local_session: Option<Value>,
    pub(crate) chat_global_session: Option<Value>,
}

/// One decoded event from a remote invocation's SSE response. Keeping this
/// transport-level shape separate from [`SseOutcome`] lets streaming callers
/// react to every chat update while final-only callers continue to use the
/// collected outcome.
#[derive(Debug, Clone)]
pub(crate) struct RemoteSseEvent {
    pub(crate) event_type: String,
    pub(crate) payload: Value,
    pub(crate) run_id: Option<String>,
}

#[async_trait]
pub(crate) trait RemoteSseEventHandler: Send {
    async fn on_event(&mut self, event: &RemoteSseEvent) -> flow_like_types::Result<()>;
}

struct IgnoreRemoteSseEvents;

#[async_trait]
impl RemoteSseEventHandler for IgnoreRemoteSseEvents {
    async fn on_event(&mut self, _event: &RemoteSseEvent) -> flow_like_types::Result<()> {
        Ok(())
    }
}

impl SseOutcome {
    pub(crate) fn status_str(&self) -> String {
        self.status.clone().unwrap_or_else(|| "Unknown".to_string())
    }

    pub(crate) fn ensure_ok(&self) -> flow_like_types::Result<()> {
        let status = self
            .status
            .as_deref()
            .ok_or_else(|| flow_like_types::anyhow!("Remote run returned no terminal status"))?;
        // ExecutionStatus is serialized in lowercase, while older producers
        // used title case. Treat only an explicit completed status as success.
        if !status.eq_ignore_ascii_case("completed") {
            return Err(flow_like_types::anyhow!(
                "Remote run {} ended with status {}: {}",
                self.run_id.clone().unwrap_or_default(),
                self.status_str(),
                self.error_message.clone().unwrap_or_default()
            ));
        }
        Ok(())
    }

    pub(crate) fn chat_result(&self) -> Option<Value> {
        self.chat_out
            .clone()
            .or_else(|| self.chat_stream.clone())
            .or_else(|| self.generic_result.clone())
    }
}

fn parse_sse_frame(frame: &str) -> Option<RemoteSseEvent> {
    // SSE concatenates all `data:` fields in one event with a newline. Normalize
    // lone CR line endings too; `str::lines` only handles LF/CRLF.
    let normalized = frame.replace("\r\n", "\n").replace('\r', "\n");
    let data = normalized
        .split('\n')
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.strip_prefix(' ').unwrap_or(data))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return None;
    }
    let parsed = flow_like_types::json::from_str::<Value>(&data).ok()?;
    let payload = parsed.get("payload").cloned().unwrap_or(Value::Null);
    let run_id = parsed
        .get("run_id")
        .or_else(|| payload.get("run_id"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let event_type = parsed
        .get("event_type")
        .and_then(|value| value.as_str())
        .filter(|event_type| !event_type.is_empty())?
        .to_string();

    Some(RemoteSseEvent {
        event_type,
        payload,
        run_id,
    })
}

fn apply_sse_event(event: &RemoteSseEvent, outcome: &mut SseOutcome) -> bool {
    let payload = &event.payload;

    if outcome.run_id.is_none()
        && let Some(run_id) = &event.run_id
    {
        outcome.run_id = Some(run_id.clone());
    }

    match event.event_type.as_str() {
        "generic_result" if outcome.generic_result.is_none() => {
            outcome.generic_result = Some(payload.clone());
        }
        "chat_out" => {
            outcome.chat_out = Some(payload.clone());
        }
        "chat_stream" => {
            outcome.chat_stream = Some(payload.clone());
        }
        "chat_local_session" => {
            outcome.chat_local_session = Some(payload.clone());
        }
        "chat_global_session" => {
            outcome.chat_global_session = Some(payload.clone());
        }
        "error" => {
            outcome.error_message = payload
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_string);
        }
        "completed" => {
            outcome.status = payload
                .get("status")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            if let Some(error_message) = payload
                .get("error_message")
                .and_then(|value| value.as_str())
            {
                outcome.error_message = Some(error_message.to_string());
            }
            return true;
        }
        _ => {}
    }
    false
}

#[cfg(test)]
fn apply_sse_frame(frame: &str, outcome: &mut SseOutcome) -> bool {
    parse_sse_frame(frame)
        .map(|event| apply_sse_event(&event, outcome))
        .unwrap_or(false)
}

fn finish_sse_outcome(
    outcome: SseOutcome,
    terminal_received: bool,
) -> flow_like_types::Result<SseOutcome> {
    if !terminal_received {
        let detail = outcome
            .error_message
            .as_deref()
            .map(|message| format!(": {message}"))
            .unwrap_or_default();
        return Err(flow_like_types::anyhow!(
            "Remote event stream ended before a terminal completion event{}",
            detail
        ));
    }
    if outcome.status.is_none() {
        return Err(flow_like_types::anyhow!(
            "Remote completion event did not include a status"
        ));
    }
    Ok(outcome)
}

pub(crate) async fn post_json(
    session: &RemoteAppSession,
    url: &str,
    body: &Value,
) -> flow_like_types::Result<reqwest::Response> {
    let response = http_client()
        .post(url)
        .bearer_auth(&session.token)
        .json(body)
        .send()
        .await
        .map_err(|err| flow_like_types::anyhow!("Failed to invoke remote event: {}", err))?;
    error_for_status(response, "Remote event invocation").await
}

pub(crate) async fn invoke_and_collect(
    session: &RemoteAppSession,
    url: &str,
    body: &Value,
    timeout: u64,
) -> flow_like_types::Result<SseOutcome> {
    let mut handler = IgnoreRemoteSseEvents;
    invoke_and_collect_with_handler(session, url, body, timeout, &mut handler).await
}

/// Invoke a remote event and visit every decoded SSE event as it arrives while
/// still collecting the terminal outcome. The handler runs inline to preserve
/// event order, which is required for chat chunks, embedded widgets and state
/// updates.
pub(crate) async fn invoke_and_collect_with_handler<H>(
    session: &RemoteAppSession,
    url: &str,
    body: &Value,
    timeout: u64,
    handler: &mut H,
) -> flow_like_types::Result<SseOutcome>
where
    H: RemoteSseEventHandler + ?Sized,
{
    flow_like_types::tokio::time::timeout(std::time::Duration::from_secs(timeout), async {
        // Include connection establishment and response headers in the
        // advertised invocation deadline, not just the SSE body.
        let response = post_json(session, url, body).await?;
        collect_sse_outcome(response, handler).await
    })
    .await
    .map_err(|_| {
        flow_like_types::anyhow!("Remote event did not finish within {} seconds", timeout)
    })?
}

async fn collect_sse_outcome<H>(
    response: reqwest::Response,
    handler: &mut H,
) -> flow_like_types::Result<SseOutcome>
where
    H: RemoteSseEventHandler + ?Sized,
{
    use futures::StreamExt;

    let mut outcome = SseOutcome::default();
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut scan_from = 0;
    let mut terminal_received = false;

    'outer: while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|err| flow_like_types::anyhow!("Failed to read event stream: {}", err))?;
        buffer.extend_from_slice(&chunk);
        let mut frame_start = 0;

        loop {
            match find_sse_frame_boundary_from(&buffer, scan_from) {
                Some((pos, delimiter_len)) => {
                    let frame_len = pos.saturating_sub(frame_start);
                    if frame_len > MAX_SSE_FRAME_BYTES {
                        return Err(flow_like_types::anyhow!(
                            "Remote event stream frame exceeded the {} byte limit",
                            MAX_SSE_FRAME_BYTES
                        ));
                    }
                    let frame = std::str::from_utf8(&buffer[frame_start..pos])
                        .map_err(|error| {
                            flow_like_types::anyhow!(
                                "Remote event stream contained invalid UTF-8: {}",
                                error
                            )
                        })?
                        .to_string();
                    frame_start = pos + delimiter_len;
                    scan_from = frame_start;

                    if let Some(event) = parse_sse_frame(&frame) {
                        let terminal = apply_sse_event(&event, &mut outcome);
                        handler.on_event(&event).await?;
                        if terminal {
                            terminal_received = true;
                            break 'outer;
                        }
                    }
                }
                None => {
                    if buffer.len().saturating_sub(frame_start) > MAX_SSE_FRAME_BYTES {
                        return Err(flow_like_types::anyhow!(
                            "Remote event stream frame exceeded the {} byte limit",
                            MAX_SSE_FRAME_BYTES
                        ));
                    }
                    if frame_start > 0 {
                        buffer.drain(..frame_start);
                    }
                    // Only a delimiter beginning in the final three existing
                    // bytes can be completed by the next network chunk.
                    scan_from = buffer
                        .len()
                        .saturating_sub(MAX_SSE_DELIMITER_BYTES.saturating_sub(1));
                    break;
                }
            }
        }
    }

    // Some HTTP/SSE producers close immediately after their final event
    // instead of writing the optional trailing blank line. At EOF the
    // remaining bytes form one last frame and must still be considered.
    if !terminal_received && !buffer.is_empty() {
        let frame = sse_eof_frame(&buffer)?;
        if let Some(event) = frame.as_deref().and_then(parse_sse_frame) {
            terminal_received = apply_sse_event(&event, &mut outcome);
            handler.on_event(&event).await?;
        }
    }

    finish_sse_outcome(outcome, terminal_received)
}

#[cfg(test)]
fn apply_sse_eof_buffer(buffer: &[u8], outcome: &mut SseOutcome) -> flow_like_types::Result<bool> {
    let Some(frame) = sse_eof_frame(buffer)? else {
        return Ok(false);
    };
    Ok(apply_sse_frame(&frame, outcome))
}

fn sse_eof_frame(buffer: &[u8]) -> flow_like_types::Result<Option<String>> {
    if buffer.len() > MAX_SSE_FRAME_BYTES {
        return Err(flow_like_types::anyhow!(
            "Remote event stream frame exceeded the {} byte limit",
            MAX_SSE_FRAME_BYTES
        ));
    }
    let frame = std::str::from_utf8(buffer).map_err(|error| {
        flow_like_types::anyhow!("Remote event stream contained invalid UTF-8: {}", error)
    })?;
    if frame.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(frame.to_string()))
}

#[cfg(test)]
fn find_sse_frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    find_sse_frame_boundary_from(buffer, 0)
}

fn find_sse_frame_boundary_from(buffer: &[u8], start: usize) -> Option<(usize, usize)> {
    for position in start.min(buffer.len())..buffer.len() {
        let remaining = &buffer[position..];
        for delimiter in [
            b"\r\n\r\n".as_slice(),
            b"\r\n\n".as_slice(),
            b"\r\n\r".as_slice(),
            b"\n\r\n".as_slice(),
            b"\r\r\n".as_slice(),
            b"\n\n".as_slice(),
            b"\n\r".as_slice(),
            b"\r\r".as_slice(),
        ] {
            if remaining.starts_with(delimiter) {
                return Some((position, delimiter.len()));
            }
        }
    }
    None
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

    let body = read_bounded_response_body(response, MAX_MCP_RESPONSE_BYTES).await?;
    let text = std::str::from_utf8(&body).map_err(|error| {
        flow_like_types::anyhow!("Remote MCP response contained invalid UTF-8: {}", error)
    })?;
    Ok((parse_mcp_response_body(&content_type, text)?, session_id))
}

async fn read_bounded_response_body(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> flow_like_types::Result<Vec<u8>> {
    use futures::StreamExt;

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            flow_like_types::anyhow!("Failed to read remote MCP response: {}", error)
        })?;
        extend_bounded_body(&mut body, &chunk, maximum_bytes)?;
    }
    Ok(body)
}

fn extend_bounded_body(
    body: &mut Vec<u8>,
    chunk: &[u8],
    maximum_bytes: usize,
) -> flow_like_types::Result<()> {
    if body
        .len()
        .checked_add(chunk.len())
        .is_none_or(|length| length > maximum_bytes)
    {
        return Err(flow_like_types::anyhow!(
            "Remote MCP response exceeded the {} byte limit",
            maximum_bytes
        ));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn parse_mcp_response_body(
    content_type: &str,
    text: &str,
) -> flow_like_types::Result<Option<flow_like_types::Value>> {
    if text.trim().is_empty() {
        return Ok(None);
    }

    if content_type
        .to_ascii_lowercase()
        .contains("text/event-stream")
    {
        // SSE joins every data field in an event with a newline. Normalize all
        // valid line endings first so pretty-printed JSON split across data
        // fields remains a valid MCP response.
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut first_message = None;
        for frame in normalized.split("\n\n") {
            let data = frame
                .split('\n')
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            if let Ok(value) = flow_like_types::json::from_str::<flow_like_types::Value>(&data) {
                // A POST SSE stream may carry notifications before the
                // response. Prefer the JSON-RPC message correlated by `id`
                // so callers do not mistake an unrelated notification for a
                // null result.
                if value.get("id").is_some() {
                    return Ok(Some(value));
                }
                first_message.get_or_insert(value);
            }
        }
        return Ok(first_message);
    }

    let value = flow_like_types::json::from_str::<flow_like_types::Value>(text.trim())?;
    Ok(Some(value))
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_SSE_FRAME_BYTES, RemoteAppSession, RemoteSseEvent, RemoteSseEventHandler, SseOutcome,
        apply_sse_eof_buffer, apply_sse_frame, collect_sse_outcome, ensure_remote_connection_fresh,
        extend_bounded_body, find_sse_frame_boundary, finish_sse_outcome,
        follow_get_redirect_without_credentials, http_client, http_client_no_redirect,
        parse_mcp_response_body, remote_connection_refresh_at,
        remote_ontology_authorization_cache_key, remote_session_deadlines,
        with_event_registration_headers,
    };
    use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use flow_like_types::tokio::net::TcpListener;
    use flow_like_types::{json::json, reqwest};

    fn empty_sse_outcome() -> SseOutcome {
        SseOutcome::default()
    }

    #[derive(Default)]
    struct RecordingSseHandler {
        events: Vec<RemoteSseEvent>,
    }

    #[flow_like_types::async_trait]
    impl RemoteSseEventHandler for RecordingSseHandler {
        async fn on_event(&mut self, event: &RemoteSseEvent) -> flow_like_types::Result<()> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn sse_run_id_is_read_from_event_payload() {
        let mut outcome = empty_sse_outcome();
        assert!(!apply_sse_frame(
            r#"data: {"event_type":"run_initiated","payload":{"run_id":"run-123"}}"#,
            &mut outcome,
        ));
        assert_eq!(outcome.run_id.as_deref(), Some("run-123"));

        assert!(apply_sse_frame(
            r#"data: {"event_type":"completed","payload":{"run_id":"run-123","status":"completed"}}"#,
            &mut outcome,
        ));
        assert_eq!(outcome.status.as_deref(), Some("completed"));
        outcome.ensure_ok().unwrap();
    }

    #[test]
    fn sse_requires_a_terminal_completion_event() {
        let mut outcome = empty_sse_outcome();
        assert!(!apply_sse_frame(
            r#"data: {"event_type":"generic_result","payload":{"value":42}}"#,
            &mut outcome,
        ));
        let error = finish_sse_outcome(outcome, false).unwrap_err();
        assert!(error.to_string().contains("terminal completion event"));
    }

    #[test]
    fn unterminated_sse_preserves_the_remote_error() {
        let mut outcome = empty_sse_outcome();
        assert!(!apply_sse_frame(
            r#"data: {"event_type":"error","payload":{"message":"setup failed"}}"#,
            &mut outcome,
        ));
        let error = finish_sse_outcome(outcome, false).unwrap_err();
        assert!(error.to_string().contains("setup failed"));
    }

    #[test]
    fn lowercase_failed_status_is_not_accepted() {
        let mut outcome = empty_sse_outcome();
        assert!(apply_sse_frame(
            r#"data: {"event_type":"completed","payload":{"run_id":"run-456","status":"failed"}}"#,
            &mut outcome,
        ));
        let outcome = finish_sse_outcome(outcome, true).unwrap();
        assert!(outcome.ensure_ok().is_err());
    }

    #[test]
    fn remote_session_refreshes_before_token_expiration() {
        let (refresh_at, valid_until) = remote_session_deadlines(
            chrono::Utc::now().timestamp() + 600,
            std::time::Instant::now(),
            std::time::Duration::from_secs(600),
        );
        let remaining = refresh_at.saturating_duration_since(std::time::Instant::now());
        assert!(remaining <= std::time::Duration::from_secs(570));
        assert!(remaining >= std::time::Duration::from_secs(568));
        assert!(
            valid_until.saturating_duration_since(std::time::Instant::now())
                >= std::time::Duration::from_secs(598)
        );

        let expired = RemoteAppSession {
            token: "token".to_string(),
            base_url: "https://example.invalid/api/v1".to_string(),
            target_app_id: "target".to_string(),
            refresh_at: std::time::Instant::now(),
            valid_until: std::time::Instant::now(),
        };
        assert!(!expired.is_fresh_for(std::time::Duration::ZERO));

        let now = std::time::Instant::now();
        let requested_at = now
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or(now);
        let (delayed_refresh, delayed_expiration) = remote_session_deadlines(
            chrono::Utc::now().timestamp() + 3_600,
            requested_at,
            std::time::Duration::from_secs(600),
        );
        assert_eq!(
            delayed_refresh,
            requested_at + std::time::Duration::from_secs(570)
        );
        assert_eq!(
            delayed_expiration,
            requested_at + std::time::Duration::from_secs(600)
        );

        let now = std::time::Instant::now();
        let long_lived = RemoteAppSession {
            token: "token".to_string(),
            base_url: "https://example.invalid/api/v1".to_string(),
            target_app_id: "target".to_string(),
            refresh_at: now + std::time::Duration::from_secs(870),
            valid_until: now + std::time::Duration::from_secs(900),
        };
        assert!(long_lived.is_fresh_for(std::time::Duration::from_secs(600)));
        assert!(!long_lived.is_fresh_for(std::time::Duration::from_secs(901)));
    }

    #[test]
    fn ontology_authorization_cache_is_revision_scoped() {
        let first = remote_ontology_authorization_cache_key("target", "ontology", Some("rev-1"));
        let second = remote_ontology_authorization_cache_key("target", "ontology", Some("rev-2"));
        let producer = remote_ontology_authorization_cache_key("target", "ontology", None);

        assert_ne!(first, second);
        assert_ne!(first, producer);
        assert!(first.ends_with("revision:rev-1"));
        assert!(producer.ends_with("producer-authoritative"));
    }

    #[test]
    fn sse_boundaries_support_all_standard_line_endings() {
        assert_eq!(find_sse_frame_boundary(b"data: {}\n\nnext"), Some((8, 2)));
        assert_eq!(
            find_sse_frame_boundary(b"data: {}\r\n\r\nnext"),
            Some((8, 4))
        );
        assert_eq!(find_sse_frame_boundary(b"data: {}\r\n\nnext"), Some((8, 3)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\n\r\nnext"), Some((8, 3)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\r\rnext"), Some((8, 2)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\r\n\rnext"), Some((8, 3)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\n\rnext"), Some((8, 2)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\r\r\nnext"), Some((8, 3)));
        assert_eq!(find_sse_frame_boundary(b"data: {}\r\nnext"), None);
    }

    #[test]
    fn sse_joins_multiline_data_fields_before_parsing() {
        let mut outcome = empty_sse_outcome();
        assert!(apply_sse_frame(
            "data: {\"event_type\":\r\ndata: \"completed\",\"payload\":{\"status\":\"completed\"}}",
            &mut outcome,
        ));
        assert_eq!(outcome.status.as_deref(), Some("completed"));
    }

    #[test]
    fn sse_accepts_terminal_frame_at_eof_without_blank_line() {
        let mut outcome = empty_sse_outcome();
        let terminal = apply_sse_eof_buffer(
            br#"data: {"event_type":"completed","payload":{"status":"completed"}}"#,
            &mut outcome,
        )
        .unwrap();
        assert!(terminal);
        assert_eq!(outcome.status.as_deref(), Some("completed"));
    }

    #[test]
    fn sse_rejects_an_oversized_unterminated_frame() {
        let mut outcome = empty_sse_outcome();
        let oversized = vec![b'x'; MAX_SSE_FRAME_BYTES + 1];
        let error = apply_sse_eof_buffer(&oversized, &mut outcome).unwrap_err();
        assert!(error.to_string().contains("frame exceeded"));
    }

    #[flow_like_types::tokio::test]
    async fn sse_handler_receives_every_event_in_wire_order() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = concat!(
            "data: {\"event_type\":\"run_initiated\",\"run_id\":\"run-order\",\"payload\":{}}\n\n",
            "data: {\"event_type\":\"chat_stream_partial\",\"payload\":{\"chunk\":{\"content\":\"first\"}}}\n\n",
            "data: {\"event_type\":\"chat_local_session\",\"payload\":{\"turn\":1}}\n\n",
            "data: {\"event_type\":\"chat_stream_partial\",\"payload\":{\"chunk\":{\"content\":\"second\"}}}\n\n",
            "data: {\"event_type\":\"chat_out\",\"payload\":{\"response\":{\"choices\":[]}}}\n\n",
            "data: {\"event_type\":\"completed\",\"payload\":{\"status\":\"completed\"}}"
        );
        let server = flow_like_types::tokio::spawn(async move {
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

            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            for chunk in body.as_bytes().chunks(17) {
                socket.write_all(chunk).await.unwrap();
                flow_like_types::tokio::task::yield_now().await;
            }
        });

        let response = http_client()
            .get(format!("http://{address}/events"))
            .send()
            .await
            .unwrap();
        let mut handler = RecordingSseHandler::default();
        let outcome = collect_sse_outcome(response, &mut handler).await.unwrap();
        server.await.unwrap();

        let event_types = handler
            .events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            [
                "run_initiated",
                "chat_stream_partial",
                "chat_local_session",
                "chat_stream_partial",
                "chat_out",
                "completed",
            ]
        );
        assert_eq!(handler.events[0].run_id.as_deref(), Some("run-order"));
        assert_eq!(handler.events[1].payload["chunk"]["content"], "first");
        assert_eq!(handler.events[3].payload["chunk"]["content"], "second");
        assert_eq!(outcome.run_id.as_deref(), Some("run-order"));
        assert_eq!(outcome.status.as_deref(), Some("completed"));
        assert_eq!(outcome.chat_out.unwrap()["response"]["choices"], json!([]));
    }

    #[test]
    fn expired_remote_credentials_are_refreshed_immediately() {
        let refresh_at = remote_connection_refresh_at(
            Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
            std::time::Instant::now(),
        );
        assert!(refresh_at <= std::time::Instant::now());
        assert!(ensure_remote_connection_fresh(refresh_at).is_err());
    }

    #[test]
    fn missing_remote_credential_expiration_uses_normal_refresh_window() {
        let requested_at = std::time::Instant::now();
        let refresh_at = remote_connection_refresh_at(None, requested_at);
        assert_eq!(
            refresh_at,
            requested_at + std::time::Duration::from_secs(59 * 60)
        );
    }

    #[test]
    fn mcp_response_body_limit_is_enforced_before_appending() {
        let mut body = b"1234".to_vec();
        let error = extend_bounded_body(&mut body, b"56", 5).unwrap_err();
        assert!(error.to_string().contains("5 byte limit"));
        assert_eq!(body, b"1234");
    }

    #[test]
    fn mcp_sse_response_joins_multiline_data_fields() {
        let response = parse_mcp_response_body(
            "Text/Event-Stream; charset=utf-8",
            "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":1,\"result\":{\"ok\":true}}\r\n\r\n",
        )
        .unwrap()
        .unwrap();
        assert_eq!(response["id"], json!(1));
        assert_eq!(response["result"]["ok"], json!(true));
    }

    #[test]
    fn mcp_json_response_is_parsed_normally() {
        let response = parse_mcp_response_body(
            "application/json",
            r#"{"jsonrpc":"2.0","id":2,"result":null}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(response["id"], json!(2));
        assert!(response["result"].is_null());
    }

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

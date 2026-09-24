use flow_like::{
    credentials::{
        SharedCredentials,
        renewable::{RenewableSharedCredentials, SharedCredentialRefresh},
    },
    flow::execution::extract_sub_from_jwt,
};
use flow_like_types::{
    authorization::{
        AuthorizationAttribution, AuthorizationError, AuthorizationFuture, AuthorizationRequest,
        RequestAuthorization, RequestAuthorizer, ResourceAudience,
    },
    base64::{
        Engine,
        engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

const MAX_SESSIONS: usize = 256;
const MAX_LEASES: usize = 64;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

struct Session {
    hub: String,
    subject: String,
    epoch: String,
    token: Mutex<String>,
    revoked: AtomicBool,
}

struct Bridge {
    window: String,
    id: String,
    sequence: u64,
    authority: Option<Arc<Session>>,
}

impl Bridge {
    fn revoke(&mut self) {
        if let Some(previous) = self.authority.take() {
            previous.revoked.store(true, Ordering::Release);
            if let Ok(mut token) = previous.token.lock() {
                token.clear();
            }
        }
    }
}

#[derive(Default)]
struct Sessions {
    bridges: HashMap<String, Bridge>,
}

static SESSIONS: LazyLock<Mutex<Sessions>> = LazyLock::new(Mutex::default);

type LeaseCell = Arc<tokio::sync::OnceCell<Arc<RenewableSharedCredentials>>>;
type LeaseEntries = HashMap<String, (Instant, LeaseCell)>;
static LEASES: LazyLock<Mutex<LeaseEntries>> = LazyLock::new(Mutex::default);
static CACHE_SWEEPER_STARTED: AtomicBool = AtomicBool::new(false);
const IDLE_LEASE_RETENTION: Duration = Duration::from_secs(15 * 60);

fn prune_idle_leases(leases: &mut LeaseEntries) {
    leases.retain(|_, (used, _)| used.elapsed() < IDLE_LEASE_RETENTION);
}

fn start_cache_sweeper() {
    if CACHE_SWEEPER_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            if let Ok(mut leases) = LEASES.lock() {
                // Removing the idle cache reference leaves active runs' own
                // provider alive. Unused providers stop background renewal.
                prune_idle_leases(&mut leases);
            }
        }
    });
}

fn canonical_hub(hub: &str) -> Result<String, AuthorizationError> {
    let normalized = if hub.contains("://") {
        hub.to_owned()
    } else {
        format!("https://{hub}")
    };
    if normalized.contains(['%', '\\']) {
        return Err(AuthorizationError::InvalidRequest);
    }
    let url = reqwest::Url::parse(&normalized).map_err(|_| AuthorizationError::InvalidRequest)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().contains('%')
    {
        return Err(AuthorizationError::InvalidRequest);
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn checked_token(token: &str) -> Result<&str, AuthorizationError> {
    let token = token.strip_prefix("Bearer ").unwrap_or(token);
    if token.is_empty() || token.len() > 16 * 1024 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AuthorizationError::InvalidResponse);
    }
    Ok(token)
}

impl Sessions {
    fn open(&mut self, window: &str, webview: &str) -> Result<String, AuthorizationError> {
        if window.is_empty()
            || webview.is_empty()
            || window.len() > 256
            || webview.len() > 256
            || (!self.bridges.contains_key(webview) && self.bridges.len() >= MAX_SESSIONS)
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        self.close_webview(webview);
        let id = uuid::Uuid::new_v4().to_string();
        self.bridges.insert(
            webview.into(),
            Bridge {
                window: window.into(),
                id: id.clone(),
                sequence: 0,
                authority: None,
            },
        );
        Ok(id)
    }

    fn close_webview(&mut self, webview: &str) {
        if let Some(mut bridge) = self.bridges.remove(webview) {
            bridge.revoke();
        }
    }

    fn close_window(&mut self, window: &str) {
        self.bridges.retain(|_, bridge| {
            if bridge.window != window {
                return true;
            }
            bridge.revoke();
            false
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        webview: &str,
        hub: String,
        subject: Option<String>,
        token: Option<String>,
        session_id: String,
        sequence: u64,
    ) -> Result<(), AuthorizationError> {
        let bridge = self
            .bridges
            .get_mut(webview)
            .ok_or(AuthorizationError::Denied)?;
        if bridge.id != session_id || sequence <= bridge.sequence {
            return Err(AuthorizationError::InvalidRequest);
        }
        // Consume an owned update before admission. A malformed replacement
        // must fence its old session; foreign and stale updates cannot do so.
        bridge.sequence = sequence;
        let admitted = (|| {
            let hub = canonical_hub(&hub)?;
            let identity = match (subject, token) {
                (Some(subject), Some(token)) => {
                    let token = checked_token(&token)?.to_owned();
                    if subject.is_empty()
                        || subject.len() > 1024
                        || extract_sub_from_jwt(&token).ok().as_deref() != Some(subject.as_str())
                    {
                        return Err(AuthorizationError::InvalidResponse);
                    }
                    Some((subject, token))
                }
                (None, None) => None,
                _ => return Err(AuthorizationError::InvalidRequest),
            };
            Ok((hub, identity))
        })();
        let (hub, identity) = match admitted {
            Ok(identity) => identity,
            Err(error) => {
                bridge.revoke();
                return Err(error);
            }
        };
        if let (Some(current), Some((subject, token))) = (&bridge.authority, &identity)
            && current.hub == hub
            && current.subject == *subject
            && !current.revoked.load(Ordering::Acquire)
        {
            *current
                .token
                .lock()
                .map_err(|_| AuthorizationError::Denied)? = token.clone();
            return Ok(());
        }
        bridge.revoke();
        bridge.authority = identity.map(|(subject, token)| {
            Arc::new(Session {
                hub,
                subject,
                epoch: uuid::Uuid::new_v4().to_string(),
                token: Mutex::new(token),
                revoked: AtomicBool::new(false),
            })
        });
        Ok(())
    }

    fn resolve(
        &self,
        hub: &str,
        token: &str,
        session_id: Option<&str>,
        webview: Option<&str>,
    ) -> Result<Arc<Session>, AuthorizationError> {
        let subject = extract_sub_from_jwt(token).map_err(|_| AuthorizationError::Denied)?;
        let matches = |session: &&Arc<Session>| {
            session.hub == hub
                && session.subject == subject
                && !session.revoked.load(Ordering::Acquire)
        };
        match (session_id, webview) {
            (Some(id), Some(webview)) => self
                .bridges
                .get(webview)
                .filter(|bridge| bridge.id == id)
                .and_then(|bridge| bridge.authority.as_ref())
                .filter(matches)
                .cloned()
                .ok_or(AuthorizationError::Denied),
            (None, None) => {
                // Legacy native JWT triggers have no webview. Only a unique
                // current identity is accepted; PAT automation is independent.
                let mut sessions = self
                    .bridges
                    .values()
                    .filter_map(|bridge| bridge.authority.as_ref())
                    .filter(matches);
                let first = sessions.next().cloned().ok_or(AuthorizationError::Denied)?;
                if sessions.next().is_some() {
                    return Err(AuthorizationError::Denied);
                }
                Ok(first)
            }
            _ => Err(AuthorizationError::Denied),
        }
    }

    fn current_token(&self, hub: &str, subject: &str) -> Option<String> {
        self.bridges
            .values()
            .filter_map(|bridge| bridge.authority.as_ref())
            .filter(|session| {
                session.hub == hub
                    && session.subject == subject
                    && !session.revoked.load(Ordering::Acquire)
            })
            .find_map(|session| {
                let token = session.token.lock().ok()?.clone();
                (!token.is_empty()).then_some(token)
            })
    }
}

/// The token a signed-in window currently holds for `subject`, for background
/// work that outlives the run which captured an older one.
pub(crate) fn session_token(hub: &str, subject: &str) -> Option<String> {
    let hub = canonical_hub(hub).ok()?;
    SESSIONS.lock().ok()?.current_token(&hub, subject)
}

pub(crate) fn open_session(window: &str, webview: &str) -> Result<String, String> {
    SESSIONS
        .lock()
        .map_err(|_| AuthorizationError::Denied.to_string())?
        .open(window, webview)
        .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_session(
    webview: &str,
    hub: String,
    subject: Option<String>,
    token: Option<String>,
    session_id: String,
    sequence: u64,
) -> Result<(), String> {
    SESSIONS
        .lock()
        .map_err(|_| AuthorizationError::Denied.to_string())?
        .update(webview, hub, subject, token, session_id, sequence)
        .map_err(|error| error.to_string())
}

pub(crate) fn revoke_webview(webview: &str) {
    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.close_webview(webview);
    }
}

pub(crate) fn revoke_window(window: &str) {
    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.close_window(window);
    }
}

#[derive(Clone)]
enum Authority {
    Session(Arc<Session>),
    PersonalToken(String),
}

impl Authority {
    fn token(&self) -> Result<String, AuthorizationError> {
        match self {
            Self::PersonalToken(token) => Ok(token.clone()),
            Self::Session(session) => {
                let token = session
                    .token
                    .lock()
                    .map_err(|_| AuthorizationError::Denied)?
                    .clone();
                if session.revoked.load(Ordering::Acquire) {
                    return Err(AuthorizationError::Denied);
                }
                Ok(token)
            }
        }
    }
}

fn resolve_authority(
    hub: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    webview: Option<&str>,
) -> Result<Authority, AuthorizationError> {
    let token = checked_token(token.ok_or(AuthorizationError::Denied)?)?;
    if token.starts_with("pat_") {
        return Ok(Authority::PersonalToken(token.to_owned()));
    }
    SESSIONS
        .lock()
        .map_err(|_| AuthorizationError::Denied)?
        .resolve(hub, token, session_id, webview)
        .map(Authority::Session)
}

fn valid_project_id(project: &str) -> bool {
    !project.is_empty()
        && project.len() <= 256
        && project
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

struct DesktopAuthorizer {
    hub: String,
    project: String,
    authority: Result<Authority, AuthorizationError>,
}

impl DesktopAuthorizer {
    fn api_base(&self) -> String {
        if self.hub.ends_with("/api/v1") {
            self.hub.clone()
        } else {
            format!("{}/api/v1", self.hub)
        }
    }

    fn validate_request(
        &self,
        request: &AuthorizationRequest<'_>,
    ) -> Result<(), AuthorizationError> {
        if self.hub.is_empty()
            || !valid_project_id(&self.project)
            || request.url.len() > 4096
            || request.url.contains('\\')
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        let url =
            reqwest::Url::parse(request.url).map_err(|_| AuthorizationError::InvalidRequest)?;
        let base = reqwest::Url::parse(&self.api_base())
            .map_err(|_| AuthorizationError::InvalidRequest)?;
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && loopback)
            || url.origin() != base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.path().contains('%')
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        let path = url
            .path()
            .strip_prefix(base.path())
            .ok_or(AuthorizationError::InvalidRequest)?;
        let allowed = match request.audience {
            ResourceAudience::HostedModels => {
                request.method == "POST"
                    && url.query().is_none()
                    && matches!(
                        path,
                        "/chat/completions" | "/responses" | "/embeddings/embed"
                    )
            }
            ResourceAudience::ProjectApi => {
                matches!(
                    request.method,
                    "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
                ) && (path == format!("/apps/{}", self.project)
                    || path.starts_with(&format!("/apps/{}/", self.project)))
            }
        };
        if !allowed {
            return Err(AuthorizationError::InvalidRequest);
        }
        Ok(())
    }
}

impl RequestAuthorizer for DesktopAuthorizer {
    fn attribution(&self) -> AuthorizationAttribution {
        AuthorizationAttribution::User
    }

    fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
        (!self.hub.is_empty()).then(|| match audience {
            ResourceAudience::HostedModels => self.api_base(),
            ResourceAudience::ProjectApi => format!("{}/apps/{}", self.api_base(), self.project),
        })
    }

    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
        Box::pin(async move {
            self.validate_request(&request)?;
            let authority = self.authority.as_ref().map_err(|error| *error)?;
            let token = authority.token()?;
            let expiry = match authority {
                Authority::Session(session) => {
                    let payload = token
                        .split('.')
                        .nth(1)
                        .ok_or(AuthorizationError::InvalidResponse)?;
                    let decoded = URL_SAFE_NO_PAD
                        .decode(payload)
                        .or_else(|_| URL_SAFE.decode(payload))
                        .map_err(|_| AuthorizationError::InvalidResponse)?;
                    let claims: serde_json::Value = serde_json::from_slice(&decoded)
                        .map_err(|_| AuthorizationError::InvalidResponse)?;
                    let expires_at = claims
                        .get("exp")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(AuthorizationError::InvalidResponse)?;
                    if claims.get("sub").and_then(serde_json::Value::as_str)
                        != Some(session.subject.as_str())
                        || session.revoked.load(Ordering::Acquire)
                    {
                        return Err(AuthorizationError::Denied);
                    }
                    // The hub verifies the JWT signature and current permissions.
                    // Local expiry checks only prevent dispatching a stale snapshot.
                    SystemTime::UNIX_EPOCH
                        .checked_add(Duration::from_secs(expires_at))
                        .ok_or(AuthorizationError::InvalidResponse)?
                }
                // PAT expiry and revocation are checked by the hub on every call.
                // This deadline bounds only this request's credential snapshot.
                Authority::PersonalToken(_) => SystemTime::now() + Duration::from_secs(60),
            };
            RequestAuthorization::new(format!("Bearer {token}"), None, expiry)
        })
    }
}

/// Capture the initiating session without contacting the hub. Local-only work
/// can start with expired or absent login; hosted calls fail at dispatch.
pub(crate) fn request_authorizer(
    hub: &str,
    project: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    webview: Option<&str>,
) -> Arc<dyn RequestAuthorizer> {
    let hub = canonical_hub(hub);
    let authority = match &hub {
        Ok(hub) if valid_project_id(project) => resolve_authority(hub, token, session_id, webview),
        _ => Err(AuthorizationError::InvalidRequest),
    };
    Arc::new(DesktopAuthorizer {
        hub: hub.unwrap_or_default(),
        project: project.to_owned(),
        authority,
    })
}

struct Source {
    hub: String,
    project: String,
    authority: Authority,
    denied: AtomicBool,
    client: reqwest::Client,
}

impl Source {
    fn token(&self) -> Result<String, AuthorizationError> {
        self.authorization_current()?;
        self.authority.token()
    }
}

#[async_trait::async_trait]
impl SharedCredentialRefresh for Source {
    async fn refresh(&self) -> Result<SharedCredentials, AuthorizationError> {
        let token = self.token()?;
        let auth = if token.starts_with("pat_") {
            token
        } else {
            format!("Bearer {token}")
        };
        let mut header = reqwest::header::HeaderValue::from_str(&auth)
            .map_err(|_| AuthorizationError::InvalidResponse)?;
        header.set_sensitive(true);
        let mut response = self
            .client
            .get(format!(
                "{}/api/v1/apps/{}/invoke/presign",
                self.hub, self.project
            ))
            .header(reqwest::header::AUTHORIZATION, header)
            .send()
            .await
            .map_err(|_| AuthorizationError::Unavailable)?;
        match response.status().as_u16() {
            200 => {}
            401 => return Err(AuthorizationError::Expired),
            403 => {
                self.denied.store(true, Ordering::Release);
                return Err(AuthorizationError::Denied);
            }
            429 | 500..=599 => return Err(AuthorizationError::Unavailable),
            _ => return Err(AuthorizationError::InvalidResponse),
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
        {
            return Err(AuthorizationError::InvalidResponse);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AuthorizationError::Unavailable)?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AuthorizationError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        self.authorization_current()?;
        serde_json::from_slice(&bytes).map_err(|_| AuthorizationError::InvalidResponse)
    }

    fn authorization_current(&self) -> Result<(), AuthorizationError> {
        if self.denied.load(Ordering::Acquire)
            || matches!(&self.authority, Authority::Session(session) if session.revoked.load(Ordering::Acquire))
        {
            return Err(AuthorizationError::Denied);
        }
        Ok(())
    }
}

pub(crate) async fn prepare(
    hub: &str,
    project: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    webview: Option<&str>,
) -> flow_like_types::Result<SharedCredentials> {
    let hub = canonical_hub(hub)?;
    if !valid_project_id(project) {
        return Err(AuthorizationError::InvalidRequest.into());
    }
    let authority = resolve_authority(&hub, token, session_id, webview)?;
    let authority_key = match &authority {
        Authority::PersonalToken(token) => blake3::hash(token.as_bytes()).to_hex().to_string(),
        Authority::Session(session) => session.epoch.clone(),
    };
    let key = blake3::hash(&serde_json::to_vec(&(&hub, project, authority_key))?)
        .to_hex()
        .to_string();
    start_cache_sweeper();
    let cell = {
        let mut leases = LEASES.lock().map_err(|_| AuthorizationError::Denied)?;
        prune_idle_leases(&mut leases);
        if leases
            .get(&key)
            .and_then(|(_, cell)| cell.get())
            .is_some_and(|lease| lease.authorization_current() == Err(AuthorizationError::Denied))
        {
            // Recheck an explicit new execution after permissions change. Old
            // runs retain their terminally denied provider and stay fenced.
            leases.remove(&key);
        }
        if let Some((used, cell)) = leases.get_mut(&key) {
            *used = Instant::now();
            cell.clone()
        } else {
            if leases.len() >= MAX_LEASES {
                let oldest = leases
                    .iter()
                    .min_by_key(|(_, (used, _))| *used)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    leases.remove(&oldest);
                }
            }
            let cell = Arc::new(tokio::sync::OnceCell::new());
            leases.insert(key, (Instant::now(), cell.clone()));
            cell
        }
    };
    // Initial requests for the same scope share one network operation; other
    // projects never wait on this scope's issuer or retry backoff.
    let credentials = cell
        .get_or_try_init(|| async move {
            let source = Arc::new(Source {
                hub,
                project: project.to_owned(),
                authority,
                denied: AtomicBool::new(false),
                client: reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(10))
                    .timeout(Duration::from_secs(30))
                    .build()?,
            });
            let initial = source.refresh().await?;
            Ok::<_, flow_like_types::Error>(
                RenewableSharedCredentials::new(initial, project.to_owned(), source).await?,
            )
        })
        .await?;
    credentials.authorization_current()?;
    Ok(SharedCredentials::Renewable(credentials.clone()))
}

/// Credentials choose where a run's data lives; whether it may run at all is
/// decided by its execution identity. An issuer that cannot be reached or
/// answers garbage therefore sends the run to device storage. A reachable
/// issuer's refusal (401, 403) or a malformed request still stops it, so an
/// online run never silently writes to the device instead of the project.
pub(crate) fn falls_back_to_device_storage(error: &flow_like_types::Error) -> bool {
    matches!(
        error.downcast_ref::<AuthorizationError>(),
        Some(AuthorizationError::Unavailable | AuthorizationError::InvalidResponse)
    )
}

pub(crate) fn install_registry(
    state: &mut flow_like::state::FlowLikeState,
    credentials: &SharedCredentials,
) -> flow_like_types::Result<()> {
    if let SharedCredentials::Renewable(credentials) = credentials {
        state.set_lance_store_registry(credentials.lance_registry_with_local()?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUB: &str = "https://api.example.test";
    const ALICE: &str = "e30.eyJzdWIiOiJhbGljZSJ9.first";
    const ALICE_REFRESHED: &str = "e30.eyJzdWIiOiJhbGljZSJ9.second";
    const BOB: &str = "e30.eyJzdWIiOiJib2IifQ.signature";

    fn jwt(subject: &str, expires_at: u64, signature: &str) -> String {
        format!(
            "e30.{}.{}",
            URL_SAFE_NO_PAD
                .encode(serde_json::json!({"sub": subject, "exp": expires_at}).to_string()),
            signature
        )
    }

    #[tokio::test]
    async fn desktop_authority_limits_routes_and_expiry_without_rebinding_old_sessions() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        let expiry = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = jwt("alice", expiry, "first");
        set(&mut sessions, "main", &id, "alice", &token, 1);
        let authorizer = DesktopAuthorizer {
            hub: HUB.into(),
            project: "project".into(),
            authority: Ok(Authority::Session(bound(&sessions, "main", &id, &token))),
        };
        assert_eq!(authorizer.attribution(), AuthorizationAttribution::User);
        for (audience, method, suffix) in [
            (
                ResourceAudience::HostedModels,
                "POST",
                "/api/v1/chat/completions",
            ),
            (ResourceAudience::HostedModels, "POST", "/api/v1/responses"),
            (
                ResourceAudience::HostedModels,
                "POST",
                "/api/v1/embeddings/embed",
            ),
            (
                ResourceAudience::ProjectApi,
                "POST",
                "/api/v1/apps/project/connections/other/token",
            ),
            (
                ResourceAudience::ProjectApi,
                "GET",
                "/api/v1/apps/project/cache?key=a%20b&namespace=%E2%82%AC",
            ),
        ] {
            let url = format!("{HUB}{suffix}");
            assert_eq!(
                authorizer
                    .authorize(AuthorizationRequest {
                        audience,
                        method,
                        url: &url
                    })
                    .await
                    .unwrap()
                    .authorization(),
                format!("Bearer {token}")
            );
        }
        for (audience, method, url) in [
            (
                ResourceAudience::HostedModels,
                "GET",
                "https://api.example.test/api/v1/chat/completions",
            ),
            (
                ResourceAudience::HostedModels,
                "POST",
                "https://api.example.test/api/v1/embeddings/embed?redirect=1",
            ),
            (
                ResourceAudience::HostedModels,
                "POST",
                "https://attacker.test/api/v1/chat/completions",
            ),
            (
                ResourceAudience::HostedModels,
                "POST",
                "https://alice@api.example.test/api/v1/chat/completions",
            ),
            (
                ResourceAudience::HostedModels,
                "POST",
                "https://api.example.test/api/v1/instances/chat/completions",
            ),
            (
                ResourceAudience::ProjectApi,
                "POST",
                "https://api.example.test/api/v1/apps/project-other/connections/app/token",
            ),
            (
                ResourceAudience::ProjectApi,
                "POST",
                "https://api.example.test/api/v1/apps/other/connections/app/token",
            ),
            (
                ResourceAudience::ProjectApi,
                "POST",
                "https://api.example.test/api/v1/apps/project/%2e%2e/admin",
            ),
            (
                ResourceAudience::ProjectApi,
                "GET",
                "https://api.example.test/api/v1/apps/project/cache%2fprivate?key=a%20b",
            ),
            (
                ResourceAudience::ProjectApi,
                "GET",
                "https://api.example.test/api/v1/apps/project-other/cache?key=a%20b",
            ),
            (
                ResourceAudience::ProjectApi,
                "GET",
                "https://attacker.test/api/v1/apps/project/cache?key=a%20b",
            ),
        ] {
            assert_eq!(
                authorizer
                    .authorize(AuthorizationRequest {
                        audience,
                        method,
                        url
                    })
                    .await
                    .unwrap_err(),
                AuthorizationError::InvalidRequest,
                "{url}"
            );
        }
        let request = || AuthorizationRequest {
            audience: ResourceAudience::HostedModels,
            method: "POST",
            url: "https://api.example.test/api/v1/chat/completions",
        };
        set(
            &mut sessions,
            "main",
            &id,
            "alice",
            &jwt("alice", 1, "expired"),
            2,
        );
        assert_eq!(
            authorizer.authorize(request()).await.unwrap_err(),
            AuthorizationError::Expired
        );
        let refreshed = jwt("alice", expiry, "refreshed");
        set(&mut sessions, "main", &id, "alice", &refreshed, 3);
        assert_eq!(
            authorizer
                .authorize(request())
                .await
                .unwrap()
                .authorization(),
            format!("Bearer {refreshed}")
        );
        sessions.close_window("main");
        let next = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &next, "alice", &refreshed, 1);
        assert_eq!(
            authorizer.authorize(request()).await.unwrap_err(),
            AuthorizationError::Denied
        );
    }

    #[tokio::test]
    async fn retained_desktop_model_and_embedding_clients_rotate_and_fence_before_dispatch() {
        use flow_like::flow_like_model_provider::{
            embedding::{EmbeddingModelLogic, proxy::ProxyEmbeddingModel},
            history::{History, HistoryMessage, Role},
            llm::{ModelLogic, openai::OpenAIModel},
            provider::{EmbeddingModelProvider, ModelApiSurface, ModelProvider, Pooling, Prefix},
        };
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hub = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut captured = Vec::new();
            for _ in 0..4 {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(10), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 4096];
                let headers = loop {
                    let count =
                        tokio::time::timeout(Duration::from_secs(10), socket.read(&mut buffer))
                            .await
                            .unwrap()
                            .unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() < 64 * 1024);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8(bytes[..end + 2].to_vec()).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|length| length.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break headers;
                        }
                    }
                };
                let response = if headers.starts_with("POST /api/v1/embeddings/embed ") {
                    r#"{"embeddings":[[1.0,2.0]],"model":"bit","usage":{"prompt_tokens":1,"total_tokens":1}}"#
                } else {
                    assert!(headers.starts_with("POST /api/v1/chat/completions "));
                    r#"{"id":"request","object":"chat.completion","created":1,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
                captured.push(headers);
            }
            captured
        });
        let expiry = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let first = jwt("alice", expiry, "first");
        let second = jwt("alice", expiry, "second");
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        sessions
            .update(
                "main",
                hub.clone(),
                Some("alice".into()),
                Some(first.clone()),
                id.clone(),
                1,
            )
            .unwrap();
        let authority = sessions
            .resolve(&hub, &first, Some(&id), Some("main"))
            .unwrap();
        let authorizer = Arc::new(DesktopAuthorizer {
            hub: hub.clone(),
            project: "project".into(),
            authority: Ok(Authority::Session(authority)),
        });
        let provider = ModelProvider {
            provider_name: "hosted:openai".into(),
            model_id: Some("model".into()),
            version: None,
            api_surface: Some(ModelApiSurface::ChatCompletions),
            params: Some(HashMap::from([
                (
                    "endpoint".into(),
                    serde_json::json!(format!("{hub}/api/v1")),
                ),
                ("model_id".into(), serde_json::json!("model")),
                (
                    "api_key".into(),
                    serde_json::json!("obsolete-startup-token"),
                ),
                (
                    "headers".into(),
                    serde_json::json!({"x-flow-like-app-id":"project","x-flow-like-run-id":"run"}),
                ),
            ])),
        };
        let model = OpenAIModel::from_provider_with_surface_and_authorizer(
            &provider,
            ModelApiSurface::ChatCompletions,
            Some(authorizer.clone()),
        )
        .await
        .unwrap();
        let embedding = ProxyEmbeddingModel::new(
            EmbeddingModelProvider {
                languages: vec!["en".into()],
                vector_length: 2,
                input_length: 32,
                prefix: Prefix {
                    query: String::new(),
                    paragraph: String::new(),
                },
                pooling: Pooling::Mean,
                provider,
                remote: None,
            },
            "bit".into(),
            "obsolete-startup-token".into(),
            vec![
                ("x-flow-like-app-id".into(), "project".into()),
                ("x-flow-like-run-id".into(), "run".into()),
            ],
            "https://untrusted-bit-endpoint.test".into(),
        )
        .with_authorizer(authorizer)
        .unwrap();
        let mut history = History::new(
            "model".into(),
            vec![HistoryMessage::from_string(Role::User, "hello")],
        );
        history.set_stream(false);
        let texts = vec!["hello".into()];
        model.invoke(&history, None).await.unwrap();
        assert_eq!(
            embedding.text_embed_query(&texts).await.unwrap(),
            vec![vec![1.0, 2.0]]
        );
        sessions
            .update(
                "main",
                hub.clone(),
                Some("alice".into()),
                Some(second.clone()),
                id.clone(),
                2,
            )
            .unwrap();
        model.invoke(&history, None).await.unwrap();
        embedding.text_embed_query(&texts).await.unwrap();

        sessions
            .update(
                "main",
                hub.clone(),
                Some("alice".into()),
                Some(jwt("alice", 1, "expired")),
                id.clone(),
                3,
            )
            .unwrap();
        assert!(
            model
                .invoke(&history, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Resource authorization has expired")
        );
        assert!(
            embedding
                .text_embed_query(&texts)
                .await
                .unwrap_err()
                .to_string()
                .contains("Resource authorization has expired")
        );
        sessions
            .update("main", hub.clone(), None, None, id.clone(), 4)
            .unwrap();
        sessions
            .update(
                "main",
                hub,
                Some("alice".into()),
                Some(second.clone()),
                id,
                5,
            )
            .unwrap();
        assert!(
            model
                .invoke(&history, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Resource authorization was denied")
        );
        assert!(
            embedding
                .text_embed_query(&texts)
                .await
                .unwrap_err()
                .to_string()
                .contains("Resource authorization was denied")
        );
        let requests = server.await.unwrap();
        for (index, headers) in requests.iter().enumerate() {
            let token = if index < 2 { &first } else { &second };
            assert!(headers.to_ascii_lowercase().contains(&format!(
                "authorization: bearer {}\r\n",
                token.to_ascii_lowercase()
            )));
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("x-flow-like-app-id: project\r\n")
            );
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("x-flow-like-run-id: run\r\n")
            );
            assert!(!headers.contains("obsolete-startup-token"));
        }
    }

    fn set(
        sessions: &mut Sessions,
        view: &str,
        id: &str,
        subject: &str,
        token: &str,
        sequence: u64,
    ) {
        sessions
            .update(
                view,
                HUB.into(),
                Some(subject.into()),
                Some(token.into()),
                id.into(),
                sequence,
            )
            .unwrap();
    }

    fn source(authority: Authority) -> Source {
        Source {
            hub: HUB.into(),
            project: "project".into(),
            authority,
            denied: AtomicBool::new(false),
            client: reqwest::Client::new(),
        }
    }

    fn bound(sessions: &Sessions, view: &str, id: &str, token: &str) -> Arc<Session> {
        sessions.resolve(HUB, token, Some(id), Some(view)).unwrap()
    }

    #[test]
    fn session_rotation_updates_existing_source_and_logout_never_revives_it() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        let first = bound(&sessions, "main", &id, ALICE);
        let live = source(Authority::Session(first.clone()));
        set(&mut sessions, "main", &id, "alice", ALICE_REFRESHED, 2);
        assert_eq!(live.token().unwrap(), ALICE_REFRESHED);
        assert!(Arc::ptr_eq(&first, &bound(&sessions, "main", &id, ALICE)));
        sessions
            .update("main", HUB.into(), None, None, id.clone(), 3)
            .unwrap();
        assert_eq!(
            live.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        set(&mut sessions, "main", &id, "alice", ALICE_REFRESHED, 4);
        assert_eq!(live.token(), Err(AuthorizationError::Denied));
        assert_ne!(first.epoch, bound(&sessions, "main", &id, ALICE).epoch);
    }

    #[test]
    fn independent_windows_reject_foreign_bridges_and_each_logout_fences_its_runs() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        let second_id = sessions.open("second", "second").unwrap();
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        set(
            &mut sessions,
            "second",
            &second_id,
            "alice",
            ALICE_REFRESHED,
            1,
        );
        let first = source(Authority::Session(bound(&sessions, "main", &id, ALICE)));
        let second = source(Authority::Session(bound(
            &sessions, "second", &second_id, ALICE,
        )));
        assert!(sessions.resolve(HUB, ALICE, None, None).is_err());
        assert!(
            sessions
                .resolve(HUB, ALICE, Some(&second_id), Some("main"))
                .is_err()
        );
        assert!(sessions.resolve(HUB, ALICE, Some(&id), None).is_err());
        assert!(
            sessions
                .update("main", HUB.into(), None, None, second_id.clone(), 2)
                .is_err()
        );
        assert!(first.authorization_current().is_ok());
        assert!(second.authorization_current().is_ok());
        sessions
            .update("main", HUB.into(), None, None, id, 2)
            .unwrap();
        assert_eq!(
            first.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(second.authorization_current().is_ok());
        assert!(sessions.resolve(HUB, ALICE, None, None).is_ok());
        sessions
            .update("second", HUB.into(), None, None, second_id, 2)
            .unwrap();
        assert_eq!(
            second.authorization_current(),
            Err(AuthorizationError::Denied)
        );
    }

    #[test]
    fn account_and_hub_changes_fence_previous_authority() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        let alice = source(Authority::Session(bound(&sessions, "main", &id, ALICE)));
        set(&mut sessions, "main", &id, "bob", BOB, 2);
        assert_eq!(
            alice.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(
            sessions
                .resolve(HUB, ALICE, Some(&id), Some("main"))
                .is_err()
        );
        let bob = source(Authority::Session(bound(&sessions, "main", &id, BOB)));
        sessions
            .update(
                "main",
                "https://other.example.test".into(),
                Some("bob".into()),
                Some(BOB.into()),
                id.clone(),
                3,
            )
            .unwrap();
        assert_eq!(bob.authorization_current(), Err(AuthorizationError::Denied));
        assert!(
            sessions
                .resolve("https://other.example.test", BOB, Some(&id), Some("main"))
                .is_ok()
        );
        assert!(
            sessions
                .update("main", HUB.into(), None, None, id, 2)
                .is_err()
        );
    }

    #[test]
    fn reloading_replaces_authority_without_accumulating_bridge_tombstones() {
        let mut sessions = Sessions::default();
        let original = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &original, "alice", ALICE, 1);
        let previous = source(Authority::Session(bound(
            &sessions, "main", &original, ALICE,
        )));
        // Backend recreation replaces the bridge even without navigation.
        let mut id = sessions.open("main", "main").unwrap();
        assert_eq!(
            previous.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        for _ in 0..MAX_SESSIONS * 2 {
            sessions.close_webview("main");
            assert!(
                sessions
                    .update(
                        "main",
                        HUB.into(),
                        Some("alice".into()),
                        Some(ALICE.into()),
                        id,
                        2
                    )
                    .is_err()
            );
            id = sessions.open("main", "main").unwrap();
            set(&mut sessions, "main", &id, "alice", ALICE, 1);
        }
        assert_eq!(sessions.bridges.len(), 1);
        assert!(
            sessions
                .update(
                    "main",
                    HUB.into(),
                    Some("alice".into()),
                    Some(ALICE.into()),
                    original,
                    2
                )
                .is_err()
        );
        assert!(sessions.resolve(HUB, ALICE, None, None).is_ok());
    }

    #[test]
    fn native_navigation_and_window_destroy_revoke_even_without_js_cleanup() {
        let mut sessions = Sessions::default();
        let main = sessions.open("main-window", "main-view").unwrap();
        let child = sessions.open("main-window", "child-view").unwrap();
        let other = sessions.open("other-window", "other-view").unwrap();
        set(&mut sessions, "main-view", &main, "alice", ALICE, 1);
        set(&mut sessions, "child-view", &child, "alice", ALICE, 1);
        set(&mut sessions, "other-view", &other, "alice", ALICE, 1);
        let main_live = source(Authority::Session(bound(
            &sessions,
            "main-view",
            &main,
            ALICE,
        )));
        let child_live = source(Authority::Session(bound(
            &sessions,
            "child-view",
            &child,
            ALICE,
        )));
        let other_live = source(Authority::Session(bound(
            &sessions,
            "other-view",
            &other,
            ALICE,
        )));
        sessions.close_webview("main-view");
        assert_eq!(
            main_live.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(child_live.authorization_current().is_ok());
        sessions.close_window("main-window");
        assert_eq!(
            child_live.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(other_live.authorization_current().is_ok());
        assert_eq!(sessions.bridges.len(), 1);
    }

    #[test]
    fn malformed_owned_replacement_fences_authority_but_stale_update_does_not() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        let live = source(Authority::Session(bound(&sessions, "main", &id, ALICE)));
        assert!(
            sessions
                .update("main", "invalid://hub".into(), None, None, id.clone(), 1)
                .is_err()
        );
        assert!(live.authorization_current().is_ok());
        assert!(
            sessions
                .update("main", "invalid://hub".into(), None, None, id.clone(), 2)
                .is_err()
        );
        assert_eq!(
            live.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        set(&mut sessions, "main", &id, "alice", ALICE, 3);
        let replacement = source(Authority::Session(bound(&sessions, "main", &id, ALICE)));
        assert!(
            sessions
                .update(
                    "main",
                    HUB.into(),
                    Some("bob".into()),
                    Some(ALICE.into()),
                    id.clone(),
                    4
                )
                .is_err()
        );
        assert_eq!(
            replacement.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(
            sessions
                .update(
                    "main",
                    HUB.into(),
                    Some("alice".into()),
                    Some(ALICE.into()),
                    id,
                    3
                )
                .is_err()
        );
    }

    #[test]
    fn current_token_follows_rotation_and_ends_with_the_session() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        assert_eq!(sessions.current_token(HUB, "alice"), None);
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        set(&mut sessions, "main", &id, "alice", ALICE_REFRESHED, 2);
        assert_eq!(
            sessions.current_token(HUB, "alice").as_deref(),
            Some(ALICE_REFRESHED)
        );
        assert_eq!(sessions.current_token(HUB, "bob"), None);
        assert_eq!(
            sessions.current_token("https://other.example.test", "alice"),
            None
        );
        sessions.close_window("main");
        assert_eq!(sessions.current_token(HUB, "alice"), None);
    }

    #[test]
    fn idle_cache_eviction_keeps_recent_entries_and_external_references() {
        let cell = Arc::new(tokio::sync::OnceCell::new());
        let mut entries = HashMap::from([
            (
                "idle".into(),
                (Instant::now() - IDLE_LEASE_RETENTION, cell.clone()),
            ),
            (
                "recent".into(),
                (Instant::now(), Arc::new(tokio::sync::OnceCell::new())),
            ),
        ]);
        prune_idle_leases(&mut entries);
        assert!(!entries.contains_key("idle"));
        assert!(entries.contains_key("recent"));
        assert_eq!(Arc::strong_count(&cell), 1);
    }

    #[test]
    fn pat_is_independent_of_ui_logout_and_hub_requires_an_explicit_valid_transport() {
        let mut sessions = Sessions::default();
        let id = sessions.open("main", "main").unwrap();
        set(&mut sessions, "main", &id, "alice", ALICE, 1);
        let pat = source(Authority::PersonalToken("pat_automation".into()));
        sessions.close_window("main");
        assert_eq!(pat.token().unwrap(), "pat_automation");
        for invalid in [
            "ftp://api.example.test",
            "https://user:secret@api.example.test",
            "https://api.example.test?token=secret",
            "https://api.example.test/#fragment",
            "https://api.example.test/%2e%2e",
        ] {
            assert!(canonical_hub(invalid).is_err());
        }
        assert_eq!(canonical_hub("https://API.EXAMPLE.test/").unwrap(), HUB);
        assert_eq!(
            canonical_hub("http://192.168.1.12:8080/").unwrap(),
            "http://192.168.1.12:8080"
        );
    }

    async fn response_source(
        status: u16,
        body: &'static str,
    ) -> (Source, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() < 16 * 1024);
            }
            socket.write_all(format!("HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let mut source = source(Authority::PersonalToken("pat_automation".into()));
        source.hub = format!("http://{address}");
        (source, server)
    }

    #[tokio::test]
    async fn expired_session_and_outage_are_retryable_but_denial_fences_the_source() {
        for (status, expected) in [
            (401, AuthorizationError::Expired),
            (503, AuthorizationError::Unavailable),
            (403, AuthorizationError::Denied),
            (404, AuthorizationError::InvalidResponse),
        ] {
            let (source, server) = response_source(status, "private issuer error").await;
            let error = source.refresh().await.unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("private"));
            assert_eq!(source.authorization_current().is_err(), status == 403);
            let request = server.await.unwrap();
            assert!(request.starts_with("GET /api/v1/apps/project/invoke/presign "));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: pat_automation\r\n")
            );
        }
    }
}

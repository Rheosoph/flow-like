use flow_like::{
    credentials::{
        SharedCredentials,
        renewable::{RenewableSharedCredentials, SharedCredentialRefresh},
    },
    flow::execution::extract_sub_from_jwt,
};
use flow_like_types::authorization::AuthorizationError;
use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
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

#[derive(Default)]
struct Sessions {
    sequence: HashMap<String, u64>,
    current: HashMap<String, Arc<Session>>,
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
    fn update(
        &mut self,
        hub: String,
        subject: Option<String>,
        token: Option<String>,
        session_id: String,
        sequence: u64,
    ) -> Result<(), AuthorizationError> {
        if session_id.is_empty() || session_id.len() > 128 || sequence == 0 {
            return Err(AuthorizationError::InvalidRequest);
        }
        if self
            .sequence
            .get(&session_id)
            .is_some_and(|previous| sequence <= *previous)
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        if !self.sequence.contains_key(&session_id) && self.sequence.len() >= MAX_SESSIONS {
            return Err(AuthorizationError::InvalidRequest);
        }
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
        self.sequence.insert(session_id.clone(), sequence);
        if let (Some(current), Some((subject, token))) = (self.current.get(&session_id), &identity)
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
        if let Some(previous) = self.current.remove(&session_id) {
            previous.revoked.store(true, Ordering::Release);
            previous
                .token
                .lock()
                .map_err(|_| AuthorizationError::Denied)?
                .clear();
        }
        if let Some((subject, token)) = identity {
            self.current.insert(
                session_id,
                Arc::new(Session {
                    hub,
                    subject,
                    epoch: uuid::Uuid::new_v4().to_string(),
                    token: Mutex::new(token),
                    revoked: AtomicBool::new(false),
                }),
            );
        }
        Ok(())
    }

    fn resolve(
        &self,
        hub: &str,
        token: &str,
        bridge: Option<&str>,
    ) -> Result<Arc<Session>, AuthorizationError> {
        let subject = extract_sub_from_jwt(token).map_err(|_| AuthorizationError::Denied)?;
        let matches = |session: &&Arc<Session>| {
            session.hub == hub
                && session.subject == subject
                && !session.revoked.load(Ordering::Acquire)
        };
        if let Some(bridge) = bridge {
            return self
                .current
                .get(bridge)
                .filter(matches)
                .cloned()
                .ok_or(AuthorizationError::Denied);
        }
        // Legacy native JWT triggers have no window ID. Accept only an
        // unambiguous current session; PAT automation does not use this path.
        let mut sessions = self.current.values().filter(matches);
        let first = sessions.next().cloned().ok_or(AuthorizationError::Denied)?;
        if sessions.next().is_some() {
            return Err(AuthorizationError::Denied);
        }
        Ok(first)
    }
}

/// Only the current webview session supplies renewable account tokens. Native
/// runs retain this session's identity, so logout cannot switch a live run to
/// another account or revive a previously revoked session.
#[tauri::command]
pub fn execution_set_auth(
    hub: String,
    subject: Option<String>,
    token: Option<String>,
    session_id: String,
    sequence: u64,
) -> Result<(), String> {
    SESSIONS
        .lock()
        .map_err(|_| AuthorizationError::Denied.to_string())?
        .update(hub, subject, token, session_id, sequence)
        .map_err(|error| error.to_string())
}

enum Authority {
    Session(Arc<Session>),
    PersonalToken(String),
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
        match &self.authority {
            Authority::PersonalToken(token) => Ok(token.clone()),
            Authority::Session(session) => {
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
) -> flow_like_types::Result<SharedCredentials> {
    let hub = canonical_hub(hub)?;
    if project.is_empty()
        || project.len() > 256
        || !project
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AuthorizationError::InvalidRequest.into());
    }
    let token = checked_token(token.ok_or(AuthorizationError::Denied)?)?;
    let (authority, authority_key) = if token.starts_with("pat_") {
        (
            Authority::PersonalToken(token.to_owned()),
            blake3::hash(token.as_bytes()).to_hex().to_string(),
        )
    } else {
        let session = SESSIONS
            .lock()
            .map_err(|_| AuthorizationError::Denied)?
            .resolve(&hub, token, session_id)?;
        let epoch = session.epoch.clone();
        (Authority::Session(session), epoch)
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

    fn set(sessions: &mut Sessions, subject: &str, token: &str, sequence: u64) {
        sessions
            .update(
                HUB.into(),
                Some(subject.into()),
                Some(token.into()),
                "window".into(),
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

    #[test]
    fn session_rotation_updates_existing_source_and_logout_never_revives_it() {
        let mut sessions = Sessions::default();
        set(&mut sessions, "alice", ALICE, 1);
        let first = sessions.resolve(HUB, ALICE, Some("window")).unwrap();
        let live = source(Authority::Session(first.clone()));
        set(&mut sessions, "alice", ALICE_REFRESHED, 2);
        assert_eq!(live.token().unwrap(), ALICE_REFRESHED);
        assert!(Arc::ptr_eq(
            &first,
            &sessions.resolve(HUB, ALICE, Some("window")).unwrap()
        ));
        sessions
            .update(HUB.into(), None, None, "window".into(), 3)
            .unwrap();
        assert_eq!(
            live.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        set(&mut sessions, "alice", ALICE_REFRESHED, 4);
        assert_eq!(live.token(), Err(AuthorizationError::Denied));
        assert_ne!(
            first.epoch,
            sessions.resolve(HUB, ALICE, Some("window")).unwrap().epoch
        );
    }

    #[test]
    fn windows_have_independent_sessions_and_each_logout_fences_its_own_runs() {
        let mut sessions = Sessions::default();
        set(&mut sessions, "alice", ALICE, 1);
        let initial = sessions.resolve(HUB, ALICE, Some("window")).unwrap();
        let first = source(Authority::Session(initial.clone()));
        sessions
            .update(
                HUB.into(),
                Some("alice".into()),
                Some(ALICE_REFRESHED.into()),
                "second-window".into(),
                1,
            )
            .unwrap();
        assert!(Arc::ptr_eq(
            &initial,
            &sessions.resolve(HUB, ALICE, Some("window")).unwrap()
        ));
        let second = source(Authority::Session(
            sessions.resolve(HUB, ALICE, Some("second-window")).unwrap(),
        ));
        assert_eq!(first.token().unwrap(), ALICE);
        assert_eq!(second.token().unwrap(), ALICE_REFRESHED);
        assert!(sessions.resolve(HUB, ALICE, None).is_err());
        sessions
            .update(HUB.into(), None, None, "window".into(), 2)
            .unwrap();
        assert_eq!(
            first.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(second.authorization_current().is_ok());
        assert!(sessions.resolve(HUB, ALICE, None).is_ok());
        assert!(
            sessions
                .update(
                    HUB.into(),
                    Some("alice".into()),
                    Some(ALICE.into()),
                    "window".into(),
                    1
                )
                .is_err()
        );
        sessions
            .update(HUB.into(), None, None, "second-window".into(), 2)
            .unwrap();
        assert_eq!(
            second.authorization_current(),
            Err(AuthorizationError::Denied)
        );
    }

    #[test]
    fn account_and_hub_changes_fence_previous_authority() {
        let mut sessions = Sessions::default();
        set(&mut sessions, "alice", ALICE, 1);
        let alice = source(Authority::Session(
            sessions.resolve(HUB, ALICE, Some("window")).unwrap(),
        ));
        set(&mut sessions, "bob", BOB, 2);
        assert_eq!(
            alice.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        assert!(sessions.resolve(HUB, ALICE, Some("window")).is_err());
        assert!(
            sessions
                .resolve("https://other.example.test", BOB, Some("window"))
                .is_err()
        );
        let bob = source(Authority::Session(
            sessions.resolve(HUB, BOB, Some("window")).unwrap(),
        ));
        sessions
            .update(
                "https://other.example.test".into(),
                Some("bob".into()),
                Some(BOB.into()),
                "window".into(),
                3,
            )
            .unwrap();
        assert_eq!(bob.authorization_current(), Err(AuthorizationError::Denied));
        assert!(
            sessions
                .resolve("https://other.example.test", BOB, Some("window"))
                .is_ok()
        );
        assert!(
            sessions
                .update(HUB.into(), None, None, "window".into(), 2)
                .is_err()
        );
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
    fn supplied_subject_must_match_token_and_pat_is_independent_of_ui_logout() {
        let mut sessions = Sessions::default();
        set(&mut sessions, "alice", ALICE, 1);
        assert!(
            sessions
                .update(
                    HUB.into(),
                    Some("bob".into()),
                    Some(ALICE.into()),
                    "window".into(),
                    2
                )
                .is_err()
        );
        let pat = source(Authority::PersonalToken("pat_automation".into()));
        sessions
            .update(HUB.into(), None, None, "window".into(), 2)
            .unwrap();
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

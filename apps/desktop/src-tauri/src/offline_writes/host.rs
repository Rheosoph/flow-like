//! `DesktopHost`: the engine's replay transport, cloud access and file hooks (design §4.8).

use super::{
    api_base, authorization,
    object_index::ObjectIndex,
    scope::{AppOfflineScope, ScopeKey, location_prefix},
    stores::{LocalCacheView, ScopeConnectivity},
    texts,
};
use anyhow::{Context, Result};
use flow_like::{
    credentials::{SharedCredentials, StoreType, renewable::RenewableSharedCredentials},
    flow_like_storage::{
        lancedb::{Connection, Table},
        object_store::{ObjectMeta, ObjectStore, path::Path as ObjectPath},
    },
};
use flow_like_device_protocol::{
    OFFLINE_ERROR_DIGEST_REUSED, OFFLINE_ERROR_SUBJECT_MISMATCH, OFFLINE_INSTALLATION_HEADER,
    OFFLINE_SUBJECT_HEADER, OfflineExpected, OfflineReplayRequest, OfflineReplayResponse,
    StoragePurpose, desktop_offline_replay_path, format_limit,
};
use flow_like_offline_writes::{
    BufferedTable, Connectivity, Observation, OfflineHost, ReplayError, ReplayErrorKind,
    optional_table,
};
use flow_like_types::{authorization::AuthorizationError, reqwest};
use serde::Deserialize;
use std::{
    sync::{Arc, Mutex, MutexGuard, Weak},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REPLAY_TIMEOUT: Duration = Duration::from_secs(150);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 2048;
const OBJECT_STORE_REUSE: Duration = Duration::from_secs(60);
const ACTIVATION_RETRY: Duration = Duration::from_secs(30);

pub(crate) type EmitSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Replay tokens: the signed-in session first, then sink PATs bound to the scope.
pub(crate) trait TokenSource: Send + Sync {
    fn session_token(&self, hub: &str, subject: &str) -> Option<String>;
    fn sink_token(&self, app_id: &str, event_id: &str) -> Option<String>;
}

/// The cloud behind a scope: its databases and its content store.
#[async_trait::async_trait]
pub(crate) trait CloudAccess: Send + Sync {
    /// A connection to the project or user database; None without a lease and a token.
    async fn database(
        &self,
        host: &DesktopHost,
        purpose: StoragePurpose,
    ) -> Result<Option<Connection>>;
    /// The bucket-relative content store, never decorated; None without a lease and a token.
    async fn objects(&self, host: &DesktopHost) -> Result<Option<Arc<dyn ObjectStore>>>;
}

/// What a scope needs from the application.
#[derive(Clone)]
pub(crate) struct Services {
    pub(crate) cloud: Arc<dyn CloudAccess>,
    pub(crate) tokens: Arc<dyn TokenSource>,
    pub(crate) emit: EmitSink,
    pub(crate) activation_retry: Duration,
}

impl Services {
    pub(crate) fn desktop(app_handle: Option<AppHandle>) -> Self {
        let emitter = app_handle.clone();
        Self {
            cloud: Arc::new(LeaseCloud),
            tokens: Arc::new(DesktopTokens { app_handle }),
            emit: Arc::new(move |event, payload| {
                if let Some(app_handle) = &emitter {
                    crate::utils::emit_to_ui(app_handle, event, payload);
                }
            }),
            activation_retry: ACTIVATION_RETRY,
        }
    }
}

struct DesktopTokens {
    app_handle: Option<AppHandle>,
}

impl TokenSource for DesktopTokens {
    fn session_token(&self, hub: &str, subject: &str) -> Option<String> {
        crate::execution_credentials::session_token(hub, subject)
    }

    fn sink_token(&self, app_id: &str, event_id: &str) -> Option<String> {
        let manager = self
            .app_handle
            .as_ref()?
            .try_state::<crate::state::TauriEventSinkManagerState>()?;
        let db = manager.0.try_lock().ok()?.db();
        match crate::event_sink::EventSinkManager::access_token(db, app_id, event_id) {
            Ok(token) => token,
            Err(error) => {
                tracing::debug!(%error, event_id, "Could not read a sink token for offline replay");
                None
            }
        }
    }
}

/// Production cloud access through the scope's weakly held lease, or a fresh one.
pub(crate) struct LeaseCloud;

impl LeaseCloud {
    async fn lease(host: &DesktopHost) -> Result<Option<Arc<RenewableSharedCredentials>>> {
        let scope = host.scope.upgrade().context(texts::FORGOTTEN)?;
        if let Some(live) = scope.lease() {
            return Ok(Some(live));
        }
        let (token, session) =
            match crate::execution_credentials::session_binding(&host.key.hub, &host.key.subject) {
                Some((session_id, webview, token)) => (token, Some((session_id, webview))),
                None => match host.token() {
                    Some(token) => (token, None),
                    None => return Ok(None),
                },
            };
        let prepared = crate::execution_credentials::prepare(
            &host.key.hub,
            &host.key.app_id,
            Some(&token),
            session.as_ref().map(|(session_id, _)| session_id.as_str()),
            session.as_ref().map(|(_, webview)| webview.as_str()),
        )
        .await?;
        let SharedCredentials::Renewable(live) = prepared else {
            return Ok(None);
        };
        scope.remember_lease(Arc::downgrade(&live));
        Ok(Some(live))
    }
}

#[async_trait::async_trait]
impl CloudAccess for LeaseCloud {
    async fn database(
        &self,
        host: &DesktopHost,
        purpose: StoragePurpose,
    ) -> Result<Option<Connection>> {
        let Some(live) = Self::lease(host).await? else {
            return Ok(None);
        };
        let builder = match purpose {
            StoragePurpose::User => {
                live.to_db_scoped(&host.key.subject, &host.key.app_id)
                    .await?
            }
            _ => live.to_db(&host.key.app_id).await?,
        };
        Ok(Some(
            builder
                .namespace_client_property("manifest_enabled", "false")
                .execute()
                .await?,
        ))
    }

    async fn objects(&self, host: &DesktopHost) -> Result<Option<Arc<dyn ObjectStore>>> {
        let Some(live) = Self::lease(host).await? else {
            return Ok(None);
        };
        Ok(Some(
            live.to_store_type_undecorated(StoreType::Content)
                .await?
                .as_generic(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportStatus {
    Idle,
    WaitingForSignIn,
    WaitingForConnection,
    HubError,
}

pub(crate) struct DesktopHost {
    scope: Weak<AppOfflineScope>,
    key: ScopeKey,
    installation_id: String,
    cache: LocalCacheView,
    index: Option<Arc<ObjectIndex>>,
    connectivity: Arc<ScopeConnectivity>,
    client: Result<reqwest::Client, String>,
    transport: Mutex<TransportStatus>,
    objects: Mutex<Option<(Instant, Arc<dyn ObjectStore>)>>,
}

impl DesktopHost {
    pub(crate) fn new(
        scope: Weak<AppOfflineScope>,
        key: &ScopeKey,
        installation_id: &str,
        cache: LocalCacheView,
        connectivity: Arc<ScopeConnectivity>,
    ) -> Self {
        Self {
            scope,
            key: key.clone(),
            installation_id: installation_id.to_owned(),
            index: cache.index.clone(),
            cache,
            connectivity,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REPLAY_TIMEOUT)
                .build()
                .map_err(|error| error.to_string()),
            transport: Mutex::new(TransportStatus::Idle),
            objects: Mutex::new(None),
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn transport(&self) -> TransportStatus {
        *Self::lock(&self.transport)
    }

    fn set_transport(&self, status: TransportStatus) {
        let previous = std::mem::replace(&mut *Self::lock(&self.transport), status);
        if previous != status
            && let Some(scope) = self.scope.upgrade()
        {
            scope.emit_status();
        }
    }

    /// The session token of the scope's account, else a bound sink PAT with a matching digest.
    pub(crate) fn token(&self) -> Option<String> {
        let scope = self.scope.upgrade()?;
        let tokens = &scope.services.tokens;
        if let Some(token) = tokens.session_token(&self.key.hub, &self.key.subject) {
            return Some(token);
        }
        scope.descriptor().sinks.iter().rev().find_map(|sink| {
            let token = tokens.sink_token(&self.key.app_id, &sink.event_id)?;
            (blake3::hash(token.as_bytes()).to_hex().as_str() == sink.token_digest).then_some(token)
        })
    }

    fn request_limit(&self) -> Option<usize> {
        self.scope
            .upgrade()?
            .descriptor()
            .hub_limits?
            .max_request_bytes
    }

    fn cloud(&self) -> Result<Arc<dyn super::host::CloudAccess>> {
        Ok(self
            .scope
            .upgrade()
            .context(texts::FORGOTTEN)?
            .services
            .cloud
            .clone())
    }

    async fn database(&self, purpose: StoragePurpose) -> Result<Connection> {
        self.cloud()?
            .database(self, purpose)
            .await?
            .context(texts::NO_TABLE_ACCESS)
    }

    /// The replay client: no redirects, bounded timeouts; never a default client.
    fn client(&self) -> Result<&reqwest::Client, ReplayError> {
        self.client.as_ref().map_err(|error| {
            failure(
                ReplayErrorKind::Unavailable,
                None,
                format!("The offline sync client could not be created: {error}"),
            )
        })
    }

    async fn send(
        &self,
        client: &reqwest::Client,
        request: &OfflineReplayRequest,
        token: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let url = format!(
            "{}{}",
            api_base(&self.key.hub),
            desktop_offline_replay_path(&self.key.app_id)
        );
        client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, authorization(token))
            .header(OFFLINE_INSTALLATION_HEADER, &self.installation_id)
            .header(OFFLINE_SUBJECT_HEADER, &self.key.subject)
            .json(request)
            .send()
            .await
    }

    fn local_file(&self, path: &ObjectPath) -> Option<std::path::PathBuf> {
        if !self.cache.roots.is_content(path.as_ref()) {
            return None;
        }
        self.cache.local_for(path).path_to_filesystem(path).ok()
    }
}

/// FlowPath's ETag sidecar of `path`: the final extension replaced by `.s3flowEtag`.
pub(crate) fn sidecar(path: &ObjectPath) -> Option<ObjectPath> {
    let key = path.as_ref();
    let base = match path.extension() {
        Some(extension) if !extension.is_empty() => {
            key.strip_suffix(&format!(".{extension}")).unwrap_or(key)
        }
        _ => key,
    };
    ObjectPath::parse(format!("{base}.s3flowEtag")).ok()
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Cache file {} has no directory", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.offline-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn remove_file(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
struct Envelope {
    error: EnvelopeBody,
}

#[derive(Deserialize)]
struct EnvelopeBody {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

fn envelope(body: &[u8]) -> (Option<String>, Option<String>) {
    match serde_json::from_slice::<Envelope>(body) {
        Ok(envelope) => (envelope.error.code, envelope.error.message),
        Err(_) => (None, None),
    }
}

fn server_message(status: u16, body: &[u8]) -> String {
    let message = envelope(body).1.unwrap_or_else(|| {
        let text = String::from_utf8_lossy(body).trim().to_owned();
        if text.is_empty() {
            format!("The hub refused this offline change (HTTP {status})")
        } else {
            text
        }
    });
    message.chars().take(MAX_MESSAGE_CHARS).collect()
}

fn failure(kind: ReplayErrorKind, code: Option<&str>, message: impl Into<String>) -> ReplayError {
    ReplayError {
        kind,
        code: code.map(str::to_owned),
        message: message.into(),
    }
}

/// §3.9: the hub's answer to one replay, and the transport state it implies.
pub(crate) fn map_answer(
    status: u16,
    body: &[u8],
    subject: &str,
    request_limit: Option<usize>,
) -> (Result<OfflineReplayResponse, ReplayError>, TransportStatus) {
    use ReplayErrorKind::{NotClaimed, Rejected, Unavailable};
    let unavailable = |state, message: String| (Err(failure(Unavailable, None, message)), state);
    match status {
        200 => match serde_json::from_slice::<OfflineReplayResponse>(body) {
            Ok(response) => (Ok(response), TransportStatus::Idle),
            Err(error) => unavailable(
                TransportStatus::HubError,
                format!("The hub answered the offline change with an unreadable response: {error}"),
            ),
        },
        400 | 422 => (
            Err(failure(
                NotClaimed,
                Some("invalid"),
                server_message(status, body),
            )),
            TransportStatus::Idle,
        ),
        401 => unavailable(
            TransportStatus::WaitingForSignIn,
            texts::sign_in_to_sync(subject),
        ),
        403 => (
            Err(failure(NotClaimed, Some("forbidden"), texts::FORBIDDEN)),
            TransportStatus::Idle,
        ),
        404 | 405 => (
            Err(failure(
                NotClaimed,
                Some("endpoint_missing"),
                texts::ENDPOINT_MISSING,
            )),
            TransportStatus::Idle,
        ),
        409 => match envelope(body).0.as_deref() {
            Some(OFFLINE_ERROR_SUBJECT_MISMATCH) => (
                Err(failure(
                    NotClaimed,
                    Some("subject_mismatch"),
                    texts::subject_mismatch(subject),
                )),
                TransportStatus::Idle,
            ),
            Some(OFFLINE_ERROR_DIGEST_REUSED) => (
                Err(failure(
                    Rejected,
                    Some("digest_reused"),
                    server_message(status, body),
                )),
                TransportStatus::Idle,
            ),
            _ => unavailable(TransportStatus::HubError, server_message(status, body)),
        },
        413 => (
            Err(failure(
                NotClaimed,
                Some("hub_limit"),
                texts::hub_limit(
                    &request_limit.map_or_else(|| "its limit".to_owned(), format_limit),
                ),
            )),
            TransportStatus::Idle,
        ),
        423 | 429 => unavailable(TransportStatus::HubError, server_message(status, body)),
        400..=499 => (
            Err(failure(Rejected, None, server_message(status, body))),
            TransportStatus::Idle,
        ),
        _ => unavailable(TransportStatus::HubError, server_message(status, body)),
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, reqwest::Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let room = MAX_RESPONSE_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if room <= chunk.len() {
            break;
        }
    }
    Ok(bytes)
}

#[async_trait::async_trait]
impl OfflineHost for DesktopHost {
    /// The desktop never quarantines; the hub enforces permissions at replay.
    fn authorization_current(&self) -> Result<(), AuthorizationError> {
        Ok(())
    }

    async fn replay(
        &self,
        request: &OfflineReplayRequest,
    ) -> Result<OfflineReplayResponse, ReplayError> {
        let client = self.client()?;
        let Some(token) = self.token() else {
            self.set_transport(TransportStatus::WaitingForSignIn);
            return Err(failure(
                ReplayErrorKind::Unavailable,
                None,
                texts::sign_in_to_sync(&self.key.subject),
            ));
        };
        let response = match self.send(client, request, &token).await {
            Ok(response) => response,
            Err(error) => {
                self.connectivity.observe(if error.is_timeout() {
                    Observation::TimedOut
                } else {
                    Observation::ConnectFailed
                });
                self.set_transport(TransportStatus::WaitingForConnection);
                return Err(failure(
                    ReplayErrorKind::Unavailable,
                    None,
                    format!("The hub could not be reached to sync an offline change: {error}"),
                ));
            }
        };
        self.connectivity.observe(Observation::Succeeded);
        let status = response.status().as_u16();
        let body = match read_limited(response).await {
            Ok(body) => body,
            Err(error) => {
                self.set_transport(TransportStatus::WaitingForConnection);
                return Err(failure(
                    ReplayErrorKind::Unavailable,
                    None,
                    format!("The hub's answer to an offline change was interrupted: {error}"),
                ));
            }
        };
        let (answer, transport) =
            map_answer(status, &body, &self.key.subject, self.request_limit());
        self.set_transport(transport);
        answer
    }

    fn location_prefix(&self, purpose: StoragePurpose) -> Option<String> {
        location_prefix(&self.key, purpose)
    }

    async fn remote_table(&self, table: &BufferedTable) -> Result<Option<Table>> {
        let connection = self.database(table.purpose).await?;
        optional_table(&connection, &table.table).await
    }

    async fn remote_objects(&self) -> Result<Arc<dyn ObjectStore>> {
        if let Some((created, store)) = Self::lock(&self.objects).as_ref()
            && created.elapsed() < OBJECT_STORE_REUSE
        {
            return Ok(store.clone());
        }
        let store = self
            .cloud()?
            .objects(self)
            .await?
            .context(texts::NO_DATA_ACCESS)?;
        *Self::lock(&self.objects) = Some((Instant::now(), store.clone()));
        Ok(store)
    }

    async fn remote_table_names(&self, database: &ObjectPath) -> Result<Vec<String>> {
        let purpose = if database.as_ref().starts_with("users/") {
            StoragePurpose::User
        } else {
            StoragePurpose::Storage
        };
        Ok(self
            .database(purpose)
            .await?
            .table_names()
            .execute()
            .await?)
    }

    fn file_acknowledged(
        &self,
        path: &ObjectPath,
        operation_id: &str,
        bytes: &[u8],
        revision: &OfflineExpected,
    ) -> Result<()> {
        let Some(file) = self.local_file(path) else {
            return Ok(());
        };
        write_file(&file, bytes)?;
        let (e_tag, version) = match revision {
            OfflineExpected::FileRevision { e_tag, version } => (e_tag.clone(), version.clone()),
            _ => (None, None),
        };
        if let Some(sidecar) = sidecar(path).and_then(|sidecar| self.local_file(&sidecar)) {
            match &e_tag {
                Some(tag) => write_file(&sidecar, tag.as_bytes())?,
                None => remove_file(&sidecar)?,
            }
        }
        if let Some(index) = &self.index {
            index.observe_present(&ObjectMeta {
                location: path.clone(),
                last_modified: chrono::Utc::now(),
                size: bytes.len() as u64,
                e_tag,
                version,
            })?;
            index.forget_queued(path, operation_id)?;
        }
        Ok(())
    }

    fn file_deleted(&self, path: &ObjectPath, operation_id: &str) -> Result<()> {
        if let Some(file) = self.local_file(path) {
            remove_file(&file)?;
        }
        if let Some(sidecar) = sidecar(path).and_then(|sidecar| self.local_file(&sidecar)) {
            remove_file(&sidecar)?;
        }
        if let Some(index) = &self.index {
            index.observe_absent(path)?;
            index.forget_queued(path, operation_id)?;
        }
        Ok(())
    }

    fn file_discarded(&self, path: &ObjectPath, operation_id: &str) -> Result<()> {
        let Some(index) = &self.index else {
            return Ok(());
        };
        if !index.forget_queued(path, operation_id)? {
            return Ok(());
        }
        if let Some(file) = self.local_file(path) {
            remove_file(&file)?;
        }
        if let Some(sidecar) = sidecar(path).and_then(|sidecar| self.local_file(&sidecar))
            && std::fs::read_to_string(&sidecar)
                .is_ok_and(|tag| tag == format!("offline-{operation_id}"))
        {
            remove_file(&sidecar)?;
        }
        Ok(())
    }

    fn queue_changed(&self) {
        if let Some(scope) = self.scope.upgrade() {
            scope.emit_status();
        }
    }

    fn mirror_changed(&self) {
        if let Some(scope) = self.scope.upgrade() {
            scope.emit_mirror();
        }
    }
}

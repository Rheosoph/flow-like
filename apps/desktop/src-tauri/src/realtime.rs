//! One native connection owner per login generation, with explicit webview subscriptions.

use crate::native_auth::SessionBinding;
use flow_like_types::realtime::{
    MAX_REALTIME_CONVERSATIONS, MAX_REALTIME_RESPONSE_BYTES, MAX_REALTIME_RUNS,
    MAX_REALTIME_TIMEOUT_MS, REALTIME_PROTOCOL_VERSION, RealtimePollRequest, RealtimePollResponse,
    RealtimeTransportPreference,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::ipc::Channel;
use tokio::sync::Notify;

mod pacing;
mod provider;
mod routing;
use pacing::{Pacing, Recovery};
use provider::{ProviderConnection, Reconnect};
use routing::{advance, aggregate, filter_response};

const MAX_SESSIONS: usize = 32;
const MAX_SUBSCRIBERS: usize = 128;
const AUTH_CHECK: Duration = Duration::from_millis(250);
const DELIVERY_RETRY: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionState {
    Connecting,
    Connected,
    Reconnecting,
    Unauthorized,
    Unsupported,
    SignedOut,
    Closed,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RealtimeMessage {
    State {
        state: ConnectionState,
    },
    Update {
        delivery_id: String,
        response: RealtimePollResponse,
    },
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Owner {
    window: String,
    webview: String,
    id: String,
}

struct Subscriber {
    request: RealtimePollRequest,
    transport: RealtimeTransportPreference,
    events: Channel<RealtimeMessage>,
    version: u64,
    pending: Option<PendingDelivery>,
}

struct PendingDelivery {
    id: String,
    response: RealtimePollResponse,
    sent: Instant,
}

struct SessionState {
    subscribers: HashMap<Owner, Subscriber>,
    connection: ConnectionState,
    revision: u64,
    rotation: usize,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            subscribers: HashMap::new(),
            connection: ConnectionState::Connecting,
            revision: 0,
            rotation: 0,
        }
    }
}

fn distinct_conversations<'a>(requests: impl Iterator<Item = &'a RealtimePollRequest>) -> usize {
    requests
        .flat_map(|request| &request.conversations)
        .map(|subscription| subscription.key())
        .collect::<HashSet<_>>()
        .len()
}

fn uses_configured_provider(state: &SessionState) -> bool {
    state.subscribers.values().any(|subscriber| {
        subscriber.transport == RealtimeTransportPreference::Configured
            && (subscriber.request.notifications.is_some()
                || !subscriber.request.runs.is_empty()
                || !subscriber.request.conversations.is_empty())
    })
}

struct Session {
    key: String,
    binding: SessionBinding,
    state: Mutex<SessionState>,
    changed: Notify,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> = LazyLock::new(Mutex::default);

struct WebviewLease {
    window: String,
    revoked: Arc<AtomicBool>,
}

static WEBVIEWS: LazyLock<Mutex<HashMap<String, WebviewLease>>> = LazyLock::new(Mutex::default);

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn profile_hub(app: &tauri::AppHandle, profile_id: &str) -> Result<String, String> {
    let settings = crate::state::TauriSettingsState::construct(app)
        .await
        .map_err(|_| "Settings are unavailable")?;
    let settings = settings.lock().await;
    let profile = settings
        .profiles
        .get(profile_id)
        .ok_or("The realtime profile no longer exists")?;
    flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
        .ok_or_else(|| "This profile has no valid hub origin".into())
}

#[tauri::command]
pub(crate) async fn realtime_subscribe(
    app_handle: tauri::AppHandle,
    webview: tauri::Webview,
    profile_id: String,
    expected_subject: String,
    expected_issuer: String,
    subscription_id: String,
    request: RealtimePollRequest,
    transport: Option<RealtimeTransportPreference>,
    events: Channel<RealtimeMessage>,
) -> Result<(), String> {
    request.validate().map_err(str::to_owned)?;
    if subscription_id.is_empty()
        || subscription_id.len() > 128
        || subscription_id.chars().any(char::is_control)
    {
        return Err("Invalid realtime subscription identifier".into());
    }
    // Capture before any await so a navigation can fence an unfinished registration.
    let webview_lease = {
        let mut views = lock(&WEBVIEWS);
        if !views.contains_key(webview.label()) && views.len() >= MAX_SUBSCRIBERS {
            return Err("Too many realtime webviews".into());
        }
        views
            .entry(webview.label().to_owned())
            .or_insert_with(|| WebviewLease {
                window: webview.window().label().to_owned(),
                revoked: Arc::default(),
            })
            .revoked
            .clone()
    };
    let hub = profile_hub(&app_handle, &profile_id).await?;
    let binding = SessionBinding::capture(&profile_id, &hub)
        .await?
        .ok_or("Sign in to subscribe to realtime updates")?;
    if !binding.is_current()
        || binding.subject != expected_subject
        || binding.issuer != expected_issuer
    {
        return Err("The authentication session changed".into());
    }
    let key = serde_json::to_string(&(
        &binding.profile_id,
        &binding.hub,
        &binding.issuer,
        &binding.subject,
        &binding.generation,
    ))
    .map_err(|_| "Cannot identify the realtime session")?;
    let owner = Owner {
        window: webview.window().label().to_owned(),
        webview: webview.label().to_owned(),
        id: subscription_id,
    };
    let (session, start) = {
        let mut sessions = lock(&SESSIONS);
        if webview_lease.load(Ordering::Acquire) || !binding.is_current() {
            return Err("The realtime subscriber navigated or signed out".into());
        }
        if !sessions.contains_key(&key) && sessions.len() >= MAX_SESSIONS {
            return Err("Too many realtime sessions".into());
        }
        let start = !sessions.contains_key(&key);
        let session = sessions.get(&key).cloned().unwrap_or_else(|| {
            Arc::new(Session {
                key,
                binding,
                state: Mutex::default(),
                changed: Notify::new(),
            })
        });
        {
            let mut state = lock(&session.state);
            if !state.subscribers.contains_key(&owner) && state.subscribers.len() >= MAX_SUBSCRIBERS
            {
                return Err("Too many realtime subscribers".into());
            }
            let combined = aggregate(
                state
                    .subscribers
                    .iter()
                    .filter(|(existing, _)| *existing != &owner)
                    .map(|(_, subscription)| &subscription.request)
                    .chain(std::iter::once(&request)),
                0,
            );
            if combined
                .as_ref()
                .is_some_and(|request| request.runs.len() > MAX_REALTIME_RUNS)
            {
                return Err("Too many simultaneous realtime run subscriptions".into());
            }
            if distinct_conversations(
                state
                    .subscribers
                    .iter()
                    .filter(|(existing, _)| *existing != &owner)
                    .map(|(_, subscription)| &subscription.request)
                    .chain(std::iter::once(&request)),
            ) > MAX_REALTIME_CONVERSATIONS
            {
                return Err("Too many simultaneous realtime conversation subscriptions".into());
            }
            events
                .send(RealtimeMessage::State {
                    state: state.connection,
                })
                .map_err(|_| "The realtime subscriber disconnected")?;
            state.revision = state.revision.wrapping_add(1);
            let version = state.revision;
            state.subscribers.insert(
                owner.clone(),
                Subscriber {
                    request,
                    transport: transport.unwrap_or_default(),
                    events,
                    version,
                    pending: None,
                },
            );
        }
        if start {
            sessions.insert(session.key.clone(), session.clone());
        }
        // An existing callback cannot remain attached to a previous account generation.
        for previous in sessions
            .values()
            .filter(|candidate| !Arc::ptr_eq(candidate, &session))
        {
            if lock(&previous.state).subscribers.remove(&owner).is_some() {
                previous.changed.notify_one();
            }
        }
        (session, start)
    };
    session.changed.notify_one();
    if start {
        tokio::spawn(run(session));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn realtime_unsubscribe(webview: tauri::Webview, subscription_id: String) {
    remove_subscribers(|owner| owner.webview == webview.label() && owner.id == subscription_id);
}

#[tauri::command]
pub(crate) fn realtime_acknowledge(
    webview: tauri::Webview,
    subscription_id: String,
    delivery_id: String,
) {
    if delivery_id.len() > 128 || subscription_id.len() > 128 {
        return;
    }
    let owner = Owner {
        window: webview.window().label().to_owned(),
        webview: webview.label().to_owned(),
        id: subscription_id,
    };
    for session in lock(&SESSIONS).values() {
        acknowledge(session, &owner, &delivery_id);
    }
}

fn acknowledge(session: &Session, owner: &Owner, delivery_id: &str) -> bool {
    let mut state = lock(&session.state);
    if !session.binding.is_current() {
        return false;
    }
    let Some(subscriber) = state.subscribers.get_mut(owner) else {
        return false;
    };
    if !subscriber
        .pending
        .as_ref()
        .is_some_and(|pending| pending.id == delivery_id)
    {
        return false;
    }
    let pending = subscriber
        .pending
        .take()
        .expect("delivery identity was checked");
    advance(&mut subscriber.request, &pending.response);
    session.changed.notify_one();
    true
}

pub(crate) fn close_webview(label: &str) {
    invalidate_webviews(|webview, _| webview == label);
    remove_subscribers(|owner| owner.webview == label);
}

pub(crate) fn close_window(label: &str) {
    invalidate_webviews(|_, lease| lease.window == label);
    remove_subscribers(|owner| owner.window == label);
}

fn invalidate_webviews(matches: impl Fn(&str, &WebviewLease) -> bool) {
    lock(&WEBVIEWS).retain(|label, lease| {
        if matches(label, lease) {
            lease.revoked.store(true, Ordering::Release);
            false
        } else {
            true
        }
    });
}

fn remove_subscribers(matches: impl Fn(&Owner) -> bool) {
    for session in lock(&SESSIONS).values() {
        let mut state = lock(&session.state);
        let before = state.subscribers.len();
        state.subscribers.retain(|owner, _| !matches(owner));
        if before != state.subscribers.len() {
            session.changed.notify_one();
        }
    }
}

fn set_state(session: &Session, connection: ConnectionState) {
    let mut state = lock(&session.state);
    if state.connection != connection {
        state.connection = connection;
        state.subscribers.retain(|_, subscriber| {
            subscriber
                .events
                .send(RealtimeMessage::State { state: connection })
                .is_ok()
        });
    }
}

fn deliver(
    session: &Session,
    versions: &HashMap<Owner, u64>,
    response: &RealtimePollResponse,
    request: &RealtimePollRequest,
) {
    let mut state = lock(&session.state);
    if !session.binding.is_current() {
        return;
    }
    state.subscribers.retain(|owner, subscriber| {
        if versions.get(owner) != Some(&subscriber.version) || subscriber.pending.is_some() {
            return true;
        }
        let response = filter_response(&subscriber.request, response, request);
        if response.notifications.is_none()
            && response.runs.is_empty()
            && response.errors.is_empty()
            && response.conversations.is_empty()
            && response.conversation_errors.is_empty()
        {
            return true;
        }
        let id = uuid::Uuid::new_v4().to_string();
        if subscriber
            .events
            .send(RealtimeMessage::Update {
                delivery_id: id.clone(),
                response: response.clone(),
            })
            .is_err()
        {
            return false;
        }
        subscriber.pending = Some(PendingDelivery {
            id,
            response,
            sent: Instant::now(),
        });
        true
    });
    state.rotation = state.rotation.wrapping_add(1);
}

fn resend_pending(session: &Session, now: Instant) {
    let mut state = lock(&session.state);
    if !session.binding.is_current() {
        return;
    }
    state.subscribers.retain(|_, subscriber| {
        let Some(pending) = &mut subscriber.pending else {
            return true;
        };
        if now.saturating_duration_since(pending.sent) < DELIVERY_RETRY {
            return true;
        }
        pending.sent = now;
        if subscriber
            .events
            .send(RealtimeMessage::Update {
                delivery_id: pending.id.clone(),
                response: pending.response.clone(),
            })
            .is_err()
        {
            session.changed.notify_one();
            return false;
        }
        true
    });
}

// Registry and session locks use this order everywhere, including subscribe and shutdown.
fn retire_if_empty(session: &Arc<Session>) -> bool {
    let mut sessions = lock(&SESSIONS);
    let state = lock(&session.state);
    if !state.subscribers.is_empty() {
        return false;
    }
    if sessions
        .get(&session.key)
        .is_some_and(|current| Arc::ptr_eq(current, session))
    {
        sessions.remove(&session.key);
    }
    true
}

fn finish(session: &Arc<Session>, connection: ConnectionState) {
    let mut sessions = lock(&SESSIONS);
    set_state(session, connection);
    if sessions
        .get(&session.key)
        .is_some_and(|current| Arc::ptr_eq(current, session))
    {
        sessions.remove(&session.key);
    }
}

/// Only `SignedOut` ends a session. `Unauthorized` and `Unsupported` are reported and retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PollFailure {
    Retry,
    Unauthorized,
    Unsupported,
    SignedOut,
}

fn status(status: u16) -> Result<(), PollFailure> {
    match status {
        200 => Ok(()),
        401 => Err(PollFailure::SignedOut),
        403 => Err(PollFailure::Unauthorized),
        404 | 405 | 501 => Err(PollFailure::Unsupported),
        _ => Err(PollFailure::Retry),
    }
}

fn endpoint(hub: &str, action: &str) -> String {
    let base = hub.trim_end_matches('/');
    if base.ends_with("/api/v1") {
        format!("{base}/realtime/{action}")
    } else {
        format!("{base}/api/v1/realtime/{action}")
    }
}

async fn bearer(session: &Session, rejected: Option<&str>) -> Result<String, PollFailure> {
    session.binding.access_token(rejected).await.map_err(|_| {
        if session.binding.is_current() {
            PollFailure::Retry
        } else {
            PollFailure::SignedOut
        }
    })
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &impl Serialize,
) -> Result<reqwest::Response, PollFailure> {
    client
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(|_| PollFailure::Retry)
}

async fn read(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, PollFailure> {
    status(response.status().as_u16())?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(PollFailure::Unsupported);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| PollFailure::Retry)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(PollFailure::Unsupported);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Authenticated realtime POST with one token renewal after a 401.
async fn post(
    client: &reqwest::Client,
    session: &Session,
    action: &str,
    body: &impl Serialize,
    limit: usize,
) -> Result<Vec<u8>, PollFailure> {
    let url = endpoint(&session.binding.hub, action);
    let mut token = bearer(session, None).await?;
    let mut response = send(client, &url, &token, body).await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        token = bearer(session, Some(&token)).await?;
        response = send(client, &url, &token, body).await?;
    }
    read(response, limit).await
}

fn decode<T: DeserializeOwned>(body: &[u8], version: impl Fn(&T) -> u8) -> Result<T, PollFailure> {
    serde_json::from_slice(body)
        .ok()
        .filter(|value| version(value) == REALTIME_PROTOCOL_VERSION)
        .ok_or(PollFailure::Unsupported)
}

async fn poll(
    client: &reqwest::Client,
    session: &Session,
    request: &RealtimePollRequest,
) -> Result<RealtimePollResponse, PollFailure> {
    let body = post(
        client,
        session,
        "poll",
        request,
        MAX_REALTIME_RESPONSE_BYTES,
    )
    .await?;
    decode(&body, |response: &RealtimePollResponse| {
        response.protocol_version
    })
}

async fn pause(session: &Session, delay: Duration) {
    let deadline = tokio::time::sleep(delay);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return,
            _ = session.changed.notified() => return,
            _ = tokio::time::sleep(AUTH_CHECK) => {
                if !session.binding.is_current() { return; }
                resend_pending(session, Instant::now());
            }
        }
    }
}

async fn provider_change(provider: &mut Option<ProviderConnection>) {
    match provider {
        Some(provider) => provider.changed().await,
        None => std::future::pending::<()>().await,
    }
}

async fn at(instant: Option<Instant>) {
    match instant {
        Some(instant) => tokio::time::sleep_until(instant.into()).await,
        None => std::future::pending::<()>().await,
    }
}

/// A blocked endpoint is not retried early for new subscribers, only for retirement.
async fn hold(session: &Session, delay: Duration) {
    let deadline = Instant::now() + delay;
    while session.binding.is_current() && !lock(&session.state).subscribers.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        pause(session, remaining).await;
    }
}

fn unserved(session: &Session, polled: &HashMap<Owner, u64>) -> bool {
    let state = lock(&session.state);
    state.subscribers.is_empty()
        || state.subscribers.iter().any(|(owner, subscriber)| {
            subscriber.pending.is_none() && polled.get(owner) != Some(&subscriber.version)
        })
}

struct Snapshot {
    request: Option<RealtimePollRequest>,
    versions: HashMap<Owner, u64>,
    configured: bool,
    subscribed: HashSet<String>,
}

fn snapshot(state: &SessionState) -> Snapshot {
    let ready = || {
        state
            .subscribers
            .iter()
            .filter(|(_, subscriber)| subscriber.pending.is_none())
    };
    Snapshot {
        request: aggregate(
            ready().map(|(_, subscriber)| &subscriber.request),
            state.rotation,
        ),
        versions: ready()
            .map(|(owner, subscriber)| (owner.clone(), subscriber.version))
            .collect(),
        configured: uses_configured_provider(state),
        subscribed: state
            .subscribers
            .values()
            .flat_map(|subscriber| &subscriber.request.runs)
            .map(|run| run.run_id.clone())
            .collect(),
    }
}

/// Polls once. A subscription change or provider change interrupts it, and so do a wakeup
/// or a due retry while it long-polls. `None` means interrupted.
async fn exchange(
    client: &reqwest::Client,
    session: &Session,
    request: &RealtimePollRequest,
    wake: &Notify,
    provider: &mut Option<ProviderConnection>,
    retry: Option<Instant>,
    woken: &mut bool,
) -> Option<Result<RealtimePollResponse, PollFailure>> {
    let pending = poll(client, session, request);
    tokio::pin!(pending);
    loop {
        tokio::select! {
            response = &mut pending => return Some(response),
            _ = session.changed.notified() => return None,
            _ = wake.notified(), if request.timeout_ms > 0 => {
                *woken = true;
                return None;
            }
            _ = provider_change(provider) => return None,
            _ = at(retry), if request.timeout_ms > 0 => return None,
            _ = tokio::time::sleep(AUTH_CHECK) => {
                if !session.binding.is_current() { return Some(Err(PollFailure::SignedOut)); }
                resend_pending(session, Instant::now());
            }
        }
    }
}

/// Waits for the next due poll. With `polled`, acknowledgements alone do not end the wait;
/// only subscribers that the last poll did not serve do. Returns whether a wakeup ended it.
async fn idle(
    session: &Session,
    until: Instant,
    wake: &Notify,
    provider: &mut Option<ProviderConnection>,
    polled: Option<&HashMap<Owner, u64>>,
) -> bool {
    let deadline = tokio::time::sleep_until(until.into());
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return false,
            _ = wake.notified() => return true,
            _ = provider_change(provider) => return false,
            _ = session.changed.notified() => {
                if polled.is_none_or(|polled| unserved(session, polled)) { return false; }
            }
            _ = tokio::time::sleep(AUTH_CHECK) => {
                if !session.binding.is_current() { return false; }
                resend_pending(session, Instant::now());
            }
        }
    }
}

async fn run(session: Arc<Session>) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            finish(&session, ConnectionState::Closed);
            return;
        }
    };
    let max_wait = Arc::new(AtomicU64::new(MAX_REALTIME_TIMEOUT_MS));
    let reconnect = Arc::new(Mutex::new(Reconnect::default()));
    let mut pacing = Pacing::default();
    let mut provider: Option<ProviderConnection> = None;
    let mut woken = false;
    loop {
        if !session.binding.is_current() {
            finish(&session, ConnectionState::SignedOut);
            return;
        }
        if retire_if_empty(&session) {
            return;
        }
        resend_pending(&session, Instant::now());
        let Snapshot {
            request,
            versions,
            configured,
            subscribed,
        } = snapshot(&lock(&session.state));
        // Every grant costs the API a provider credential; a blocked API gets none.
        if configured && !pacing.is_blocked() {
            provider.get_or_insert_with(|| {
                ProviderConnection::start(
                    client.clone(),
                    Arc::clone(&session),
                    Arc::clone(&max_wait),
                    Arc::clone(&reconnect),
                )
            });
        } else {
            provider = None;
        }
        let now = Instant::now();
        let Some(mut request) =
            request.and_then(|request| pacing.select(request, &subscribed, now))
        else {
            let delay = pacing.retry_at(now).map_or(DELIVERY_RETRY, |at| {
                at.saturating_duration_since(now).min(DELIVERY_RETRY)
            });
            pause(&session, delay).await;
            continue;
        };
        // Healthy providers wake an immediate replay. A periodic reconciliation
        // covers missed hints; disconnected providers poll as the API allows.
        let live = provider
            .as_ref()
            .is_some_and(ProviderConnection::is_healthy);
        let max_wait_ms = max_wait.load(Ordering::Relaxed);
        request.timeout_ms = pacing::timeout_ms(live, max_wait_ms);
        pacing.begin(std::mem::take(&mut woken), now);
        let retry = pacing.retry_at(now);
        let wake = provider
            .as_ref()
            .map(|provider| Arc::clone(&provider.wake))
            .unwrap_or_default();
        let started = Instant::now();
        let outcome = exchange(
            &client,
            &session,
            &request,
            &wake,
            &mut provider,
            retry,
            &mut woken,
        )
        .await;
        match outcome {
            None => continue,
            Some(Ok(response)) => {
                if !session.binding.is_current() {
                    continue;
                }
                let now = Instant::now();
                pacing.succeeded(&request, &response, live, now);
                set_state(&session, ConnectionState::Connected);
                deliver(&session, &versions, &response, &request);
                if let Some(until) = pacing.due(
                    &request,
                    &response,
                    live,
                    max_wait_ms,
                    started.elapsed(),
                    now,
                ) {
                    let polled = (!live).then_some(&versions);
                    woken = idle(&session, until, &wake, &mut provider, polled).await;
                }
                // A proxy that returns empty responses immediately must not create a hot loop.
                if started.elapsed() < Duration::from_millis(100) {
                    pause(&session, Duration::from_millis(100)).await;
                }
            }
            Some(Err(failure)) => {
                if !recover(&session, pacing.failed(failure), &mut provider).await {
                    return;
                }
            }
        }
    }
}

/// Reports a failed poll and waits out its backoff. Returns `false` once the session ended.
async fn recover(
    session: &Arc<Session>,
    recovery: Recovery,
    provider: &mut Option<ProviderConnection>,
) -> bool {
    match recovery {
        Recovery::SignedOut => {
            finish(session, ConnectionState::SignedOut);
            return false;
        }
        Recovery::Blocked(state, delay) => {
            *provider = None;
            set_state(session, state);
            hold(session, delay).await;
        }
        Recovery::Retry(delay) => {
            set_state(session, ConnectionState::Reconnecting);
            pause(session, delay).await;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::realtime::{
        RealtimeNotificationOverview, RealtimeNotificationSubscription, RealtimeNotificationUpdate,
        RealtimeRunEvent, RealtimeRunSubscription, RealtimeRunUpdate,
    };
    use tauri::ipc::InvokeResponseBody;

    fn session() -> Arc<Session> {
        Arc::new(Session {
            key: uuid::Uuid::new_v4().to_string(),
            binding: SessionBinding::for_test("profile", "https://hub.test", "issuer", "user"),
            state: Mutex::default(),
            changed: Notify::new(),
        })
    }

    fn subscriber(
        id: &str,
        version: u64,
        received: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> (Owner, Subscriber) {
        (
            Owner {
                window: id.into(),
                webview: id.into(),
                id: "inbox".into(),
            },
            Subscriber {
                version,
                pending: None,
                transport: RealtimeTransportPreference::default(),
                request: RealtimePollRequest {
                    notifications: Some(RealtimeNotificationSubscription::default()),
                    ..Default::default()
                },
                events: Channel::new(move |body| {
                    if let InvokeResponseBody::Json(body) = body {
                        lock(&received).push(serde_json::from_str(&body).unwrap());
                    }
                    Ok(())
                }),
            },
        )
    }

    fn deliver_test(
        session: &Session,
        versions: &HashMap<Owner, u64>,
        response: &RealtimePollResponse,
    ) {
        let request = aggregate(
            lock(&session.state)
                .subscribers
                .values()
                .map(|subscriber| &subscriber.request),
            0,
        )
        .unwrap_or_default();
        deliver(session, versions, response, &request);
    }

    fn run_response() -> RealtimePollResponse {
        RealtimePollResponse {
            runs: vec![RealtimeRunUpdate {
                run_id: "run".into(),
                revision: "revision".into(),
                status: "RUNNING".into(),
                progress: 1,
                current_step: None,
                error: None,
                events: vec![RealtimeRunEvent {
                    event_id: None,
                    sequence: 0,
                    event_type: "chunk".into(),
                    payload: serde_json::json!("answer"),
                    created_at: "2026-11-01T00:00:00Z".into(),
                }],
                after_sequence: 0,
                has_more: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_window_acknowledges_before_its_cursor_advances_and_retries_keep_the_same_delivery() {
        let session = session();
        let received = Arc::default();
        let (owner, mut subscriber) = subscriber("window", 1, Arc::clone(&received));
        subscriber.request.runs.push(RealtimeRunSubscription {
            poll_token: None,
            run_id: "run".into(),
            after_sequence: -1,
            revision: None,
        });
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), subscriber);
        let versions = HashMap::from([(owner.clone(), 1)]);
        deliver_test(&session, &versions, &run_response());
        let id = {
            let state = lock(&session.state);
            let subscriber = &state.subscribers[&owner];
            assert_eq!(subscriber.request.runs[0].after_sequence, -1);
            assert!(
                aggregate(
                    state
                        .subscribers
                        .values()
                        .filter(|subscriber| subscriber.pending.is_none())
                        .map(|subscriber| &subscriber.request),
                    0
                )
                .is_none()
            );
            subscriber.pending.as_ref().unwrap().id.clone()
        };
        deliver_test(&session, &versions, &run_response());
        assert_eq!(lock(&received).len(), 1);
        resend_pending(&session, Instant::now() + DELIVERY_RETRY);
        let messages = lock(&received);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], messages[1]);
        drop(messages);
        assert!(acknowledge(&session, &owner, &id));
        assert_eq!(
            lock(&session.state).subscribers[&owner].request.runs[0].after_sequence,
            0
        );
        assert!(!acknowledge(&session, &owner, &id));
    }

    #[test]
    fn another_window_or_a_replaced_delivery_cannot_acknowledge_the_current_batch() {
        let session = session();
        let (owner, initial) = subscriber("window", 1, Arc::default());
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), initial);
        let response = RealtimePollResponse {
            notifications: Some(RealtimeNotificationUpdate {
                revision: "one".into(),
                overview: RealtimeNotificationOverview::default(),
            }),
            ..Default::default()
        };
        deliver_test(&session, &HashMap::from([(owner.clone(), 1)]), &response);
        let first_id = lock(&session.state).subscribers[&owner]
            .pending
            .as_ref()
            .unwrap()
            .id
            .clone();
        let other = Owner {
            window: "other".into(),
            webview: "other".into(),
            id: owner.id.clone(),
        };
        assert!(!acknowledge(&session, &other, &first_id));
        let (_, replacement) = subscriber("window", 2, Arc::default());
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), replacement);
        deliver_test(&session, &HashMap::from([(owner.clone(), 2)]), &response);
        assert!(!acknowledge(&session, &owner, &first_id));
        assert!(
            lock(&session.state).subscribers[&owner]
                .request
                .notifications
                .as_ref()
                .unwrap()
                .revision
                .is_none()
        );
        let current_id = lock(&session.state).subscribers[&owner]
            .pending
            .as_ref()
            .unwrap()
            .id
            .clone();
        assert!(acknowledge(&session, &owner, &current_id));
    }

    #[test]
    fn http_subscribers_and_failed_run_subscribers_do_not_open_a_provider() {
        let mut state = SessionState::default();
        let (http_owner, mut http) = subscriber("http", 1, Arc::default());
        http.transport = RealtimeTransportPreference::Http;
        state.subscribers.insert(http_owner, http);
        assert!(!uses_configured_provider(&state));
        let (owner, configured) = subscriber("configured", 2, Arc::default());
        state.subscribers.insert(owner.clone(), configured);
        assert!(uses_configured_provider(&state));
        state.subscribers.get_mut(&owner).unwrap().request = RealtimePollRequest::default();
        assert!(!uses_configured_provider(&state));
    }

    #[test]
    fn a_new_subscriber_prevents_retirement_of_an_idle_task() {
        let session = session();
        lock(&SESSIONS).insert(session.key.clone(), session.clone());
        let (owner, subscriber) = subscriber("window", 1, Arc::default());
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), subscriber);
        assert!(!retire_if_empty(&session));
        assert!(Arc::ptr_eq(
            lock(&SESSIONS).get(&session.key).unwrap(),
            &session
        ));
        lock(&session.state).subscribers.remove(&owner);
        assert!(retire_if_empty(&session));
        assert!(!lock(&SESSIONS).contains_key(&session.key));
    }

    #[test]
    fn a_retiring_task_cannot_remove_its_replacement() {
        let previous = session();
        let replacement = Arc::new(Session {
            key: previous.key.clone(),
            binding: previous.binding.clone(),
            state: Mutex::default(),
            changed: Notify::new(),
        });
        lock(&SESSIONS).insert(previous.key.clone(), replacement.clone());
        finish(&previous, ConnectionState::SignedOut);
        assert!(Arc::ptr_eq(
            lock(&SESSIONS).get(&previous.key).unwrap(),
            &replacement
        ));
        assert!(retire_if_empty(&replacement));
    }

    #[test]
    fn only_a_rejected_renewal_signs_out_while_blocked_statuses_stay_retryable() {
        assert_eq!(status(200), Ok(()));
        assert_eq!(status(401), Err(PollFailure::SignedOut));
        assert_eq!(status(403), Err(PollFailure::Unauthorized));
        for code in [404, 405, 501] {
            assert_eq!(status(code), Err(PollFailure::Unsupported), "{code}");
        }
        for code in [400, 408, 429, 500, 502, 503, 504] {
            assert_eq!(status(code), Err(PollFailure::Retry), "{code}");
        }
    }

    #[test]
    fn unparseable_and_mismatched_responses_are_protocol_failures() {
        let version = |response: &RealtimePollResponse| response.protocol_version;
        for body in [
            &b"<html>maintenance</html>"[..],
            br#"{"protocol_version":2,"runs":[],"errors":[]}"#,
            br#"{"runs":[]}"#,
        ] {
            assert_eq!(decode(body, version), Err(PollFailure::Unsupported));
        }
        assert_eq!(
            decode(br#"{"protocol_version":1}"#, version),
            Ok(RealtimePollResponse::default())
        );
    }

    #[test]
    fn interval_waits_end_for_new_or_resumed_subscribers_but_not_acknowledgements() {
        let session = session();
        let (owner, served) = subscriber("served", 1, Arc::default());
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), served);
        let polled = HashMap::from([(owner.clone(), 1)]);
        assert!(!unserved(&session, &polled));

        let (resumed, mut waiting) = subscriber("resumed", 2, Arc::default());
        waiting.pending = Some(PendingDelivery {
            id: "delivery".into(),
            response: RealtimePollResponse::default(),
            sent: Instant::now(),
        });
        lock(&session.state)
            .subscribers
            .insert(resumed.clone(), waiting);
        assert!(!unserved(&session, &polled));
        lock(&session.state)
            .subscribers
            .get_mut(&resumed)
            .unwrap()
            .pending = None;
        assert!(unserved(&session, &polled));

        lock(&session.state).subscribers.remove(&resumed);
        let (replaced, replacement) = subscriber("served", 3, Arc::default());
        lock(&session.state)
            .subscribers
            .insert(replaced, replacement);
        assert!(unserved(&session, &polled));
        lock(&session.state).subscribers.clear();
        assert!(unserved(&session, &polled));
    }

    #[test]
    fn navigation_fences_a_registration_still_waiting_for_authentication() {
        let label = uuid::Uuid::new_v4().to_string();
        let revoked = Arc::new(AtomicBool::new(false));
        lock(&WEBVIEWS).insert(
            label.clone(),
            WebviewLease {
                window: label.clone(),
                revoked: revoked.clone(),
            },
        );
        close_webview(&label);
        assert!(revoked.load(Ordering::Acquire));
        assert!(!lock(&WEBVIEWS).contains_key(&label));
    }

    #[test]
    fn replacing_a_subscription_fences_the_previous_inflight_response() {
        let session = session();
        let received = Arc::default();
        let (owner, subscriber) = subscriber("window", 2, Arc::clone(&received));
        lock(&session.state)
            .subscribers
            .insert(owner.clone(), subscriber);
        let response = RealtimePollResponse {
            notifications: Some(RealtimeNotificationUpdate {
                revision: "new".into(),
                overview: RealtimeNotificationOverview::default(),
            }),
            ..Default::default()
        };
        deliver_test(&session, &HashMap::from([(owner.clone(), 1)]), &response);
        assert!(lock(&received).is_empty());
        deliver_test(&session, &HashMap::from([(owner, 2)]), &response);
        assert_eq!(lock(&received).len(), 1);
    }
}

//! One native connection owner per login generation, with explicit webview subscriptions.

use crate::native_auth::SessionBinding;
use flow_like_types::realtime::{
    MAX_REALTIME_RESPONSE_BYTES, MAX_REALTIME_RUNS, MAX_REALTIME_TIMEOUT_MS,
    REALTIME_PROTOCOL_VERSION, RealtimeNotificationSubscription, RealtimePollRequest,
    RealtimePollResponse, RealtimeRunSubscription,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tauri::ipc::Channel;
use tokio::sync::Notify;

const MAX_SESSIONS: usize = 32;
const MAX_SUBSCRIBERS: usize = 128;
const AUTH_CHECK: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionState {
    Connecting,
    Connected,
    Reconnecting,
    Unsupported,
    SignedOut,
    Closed,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RealtimeMessage {
    State { state: ConnectionState },
    Update { response: RealtimePollResponse },
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Owner {
    window: String,
    webview: String,
    id: String,
}

struct Subscriber {
    request: RealtimePollRequest,
    events: Channel<RealtimeMessage>,
    version: u64,
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

struct Session {
    key: String,
    binding: SessionBinding,
    state: Mutex<SessionState>,
    changed: Notify,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(Mutex::default);

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    subscription_id: String,
    request: RealtimePollRequest,
    events: Channel<RealtimeMessage>,
) -> Result<(), String> {
    request.validate().map_err(str::to_owned)?;
    if subscription_id.is_empty()
        || subscription_id.len() > 128
        || subscription_id.chars().any(char::is_control)
    {
        return Err("Invalid realtime subscription identifier".into());
    }
    let hub = profile_hub(&app_handle, &profile_id).await?;
    let binding = SessionBinding::capture(&profile_id, &hub)
        .await?
        .ok_or("Sign in to subscribe to realtime updates")?;
    if !binding.is_current() {
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
        if !sessions.contains_key(&key) && sessions.len() >= MAX_SESSIONS {
            return Err("Too many realtime sessions".into());
        }
        let start = !sessions.contains_key(&key);
        let session = sessions
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(Session {
                    key,
                    binding,
                    state: Mutex::default(),
                    changed: Notify::new(),
                })
            })
            .clone();
        {
            let mut state = lock(&session.state);
            if !state.subscribers.contains_key(&owner)
                && state.subscribers.len() >= MAX_SUBSCRIBERS
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
            if combined.as_ref().is_some_and(|request| request.runs.len() > MAX_REALTIME_RUNS) {
                return Err("Too many simultaneous realtime run subscriptions".into());
            }
            events
                .send(RealtimeMessage::State { state: state.connection })
                .map_err(|_| "The realtime subscriber disconnected")?;
            state.revision = state.revision.wrapping_add(1);
            let version = state.revision;
            state.subscribers.insert(owner.clone(), Subscriber { request, events, version });
        }
        // An existing callback cannot remain attached to a previous account generation.
        for previous in sessions.values().filter(|candidate| !Arc::ptr_eq(candidate, &session)) {
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

pub(crate) fn close_webview(label: &str) {
    remove_subscribers(|owner| owner.webview == label);
}

pub(crate) fn close_window(label: &str) {
    remove_subscribers(|owner| owner.window == label);
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

fn aggregate<'a>(
    requests: impl Iterator<Item = &'a RealtimePollRequest>,
    rotation: usize,
) -> Option<RealtimePollRequest> {
    let mut notifications: Option<RealtimeNotificationSubscription> = None;
    let mut runs = BTreeMap::<String, RealtimeRunSubscription>::new();
    for request in requests {
        if let Some(subscription) = &request.notifications {
            match &mut notifications {
                Some(current) if current.revision != subscription.revision => current.revision = None,
                None => notifications = Some(subscription.clone()),
                _ => {}
            }
        }
        for subscription in &request.runs {
            match runs.get_mut(&subscription.run_id) {
                Some(current) => {
                    current.after_sequence = current.after_sequence.min(subscription.after_sequence);
                    if current.revision != subscription.revision {
                        current.revision = None;
                    }
                }
                None => { runs.insert(subscription.run_id.clone(), subscription.clone()); }
            }
        }
    }
    let mut runs: Vec<_> = runs.into_values().collect();
    if !runs.is_empty() {
        let offset = rotation % runs.len();
        runs.rotate_left(offset);
    }
    (notifications.is_some() || !runs.is_empty()).then_some(RealtimePollRequest {
        notifications,
        runs,
        timeout_ms: MAX_REALTIME_TIMEOUT_MS,
    })
}

fn filter_response(request: &RealtimePollRequest, response: &RealtimePollResponse) -> RealtimePollResponse {
    let notifications = response.notifications.as_ref().filter(|update| {
        request.notifications.as_ref().is_some_and(|subscription| {
            subscription.revision.as_deref() != Some(update.revision.as_str())
        })
    }).cloned();
    let runs = response.runs.iter().filter_map(|update| {
        let subscription = request.runs.iter().find(|run| run.run_id == update.run_id)?;
        let mut update = update.clone();
        update.events.retain(|event| event.sequence > subscription.after_sequence);
        update.after_sequence = update.after_sequence.max(subscription.after_sequence);
        if update.events.is_empty() && subscription.revision.as_deref() == Some(update.revision.as_str()) {
            return None;
        }
        Some(update)
    }).collect();
    let errors = response.errors.iter().filter(|error| {
        request.runs.iter().any(|run| run.run_id == error.run_id)
    }).cloned().collect();
    RealtimePollResponse { protocol_version: response.protocol_version, notifications, runs, errors }
}

fn advance(request: &mut RealtimePollRequest, response: &RealtimePollResponse) {
    if let (Some(subscription), Some(update)) = (&mut request.notifications, &response.notifications) {
        subscription.revision = Some(update.revision.clone());
    }
    for update in &response.runs {
        if let Some(subscription) = request.runs.iter_mut().find(|run| run.run_id == update.run_id) {
            subscription.after_sequence = subscription.after_sequence.max(update.after_sequence);
            subscription.revision = Some(update.revision.clone());
        }
    }
    request.runs.retain(|run| !response.errors.iter().any(|error| error.run_id == run.run_id));
}

fn set_state(session: &Session, connection: ConnectionState) {
    let mut state = lock(&session.state);
    if state.connection != connection {
        state.connection = connection;
        state.subscribers.retain(|_, subscriber| {
            subscriber.events.send(RealtimeMessage::State { state: connection }).is_ok()
        });
    }
}

fn deliver(session: &Session, versions: &HashMap<Owner, u64>, response: &RealtimePollResponse) {
    let mut state = lock(&session.state);
    if !session.binding.is_current() { return; }
    state.subscribers.retain(|owner, subscriber| {
        if versions.get(owner) != Some(&subscriber.version) { return true; }
        let response = filter_response(&subscriber.request, response);
        if response.notifications.is_none() && response.runs.is_empty() && response.errors.is_empty() {
            return true;
        }
        if subscriber.events.send(RealtimeMessage::Update { response: response.clone() }).is_err() {
            return false;
        }
        advance(&mut subscriber.request, &response);
        true
    });
    state.rotation = state.rotation.wrapping_add(1);
}

// Registry and session locks use this order everywhere, including subscribe and shutdown.
fn retire_if_empty(session: &Arc<Session>) -> bool {
    let mut sessions = lock(&SESSIONS);
    let state = lock(&session.state);
    if !state.subscribers.is_empty() { return false; }
    if sessions.get(&session.key).is_some_and(|current| Arc::ptr_eq(current, session)) {
        sessions.remove(&session.key);
    }
    true
}

fn finish(session: &Arc<Session>, connection: ConnectionState) {
    let mut sessions = lock(&SESSIONS);
    set_state(session, connection);
    if sessions.get(&session.key).is_some_and(|current| Arc::ptr_eq(current, session)) {
        sessions.remove(&session.key);
    }
}

enum PollFailure { Retry, Unsupported, SignedOut }

async fn poll(client: &reqwest::Client, session: &Session, request: &RealtimePollRequest) -> Result<RealtimePollResponse, PollFailure> {
    let base = session.binding.hub.trim_end_matches('/');
    let url = if base.ends_with("/api/v1") { format!("{base}/realtime/poll") } else { format!("{base}/api/v1/realtime/poll") };
    let mut token = session.binding.access_token(None).await.map_err(|_| {
        if session.binding.is_current() { PollFailure::Retry } else { PollFailure::SignedOut }
    })?;
    let mut response = client.post(&url).bearer_auth(&token).json(request).send().await.map_err(|_| PollFailure::Retry)?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        token = session.binding.access_token(Some(&token)).await.map_err(|_| PollFailure::SignedOut)?;
        response = client.post(&url).bearer_auth(token).json(request).send().await.map_err(|_| PollFailure::Retry)?;
    }
    match response.status().as_u16() {
        401 | 403 => return Err(PollFailure::SignedOut),
        404 | 405 | 501 => return Err(PollFailure::Unsupported),
        200 => {}
        _ => return Err(PollFailure::Retry),
    }
    if response.content_length().is_some_and(|length| length > MAX_REALTIME_RESPONSE_BYTES as u64) {
        return Err(PollFailure::Unsupported);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| PollFailure::Retry)? {
        if body.len().saturating_add(chunk.len()) > MAX_REALTIME_RESPONSE_BYTES {
            return Err(PollFailure::Unsupported);
        }
        body.extend_from_slice(&chunk);
    }
    let response: RealtimePollResponse = serde_json::from_slice(&body).map_err(|_| PollFailure::Unsupported)?;
    if response.protocol_version != REALTIME_PROTOCOL_VERSION { return Err(PollFailure::Unsupported); }
    Ok(response)
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
        Err(_) => { finish(&session, ConnectionState::Closed); return; }
    };
    let mut failures = 0_u32;
    loop {
        if !session.binding.is_current() {
            finish(&session, ConnectionState::SignedOut);
            return;
        }
        if retire_if_empty(&session) { return; }
        let (request, versions) = {
            let state = lock(&session.state);
            (aggregate(state.subscribers.values().map(|subscriber| &subscriber.request), state.rotation),
             state.subscribers.iter().map(|(owner, subscriber)| (owner.clone(), subscriber.version)).collect())
        };
        let Some(request) = request else {
            pause(&session, Duration::from_secs(30)).await;
            continue;
        };
        let started = Instant::now();
        let pending = poll(&client, &session, &request);
        tokio::pin!(pending);
        let outcome = loop {
            tokio::select! {
                response = &mut pending => break Some(response),
                _ = session.changed.notified() => break None,
                _ = tokio::time::sleep(AUTH_CHECK) => {
                    if !session.binding.is_current() { break Some(Err(PollFailure::SignedOut)); }
                }
            }
        };
        match outcome {
            None => continue,
            Some(Ok(response)) => {
                if !session.binding.is_current() { continue; }
                failures = 0;
                set_state(&session, ConnectionState::Connected);
                deliver(&session, &versions, &response);
                // A proxy that returns empty responses immediately must not create a hot loop.
                if started.elapsed() < Duration::from_millis(100) {
                    pause(&session, Duration::from_millis(100)).await;
                }
            }
            Some(Err(PollFailure::SignedOut)) => { finish(&session, ConnectionState::SignedOut); return; }
            Some(Err(PollFailure::Unsupported)) => { finish(&session, ConnectionState::Unsupported); return; }
            Some(Err(PollFailure::Retry)) => {
                set_state(&session, ConnectionState::Reconnecting);
                failures = failures.saturating_add(1);
                let delay = Duration::from_millis(500_u64.saturating_mul(1 << failures.min(6)).min(30_000));
                pause(&session, delay).await;
            }
        }
    }
}

//! Browser-protocol listeners stay outside the page's JavaScript environment.
#[cfg(feature = "execute")]
use super::driver::PageContext;
#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_browser::{Page, connection::MethodMatch, fetch::Observation, types::SessionId};
#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Value, json::json};
#[cfg(feature = "execute")]
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

#[cfg(feature = "execute")]
const MAX_BUFFERED_EVENTS: usize = 10_000;

#[cfg(feature = "execute")]
pub(crate) struct NetworkState {
    pub pending: HashSet<String>,
    pub url_pattern: String,
    pub requests: VecDeque<super::observe::NetworkRequest>,
    pub request_ids: VecDeque<String>,
    started: HashMap<String, f64>,
    pub console_logs: VecDeque<super::observe::ConsoleMessage>,
    pub failure: Option<String>,
    pub last_activity: std::time::Instant,
}

#[cfg(feature = "execute")]
impl Default for NetworkState {
    fn default() -> Self {
        Self {
            pending: HashSet::new(),
            url_pattern: String::new(),
            requests: VecDeque::new(),
            request_ids: VecDeque::new(),
            started: HashMap::new(),
            console_logs: VecDeque::new(),
            failure: None,
            last_activity: std::time::Instant::now(),
        }
    }
}

#[cfg(feature = "execute")]
impl NetworkState {
    fn push_console(&mut self, message: super::observe::ConsoleMessage) {
        if self.console_logs.len() >= MAX_BUFFERED_EVENTS {
            self.console_logs.pop_front();
        }
        self.console_logs.push_back(message);
    }

    /// Applies one console or `Network.*` event; other events are ignored.
    fn record(&mut self, method: &str, params: &Value) {
        if let Some(message) = console_message(method, params) {
            self.push_console(message);
            return;
        }
        let request_id = params["requestId"].as_str().unwrap_or_default();
        match method {
            "Network.requestWillBeSent" => self.request_sent(request_id, params),
            "Network.responseReceived" => self.response_received(request_id, &params["response"]),
            "Network.loadingFinished" | "Network.loadingFailed" => {
                self.loading_ended(request_id, params)
            }
            _ => {}
        }
    }

    fn request_sent(&mut self, request_id: &str, params: &Value) {
        self.last_activity = std::time::Instant::now();
        self.pending.insert(request_id.to_string());
        if let Some(timestamp) = params["timestamp"].as_f64() {
            self.started.insert(request_id.to_owned(), timestamp);
        }
        let request = &params["request"];
        let url = request["url"].as_str().unwrap_or_default();
        if !self.url_pattern.is_empty() && !url.contains(&self.url_pattern) {
            return;
        }
        if self.requests.len() >= MAX_BUFFERED_EVENTS {
            self.requests.pop_front();
            self.request_ids.pop_front();
        }
        self.requests.push_back(super::observe::NetworkRequest {
            url: url.into(),
            method: request["method"].as_str().unwrap_or_default().into(),
            status: None,
            status_text: None,
            request_headers: request.get("headers").map(Value::to_string),
            response_headers: None,
            duration_ms: None,
            size_bytes: None,
            resource_type: params["type"].as_str().map(str::to_owned),
        });
        self.request_ids.push_back(request_id.to_owned());
    }

    fn response_received(&mut self, request_id: &str, response: &Value) {
        if let Some(request) = self.recorded(request_id) {
            request.status = response["status"].as_f64().map(|status| status as i32);
            request.status_text = response["statusText"].as_str().map(str::to_owned);
            request.response_headers = response.get("headers").map(Value::to_string);
        }
    }

    fn loading_ended(&mut self, request_id: &str, params: &Value) {
        self.last_activity = std::time::Instant::now();
        self.pending.remove(request_id);
        let started = self.started.remove(request_id);
        if let Some(request) = self.recorded(request_id) {
            request.size_bytes = params["encodedDataLength"]
                .as_f64()
                .map(|length| length as i64);
            request.duration_ms = started
                .zip(params["timestamp"].as_f64())
                .map(|(start, end)| ((end - start).max(0.0) * 1000.0) as i64);
        }
    }

    fn recorded(&mut self, request_id: &str) -> Option<&mut super::observe::NetworkRequest> {
        let index = self.request_ids.iter().rposition(|id| id == request_id)?;
        self.requests.get_mut(index)
    }
}

/// Console entries from `Runtime.consoleAPICalled`, uncaught `Runtime.exceptionThrown`
/// errors, and browser-originated `Log.entryAdded` messages (failed loads, CSP, …).
#[cfg(any(feature = "execute", test))]
pub(crate) fn console_message(
    method: &str,
    params: &Value,
) -> Option<super::observe::ConsoleMessage> {
    match method {
        "Runtime.consoleAPICalled" => {
            let text = params["args"]
                .as_array()
                .map(|args| {
                    args.iter()
                        .map(|arg| {
                            arg.get("value")
                                .map(|value| {
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| value.to_string())
                                })
                                .or_else(|| arg["description"].as_str().map(str::to_owned))
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let frame = &params["stackTrace"]["callFrames"][0];
            Some(super::observe::ConsoleMessage {
                level: console_level(params["type"].as_str().unwrap_or("log")),
                text,
                timestamp: params["timestamp"].as_f64().unwrap_or_default() as i64,
                source: frame["url"].as_str().map(str::to_owned),
                line_number: frame["lineNumber"].as_i64().map(|line| line as i32),
            })
        }
        "Runtime.exceptionThrown" => {
            let details = &params["exceptionDetails"];
            let frame = &details["stackTrace"]["callFrames"][0];
            let text = details["exception"]["description"]
                .as_str()
                .or_else(|| details["exception"]["value"].as_str())
                .or_else(|| details["text"].as_str())
                .unwrap_or("Uncaught exception")
                .to_owned();
            Some(super::observe::ConsoleMessage {
                level: "error".into(),
                text,
                timestamp: params["timestamp"].as_f64().unwrap_or_default() as i64,
                source: details["url"]
                    .as_str()
                    .filter(|url| !url.is_empty())
                    .or_else(|| frame["url"].as_str())
                    .map(str::to_owned),
                line_number: details["lineNumber"]
                    .as_i64()
                    .or_else(|| frame["lineNumber"].as_i64())
                    .map(|line| line as i32),
            })
        }
        "Log.entryAdded" => {
            let entry = &params["entry"];
            Some(super::observe::ConsoleMessage {
                level: console_level(entry["level"].as_str().unwrap_or("info")),
                text: entry["text"].as_str().unwrap_or_default().to_owned(),
                timestamp: entry["timestamp"].as_f64().unwrap_or_default() as i64,
                source: entry["url"].as_str().map(str::to_owned),
                line_number: entry["lineNumber"].as_i64().map(|line| line as i32),
            })
        }
        _ => None,
    }
}

#[cfg(any(feature = "execute", test))]
fn console_level(level: &str) -> String {
    match level {
        "warning" => "warn",
        "verbose" => "debug",
        level => level,
    }
    .to_string()
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn has_been_idle(
    pending: usize,
    last_activity: std::time::Instant,
    now: std::time::Instant,
    required: std::time::Duration,
) -> bool {
    pending == 0
        && now
            .checked_duration_since(last_activity)
            .is_some_and(|elapsed| elapsed >= required)
}

/// `observed` is the DevTools session a CDP observer follows; the basic-auth entry has none.
#[cfg(feature = "execute")]
pub(crate) struct Listener {
    pub network: Arc<tokio::sync::Mutex<NetworkState>>,
    abort: Option<tokio::task::AbortHandle>,
    observed: Option<SessionId>,
}
#[cfg(feature = "execute")]
impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(abort) = &self.abort {
            abort.abort();
        }
    }
}
#[cfg(feature = "execute")]
impl Cacheable for Listener {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
pub(crate) fn listener_key(session: &AutomationSession, kind: &str) -> String {
    format!(
        "automation:{kind}:{}:{}",
        session.session_ref,
        session
            .current_window_handle
            .as_deref()
            .unwrap_or("current")
    )
}

#[cfg(feature = "execute")]
pub(crate) async fn network_state(
    context: &ExecutionContext,
    session: &AutomationSession,
) -> flow_like_types::Result<Arc<tokio::sync::Mutex<NetworkState>>> {
    let cache = context.cache.read().await;
    let entry = cache
        .get(&listener_key(session, "network"))
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Start Network Observer before triggering requests or navigation"
            )
        })?;
    let listener = entry
        .as_any()
        .downcast_ref::<Listener>()
        .ok_or_else(|| flow_like_types::anyhow!("Network observer state is invalid"))?;
    Ok(listener.network.clone())
}

#[cfg(feature = "execute")]
const OBSERVER_ENABLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(feature = "execute")]
const OBSERVED_PAGE_CHECK: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(feature = "execute")]
const OBSERVED_PAGE_GONE: &str =
    "The observed browser tab was closed or replaced; start the console or network observer again";

#[cfg(feature = "execute")]
const OBSERVED_METHODS: [MethodMatch; 4] = [
    MethodMatch::Prefix("Network."),
    MethodMatch::Exact("Runtime.consoleAPICalled"),
    MethodMatch::Exact("Runtime.exceptionThrown"),
    MethodMatch::Exact("Log.entryAdded"),
];

/// Follows console and network events of the current page. The route is registered before
/// `Network.enable`/`Log.enable`, so no event of the enabled domains is missed, and the
/// console backlog replays messages of the current document logged before the observer.
#[cfg(feature = "execute")]
pub(crate) async fn start_observer(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    ctx: &PageContext,
) -> flow_like_types::Result<()> {
    if observer_follows(context, session, &ctx.page).await {
        return Ok(());
    }
    let observation = observe_page(&ctx.page, ctx.browser.settings().page_load_timeout()).await?;
    let network = Arc::new(tokio::sync::Mutex::new(NetworkState::default()));
    let task = tokio::spawn(follow_observation(
        observation,
        ctx.page.clone(),
        network.clone(),
        context.get_cancellation_token(),
    ));
    context.cache.write().await.insert(
        listener_key(session, "network"),
        Arc::new(Listener {
            network,
            abort: Some(task.abort_handle()),
            observed: page_session(&ctx.page),
        }),
    );
    Ok(())
}

/// The observer of the current tab has not failed and still follows the session the tab
/// is attached through; a prerender activation or re-attach keeps the tab id but not the
/// session.
#[cfg(feature = "execute")]
async fn observer_follows(
    context: &ExecutionContext,
    session: &AutomationSession,
    page: &Page,
) -> bool {
    let followed = {
        let cache = context.cache.read().await;
        cache
            .get(&listener_key(session, "network"))
            .and_then(|entry| entry.as_any().downcast_ref::<Listener>())
            .map(|listener| (listener.observed.clone(), listener.network.clone()))
    };
    let Some((observed, network)) = followed else {
        return false;
    };
    !page.is_closed()
        && observed.is_some()
        && observed == page_session(page)
        && network.lock().await.failure.is_none()
}

#[cfg(feature = "execute")]
fn page_session(page: &Page) -> Option<SessionId> {
    page.session().id().cloned()
}

/// `Page::cdp` first waits for a pending navigation, up to the page load timeout (past it,
/// the load is stopped and the renderer timeout is reported), so each domain gets that wait
/// plus the confirmation window.
#[cfg(feature = "execute")]
async fn observe_page(
    page: &Page,
    page_load_timeout: std::time::Duration,
) -> flow_like_types::Result<Observation> {
    let observation = page.observe(OBSERVED_METHODS.to_vec());
    let budget = page_load_timeout + OBSERVER_ENABLE_TIMEOUT;
    for method in ["Network.enable", "Log.enable"] {
        tokio::time::timeout(budget, super::cdp::send_to(page, method, json!({})))
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "Browser did not confirm {method} within {} seconds (page load wait included)",
                    budget.as_secs()
                )
            })??;
    }
    Ok(observation)
}

/// Replays the backlog, then the route; an event in both (hooks run before routes) is
/// applied once. Ends with a failure when the run is cancelled, the connection closes or
/// the observed tab is closed or replaced, so waits never read a frozen state as idle.
#[cfg(feature = "execute")]
async fn follow_observation(
    observation: Observation,
    page: Page,
    network: Arc<tokio::sync::Mutex<NetworkState>>,
    cancellation: Option<CancellationToken>,
) {
    let Observation { backlog, mut route } = observation;
    let replayed: HashSet<u64> = backlog.iter().map(|event| event.seq).collect();
    for event in &backlog {
        network.lock().await.record(&event.method, &event.params);
    }
    let mut page_check = tokio::time::interval(OBSERVED_PAGE_CHECK);
    page_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let failure = loop {
        let event = tokio::select! {
            biased;
            _ = cancelled(cancellation.as_ref()) => break "Browser observer was cancelled",
            next = route.recv() => match next {
                Some(event) => event,
                None => break "Browser protocol connection closed",
            },
            _ = page_check.tick() => {
                if page.is_closed() {
                    break OBSERVED_PAGE_GONE;
                }
                continue;
            }
        };
        if !replayed.contains(&event.seq) {
            network.lock().await.record(&event.method, &event.params);
        }
    };
    network.lock().await.failure = Some(failure.into());
}

#[cfg(feature = "execute")]
async fn cancelled(token: Option<&CancellationToken>) {
    match token {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

/// Answers HTTP basic-auth challenges of `origin` on the current page and its frames.
#[cfg(feature = "execute")]
pub(crate) async fn start_basic_auth(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    ctx: &PageContext,
    origin: String,
    username: String,
    password: String,
) -> flow_like_types::Result<()> {
    let decider_origin = origin.clone();
    ctx.page
        .enable_basic_auth(
            &origin,
            Arc::new(move |challenge, first_attempt| {
                auth_response(
                    &decider_origin,
                    &username,
                    &password,
                    challenge,
                    first_attempt,
                )
            }),
        )
        .await?;
    context.cache.write().await.insert(
        listener_key(session, "auth"),
        Arc::new(Listener {
            network: Arc::default(),
            abort: None,
            observed: None,
        }),
    );
    Ok(())
}

pub(crate) fn normalized_origin(value: &str) -> flow_like_types::Result<String> {
    let url = flow_like_types::reqwest::Url::parse(value)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(flow_like_types::anyhow!(
            "Authentication origin must be an HTTP(S) origin without credentials"
        ));
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(feature = "execute")]
pub(crate) async fn clear_listeners(context: &ExecutionContext, session: &AutomationSession) {
    let auth_prefix = format!("automation:auth:{}:", session.session_ref);
    let network_prefix = format!("automation:network:{}:", session.session_ref);
    context
        .cache
        .write()
        .await
        .retain(|key, _| !key.starts_with(&auth_prefix) && !key.starts_with(&network_prefix));
}

fn auth_response(
    origin: &str,
    username: &str,
    password: &str,
    challenge: &Value,
    first_attempt: bool,
) -> Value {
    let matches_origin = challenge["origin"]
        .as_str()
        .and_then(|v| normalized_origin(v).ok())
        .as_deref()
        == Some(origin);
    if first_attempt
        && matches_origin
        && challenge["source"] == "Server"
        && challenge["scheme"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("basic"))
    {
        json!({"response":"ProvideCredentials","username":username,"password":password})
    } else {
        json!({"response":"CancelAuth"})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_request_bursts_restart_the_continuous_idle_window() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let quiet = Duration::from_millis(100);
        assert!(!has_been_idle(
            0,
            start + Duration::from_millis(90),
            start + Duration::from_millis(100),
            quiet
        ));
        assert!(has_been_idle(
            0,
            start + Duration::from_millis(90),
            start + Duration::from_millis(190),
            quiet
        ));
        assert!(!has_been_idle(
            1,
            start,
            start + Duration::from_millis(500),
            quiet
        ));
    }
    #[test]
    fn uncaught_exceptions_and_browser_log_entries_are_console_errors() {
        let exception = console_message(
            "Runtime.exceptionThrown",
            &json!({
                "timestamp": 1700000000000.0,
                "exceptionDetails": {
                    "text": "Uncaught",
                    "lineNumber": 41,
                    "url": "https://example.com/app.js",
                    "exception": {"description": "TypeError: x is undefined\n    at app.js:42:7"}
                }
            }),
        )
        .unwrap();
        assert_eq!(exception.level, "error");
        assert!(exception.text.starts_with("TypeError: x is undefined"));
        assert!(exception.text.contains("app.js:42:7"));
        assert_eq!(
            exception.source.as_deref(),
            Some("https://example.com/app.js")
        );
        assert_eq!(exception.line_number, Some(41));
        assert_eq!(exception.timestamp, 1700000000000);

        let thrown_string = console_message(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails": {"text": "Uncaught", "exception": {"type": "string", "value": "boom"}, "stackTrace": {"callFrames": [{"url": "inline.js", "lineNumber": 3}]}}}),
        )
        .unwrap();
        assert_eq!(thrown_string.text, "boom");
        assert_eq!(thrown_string.source.as_deref(), Some("inline.js"));

        let failed_load = console_message(
            "Log.entryAdded",
            &json!({"entry": {"source": "network", "level": "error", "text": "Failed to load resource: 404", "timestamp": 5.0, "url": "https://example.com/missing.png"}}),
        )
        .unwrap();
        assert_eq!(failed_load.level, "error");
        assert_eq!(failed_load.text, "Failed to load resource: 404");

        let warning = console_message(
            "Runtime.consoleAPICalled",
            &json!({"type": "warning", "args": [{"type": "string", "value": "careful"}, {"type": "number", "value": 3}]}),
        )
        .unwrap();
        assert_eq!(warning.level, "warn");
        assert_eq!(warning.text, "careful 3");
        assert!(console_message("Network.requestWillBeSent", &json!({})).is_none());
    }

    #[test]
    fn credentials_are_limited_to_exact_server_origin() {
        let origin = normalized_origin("https://example.com/account").unwrap();
        let challenge = json!({"origin":"https://example.com","source":"Server","scheme":"basic"});
        assert_eq!(
            auth_response(&origin, "user", "secret", &challenge, true)["response"],
            "ProvideCredentials"
        );
        for hostile in [
            json!({"origin":"https://example.com.evil.test","source":"Server","scheme":"basic"}),
            json!({"origin":"http://example.com","source":"Server","scheme":"basic"}),
            json!({"origin":"https://example.com:8443","source":"Server","scheme":"basic"}),
            json!({"origin":"https://example.com","source":"Proxy","scheme":"basic"}),
            json!({"origin":"https://example.com","source":"Server","scheme":"digest"}),
        ] {
            let response = auth_response(&origin, "user", "secret", &hostile, true);
            assert_eq!(response, json!({"response":"CancelAuth"}));
            assert!(!response.to_string().contains("secret"));
        }
        assert_eq!(
            auth_response(&origin, "user", "secret", &challenge, false),
            json!({"response":"CancelAuth"})
        );
        assert!(normalized_origin("file:///tmp/a").is_err());
        assert!(normalized_origin("https://user:secret@example.com").is_err());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn network_events_fill_requests_with_status_size_and_duration() {
        let mut state = NetworkState {
            url_pattern: "/api/".into(),
            ..NetworkState::default()
        };
        let request = |id: &str, url: &str| json!({"requestId": id, "timestamp": 10.0, "type": "Fetch", "request": {"url": url, "method": "POST", "headers": {"a": "b"}}});
        state.record(
            "Network.requestWillBeSent",
            &request("1", "https://example.com/api/items"),
        );
        state.record(
            "Network.requestWillBeSent",
            &request("2", "https://example.com/logo.png"),
        );
        state.record(
            "Network.responseReceived",
            &json!({"requestId": "1", "response": {"status": 201.0, "statusText": "Created", "headers": {}}}),
        );
        state.record(
            "Network.loadingFinished",
            &json!({"requestId": "1", "timestamp": 10.25, "encodedDataLength": 512.0}),
        );
        state.record(
            "Runtime.consoleAPICalled",
            &json!({"type": "log", "args": [{"type": "string", "value": "ready"}]}),
        );
        state.record("Page.frameNavigated", &json!({}));

        assert_eq!(state.requests.len(), 1);
        let item = &state.requests[0];
        assert_eq!((item.method.as_str(), item.status), ("POST", Some(201)));
        assert_eq!((item.size_bytes, item.duration_ms), (Some(512), Some(250)));
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.console_logs.len(), 1);
    }

    #[cfg(feature = "execute")]
    mod observation {
        use super::super::*;
        use flow_like_browser::connection::{RouteFilter, RouteReceiver, SessionScope};
        use flow_like_browser::event_log::Event;
        use flow_like_browser::testing::{PageHarness, default_auto_reply};
        use std::time::Duration;

        const SESSION: &str = "S1";
        const MAIN_FRAME: &str = "T1";
        const WAIT: Duration = Duration::from_secs(5);
        const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);

        async fn start_loading(harness: &PageHarness) {
            let cursor = harness.connection.events().cursor();
            harness.emit("Page.frameStartedLoading", json!({"frameId": MAIN_FRAME}));
            let deadline = tokio::time::Instant::now() + WAIT;
            let processed = harness
                .connection
                .events()
                .wait_for(cursor, deadline, |event| {
                    &*event.method == "Page.frameStartedLoading"
                })
                .await
                .expect("the event log stays open");
            assert!(
                processed.is_some(),
                "Page.frameStartedLoading was never processed"
            );
        }

        fn sent_count(harness: &PageHarness, method: &str) -> usize {
            harness
                .control
                .commands_seen()
                .iter()
                .filter(|command| {
                    command.method == method && command.session.as_deref() == Some(SESSION)
                })
                .count()
        }

        fn console(text: &str) -> Value {
            json!({"type": "log", "args": [{"type": "string", "value": text}]})
        }

        fn request(id: &str) -> Value {
            json!({"requestId": id, "request": {"url": "https://example.com/", "method": "GET"}})
        }

        fn page_route(harness: &PageHarness) -> RouteReceiver {
            harness.connection.route(RouteFilter {
                sessions: SessionScope::Exactly(SessionId::from(SESSION)),
                methods: OBSERVED_METHODS.to_vec(),
            })
        }

        async fn received(route: &mut RouteReceiver, count: usize) -> Vec<Event> {
            let mut events = Vec::new();
            while events.len() < count {
                let event = tokio::time::timeout(WAIT, route.recv())
                    .await
                    .expect("the scripted event reaches the route")
                    .expect("the route stays open");
                events.push(event);
            }
            events
        }

        fn console_texts(state: &NetworkState) -> Vec<String> {
            state
                .console_logs
                .iter()
                .map(|message| message.text.clone())
                .collect()
        }

        fn follow(
            harness: &PageHarness,
            observation: Observation,
            cancellation: Option<CancellationToken>,
        ) -> (
            Arc<tokio::sync::Mutex<NetworkState>>,
            tokio::task::JoinHandle<()>,
        ) {
            let network = Arc::new(tokio::sync::Mutex::new(NetworkState::default()));
            let follower = tokio::spawn(follow_observation(
                observation,
                harness.page.clone(),
                network.clone(),
                cancellation,
            ));
            (network, follower)
        }

        async fn stopped(follower: tokio::task::JoinHandle<()>) {
            tokio::time::timeout(WAIT, follower)
                .await
                .expect("the observer stops")
                .expect("the observer task does not panic");
        }

        #[tokio::test]
        async fn the_backlog_is_applied_once_and_no_routed_event_is_lost() {
            let harness = PageHarness::new().await;
            let route = page_route(&harness);
            let mut witness = page_route(&harness);
            harness.emit("Network.requestWillBeSent", request("7"));
            harness.emit("Runtime.consoleAPICalled", console("before observer"));
            harness.emit("Runtime.consoleAPICalled", console("after observer"));
            harness.emit_on("S-other", "Runtime.consoleAPICalled", console("other page"));
            let events = received(&mut witness, 3).await;
            assert!(events[0].seq < events[1].seq);
            let observation = Observation {
                backlog: vec![events[1].clone()],
                route,
            };
            let (network, follower) = follow(&harness, observation, None);
            harness.control.close("scripted browser went away");
            stopped(follower).await;

            let state = network.lock().await;
            assert_eq!(
                console_texts(&state),
                vec!["before observer".to_owned(), "after observer".to_owned()]
            );
            assert_eq!(state.requests.len(), 1);
            assert_eq!(
                state.failure.as_deref(),
                Some("Browser protocol connection closed")
            );
        }

        #[tokio::test]
        async fn cancelling_the_run_stops_the_observer() {
            let harness = PageHarness::new().await;
            let observation = Observation {
                backlog: Vec::new(),
                route: page_route(&harness),
            };
            let token = CancellationToken::new();
            let (network, follower) = follow(&harness, observation, Some(token.clone()));
            token.cancel();
            stopped(follower).await;
            assert_eq!(
                network.lock().await.failure.as_deref(),
                Some("Browser observer was cancelled")
            );
        }

        #[tokio::test]
        async fn a_closed_or_replaced_tab_ends_the_observer() {
            let harness = PageHarness::new().await;
            let observation = harness.page.observe(OBSERVED_METHODS.to_vec());
            let (network, follower) = follow(&harness, observation, None);
            harness.control.emit(
                "Target.detachedFromTarget",
                None,
                json!({"sessionId": SESSION, "targetId": harness.page.target_id().as_str()}),
            );
            stopped(follower).await;
            assert!(harness.page.is_closed());
            assert_eq!(
                network.lock().await.failure.as_deref(),
                Some(OBSERVED_PAGE_GONE)
            );
        }

        #[tokio::test]
        async fn events_before_network_enable_is_answered_reach_the_observer() {
            let mut harness = PageHarness::new().await;
            harness.control.set_auto_reply(|command| {
                (command.method != "Network.enable")
                    .then(|| default_auto_reply(command))
                    .flatten()
            });
            let page = harness.page.clone();
            let observing =
                tokio::spawn(async move { observe_page(&page, PAGE_LOAD_TIMEOUT).await });
            let enable = harness
                .control
                .wait_for("Network.enable", Some(SESSION))
                .await;
            harness.emit("Network.requestWillBeSent", request("1"));
            harness.control.reply(&enable, json!({}));
            let mut observation = tokio::time::timeout(WAIT, observing)
                .await
                .expect("the observer confirms both domains")
                .expect("the observing task does not panic")
                .expect("Network.enable and Log.enable succeed");
            let events = received(&mut observation.route, 1).await;
            assert_eq!(&*events[0].method, "Network.requestWillBeSent");
        }

        #[tokio::test]
        async fn a_page_load_longer_than_the_enable_timeout_delays_the_observer_without_failing_it()
        {
            let harness = PageHarness::new().await;
            harness.set_page_load_timeout(PAGE_LOAD_TIMEOUT);
            start_loading(&harness).await;
            let page = harness.page.clone();
            let observing =
                tokio::spawn(async move { observe_page(&page, PAGE_LOAD_TIMEOUT).await });
            tokio::time::sleep(OBSERVER_ENABLE_TIMEOUT + Duration::from_millis(500)).await;
            if observing.is_finished() {
                let outcome = observing.await.expect("the observing task does not panic");
                panic!(
                    "the observer ended while the page was still loading: {:?}",
                    outcome.err()
                );
            }
            assert_eq!(
                sent_count(&harness, "Network.enable"),
                0,
                "Network.enable was sent while the page was loading"
            );
            harness.emit("Page.frameStoppedLoading", json!({"frameId": MAIN_FRAME}));
            tokio::time::timeout(WAIT, observing)
                .await
                .expect("the observer starts once the load stopped")
                .expect("the observing task does not panic")
                .expect("Network.enable and Log.enable succeed after the load");
            assert_eq!(sent_count(&harness, "Network.enable"), 1);
            assert_eq!(sent_count(&harness, "Log.enable"), 1);
        }

        #[tokio::test]
        async fn a_load_past_the_page_load_timeout_reports_the_renderer_timeout() {
            let harness = PageHarness::new().await;
            let page_load_timeout = Duration::from_millis(300);
            harness.set_page_load_timeout(page_load_timeout);
            start_loading(&harness).await;
            let page = harness.page.clone();
            let observing =
                tokio::spawn(async move { observe_page(&page, page_load_timeout).await });
            harness
                .sent("Page.stopLoading", |command| {
                    command.method == "Page.stopLoading"
                })
                .await;
            harness.emit("Page.frameStoppedLoading", json!({"frameId": MAIN_FRAME}));
            let error = tokio::time::timeout(WAIT, observing)
                .await
                .expect("the observer gives up at the page load timeout")
                .expect("the observing task does not panic")
                .err()
                .expect("a load past the page load timeout fails the observer");
            assert!(
                matches!(
                    super::super::super::driver::browser_error(&error),
                    Some(flow_like_browser::BrowserError::RendererTimeout { .. })
                ),
                "{error:?}"
            );
            assert_eq!(sent_count(&harness, "Network.enable"), 0);
        }
    }
}

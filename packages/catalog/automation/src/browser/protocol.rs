//! Browser-protocol listeners stay outside the page's JavaScript environment.
#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
use flow_like_types::{Value, json::json};
#[cfg(feature = "execute")]
use futures::{SinkExt, StreamExt};
#[cfg(feature = "execute")]
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

#[cfg(feature = "execute")]
const MAX_BUFFERED_EVENTS: usize = 10_000;
#[cfg(feature = "execute")]
const DEBUGGER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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

#[cfg(feature = "execute")]
async fn send_confirmed<S>(
    socket: &mut S,
    id: u64,
    method: &str,
    params: Value,
    queued_events: &mut VecDeque<Value>,
) -> flow_like_types::Result<()>
where
    S: futures::Sink<
            tokio_tungstenite::tungstenite::Message,
            Error = tokio_tungstenite::tungstenite::Error,
        > + futures::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    use tokio_tungstenite::tungstenite::Message;
    socket
        .send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await?;
    tokio::time::timeout(DEBUGGER_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            if let Message::Text(text) = message? {
                let value: Value = flow_like_types::json::from_str(&text)?;
                if value["id"] == id {
                    if let Some(error) = value.get("error") {
                        return Err(flow_like_types::anyhow!(
                            "Browser rejected {method}: {error}"
                        ));
                    }
                    return Ok(());
                }
                queued_events.push_back(value);
            }
        }
        Err(flow_like_types::anyhow!(
            "Browser protocol connection closed while waiting for {method}"
        ))
    })
    .await
    .map_err(|_| {
        flow_like_types::anyhow!(
            "Browser did not confirm {method} within {} seconds",
            DEBUGGER_TIMEOUT.as_secs()
        )
    })?
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

#[cfg(feature = "execute")]
pub(crate) struct Listener {
    pub network: Arc<tokio::sync::Mutex<NetworkState>>,
    abort: tokio::task::AbortHandle,
}
#[cfg(feature = "execute")]
impl Drop for Listener {
    fn drop(&mut self) {
        self.abort.abort();
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
struct DebuggerEndpoint(String);
#[cfg(feature = "execute")]
impl Cacheable for DebuggerEndpoint {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
#[derive(Clone)]
struct DriverClient {
    client: flow_like_types::reqwest::Client,
    debugger_address: Arc<std::sync::Mutex<Option<String>>>,
}

#[cfg(feature = "execute")]
#[flow_like_types::async_trait]
impl thirtyfour::session::http::HttpClient for DriverClient {
    async fn send(
        &self,
        request: http::Request<thirtyfour::session::http::Body<'_>>,
    ) -> thirtyfour::error::WebDriverResult<http::Response<flow_like_types::Bytes>> {
        let create_session =
            request.method() == http::Method::POST && request.uri().path().ends_with("/session");
        let response = thirtyfour::session::http::HttpClient::send(&self.client, request).await?;
        if create_session && response.status().is_success() {
            if let Ok(value) = flow_like_types::json::from_slice::<Value>(response.body()) {
                let capabilities = &value["value"]["capabilities"];
                let address = capabilities["goog:chromeOptions"]["debuggerAddress"]
                    .as_str()
                    .or_else(|| capabilities["ms:edgeOptions"]["debuggerAddress"].as_str());
                if let Some(address) = address {
                    *self
                        .debugger_address
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(address.to_owned());
                }
            }
        }
        Ok(response)
    }
    async fn new(&self) -> Arc<dyn thirtyfour::session::http::HttpClient> {
        Arc::new(self.clone())
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn connect_webdriver(
    url: &str,
    mut capabilities: thirtyfour::Capabilities,
) -> flow_like_types::Result<(thirtyfour::WebDriver, Option<String>)> {
    capabilities.insert("unhandledPromptBehavior".into(), json!("ignore"));
    let address = Arc::new(std::sync::Mutex::new(None));
    let config = thirtyfour::common::config::WebDriverConfig::default();
    let client = DriverClient {
        client: flow_like_types::reqwest::Client::builder()
            .timeout(config.reqwest_timeout)
            .build()?,
        debugger_address: address.clone(),
    };
    let driver =
        thirtyfour::WebDriver::new_with_config_and_client(url, capabilities, config, client)
            .await?;
    let address = address
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    Ok((driver, address))
}

#[cfg(feature = "execute")]
pub(crate) async fn remember_debugger_address(
    context: &ExecutionContext,
    session: &AutomationSession,
    address: &str,
) {
    context.cache.write().await.insert(
        format!("automation:debugger:{}", session.session_ref),
        Arc::new(DebuggerEndpoint(address.to_owned())),
    );
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
pub(crate) async fn start_listener(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    driver: &thirtyfour::WebDriver,
    debugger_address: &str,
    auth: Option<(String, String, String)>,
) -> flow_like_types::Result<()> {
    if auth.is_none() {
        if let Ok(state) = network_state(context, session).await {
            if state.lock().await.failure.is_none() {
                return Ok(());
            }
        }
    }
    use tokio_tungstenite::tungstenite::Message;
    let saved_address = {
        let cache = context.cache.read().await;
        cache
            .get(&format!("automation:debugger:{}", session.session_ref))
            .and_then(|entry| entry.as_any().downcast_ref::<DebuggerEndpoint>())
            .map(|entry| entry.0.clone())
    };
    let debugger_address = if debugger_address.is_empty() {
        saved_address.as_deref().ok_or_else(|| flow_like_types::anyhow!("This browser did not expose a debugger address. Supply a Chrome or Edge debugging endpoint."))?
    } else {
        debugger_address
    };
    let address = if debugger_address.contains("://") {
        debugger_address.to_owned()
    } else {
        format!("http://{debugger_address}")
    };
    let endpoint = flow_like_types::reqwest::Url::parse(&address)?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(flow_like_types::anyhow!(
            "Debugger address must use HTTP or HTTPS"
        ));
    }
    let info = super::cdp::cdp(driver, "Target.getTargetInfo", json!({})).await?;
    let target_id = info["targetInfo"]["targetId"]
        .as_str()
        .ok_or_else(|| flow_like_types::anyhow!("Browser did not return a target ID"))?;
    let list_url = endpoint.join("/json/list")?;
    let targets: Value = flow_like_types::reqwest::Client::builder()
        .timeout(DEBUGGER_TIMEOUT)
        .build()?
        .get(list_url.clone())
        .send()
        .await
        .map_err(|e| {
            flow_like_types::anyhow!("Failed to list debugger targets at {list_url}: {e}")
        })?
        .error_for_status()?
        .json()
        .await?;
    let websocket = targets
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target["id"].as_str() == Some(target_id))
        })
        .and_then(|target| target["webSocketDebuggerUrl"].as_str())
        .ok_or_else(|| {
            flow_like_types::anyhow!("Current tab is not exposed by this debugger endpoint")
        })?;
    let (mut socket, _) = tokio::time::timeout(
        DEBUGGER_TIMEOUT,
        tokio_tungstenite::connect_async(websocket),
    )
    .await
    .map_err(|_| {
        flow_like_types::anyhow!("Timed out connecting to the browser debugger at {websocket}")
    })??;
    let (kind, method, params) = if let Some((origin, _, _)) = &auth {
        (
            "auth",
            "Fetch.enable",
            json!({"handleAuthRequests":true,"patterns":[{"urlPattern":format!("{origin}/*")}]}),
        )
    } else {
        ("network", "Network.enable", json!({}))
    };
    // Confirm protocol support before reporting that the node succeeded.
    let mut queued_events = VecDeque::new();
    send_confirmed(&mut socket, 1, method, params, &mut queued_events).await?;
    if auth.is_none() {
        send_confirmed(
            &mut socket,
            2,
            "Runtime.enable",
            json!({}),
            &mut queued_events,
        )
        .await?;
        send_confirmed(&mut socket, 3, "Log.enable", json!({}), &mut queued_events).await?;
    }
    let network = Arc::new(tokio::sync::Mutex::new(NetworkState::default()));
    let task_state = network.clone();
    let cancellation = context.get_cancellation_token();
    let task = tokio::spawn(async move {
        let mut id = 4u64;
        let mut authenticated = HashSet::new();
        loop {
            let event = if let Some(event) = queued_events.pop_front() {
                event
            } else {
                let next = tokio::select! {
                    biased;
                _ = async { if let Some(token) = &cancellation { token.cancelled().await } else { std::future::pending::<()>().await } } => {
                    task_state.lock().await.failure = Some("Browser observer was cancelled".into());
                    break;
                },
                    next = socket.next() => next,
                };
                let text = match next {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        task_state.lock().await.failure =
                            Some("Browser protocol connection closed".into());
                        break;
                    }
                    Some(Ok(_)) => continue,
                };
                let Ok(event) = flow_like_types::json::from_str::<Value>(&text) else {
                    continue;
                };
                event
            };
            let params = &event["params"];
            let request_id = params["requestId"].as_str().unwrap_or_default();
            let mut command = None;
            let method = event["method"].as_str().unwrap_or_default();
            if let Some(message) = console_message(method, params) {
                task_state.lock().await.push_console(message);
                continue;
            }
            match method {
                "Fetch.requestPaused" => {
                    command = Some(("Fetch.continueRequest", json!({"requestId":request_id})))
                }
                "Fetch.authRequired" => {
                    if let Some((origin, username, password)) = &auth {
                        let challenge = &params["authChallenge"];
                        let first_attempt = authenticated.insert(request_id.to_string());
                        let response =
                            auth_response(origin, username, password, challenge, first_attempt);
                        command = Some((
                            "Fetch.continueWithAuth",
                            json!({"requestId":request_id,"authChallengeResponse":response}),
                        ));
                    }
                }
                "Network.requestWillBeSent" => {
                    let mut state = task_state.lock().await;
                    state.last_activity = std::time::Instant::now();
                    state.pending.insert(request_id.to_string());
                    if let Some(timestamp) = params["timestamp"].as_f64() {
                        state.started.insert(request_id.to_owned(), timestamp);
                    }
                    let request = &params["request"];
                    if state.url_pattern.is_empty()
                        || request["url"]
                            .as_str()
                            .unwrap_or_default()
                            .contains(&state.url_pattern)
                    {
                        if state.requests.len() >= MAX_BUFFERED_EVENTS {
                            state.requests.pop_front();
                            state.request_ids.pop_front();
                        }
                        state.requests.push_back(super::observe::NetworkRequest {
                            url: request["url"].as_str().unwrap_or_default().into(),
                            method: request["method"].as_str().unwrap_or_default().into(),
                            status: None,
                            status_text: None,
                            request_headers: request.get("headers").map(Value::to_string),
                            response_headers: None,
                            duration_ms: None,
                            size_bytes: None,
                            resource_type: params["type"].as_str().map(str::to_owned),
                        });
                        state.request_ids.push_back(request_id.to_owned());
                    }
                }
                "Network.responseReceived" => {
                    let mut state = task_state.lock().await;
                    if let Some(index) = state.request_ids.iter().rposition(|id| id == request_id) {
                        let response = &params["response"];
                        let request = &mut state.requests[index];
                        request.status = response["status"].as_f64().map(|status| status as i32);
                        request.status_text = response["statusText"].as_str().map(str::to_owned);
                        request.response_headers = response.get("headers").map(Value::to_string);
                    }
                }
                "Network.loadingFinished" | "Network.loadingFailed" => {
                    let mut state = task_state.lock().await;
                    state.last_activity = std::time::Instant::now();
                    state.pending.remove(request_id);
                    let started = state.started.remove(request_id);
                    if let Some(index) = state.request_ids.iter().rposition(|id| id == request_id) {
                        let request = &mut state.requests[index];
                        request.size_bytes = params["encodedDataLength"]
                            .as_f64()
                            .map(|length| length as i64);
                        request.duration_ms = started
                            .zip(params["timestamp"].as_f64())
                            .map(|(start, end)| ((end - start).max(0.0) * 1000.0) as i64);
                    }
                }
                _ => {}
            }
            if let Some((method, params)) = command {
                if socket
                    .send(Message::Text(
                        json!({"id":id,"method":method,"params":params})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .is_err()
                {
                    task_state.lock().await.failure =
                        Some("Browser protocol command failed".into());
                    break;
                }
                id += 1;
            }
        }
    });
    context.cache.write().await.insert(
        listener_key(session, kind),
        Arc::new(Listener {
            network,
            abort: task.abort_handle(),
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
}

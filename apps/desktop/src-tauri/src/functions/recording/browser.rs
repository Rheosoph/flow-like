use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

use super::capture::{CaptureMessage, InputEvent};
use super::state::{ActionType, BrowserAction, BrowserActionKind};
use crate::functions::TauriFunctionError;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const SCRIPT: &str = include_str!("browser-script.js");
const BINDING: &str = "__flowLikeRecorder";

#[derive(Clone, Deserialize)]
struct Target {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket: Option<String>,
}

pub(super) struct BrowserCapture {
    stop: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl BrowserCapture {
    pub(super) async fn start(
        address: &str,
        webdriver_url: &str,
        browser_type: &str,
        tx: mpsc::UnboundedSender<CaptureMessage>,
        active: Arc<AtomicBool>,
        app: tauri::AppHandle,
    ) -> Result<Self, TauriFunctionError> {
        let endpoint = debugger_endpoint(address)?;
        if !matches!(browser_type, "Chrome" | "Edge") {
            return Err(TauriFunctionError::new(
                "Browser recording supports Chrome and Edge",
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(error)?;
        let targets = list_targets(&client, &endpoint).await?;
        if targets.is_empty() {
            return Err(TauriFunctionError::new(
                "Open a browser tab in the debugging-enabled Chrome or Edge instance before recording",
            ));
        }
        let mut prepared = Vec::new();
        for target in targets {
            match connect_page(&target).await {
                Ok(page) => prepared.push((target.clone(), page)),
                Err(error) => {
                    futures::future::join_all(
                        prepared.into_iter().map(|(_, page)| cleanup_page(page)),
                    )
                    .await;
                    return Err(error);
                }
            }
        }
        let _ = tx.send(CaptureMessage::Input(InputEvent::browser(
            ActionType::BrowserAttach {
                debugger_address: address.trim().to_string(),
                webdriver_url: webdriver_url.to_string(),
                browser_type: browser_type.to_string(),
            },
            Utc::now(),
        )));
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut seen = HashSet::new();
            let mut pages = Vec::new();
            for (target, socket) in prepared {
                seen.insert(target.id.clone());
                pages.push(tokio::spawn(record_page(
                    target.id.clone(),
                    socket,
                    tx.clone(),
                    active.clone(),
                    app.clone(),
                    stopped.clone(),
                )));
            }
            let mut timer = tokio::time::interval(Duration::from_millis(500));
            loop {
                if *stopped.borrow() {
                    break;
                }
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = timer.tick() => {
                        if let Ok(targets) = list_targets(&client, &endpoint).await {
                            for target in targets {
                                if seen.contains(&target.id) { continue; }
                                match connect_page(&target).await {
                                    Ok(socket) => {
                                        seen.insert(target.id.clone());
                                        pages.push(tokio::spawn(record_page(target.id.clone(), socket, tx.clone(), active.clone(), app.clone(), stopped.clone())));
                                    }
                                    Err(failure) => crate::utils::emit_to_ui(&app, "recording:error", format!("Could not record a browser tab: {failure:?}")),
                                }
                            }
                        }
                    }
                }
            }
            for page in pages {
                let _ = page.await;
            }
        });
        Ok(Self {
            stop,
            task: Some(task),
        })
    }

    pub(super) async fn finish(mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for BrowserCapture {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

fn error(error: impl std::fmt::Display) -> TauriFunctionError {
    TauriFunctionError::new(&error.to_string())
}

fn debugger_endpoint(address: &str) -> Result<String, TauriFunctionError> {
    let url =
        reqwest::Url::parse(&format!("http://{}/json/list", address.trim())).map_err(error)?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/json/list"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(TauriFunctionError::new(
            "Browser debugger address must be host:port, for example 127.0.0.1:9222",
        ));
    }
    Ok(url.to_string())
}

async fn list_targets(
    client: &reqwest::Client,
    endpoint: &str,
) -> Result<Vec<Target>, TauriFunctionError> {
    let targets: Vec<Target> = client
        .get(endpoint)
        .send()
        .await
        .map_err(error)?
        .error_for_status()
        .map_err(error)?
        .json()
        .await
        .map_err(error)?;
    let targets: Vec<_> = targets
        .into_iter()
        .filter(|target| {
            target.kind == "page"
                && target.websocket.is_some()
                && (target.url.starts_with("http://") || target.url.starts_with("https://"))
        })
        .collect();
    if targets.len() > 32 {
        return Err(TauriFunctionError::new(
            "Browser recording supports up to 32 open web pages. Close unused tabs before recording.",
        ));
    }
    Ok(targets)
}

async fn request(
    socket: &mut Socket,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, TauriFunctionError> {
    socket
        .send(Message::Text(
            json!({ "id": id, "method": method, "params": params })
                .to_string()
                .into(),
        ))
        .await
        .map_err(error)?;
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(error)?;
            if let Message::Text(text) = message {
                let response: Value = serde_json::from_str(&text).map_err(error)?;
                if response["id"].as_u64() == Some(id) {
                    if response.get("error").is_some()
                        || response["result"].get("exceptionDetails").is_some()
                    {
                        return Err(error(format!(
                            "Browser recorder command failed: {response}"
                        )));
                    }
                    return Ok(response["result"].clone());
                }
            }
        }
        Err(TauriFunctionError::new("Browser tab disconnected"))
    })
    .await
    .map_err(|_| TauriFunctionError::new("Browser recorder timed out"))?
}

struct PageSocket {
    socket: Socket,
    script_id: String,
    has_cross_origin_frames: bool,
    main_frame_id: String,
}

async fn connect_page(target: &Target) -> Result<PageSocket, TauriFunctionError> {
    let address = target
        .websocket
        .as_deref()
        .ok_or_else(|| TauriFunctionError::new("Browser tab has no debugger endpoint"))?;
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(address),
    )
    .await
    .map_err(error)?
    .map_err(error)?;
    let mut page = PageSocket {
        socket,
        script_id: String::new(),
        has_cross_origin_frames: false,
        main_frame_id: String::new(),
    };
    let initialized: Result<(), TauriFunctionError> = async {
        request(&mut page.socket, 1, "Runtime.enable", json!({})).await?;
        request(&mut page.socket, 2, "Page.enable", json!({})).await?;
        request(
            &mut page.socket,
            3,
            "Runtime.addBinding",
            json!({"name": BINDING}),
        )
        .await?;
        let installed = request(
            &mut page.socket,
            4,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": SCRIPT, "runImmediately": true}),
        )
        .await?;
        page.script_id = installed["identifier"]
            .as_str()
            .ok_or_else(|| {
                TauriFunctionError::new("Browser did not register the recording script")
            })?
            .into();
        request(
            &mut page.socket,
            5,
            "Runtime.evaluate",
            json!({"expression": SCRIPT}),
        )
        .await?;
        let targets = request(&mut page.socket, 6, "Target.getTargets", json!({})).await?;
        page.has_cross_origin_frames = targets["targetInfos"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["type"] == "iframe"));
        request(
            &mut page.socket,
            7,
            "Target.setDiscoverTargets",
            json!({"discover": true}),
        )
        .await?;
        let tree = request(&mut page.socket, 8, "Page.getFrameTree", json!({})).await?;
        page.main_frame_id = tree["frameTree"]["frame"]["id"]
            .as_str()
            .unwrap_or_default()
            .into();
        Ok(())
    }
    .await;
    if let Err(error) = initialized {
        cleanup_page(page).await;
        return Err(error);
    }
    Ok(page)
}

fn binding_action(payload: &str) -> Result<(BrowserAction, DateTime<Utc>), String> {
    let value: Value = serde_json::from_str(payload).map_err(|failure| failure.to_string())?;
    if let Some(error) = value["error"].as_str() {
        return Err(error.to_string());
    }
    let action: BrowserAction =
        serde_json::from_value(value["action"].clone()).map_err(|failure| failure.to_string())?;
    if action.url.is_empty() || action.selector.is_empty() {
        return Err("The browser action has no stable page or element locator".into());
    }
    if !matches!(
        action.kind,
        BrowserActionKind::Click
            | BrowserActionKind::DoubleClick
            | BrowserActionKind::Type
            | BrowserActionKind::Select
            | BrowserActionKind::Key
            | BrowserActionKind::Scroll
    ) {
        return Err("Unsupported browser recording action".into());
    }
    let timestamp = value["timestamp"]
        .as_i64()
        .and_then(DateTime::from_timestamp_millis)
        .unwrap_or_else(Utc::now);
    Ok((action, timestamp))
}

fn stop_binding(message: &Value) -> bool {
    message["method"] == "Runtime.bindingCalled"
        && message["params"]["name"] == BINDING
        && message["params"]["payload"]
            .as_str()
            .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
            .is_some_and(|payload| payload["stop"] == true)
}

async fn record_page(
    tab_id: String,
    mut page: PageSocket,
    tx: mpsc::UnboundedSender<CaptureMessage>,
    active: Arc<AtomicBool>,
    app: tauri::AppHandle,
    mut stopped: watch::Receiver<bool>,
) {
    let mut last_user_action: Option<std::time::Instant> = None;
    let mut renderer_navigation_pending = false;
    let mut warned_about_frames = page.has_cross_origin_frames;
    if warned_about_frames {
        crate::utils::emit_to_ui(
            &app,
            "recording:error",
            "Cross-origin embedded browser pages are not recorded. Add their workflow steps explicitly.",
        );
    }
    loop {
        if *stopped.borrow() {
            break;
        }
        tokio::select! {
            _ = stopped.changed() => break,
            message = page.socket.next() => {
                let text = match message { Some(Ok(Message::Text(text))) => text, Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break, _ => continue };
                let Ok(message) = serde_json::from_str::<Value>(&text) else { continue; };
                if stop_binding(&message) {
                    active.store(false, Ordering::SeqCst);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { let _ = super::stop_recording(app).await; });
                    continue;
                }
                if !active.load(Ordering::SeqCst) { continue; }
                if !warned_about_frames && message["method"] == "Target.targetCreated" && message["params"]["targetInfo"]["type"] == "iframe" {
                    warned_about_frames = true;
                    crate::utils::emit_to_ui(&app, "recording:error", "Cross-origin embedded browser pages are not recorded. Add their workflow steps explicitly.");
                }
                if message["method"] == "Page.frameRequestedNavigation" && message["params"]["frameId"] == page.main_frame_id {
                    renderer_navigation_pending = true;
                }
                if message["method"] == "Page.frameNavigated" && message["params"]["frame"].get("parentId").is_none() {
                    if let Some(id) = message["params"]["frame"]["id"].as_str() { page.main_frame_id = id.into(); }
                    if let Some(url) = message["params"]["frame"]["url"].as_str().filter(|url| url.starts_with("http://") || url.starts_with("https://")) {
                        let kind = if std::mem::take(&mut renderer_navigation_pending) { BrowserActionKind::WaitForUrl } else { BrowserActionKind::Navigate };
                        if let Ok(mut action) = serde_json::from_value::<BrowserAction>(json!({"kind": kind, "url": url})) {
                            action.tab_id = tab_id.clone();
                            let _ = tx.send(CaptureMessage::Input(InputEvent::browser(ActionType::Browser { action }, Utc::now())));
                        }
                    }
                }
                if message["method"] == "Page.navigatedWithinDocument" && message["params"]["frameId"] == page.main_frame_id {
                    if let Some(url) = message["params"]["url"].as_str() {
                        let kind = if message["params"]["navigationType"] == "historyApi" || last_user_action.take().is_some_and(|at| at.elapsed() < Duration::from_secs(2)) { BrowserActionKind::WaitForUrl } else { BrowserActionKind::Navigate };
                        if let Ok(mut action) = serde_json::from_value::<BrowserAction>(json!({"kind": kind, "url": url})) {
                            action.tab_id = tab_id.clone();
                            let _ = tx.send(CaptureMessage::Input(InputEvent::browser(ActionType::Browser { action }, Utc::now())));
                        }
                    }
                }
                if message["method"] == "Runtime.bindingCalled" && message["params"]["name"] == BINDING {
                    let Some(payload) = message["params"]["payload"].as_str() else { continue; };
                    match binding_action(payload) {
                        Ok((mut action, timestamp)) => {
                            if matches!(action.kind, BrowserActionKind::Click | BrowserActionKind::DoubleClick | BrowserActionKind::Key) { last_user_action = Some(std::time::Instant::now()); }
                            action.tab_id = tab_id.clone(); let _ = tx.send(CaptureMessage::Input(InputEvent::browser(ActionType::Browser { action }, timestamp))); }
                        Err(error) => crate::utils::emit_to_ui(&app, "recording:error", error),
                    }
                }
            }
        }
    }
    cleanup_page(page).await;
}

async fn cleanup_page(mut page: PageSocket) {
    let _ = request(
        &mut page.socket,
        90,
        "Runtime.evaluate",
        json!({"expression": "globalThis.__flowLikeRecordingCleanup?.()"}),
    )
    .await;
    let _ = request(
        &mut page.socket,
        91,
        "Page.removeScriptToEvaluateOnNewDocument",
        json!({"identifier": page.script_id}),
    )
    .await;
    let _ = request(
        &mut page.socket,
        92,
        "Runtime.removeBinding",
        json!({"name": BINDING}),
    )
    .await;
    let _ = page.socket.close(None).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugger_address_cannot_change_discovery_path_or_include_credentials() {
        assert_eq!(
            debugger_endpoint("127.0.0.1:9222").unwrap(),
            "http://127.0.0.1:9222/json/list"
        );
        assert!(debugger_endpoint("127.0.0.1:9222/other").is_err());
        assert!(debugger_endpoint("user:password@localhost:9222").is_err());
    }

    #[test]
    fn binding_payload_preserves_unicode_input_and_frames() {
        let payload = json!({"timestamp": 1700000000000i64, "action": {"kind": "type", "selector": "#name", "value": "Zoë 文", "url": "https://example.test/form", "frames": ["#form-frame"]}});
        let (action, timestamp) = binding_action(&payload.to_string()).unwrap();
        assert_eq!(action.kind, BrowserActionKind::Type);
        assert_eq!(action.value, "Zoë 文");
        assert_eq!(action.frames, ["#form-frame"]);
        assert_eq!(timestamp.timestamp_millis(), 1700000000000);
        assert!(binding_action(r#"{"error":"Cross-origin frame is unavailable"}"#).is_err());
    }

    #[test]
    fn page_bindings_cannot_inject_workflow_control_actions() {
        let payload =
            json!({"action": {"kind":"select_tab", "selector":"#x", "url":"https://example.test"}});
        assert!(binding_action(&payload.to_string()).is_err());
    }
}

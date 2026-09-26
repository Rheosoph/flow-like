#![cfg(feature = "execute")]
extern crate flow_like_runtime as flow_like;

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::NodeLogic,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog_automation::{
    browser::interact::BrowserClickNode,
    types::{
        handles::{AutomationSession, BrowserContextOptions},
        selectors::{Selector, SelectorKind},
    },
};
use flow_like_types::{
    Value,
    json::json,
    sync::{Mutex, RwLock},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

struct MockWebDriver {
    url: String,
    requests: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    stopped: Arc<AtomicBool>,
    debugger_websocket: Arc<std::sync::Mutex<Option<String>>>,
}
impl MockWebDriver {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let shutdown = stopped.clone();
        let debugger_websocket = Arc::new(std::sync::Mutex::new(None::<String>));
        let debugger = debugger_websocket.clone();
        let debugger_address = url.trim_start_matches("http://").to_owned();
        std::thread::spawn(move || {
            while !shutdown.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 4096];
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let request = header
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                let body = if length > 0 {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                } else {
                    Value::Null
                };
                captured.lock().unwrap().push((request.clone(), body));
                let value = if request == "POST /session" {
                    json!({"sessionId":"test-session","capabilities":{"goog:chromeOptions":{"debuggerAddress":debugger_address}}})
                } else if request == "GET /json/list" {
                    json!([{"id":"tab-a","webSocketDebuggerUrl":debugger.lock().unwrap().clone().unwrap()}])
                } else if request.ends_with("/goog/cdp/execute") {
                    json!({"targetInfo":{"targetId":"tab-a"}})
                } else if request == "GET /session/test-session/window" {
                    json!("tab-a")
                } else if request.ends_with("/element") {
                    json!({"element-6066-11e4-a52e-4f735466cecf":"test-element"})
                } else {
                    Value::Null
                };
                let response = if request == "GET /json/list" {
                    value
                } else {
                    json!({"value":value})
                }
                .to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
            }
        });
        Self {
            url,
            requests,
            stopped,
            debugger_websocket,
        }
    }
}
impl Drop for MockWebDriver {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

async fn context(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
    context_with_schema(logic.clone(), logic.get_node()).await
}

async fn context_with_schema(
    logic: Arc<dyn NodeLogic>,
    node: flow_like::flow::node::Node,
) -> ExecutionContext {
    let pins = node
        .pins
        .values()
        .map(|pin| (pin.id.clone(), Arc::new(InternalPin::new(pin, false))))
        .collect();
    let node = Arc::new(InternalNode::new(node, pins, logic, AHashMap::new()));
    for pin in node.pins.iter() {
        pin.init_node(Arc::downgrade(&node));
        pin.init_connected_to(vec![]);
        pin.init_depends_on(vec![]);
    }
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::new()));
    let cache = Arc::new(RwLock::new(AHashMap::new()));
    let run: Weak<Mutex<Run>> = Weak::new();
    ExecutionContext::new(
        Arc::new(AHashMap::from_iter([(
            node.node_id().to_string(),
            node.clone(),
        )])),
        &run,
        &state,
        &node,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(vec![])),
        None,
        None,
        Arc::new(AHashMap::new()),
        None,
    )
    .await
}

#[tokio::test]
async fn legacy_browser_click_without_new_pins_still_uses_css() {
    let server = MockWebDriver::new();
    let logic = Arc::new(BrowserClickNode::new());
    let mut schema = logic.get_node();
    schema
        .pins
        .retain(|_, pin| !matches!(pin.name.as_str(), "locator" | "button" | "modifiers"));
    let mut context = context_with_schema(logic.clone(), schema).await;
    let driver = thirtyfour::WebDriver::new(&server.url, thirtyfour::DesiredCapabilities::chrome())
        .await
        .unwrap();
    let mut session = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    session
        .attach_browser(&mut context, driver, &BrowserContextOptions::default())
        .await
        .unwrap();
    session
        .set_current_page(&mut context, "tab-a".into())
        .await
        .unwrap();
    context
        .set_pin_value("session", json!(session))
        .await
        .unwrap();
    context
        .set_pin_value("selector", json!("#old-button"))
        .await
        .unwrap();
    logic.run(&mut context).await.unwrap();
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(path, body)| path.ends_with("/element")
                && body == &json!({"using":"css selector","value":"#old-button"}))
    );
    session.close(&mut context).await.unwrap();
}

#[tokio::test]
async fn opening_browser_preserves_the_initial_tab_and_debugger_endpoint() {
    use flow_like_catalog_automation::browser::context::BrowserOpenNode;
    let server = MockWebDriver::new();
    let logic = Arc::new(BrowserOpenNode::new());
    let mut context = context(logic.clone()).await;
    let session = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    context
        .set_pin_value("session", json!(session))
        .await
        .unwrap();
    context
        .set_pin_value("webdriver_url", json!(server.url))
        .await
        .unwrap();
    logic.run(&mut context).await.unwrap();
    let opened: AutomationSession = context.evaluate_pin("session_out").await.unwrap();
    let address: String = context.evaluate_pin("debugger_address").await.unwrap();
    assert_eq!(opened.current_window_handle.as_deref(), Some("tab-a"));
    assert_eq!(address, server.url.trim_start_matches("http://"));
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(path, body)| path == "POST /session"
                && body["capabilities"]["alwaysMatch"]["unhandledPromptBehavior"] == "ignore")
    );
    opened.close(&mut context).await.unwrap();
}

#[tokio::test]
async fn existing_browser_driver_sessions_are_released_before_reattaching() {
    use flow_like_catalog_automation::browser::manage::BrowserAttachNode;
    let server = MockWebDriver::new();
    let logic = Arc::new(BrowserAttachNode::new());
    let mut context = context(logic.clone()).await;
    let mut session = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    context
        .set_pin_value("webdriver_url", json!(server.url))
        .await
        .unwrap();
    context
        .set_pin_value("debugger_address", json!("127.0.0.1:9222"))
        .await
        .unwrap();
    for _ in 0..2 {
        context
            .set_pin_value("session", json!(session))
            .await
            .unwrap();
        logic.run(&mut context).await.unwrap();
        session = context.evaluate_pin("session_out").await.unwrap();
        session.detach_browser(&mut context).await.unwrap();
        assert!(!session.has_browser());
    }
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .filter(|(path, _)| path == "POST /session")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(path, _)| path == "DELETE /session/test-session")
            .count(),
        2
    );
    assert!(
        !requests
            .iter()
            .any(|(path, _)| path == "DELETE /session/test-session/window")
    );
    session.close(&mut context).await.unwrap();
}

#[tokio::test]
async fn browser_actions_follow_each_session_tab_and_typed_scoped_selector() {
    let server = MockWebDriver::new();
    let logic = Arc::new(BrowserClickNode::new());
    let mut context = context(logic.clone()).await;
    let driver = thirtyfour::WebDriver::new(&server.url, thirtyfour::DesiredCapabilities::chrome())
        .await
        .unwrap();
    let mut first = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    first
        .attach_browser(&mut context, driver, &BrowserContextOptions::default())
        .await
        .unwrap();
    first
        .set_current_page(&mut context, "tab-a".into())
        .await
        .unwrap();
    let mut second = first.clone();
    second
        .set_current_page(&mut context, "tab-b".into())
        .await
        .unwrap();
    let locator = Selector {
        kind: SelectorKind::AriaLabel,
        value: "Bob's \"save\"".into(),
        confidence: None,
        scope: Some("#toolbar".into()),
    };
    context
        .set_pin_value("locator", json!(locator))
        .await
        .unwrap();
    context
        .set_pin_value("button", json!("middle"))
        .await
        .unwrap();
    context
        .set_pin_value("modifiers", json!(["Control"]))
        .await
        .unwrap();
    for session in [&first, &second, &first] {
        context
            .set_pin_value("session", json!(session))
            .await
            .unwrap();
        logic.run(&mut context).await.unwrap();
    }
    let requests = server.requests.lock().unwrap().clone();
    let windows: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "POST /session/test-session/window")
        .map(|(_, body)| body["handle"].clone())
        .collect();
    assert_eq!(
        windows,
        vec![json!("tab-a"), json!("tab-b"), json!("tab-a")]
    );
    assert!(
        requests
            .iter()
            .any(|(path, body)| path == "POST /session/test-session/element"
                && body == &json!({"using":"css selector","value":"#toolbar"}))
    );
    assert!(requests.iter().any(|(path, body)| {
        path == "POST /session/test-session/element/test-element/element"
            && body["using"] == "xpath"
            && body["value"]
                .as_str()
                .unwrap()
                .contains("@aria-label = concat('Bob',\"'\",'s \"save\"')")
    }));
    let actions: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "POST /session/test-session/actions")
        .collect();
    assert_eq!(actions.len(), 3);
    assert!(actions[0].1.to_string().contains("pointerDown"));
    assert!(
        actions[0].1["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action["type"] == "pointerDown" && action["button"] == 1))
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(path, _)| path == "DELETE /session/test-session/actions")
            .count(),
        3
    );
    first.close(&mut context).await.unwrap();
    assert!(second.get_browser_driver(&context).await.is_err());
}

#[tokio::test]
async fn frame_context_is_replayed_before_each_browser_operation() {
    let server = MockWebDriver::new();
    let mut context = context(Arc::new(BrowserClickNode::new())).await;
    let driver = thirtyfour::WebDriver::new(&server.url, thirtyfour::DesiredCapabilities::chrome())
        .await
        .unwrap();
    let mut session = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    session
        .attach_browser(&mut context, driver, &BrowserContextOptions::default())
        .await
        .unwrap();
    session
        .set_current_page(&mut context, "tab-a".into())
        .await
        .unwrap();
    session.browser_frame_selectors =
        vec![Selector::css("iframe#outer"), Selector::css("iframe#inner")];
    drop(
        session
            .get_browser_driver_and_switch(&context)
            .await
            .unwrap(),
    );
    let requests = server.requests.lock().unwrap().clone();
    let frames: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "POST /session/test-session/frame")
        .collect();
    assert_eq!(frames.len(), 3);
    assert!(frames[0].1["id"].is_null());
    assert_eq!(
        frames[1].1["id"]["element-6066-11e4-a52e-4f735466cecf"],
        "test-element"
    );
    let selectors: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path.ends_with("/element"))
        .map(|(_, body)| body["value"].clone())
        .collect();
    assert_eq!(
        selectors,
        vec![json!("iframe#outer"), json!("iframe#inner")]
    );
    session.close(&mut context).await.unwrap();
}

#[tokio::test]
async fn basic_auth_protocol_never_sends_credentials_to_other_origins_or_proxies() {
    use flow_like_catalog_automation::browser::auth::BrowserSetBasicAuthNode;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let server = MockWebDriver::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    *server.debugger_websocket.lock().unwrap() =
        Some(format!("ws://{}", listener.local_addr().unwrap()));
    let protocol = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let enable: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(enable["method"], "Fetch.enable");
        assert_eq!(
            enable["params"]["patterns"][0]["urlPattern"],
            "https://allowed.test/*"
        );
        assert!(!enable.to_string().contains("secret-value"));
        // A paused request can arrive before the enable acknowledgement.
        socket
            .send(Message::Text(
                json!({"method":"Fetch.requestPaused","params":{"requestId":"one"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({"id":1,"result":{}}).to_string().into(),
            ))
            .await
            .unwrap();
        for (request, origin, source) in [
            ("one", "https://allowed.test", "Server"),
            ("two", "https://allowed.test.evil.test", "Server"),
            ("three", "https://allowed.test", "Proxy"),
            ("one", "https://allowed.test", "Server"),
        ] {
            socket.send(Message::Text(json!({"method":"Fetch.authRequired","params":{"requestId":request,"authChallenge":{"origin":origin,"source":source,"scheme":"basic"}}}).to_string().into())).await.unwrap();
        }
        let mut commands = Vec::new();
        while commands.len() < 5 {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                commands.push(serde_json::from_str::<Value>(&text).unwrap());
            }
        }
        commands
    });
    let logic = Arc::new(BrowserSetBasicAuthNode::new());
    let mut context = context(logic.clone()).await;
    let driver = thirtyfour::WebDriver::new(&server.url, thirtyfour::DesiredCapabilities::chrome())
        .await
        .unwrap();
    let mut session = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .unwrap();
    session
        .attach_browser(&mut context, driver, &BrowserContextOptions::default())
        .await
        .unwrap();
    session
        .set_current_page(&mut context, "tab-a".into())
        .await
        .unwrap();
    for (name, value) in [
        ("session", json!(session)),
        ("origin", json!("https://allowed.test")),
        ("debugger_address", json!(server.url)),
        ("username", json!("test-user")),
        ("password", json!("secret-value")),
    ] {
        context.set_pin_value(name, value).await.unwrap();
    }
    logic.run(&mut context).await.unwrap();
    let commands = tokio::time::timeout(std::time::Duration::from_secs(5), protocol)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(commands[0]["method"], "Fetch.continueRequest");
    assert_eq!(
        commands[1]["params"]["authChallengeResponse"]["response"],
        "ProvideCredentials"
    );
    assert_eq!(
        commands[1]["params"]["authChallengeResponse"]["password"],
        "secret-value"
    );
    for command in &commands[2..] {
        assert_eq!(
            command["params"]["authChallengeResponse"],
            json!({"response":"CancelAuth"})
        );
        assert!(!command.to_string().contains("secret-value"));
    }
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(path, body)| path.contains("/execute/sync")
                || body.to_string().contains("secret-value"))
    );
    session.close(&mut context).await.unwrap();
}

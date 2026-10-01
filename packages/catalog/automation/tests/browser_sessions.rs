#![cfg(feature = "execute")]
extern crate flow_like_runtime as flow_like;

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin, resources::RunResources,
        },
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_browser::{
    Browser, BrowserError, ConnectionKind, Page,
    script::ELEMENT_KEY,
    test_hooks::{BrowserProcess, TestSetup},
    testing::{
        default_auto_reply,
        mock_server::{MockCdpServer, MockJsonVersion, MockServerOptions},
    },
    transport::memory::{InMemoryControl, InMemoryTransport, SentCommand},
    types::TargetId,
};
use flow_like_catalog_automation::{
    browser::{
        auth::BrowserSetBasicAuthNode, interact::BrowserClickNode, manage::BrowserAttachNode,
    },
    types::{
        handles::{AutomationSession, BrowserContextOptions, BrowserType},
        selectors::{Selector, SelectorKind},
    },
};
use flow_like_types::{
    Value,
    json::json,
    sync::{Mutex, RwLock},
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(15);
const SETTLE: Duration = Duration::from_secs(5);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const CHROME_PRODUCT: &str = "Chrome/154.0.8037.92";
/// The utility world name of flow-like-browser (`frames::UTIL_WORLD`, crate-private).
const UTIL_WORLD: &str = "__flowlike_util__";
const RESOLVE_SCRIPT_MARKER: &str = "const [kind, value, xpath, scope,";
const CALL_WRAPPER_PREFIX: &str = "function(script,args,w3c)";
/// Only the CSS find declaration contains this; the selector script and the atoms call
/// `querySelectorAll` on other receivers.
const FIND_CSS_MARKER: &str = "root.querySelectorAll(selector)";
const NO_CONTEXT: &str = "Cannot find context with specified id";
const OCCUPIED: &str = "Close the attached browser before replacing it";
const SECRET: &str = "secret-value";
const BUTTON: i64 = 42;
const FRAME_BUTTON: i64 = 43;
const OUTER_OWNER: i64 = 11;
const INNER_OWNER: i64 = 12;
const QUAD: [f64; 8] = [100.0, 200.0, 140.0, 200.0, 140.0, 220.0, 100.0, 220.0];
const CENTRE: (f64, f64) = (120.0, 210.0);

/// A same-process iframe of the first tab and the backend node of its owner element.
struct ChildFrame {
    id: &'static str,
    parent: &'static str,
    owner: i64,
}

struct FailingFind {
    frame: &'static str,
    selector: &'static str,
    left: usize,
}

#[derive(Default)]
struct Holds {
    booted: HashSet<String>,
    fetch_enable: bool,
    failing_finds: Vec<FailingFind>,
}

/// Scripted Chrome: each tab `T` attaches as session `S-T`, every frame `F` has loader `L-F`
/// and the contexts `S-T/F/main` and `S-T/F/util`.
struct Chrome {
    tabs: Vec<&'static str>,
    frames: Vec<ChildFrame>,
    holds: StdMutex<Holds>,
}

impl Chrome {
    fn with_tabs(tabs: &[&'static str]) -> Arc<Chrome> {
        Self::with_frames(tabs, Vec::new())
    }

    fn with_frames(tabs: &[&'static str], frames: Vec<ChildFrame>) -> Arc<Chrome> {
        Arc::new(Chrome {
            tabs: tabs.to_vec(),
            frames,
            holds: StdMutex::default(),
        })
    }

    fn holds(&self) -> MutexGuard<'_, Holds> {
        self.holds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn boot(&self, session: &str) {
        self.holds().booted.insert(session.to_owned());
    }

    fn booted(&self, session: &str) -> bool {
        self.holds().booted.contains(session)
    }

    fn hold_fetch_enable(&self) {
        self.holds().fetch_enable = true;
    }

    fn fail_find(&self, frame: &'static str, selector: &'static str, times: usize) {
        self.holds().failing_finds.push(FailingFind {
            frame,
            selector,
            left: times,
        });
    }

    fn fails(&self, frame: &str, selector: &str) -> bool {
        let mut holds = self.holds();
        let failing = holds.failing_finds.iter_mut().find(|failing| {
            failing.frame == frame && failing.selector == selector && failing.left > 0
        });
        match failing {
            Some(failing) => {
                failing.left -= 1;
                true
            }
            None => false,
        }
    }

    /// Commands the test answers itself: a tab boot (its contexts are announced first) and a
    /// held `Fetch.enable`. Finds scripted to fail are held by `find_css`.
    fn withholds(&self, command: &SentCommand) -> bool {
        match command.method.as_str() {
            "Runtime.enable" => command
                .session
                .as_deref()
                .is_some_and(|session| !self.booted(session)),
            "Fetch.enable" => self.holds().fetch_enable,
            _ => false,
        }
    }

    fn reply(&self, command: &SentCommand) -> Option<Value> {
        if self.withholds(command) {
            return None;
        }
        let params = &command.params;
        Some(match command.method.as_str() {
            "Runtime.callFunctionOn" => return self.call(params),
            "Runtime.getProperties" => properties(params),
            "DOM.describeNode" => self.describe(params),
            "DOM.resolveNode" => resolved_node(params),
            "DOM.getContentQuads" => json!({"quads": [QUAD]}),
            "Page.getLayoutMetrics" => layout_metrics(),
            _ => self.browser_reply(command),
        })
    }

    fn browser_reply(&self, command: &SentCommand) -> Value {
        let params = &command.params;
        match command.method.as_str() {
            "Browser.getVersion" => version(),
            "Target.getTargets" => {
                let infos: Vec<Value> = self.tabs.iter().copied().map(tab_info).collect();
                json!({"targetInfos": infos})
            }
            "Target.attachToTarget" => {
                json!({"sessionId": session_of(params["targetId"].as_str().unwrap_or_default())})
            }
            "Page.getFrameTree" => {
                let session = command.session.as_deref().unwrap_or_default();
                json!({"frameTree": self.frame_tree(target_of(session))})
            }
            _ => default_auto_reply(command).unwrap_or_else(|| json!({})),
        }
    }

    fn call(&self, params: &Value) -> Option<Value> {
        let declaration = params["functionDeclaration"].as_str().unwrap_or_default();
        if declaration.contains(FIND_CSS_MARKER) {
            return self.find_css(params);
        }
        if declaration.starts_with(CALL_WRAPPER_PREFIX) {
            return Some(script_result(params));
        }
        Some(json!({"result": {"type": "boolean", "value": true}}))
    }

    fn find_css(&self, params: &Value) -> Option<Value> {
        let frame = context_frame(params);
        let selector = first_argument(params);
        if self.fails(frame, selector) {
            return None;
        }
        let found: Vec<String> = self
            .nodes(frame, selector)
            .iter()
            .map(i64::to_string)
            .collect();
        Some(json!({"result": {
            "type": "object",
            "subtype": "array",
            "objectId": format!("found:{}", found.join(",")),
        }}))
    }

    fn nodes(&self, frame: &str, selector: &str) -> Vec<i64> {
        let on_tab = self.tabs.contains(&frame);
        match (frame, selector) {
            ("F-outer", "iframe#inner") => vec![INNER_OWNER],
            ("F-inner", "#go") => vec![FRAME_BUTTON],
            (_, "iframe#outer") if on_tab => vec![OUTER_OWNER],
            (_, "#old-button" | "#go") if on_tab => vec![BUTTON],
            _ => Vec::new(),
        }
    }

    fn describe(&self, params: &Value) -> Value {
        let backend_node_id = params["objectId"]
            .as_str()
            .and_then(|object| object.strip_prefix("node:"))
            .and_then(|id| id.parse::<i64>().ok())
            .or_else(|| params["backendNodeId"].as_i64())
            .unwrap_or_default();
        let mut node = json!({"nodeId": 0, "backendNodeId": backend_node_id, "nodeName": "BUTTON"});
        if let Some(frame) = self
            .frames
            .iter()
            .find(|frame| frame.owner == backend_node_id)
        {
            node["nodeName"] = json!("IFRAME");
            node["frameId"] = json!(frame.id);
        }
        json!({"node": node})
    }

    fn frame_tree(&self, frame: &str) -> Value {
        let parent = self
            .frames
            .iter()
            .find(|child| child.id == frame)
            .map(|child| child.parent);
        let children: Vec<Value> = self
            .frames
            .iter()
            .filter(|child| child.parent == frame)
            .map(|child| self.frame_tree(child.id))
            .collect();
        json!({
            "frame": {"id": frame, "parentId": parent, "loaderId": loader_of(frame), "url": url_of(frame)},
            "childFrames": children,
        })
    }

    /// The tab frame first, then its same-process descendants.
    fn frames_of(&self, target: &str) -> Vec<String> {
        let mut frames = vec![target.to_owned()];
        let mut index = 0;
        while let Some(parent) = frames.get(index).cloned() {
            let children = self.frames.iter().filter(|frame| frame.parent == parent);
            frames.extend(children.map(|frame| frame.id.to_owned()));
            index += 1;
        }
        frames
    }
}

fn session_of(target: &str) -> String {
    format!("S-{target}")
}

fn target_of(session: &str) -> &str {
    session.strip_prefix("S-").unwrap_or(session)
}

fn loader_of(frame: &str) -> String {
    format!("L-{frame}")
}

fn url_of(frame: &str) -> String {
    format!("http://127.0.0.1/{frame}")
}

fn tab_info(target: &str) -> Value {
    json!({"targetId": target, "type": "page", "title": target, "url": url_of(target), "attached": false})
}

fn version() -> Value {
    json!({
        "protocolVersion": "1.3",
        "product": CHROME_PRODUCT,
        "revision": "@0",
        "userAgent": "Mozilla/5.0 Chrome/154.0.8037.92",
        "jsVersion": "15.4",
    })
}

fn layout_metrics() -> Value {
    json!({"cssLayoutViewport": {"pageX": 0, "pageY": 0, "clientWidth": 1280, "clientHeight": 720}})
}

fn resolved_node(params: &Value) -> Value {
    json!({"object": {
        "type": "object",
        "subtype": "node",
        "objectId": format!("obj-{}", params["backendNodeId"]),
    }})
}

fn context_frame(params: &Value) -> &str {
    params["uniqueContextId"]
        .as_str()
        .and_then(|context| context.split('/').nth(1))
        .unwrap_or("scoped")
}

fn first_argument(params: &Value) -> &str {
    params["arguments"][0]["value"].as_str().unwrap_or_default()
}

fn properties(params: &Value) -> Value {
    let found = params["objectId"]
        .as_str()
        .and_then(|object| object.strip_prefix("found:"))
        .unwrap_or_default();
    let items: Vec<Value> = found
        .split(',')
        .filter(|id| !id.is_empty())
        .enumerate()
        .map(|(index, id)| {
            json!({"name": index.to_string(), "value": {"type": "object", "subtype": "node", "objectId": format!("node:{id}")}})
        })
        .collect();
    json!({"result": items})
}

/// `RESOLVE_SCRIPT` finds the button in the frame of its context; other scripts return null.
fn script_result(params: &Value) -> Value {
    if !first_argument(params).contains(RESOLVE_SCRIPT_MARKER) {
        return serialized(&json!({"status": 0, "value": null}), Vec::new());
    }
    let node = json!({"type": "node", "value": {
        "backendNodeId": BUTTON,
        "loaderId": loader_of(context_frame(params)),
    }});
    serialized(
        &json!({"status": 0, "value": [{ ELEMENT_KEY: 0 }]}),
        vec![node],
    )
}

fn serialized(response: &Value, nodes: Vec<Value>) -> Value {
    let mut list = vec![json!({"type": "string", "value": response.to_string()})];
    list.extend(nodes);
    json!({"result": {"type": "object", "deepSerializedValue": {"type": "array", "value": list}}})
}

fn context_created(session: &str, frame: &str, index: usize, util: bool) -> Value {
    let (world, name, kind, offset) = if util {
        ("util", UTIL_WORLD, "isolated", 2)
    } else {
        ("main", "", "default", 1)
    };
    json!({"context": {
        "id": index * 2 + offset,
        "origin": "http://127.0.0.1",
        "name": name,
        "uniqueId": format!("{session}/{frame}/{world}"),
        "auxData": {"isDefault": !util, "type": kind, "frameId": frame},
    }})
}

fn answer_with(control: &InMemoryControl, chrome: &Arc<Chrome>) {
    let chrome = chrome.clone();
    control.set_auto_reply(move |command| chrome.reply(command));
}

fn setup(kind: ConnectionKind, process: Option<BrowserProcess>) -> TestSetup {
    TestSetup {
        kind,
        headless: true,
        page_load_timeout: PAGE_LOAD_TIMEOUT,
        process,
        staging: None,
        run_owned_setup: false,
    }
}

async fn within<T>(what: &str, future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("{what} did not finish within {} s", WAIT.as_secs()))
}

async fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + SETTLE;
    while !done() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} never happened"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Attaches the tab like the first node operation on it would, announcing the main and
/// utility contexts of every frame before `Runtime.enable` returns, as Chrome does.
async fn open_tab(
    chrome: &Chrome,
    browser: &Browser,
    control: &mut InMemoryControl,
    target: &str,
) -> Page {
    let session = session_of(target);
    let target_id = TargetId::from(target);
    let announce = async {
        let enable = control
            .wait_for("Runtime.enable", Some(session.as_str()))
            .await;
        for (index, frame) in chrome.frames_of(target).iter().enumerate() {
            for util in [false, true] {
                let created = context_created(&session, frame, index, util);
                control.emit(
                    "Runtime.executionContextCreated",
                    Some(session.as_str()),
                    created,
                );
            }
        }
        chrome.boot(&session);
        control.reply(&enable, json!({}));
    };
    let (attached, ()) = tokio::join!(
        within("attaching the tab", browser.page(&target_id)),
        announce
    );
    attached.unwrap_or_else(|error| panic!("tab {target} did not attach: {error}"))
}

struct Scripted {
    chrome: Arc<Chrome>,
    browser: Browser,
    control: InMemoryControl,
}

impl Scripted {
    async fn direct(chrome: Arc<Chrome>) -> Scripted {
        let (transport, control) = InMemoryTransport::new();
        answer_with(&control, &chrome);
        let connecting =
            Browser::connect_transport(Box::new(transport), setup(ConnectionKind::Direct, None));
        let browser = within("connecting the scripted browser", connecting)
            .await
            .expect("the scripted browser connects");
        Scripted {
            chrome,
            browser,
            control,
        }
    }

    async fn attach(&mut self, context: &mut ExecutionContext) -> AutomationSession {
        for tab in self.chrome.tabs.clone() {
            open_tab(&self.chrome, &self.browser, &mut self.control, tab).await;
        }
        let mut session = AutomationSession::new(context, 0, 0, false)
            .await
            .expect("the automation session starts");
        session
            .prepare_browser_slot(context)
            .await
            .expect("a live run owns the browser slot");
        session
            .attach_cdp_browser(
                context,
                self.browser.clone(),
                &BrowserContextOptions::default(),
            )
            .await
            .expect("the scripted browser attaches to the session");
        session
    }
}

async fn mock_browser(chrome: &Arc<Chrome>) -> MockCdpServer {
    let server = MockCdpServer::start(MockServerOptions {
        json_version: MockJsonVersion::Ok,
        expect_header: None,
    })
    .await;
    answer_with(&server.control, chrome);
    server
}

fn sent(control: &InMemoryControl, method: &str) -> Vec<SentCommand> {
    control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method == method)
        .collect()
}

fn declaration(command: &SentCommand) -> &str {
    command.params["functionDeclaration"]
        .as_str()
        .unwrap_or_default()
}

fn is_find(command: &SentCommand) -> bool {
    command.method == "Runtime.callFunctionOn" && declaration(command).contains(FIND_CSS_MARKER)
}

fn is_resolve(command: &SentCommand) -> bool {
    command.method == "Runtime.callFunctionOn"
        && declaration(command).starts_with(CALL_WRAPPER_PREFIX)
        && first_argument(&command.params).contains(RESOLVE_SCRIPT_MARKER)
}

/// Every CSS find as (frame of its context, selector), in the order sent.
fn finds(control: &InMemoryControl) -> Vec<(String, String)> {
    control
        .commands_seen()
        .iter()
        .filter(|command| is_find(command))
        .map(|command| {
            let frame = context_frame(&command.params).to_owned();
            (frame, first_argument(&command.params).to_owned())
        })
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(frame, selector)| ((*frame).to_owned(), (*selector).to_owned()))
        .collect()
}

fn backend_nodes(control: &InMemoryControl, method: &str) -> Vec<i64> {
    sent(control, method)
        .iter()
        .filter_map(|command| command.params["backendNodeId"].as_i64())
        .collect()
}

fn mouse(control: &InMemoryControl, kind: &str) -> Vec<SentCommand> {
    sent(control, "Input.dispatchMouseEvent")
        .into_iter()
        .filter(|event| event.params["type"] == kind)
        .collect()
}

fn keys(control: &InMemoryControl, kind: &str, key: &str) -> Vec<SentCommand> {
    sent(control, "Input.dispatchKeyEvent")
        .into_iter()
        .filter(|event| event.params["type"] == kind && event.params["key"] == key)
        .collect()
}

fn sessions(commands: &[SentCommand]) -> Vec<String> {
    commands
        .iter()
        .map(|command| command.session.clone().unwrap_or_default())
        .collect()
}

fn targets_of(commands: &[SentCommand]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|command| command.params["targetId"].as_str().map(str::to_owned))
        .collect()
}

fn tab_ids(browser: &Browser) -> Vec<String> {
    browser
        .pages()
        .into_iter()
        .map(|page| page.target_id.as_str().to_owned())
        .collect()
}

async fn context(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
    context_with_schema(logic.clone(), logic.get_node()).await
}

/// A node context of a live run: browser nodes register their browser with the run resources.
async fn context_with_schema(logic: Arc<dyn NodeLogic>, node: Node) -> ExecutionContext {
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
    let mut context = ExecutionContext::new(
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
    .await;
    context.resources = Arc::new(RunResources::default());
    context
}

async fn set_pins<const N: usize>(context: &mut ExecutionContext, pins: [(&str, Value); N]) {
    for (name, value) in pins {
        context
            .set_pin_value(name, value)
            .await
            .unwrap_or_else(|error| panic!("setting pin {name} failed: {error}"));
    }
}

#[tokio::test]
async fn legacy_browser_click_without_new_pins_still_uses_css() {
    let mut schema = BrowserClickNode::new().get_node();
    schema
        .pins
        .retain(|_, pin| !matches!(pin.name.as_str(), "locator" | "button" | "modifiers"));
    assert_click_finds_css_selector(schema).await;
}

#[tokio::test]
async fn browser_click_with_an_unwired_locator_uses_the_css_selector() {
    assert_click_finds_css_selector(BrowserClickNode::new().get_node()).await;
}

async fn assert_click_finds_css_selector(schema: Node) {
    let mut scripted = Scripted::direct(Chrome::with_tabs(&["T1"])).await;
    let logic = Arc::new(BrowserClickNode::new());
    let mut context = context_with_schema(logic.clone(), schema).await;
    let session = scripted.attach(&mut context).await;
    set_pins(
        &mut context,
        [
            ("session", json!(session)),
            ("selector", json!("#old-button")),
        ],
    )
    .await;
    within("the click", logic.run(&mut context))
        .await
        .expect("the click succeeds");

    let control = &scripted.control;
    assert_eq!(finds(control), pairs(&[("T1", "#old-button")]));
    let find = control
        .commands_seen()
        .into_iter()
        .find(is_find)
        .expect("the CSS selector is searched");
    assert_eq!(find.session.as_deref(), Some("S-T1"));
    assert_eq!(find.params["uniqueContextId"], "S-T1/T1/util");
    assert!(!control.commands_seen().iter().any(is_resolve));
    assert_eq!(
        backend_nodes(control, "DOM.scrollIntoViewIfNeeded"),
        [BUTTON]
    );
    for kind in ["mousePressed", "mouseReleased"] {
        let events = mouse(control, kind);
        assert_eq!(sessions(&events), ["S-T1"], "{kind}");
        let params = &events[0].params;
        assert_eq!(params["button"], "left", "{kind}");
        assert_eq!(params["modifiers"], 0, "{kind}");
        assert_eq!(
            (params["x"].as_f64(), params["y"].as_f64()),
            (Some(CENTRE.0), Some(CENTRE.1))
        );
    }
    session
        .close(&mut context)
        .await
        .expect("Stop Session closes the browser");
    context.resources.shutdown().await;
}

fn attach_pins(session: &AutomationSession, address: &str) -> [(&'static str, Value); 4] {
    [
        ("session", json!(session)),
        ("webdriver_url", json!("")),
        ("debugger_address", json!(address)),
        ("browser_type", json!("Chrome")),
    ]
}

async fn attach_to(
    logic: &BrowserAttachNode,
    context: &mut ExecutionContext,
    session: &AutomationSession,
    address: &str,
) -> flow_like_types::Result<AutomationSession> {
    set_pins(context, attach_pins(session, address)).await;
    within("Attach to Browser", logic.run(context)).await?;
    context.evaluate_pin("session_out").await
}

fn open_dialog(control: &InMemoryControl, session: &str) {
    control.emit(
        "Page.javascriptDialogOpening",
        Some(session),
        json!({
            "url": url_of("T1"),
            "message": "Leave this page?",
            "type": "confirm",
            "hasBrowserHandler": false,
            "defaultPrompt": "",
        }),
    );
}

/// Open Browser reports `localhost:<port>`; that endpoint attaches to the first-seen tab
/// and leaves every tab and dialog to the flow.
#[tokio::test]
async fn opening_browser_preserves_the_initial_tab_and_debugger_endpoint() {
    let chrome = Chrome::with_tabs(&["T1", "T2"]);
    let mut server = mock_browser(&chrome).await;
    let logic = Arc::new(BrowserAttachNode::new());
    let mut context = context(logic.clone()).await;
    let fresh = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .expect("the automation session starts");
    let endpoint = format!("localhost:{}", server.address.port());
    let attached = attach_to(&logic, &mut context, &fresh, &endpoint)
        .await
        .expect("Attach to Browser connects to the reported endpoint");

    assert_eq!(attached.current_window_handle.as_deref(), Some("T1"));
    assert!(matches!(attached.browser_type, Some(BrowserType::Chrome)));
    assert_eq!(attached.browser_headless, Some(false));
    let browser = attached
        .cdp_browser(&context)
        .await
        .expect("the session holds the attached browser");
    assert_eq!(browser.kind(), ConnectionKind::AttachedPort);
    assert_eq!(browser.version().product, CHROME_PRODUCT);
    assert_eq!(tab_ids(&browser), ["T1", "T2"]);

    let page = open_tab(&chrome, &browser, &mut server.control, "T1").await;
    open_dialog(&server.control, "S-T1");
    wait_until("the dialog is reported", || page.pending_dialog().is_some()).await;
    let refused = operation_refused_by_the_dialog(&attached, &context).await;
    assert!(
        matches!(refused, BrowserError::DialogOpen { .. }),
        "an operation on a tab with an open dialog fails with DialogOpen, got {refused:?}"
    );
    assert_initial_tab_kept(&server.control);
    assert_eq!(
        page.pending_dialog().map(|dialog| dialog.message),
        Some("Leave this page?".to_owned())
    );

    attached
        .close(&mut context)
        .await
        .expect("Stop Session disconnects");
    assert!(!browser.is_alive());
    assert!(sent(&server.control, "Browser.close").is_empty());
    context.resources.shutdown().await;
}

/// A session operation on the tab leaves the open dialog to the flow: it fails before any
/// command is sent, so the refusal is deterministic.
async fn operation_refused_by_the_dialog(
    session: &AutomationSession,
    context: &ExecutionContext,
) -> BrowserError {
    let operation = within("starting an operation", session.browser_page(context))
        .await
        .expect("the current tab is reachable while a dialog is open");
    within("reading the URL", operation.page.url())
        .await
        .expect_err("an open dialog refuses the operation")
}

fn assert_initial_tab_kept(control: &InMemoryControl) {
    assert_eq!(sent(control, "Browser.getVersion").len(), 1);
    assert_eq!(targets_of(&sent(control, "Target.attachToTarget")), ["T1"]);
    for method in [
        "Page.handleJavaScriptDialog",
        "Target.createTarget",
        "Target.closeTarget",
        "Target.activateTarget",
    ] {
        assert!(sent(control, method).is_empty(), "{method} was sent");
    }
}

#[tokio::test]
async fn existing_browser_sessions_are_released_before_reattaching() {
    let first = mock_browser(&Chrome::with_tabs(&["T1"])).await;
    let second = mock_browser(&Chrome::with_tabs(&["T1"])).await;
    let logic = Arc::new(BrowserAttachNode::new());
    let mut context = context(logic.clone()).await;
    let fresh = AutomationSession::new(&mut context, 0, 0, false)
        .await
        .expect("the automation session starts");
    let mut session = attach_to(&logic, &mut context, &fresh, &first.address.to_string())
        .await
        .expect("the first browser attaches");
    let first_browser = session
        .cdp_browser(&context)
        .await
        .expect("the session holds the first browser");

    let refused = attach_to(&logic, &mut context, &session, &second.address.to_string())
        .await
        .expect_err("a second browser is refused while one is attached");
    assert_eq!(refused.to_string(), OCCUPIED);
    assert!(first_browser.is_alive());
    assert!(second.control.commands_seen().is_empty());

    session
        .detach_browser(&mut context)
        .await
        .expect("detaching releases the browser");
    assert!(!session.has_browser());
    assert!(session.current_window_handle.is_none());
    assert!(!first_browser.is_alive());
    assert!(first_browser.connection().is_closed());

    let session = attach_to(&logic, &mut context, &session, &second.address.to_string())
        .await
        .expect("the released session attaches again");
    assert_eq!(session.current_window_handle.as_deref(), Some("T1"));
    let second_browser = session
        .cdp_browser(&context)
        .await
        .expect("the session holds the second browser");
    session
        .close(&mut context)
        .await
        .expect("Stop Session disconnects");
    assert!(!second_browser.is_alive());
    for server in [&first, &second] {
        assert_attached_browser_left_alone(&server.control);
    }
    context.resources.shutdown().await;
}

fn assert_attached_browser_left_alone(control: &InMemoryControl) {
    assert_eq!(sent(control, "Browser.getVersion").len(), 1);
    for method in ["Browser.close", "Target.closeTarget", "Target.createTarget"] {
        assert!(
            sent(control, method).is_empty(),
            "{method} was sent to an attached browser"
        );
    }
}

#[cfg(unix)]
mod owned {
    use super::*;
    use flow_like_browser::test_hooks::{Profile as BrowserProfile, adopt_process};
    use flow_like_catalog_automation::browser::context::BrowserCloseNode;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn running(pid: u32) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        !state.is_empty() && !state.starts_with('Z')
    }

    fn stand_in_browser() -> tokio::process::Child {
        tokio::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("the stand-in browser starts")
    }

    struct Owned {
        browser: Browser,
        control: InMemoryControl,
        pid: u32,
        running_at_close: Arc<AtomicBool>,
    }

    /// Records whether the process still ran when `Browser.close` arrived, so the test can
    /// tell close-then-kill from kill-then-close.
    fn answer_and_watch_close(control: &InMemoryControl, pid: u32) -> Arc<AtomicBool> {
        let chrome = Chrome::with_tabs(&["T1"]);
        let running_at_close = Arc::new(AtomicBool::new(false));
        let watched = running_at_close.clone();
        control.set_auto_reply(move |command| {
            if command.method == "Browser.close" {
                watched.store(running(pid), Ordering::SeqCst);
            }
            chrome.reply(command)
        });
        running_at_close
    }

    async fn owned_browser(profile: &std::path::Path) -> Owned {
        let child = stand_in_browser();
        let pid = child.id().expect("the stand-in browser has a pid");
        let owned = BrowserProfile {
            dir: profile.to_path_buf(),
            temporary: true,
        };
        let process = adopt_process(child, owned, None);
        let (transport, control) = InMemoryTransport::new();
        let running_at_close = answer_and_watch_close(&control, pid);
        let connecting = Browser::connect_transport(
            Box::new(transport),
            setup(ConnectionKind::Launched, Some(process)),
        );
        let browser = within("connecting the owned browser", connecting)
            .await
            .expect("the owned browser connects");
        Owned {
            browser,
            control,
            pid,
            running_at_close,
        }
    }

    fn temporary_profile() -> std::path::PathBuf {
        let profile = std::env::temp_dir().join(format!(
            "flow-like-browser-sessions-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&profile).expect("the temporary profile is created");
        profile
    }

    async fn session_owning(
        context: &mut ExecutionContext,
        browser: &Browser,
    ) -> AutomationSession {
        let mut session = AutomationSession::new(context, 0, 0, false)
            .await
            .expect("the automation session starts");
        session
            .attach_cdp_browser(context, browser.clone(), &BrowserContextOptions::default())
            .await
            .expect("the owned browser attaches to the session");
        assert_eq!(session.current_window_handle.as_deref(), Some("T1"));
        session
    }

    /// The stand-in ignores `Browser.close`, so the process only ends when it is killed.
    #[tokio::test]
    async fn closing_an_owned_browser_asks_it_to_close_then_kills_it() {
        let profile = temporary_profile();
        let Owned {
            browser,
            control,
            pid,
            running_at_close,
        } = owned_browser(&profile).await;
        assert!(browser.is_owned());
        let logic = Arc::new(BrowserCloseNode::new());
        let mut context = context(logic.clone()).await;
        let session = session_owning(&mut context, &browser).await;
        assert!(running(pid));

        set_pins(&mut context, [("session", json!(session))]).await;
        within("Close Browser", logic.run(&mut context))
            .await
            .expect("Close Browser closes the owned browser");
        assert_eq!(sent(&control, "Browser.close").len(), 1);
        assert!(
            running_at_close.load(Ordering::SeqCst),
            "Browser.close must reach the browser before its process is killed"
        );
        wait_until("the owned browser exits", || !running(pid)).await;
        assert!(!browser.is_alive());
        assert!(!profile.exists(), "the temporary profile is removed");

        let closed: AutomationSession = context
            .evaluate_pin("session_out")
            .await
            .expect("Close Browser outputs the session");
        assert!(!closed.has_browser());
        closed
            .close(&mut context)
            .await
            .expect("Stop Session succeeds without a browser");
        context.resources.shutdown().await;
    }
}

fn aria_locator() -> Selector {
    Selector {
        kind: SelectorKind::AriaLabel,
        value: "Bob's \"save\"".into(),
        confidence: None,
        scope: Some("#toolbar".into()),
    }
}

#[tokio::test]
async fn browser_actions_follow_each_session_tab_and_typed_scoped_selector() {
    let mut scripted = Scripted::direct(Chrome::with_tabs(&["T1", "T2"])).await;
    let logic = Arc::new(BrowserClickNode::new());
    let mut context = context(logic.clone()).await;
    let first = scripted.attach(&mut context).await;
    let mut second = first.clone();
    second
        .set_current_page_target(&mut context, &TargetId::from("T2"))
        .await
        .expect("the second branch selects the second tab");
    assert_ne!(first.current_page_ref, second.current_page_ref);
    set_pins(
        &mut context,
        [
            ("locator", json!(aria_locator())),
            ("button", json!("middle")),
            ("modifiers", json!(["Control"])),
        ],
    )
    .await;
    for session in [&first, &second, &first] {
        set_pins(&mut context, [("session", json!(session))]).await;
        within("the click", logic.run(&mut context))
            .await
            .expect("the click succeeds");
    }

    let tabs = ["S-T1", "S-T2", "S-T1"];
    assert_resolved_on_each_tab(&scripted.control, &tabs);
    assert_clicked_on_each_tab(&scripted.control, &tabs);

    first
        .close(&mut context)
        .await
        .expect("Stop Session closes the shared browser");
    let closed = second
        .browser_page(&context)
        .await
        .err()
        .map(|error| error.to_string());
    assert!(
        closed
            .as_deref()
            .is_some_and(|error| error.contains("closed")),
        "{closed:?}"
    );
    assert!(!scripted.browser.is_alive());
    context.resources.shutdown().await;
}

fn assert_resolved_on_each_tab(control: &InMemoryControl, tabs: &[&str]) {
    let resolved: Vec<SentCommand> = control
        .commands_seen()
        .into_iter()
        .filter(is_resolve)
        .collect();
    assert_eq!(sessions(&resolved), tabs);
    for command in &resolved {
        let session = command.session.as_deref().unwrap_or_default();
        let main_world = format!("{session}/{}/main", target_of(session));
        assert_eq!(command.params["uniqueContextId"], main_world);
        let args = &command.params["arguments"][1]["value"];
        assert_eq!(args[0], "AriaLabel");
        let xpath = args[2].as_str().unwrap_or_default();
        assert!(
            xpath.contains("@aria-label = concat('Bob',\"'\",'s \"save\"')"),
            "{args}"
        );
        assert_eq!(args[3], "#toolbar");
    }
    assert!(finds(control).is_empty(), "a typed locator is not CSS");
}

fn assert_clicked_on_each_tab(control: &InMemoryControl, tabs: &[&str]) {
    let pressed = mouse(control, "mousePressed");
    assert_eq!(sessions(&pressed), tabs);
    assert!(
        pressed
            .iter()
            .all(|event| event.params["button"] == "middle" && event.params["modifiers"] == 2)
    );
    assert_eq!(sessions(&mouse(control, "mouseReleased")), tabs);
    assert_eq!(sessions(&keys(control, "rawKeyDown", "Control")), tabs);
    assert_eq!(sessions(&keys(control, "keyUp", "Control")), tabs);
    assert_eq!(
        sessions(&sent(control, "DOM.resolveNode")),
        tabs,
        "each element is resolved on its own tab"
    );
    assert_eq!(
        backend_nodes(control, "DOM.scrollIntoViewIfNeeded"),
        [BUTTON; 3]
    );
    assert_eq!(
        targets_of(&sent(control, "Target.attachToTarget")),
        ["T1", "T2"],
        "each tab is attached once"
    );
    assert!(
        sent(control, "Target.activateTarget").is_empty(),
        "operations never switch the active tab"
    );
}

fn nested_frames() -> Arc<Chrome> {
    Chrome::with_frames(
        &["T1"],
        vec![
            ChildFrame {
                id: "F-outer",
                parent: "T1",
                owner: OUTER_OWNER,
            },
            ChildFrame {
                id: "F-inner",
                parent: "F-outer",
                owner: INNER_OWNER,
            },
        ],
    )
}

/// The first two finds in the inner frame fail with a lost context, so the third attempt
/// runs in the main frame (chromedriver attempt #3).
async fn click_falling_back_to_the_main_frame(
    logic: &BrowserClickNode,
    context: &mut ExecutionContext,
    control: &mut InMemoryControl,
) {
    let fail_twice = async {
        for _ in 0..2 {
            let find = control
                .wait_for("Runtime.callFunctionOn", Some("S-T1"))
                .await;
            assert_eq!(context_frame(&find.params), "F-inner");
            assert_eq!(first_argument(&find.params), "#go");
            control.reply_error(&find, -32000, NO_CONTEXT);
        }
    };
    let (clicked, ()) = tokio::join!(within("the click", logic.run(context)), fail_twice);
    clicked.expect("the third attempt clicks in the main frame");
}

#[tokio::test]
async fn frame_context_is_replayed_before_each_browser_operation() {
    let chrome = nested_frames();
    let mut scripted = Scripted::direct(chrome.clone()).await;
    let logic = Arc::new(BrowserClickNode::new());
    let mut context = context(logic.clone()).await;
    let mut session = scripted.attach(&mut context).await;
    session.browser_frame_selectors =
        vec![Selector::css("iframe#outer"), Selector::css("iframe#inner")];
    drop(
        within("replaying the frame path", session.browser_page(&context))
            .await
            .expect("the frame path replays"),
    );

    chrome.fail_find("F-inner", "#go", 2);
    set_pins(
        &mut context,
        [("session", json!(session)), ("selector", json!("#go"))],
    )
    .await;
    click_falling_back_to_the_main_frame(&logic, &mut context, &mut scripted.control).await;
    within("the click", logic.run(&mut context))
        .await
        .expect("the next click replays the frame path");

    assert_frame_path_replayed(&scripted.control);
    session
        .close(&mut context)
        .await
        .expect("Stop Session closes the browser");
    context.resources.shutdown().await;
}

fn assert_frame_path_replayed(control: &InMemoryControl) {
    let replay = [("T1", "iframe#outer"), ("F-outer", "iframe#inner")];
    let fallback = [("F-inner", "#go"), ("F-inner", "#go"), ("T1", "#go")];
    let replayed = [("F-inner", "#go")];
    let expected = [
        &replay[..],
        &replay[..],
        &fallback[..],
        &replay[..],
        &replayed[..],
    ]
    .concat();
    assert_eq!(finds(control), pairs(&expected));
    assert!(
        control
            .commands_seen()
            .iter()
            .filter(|command| is_find(command))
            .all(|command| command.params["uniqueContextId"]
                .as_str()
                .is_some_and(|context| context.ends_with("/util")))
    );
    assert_eq!(
        backend_nodes(control, "DOM.describeNode"),
        [OUTER_OWNER, INNER_OWNER].repeat(3)
    );
    assert_eq!(
        backend_nodes(control, "DOM.scrollIntoViewIfNeeded"),
        [BUTTON, FRAME_BUTTON]
    );
    assert_eq!(sessions(&mouse(control, "mousePressed")), ["S-T1", "S-T1"]);
}

fn auth_challenge(request: &str, origin: &str, source: &str) -> Value {
    json!({
        "requestId": request,
        "request": {"url": format!("{origin}/"), "method": "GET"},
        "frameId": "T1",
        "resourceType": "Document",
        "authChallenge": {"source": source, "origin": origin, "scheme": "basic", "realm": "test"},
    })
}

/// A paused request can arrive before the `Fetch.enable` acknowledgement.
async fn enable_basic_auth(
    logic: &BrowserSetBasicAuthNode,
    context: &mut ExecutionContext,
    control: &mut InMemoryControl,
) -> SentCommand {
    let answer_enable = async {
        let enable = control.wait_for("Fetch.enable", Some("S-T1")).await;
        let paused = json!({"requestId": "one", "request": {"url": "https://allowed.test/"}});
        control.emit("Fetch.requestPaused", Some("S-T1"), paused);
        control.reply(&enable, json!({}));
        enable
    };
    let (enabled, enable) =
        tokio::join!(within("Set Basic Auth", logic.run(context)), answer_enable);
    enabled.expect("Set Basic Auth intercepts authentication");
    enable
}

#[tokio::test]
async fn basic_auth_protocol_never_sends_credentials_to_other_origins_or_proxies() {
    let chrome = Chrome::with_tabs(&["T1"]);
    let mut scripted = Scripted::direct(chrome.clone()).await;
    let logic = Arc::new(BrowserSetBasicAuthNode::new());
    let mut context = context(logic.clone()).await;
    let session = scripted.attach(&mut context).await;
    chrome.hold_fetch_enable();
    set_pins(
        &mut context,
        [
            ("session", json!(session)),
            ("origin", json!("https://allowed.test")),
            ("debugger_address", json!("")),
            ("username", json!("test-user")),
            ("password", json!(SECRET)),
        ],
    )
    .await;
    let enable = enable_basic_auth(&logic, &mut context, &mut scripted.control).await;
    assert_eq!(
        enable.params,
        json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "https://allowed.test/*"}]})
    );
    for (request, origin, source) in [
        ("one", "https://allowed.test", "Server"),
        ("two", "https://allowed.test.evil.test", "Server"),
        ("three", "https://allowed.test", "Proxy"),
        ("one", "https://allowed.test", "Server"),
    ] {
        let challenge = auth_challenge(request, origin, source);
        scripted
            .control
            .emit("Fetch.authRequired", Some("S-T1"), challenge);
    }
    wait_until(
        "4 Fetch.continueWithAuth answers and 1 Fetch.continueRequest",
        || {
            sent(&scripted.control, "Fetch.continueWithAuth").len() >= 4
                && !sent(&scripted.control, "Fetch.continueRequest").is_empty()
        },
    )
    .await;

    assert_credentials_only_for_the_origin(&scripted.control);
    session
        .close(&mut context)
        .await
        .expect("Stop Session closes the browser");
    context.resources.shutdown().await;
}

fn fetch_commands(control: &InMemoryControl) -> Vec<String> {
    control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method.starts_with("Fetch."))
        .map(|command| format!("{} {}", command.method, command.params))
        .collect()
}

fn assert_credentials_only_for_the_origin(control: &InMemoryControl) {
    let continued = sent(control, "Fetch.continueRequest");
    let answers = sent(control, "Fetch.continueWithAuth");
    assert_eq!(
        (continued.len(), answers.len()),
        (1, 4),
        "one continueRequest and four continueWithAuth; Fetch commands sent: {:#?}",
        fetch_commands(control)
    );
    assert_eq!(continued[0].params, json!({"requestId": "one"}));
    assert_eq!(sessions(&answers), ["S-T1"; 4]);
    let provided: Vec<&SentCommand> = answers
        .iter()
        .filter(|answer| answer.params["authChallengeResponse"]["response"] == "ProvideCredentials")
        .collect();
    assert_eq!(provided.len(), 1, "credentials are provided exactly once");
    assert_eq!(
        provided[0].params,
        json!({"requestId": "one", "authChallengeResponse": {
            "response": "ProvideCredentials",
            "username": "test-user",
            "password": SECRET,
        }})
    );
    let mut cancelled: Vec<String> = answers
        .iter()
        .filter(|answer| {
            answer.params["authChallengeResponse"] == json!({"response": "CancelAuth"})
        })
        .filter_map(|answer| answer.params["requestId"].as_str().map(str::to_owned))
        .collect();
    cancelled.sort();
    assert_eq!(cancelled, ["one", "three", "two"]);
    let carriers: Vec<String> = control
        .commands_seen()
        .into_iter()
        .filter(|command| command.params.to_string().contains(SECRET))
        .map(|command| command.method)
        .collect();
    assert_eq!(carriers, ["Fetch.continueWithAuth"]);
}

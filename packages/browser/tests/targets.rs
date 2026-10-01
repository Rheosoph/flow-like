use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use flow_like_browser::test_hooks::TestSetup;
use flow_like_browser::testing::default_auto_reply;
use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport, SentCommand};
use flow_like_browser::types::{FrameId, TargetId};
use flow_like_browser::{Browser, BrowserError, ClosePageOutcome, ConnectionKind, Page};
use serde_json::{Value, json};
use tokio::time::Instant;

const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(5);
const PAGE_SETUP: [&str; 8] = [
    "Page.enable",
    "Page.getFrameTree",
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Page.addScriptToEvaluateOnNewDocument",
    "Emulation.setFocusEmulationEnabled",
    "Target.setAutoAttach",
    "Runtime.runIfWaitingForDebugger",
];
const IFRAME_SETUP: [&str; 7] = [
    "Page.enable",
    "Page.getFrameTree",
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Page.addScriptToEvaluateOnNewDocument",
    "Target.setAutoAttach",
    "Runtime.runIfWaitingForDebugger",
];

type Hold = Box<dyn Fn(&SentCommand) -> bool + Send + Sync>;

struct Fake {
    targets: Vec<Value>,
    titles: HashMap<String, String>,
    load_states: HashMap<String, Value>,
    child_frames: HashMap<String, (String, String)>,
    hold: Hold,
    attaches: usize,
    created: usize,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            titles: HashMap::new(),
            load_states: HashMap::new(),
            child_frames: HashMap::new(),
            hold: Box::new(|_| false),
            attaches: 0,
            created: 0,
        }
    }
}

type SharedFake = Arc<Mutex<Fake>>;

fn lock(fake: &SharedFake) -> std::sync::MutexGuard<'_, Fake> {
    fake.lock().unwrap_or_else(PoisonError::into_inner)
}

fn page_target(id: &str, url: &str, title: &str) -> Value {
    json!({"targetId": id, "type": "page", "url": url, "title": title, "attached": false})
}

fn prerender_target(id: &str, url: &str) -> Value {
    json!({"targetId": id, "type": "page", "subtype": "prerender", "url": url, "title": "", "attached": false})
}

fn target_of(session: &str) -> &str {
    session
        .split_once('-')
        .map_or(session, |(_, target)| target)
}

fn url_of(fake: &Fake, target: &str) -> String {
    fake.targets
        .iter()
        .find(|info| info["targetId"] == target)
        .and_then(|info| info["url"].as_str())
        .unwrap_or("about:blank")
        .to_owned()
}

fn frame_tree(fake: &Fake, session: &str) -> Value {
    let (frame, parent, url) = match fake.child_frames.get(session) {
        Some((frame, parent)) => (frame.clone(), Some(parent.clone()), String::new()),
        None => {
            let target = target_of(session);
            (target.to_owned(), None, url_of(fake, target))
        }
    };
    json!({"frameTree": {
        "frame": {"id": frame, "parentId": parent, "loaderId": format!("L-{frame}"), "url": url},
        "childFrames": [],
    }})
}

fn evaluate(fake: &Fake, command: &SentCommand) -> Option<Value> {
    let session = command.session.as_deref()?;
    let target = target_of(session);
    let expression = command.params["expression"].as_str().unwrap_or_default();
    if expression == "document.title" {
        let title = fake.titles.get(target)?;
        return Some(json!({"result": {"type": "string", "value": title}}));
    }
    if expression.starts_with("({s:") {
        let url = url_of(fake, target);
        let value = fake
            .load_states
            .get(target)
            .cloned()
            .unwrap_or_else(|| json!({"s": "complete", "u": url, "b": url}));
        return Some(json!({"result": {"type": "object", "value": value}}));
    }
    default_auto_reply(command)
}

fn scripted_reply(fake: &SharedFake, command: &SentCommand) -> Option<Value> {
    let mut fake = lock(fake);
    if (fake.hold)(command) {
        return None;
    }
    match command.method.as_str() {
        "Browser.getVersion" => Some(json!({
            "protocolVersion": "1.3",
            "product": "HeadlessChrome/154.0.8037.92",
            "userAgent": "Mozilla/5.0 HeadlessChrome/154.0.0.0",
        })),
        "Target.getTargets" => Some(json!({"targetInfos": fake.targets})),
        "Target.attachToTarget" => {
            fake.attaches += 1;
            let target = command.params["targetId"].as_str().unwrap_or_default();
            Some(json!({"sessionId": format!("S{}-{target}", fake.attaches)}))
        }
        "Target.createTarget" => {
            fake.created += 1;
            let target = format!("N{}", fake.created);
            fake.targets.push(page_target(&target, "about:blank", ""));
            Some(json!({"targetId": target}))
        }
        "Target.setDiscoverTargets"
        | "Target.activateTarget"
        | "Target.detachFromTarget"
        | "Target.closeTarget"
        | "Browser.close"
        | "Browser.setDownloadBehavior"
        | "Security.setIgnoreCertificateErrors" => Some(json!({})),
        "Page.getFrameTree" => Some(frame_tree(&fake, command.session.as_deref()?)),
        "Runtime.evaluate" => evaluate(&fake, command),
        _ => default_auto_reply(command),
    }
}

struct Scripted {
    browser: Browser,
    control: InMemoryControl,
    fake: SharedFake,
    _staging: Option<tempfile::TempDir>,
}

struct Boot {
    kind: ConnectionKind,
    replay: Vec<Value>,
    listed: Vec<Value>,
    run_owned_setup: bool,
}

impl Boot {
    fn new(kind: ConnectionKind, replay: Vec<Value>) -> Self {
        Self {
            kind,
            listed: replay.clone(),
            replay,
            run_owned_setup: false,
        }
    }

    async fn start(self) -> Scripted {
        let (transport, mut control) = InMemoryTransport::new();
        let fake: SharedFake = Arc::new(Mutex::new(Fake {
            targets: self.listed,
            ..Fake::default()
        }));
        let replies = fake.clone();
        control.set_auto_reply(move |command| {
            if command.method == "Target.setDiscoverTargets" {
                return None;
            }
            scripted_reply(&replies, command)
        });
        let staging = self
            .run_owned_setup
            .then(|| tempfile::tempdir().expect("staging dir"));
        let setup = TestSetup {
            kind: self.kind,
            headless: true,
            page_load_timeout: PAGE_LOAD_TIMEOUT,
            process: None,
            staging: staging.as_ref().map(|dir| dir.path().to_path_buf()),
            run_owned_setup: self.run_owned_setup,
        };
        let connecting = tokio::spawn(Browser::connect_transport(Box::new(transport), setup));
        let discover = control.wait_for("Target.setDiscoverTargets", None).await;
        for info in &self.replay {
            control.emit("Target.targetCreated", None, json!({"targetInfo": info}));
        }
        control.reply(&discover, json!({}));
        let browser = finish_construction(&mut control, connecting).await;
        Scripted {
            browser,
            control,
            fake,
            _staging: staging,
        }
    }
}

/// Commands this fake does not script (a sibling lane's owned-setup steps such as the viewport
/// fit) get the -32601 reply of a Chrome without that method.
async fn finish_construction(
    control: &mut InMemoryControl,
    connecting: tokio::task::JoinHandle<flow_like_browser::Result<Browser>>,
) -> Browser {
    let mut connecting = std::pin::pin!(tokio::time::timeout(SETTLE, connecting));
    loop {
        tokio::select! {
            finished = &mut connecting => {
                return finished
                    .expect("construction finishes")
                    .expect("construction task joins")
                    .expect("construction succeeds");
            }
            unscripted = control.next_command() => {
                let message = format!("'{}' wasn't found", unscripted.method);
                control.reply_error(&unscripted, -32601, &message);
            }
        }
    }
}

impl Scripted {
    fn hold(&self, hold: impl Fn(&SentCommand) -> bool + Send + Sync + 'static) {
        lock(&self.fake).hold = Box::new(hold);
    }

    fn seen(&self) -> Vec<SentCommand> {
        self.control.commands_seen()
    }

    async fn eventually(&self, what: &str, done: impl Fn(&[SentCommand]) -> bool) {
        let deadline = Instant::now() + SETTLE;
        loop {
            if done(&self.seen()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what} never happened; commands seen: {:?}",
                describe(&self.seen())
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn page(&self, target: &str) -> Page {
        tokio::time::timeout(SETTLE, self.browser.page(&TargetId::from(target)))
            .await
            .expect("page() finishes")
            .expect("page() attaches")
    }
}

fn describe(seen: &[SentCommand]) -> Vec<String> {
    seen.iter()
        .map(|command| {
            format!(
                "{}@{}",
                command.method,
                command.session.as_deref().unwrap_or("root")
            )
        })
        .collect()
}

fn methods_on(seen: &[SentCommand], session: &str) -> Vec<String> {
    seen.iter()
        .filter(|command| command.session.as_deref() == Some(session))
        .map(|command| command.method.clone())
        .collect()
}

fn sent(seen: &[SentCommand], method: &str) -> usize {
    seen.iter()
        .filter(|command| command.method == method)
        .count()
}

fn session_of(page: &Page) -> String {
    page.session()
        .id()
        .expect("a page has a session")
        .to_string()
}

fn tab_ids(browser: &Browser) -> Vec<String> {
    browser
        .pages()
        .into_iter()
        .map(|info| info.target_id.to_string())
        .collect()
}

fn no_browser_level_auto_attach(seen: &[SentCommand]) -> bool {
    !seen
        .iter()
        .any(|command| command.method == "Target.setAutoAttach" && command.session.is_none())
}

fn position(methods: &[String], method: &str) -> usize {
    methods
        .iter()
        .position(|candidate| candidate == method)
        .unwrap_or_else(|| panic!("{method} was not sent; sent: {methods:?}"))
}

fn first(seen: &[SentCommand], method: &str) -> SentCommand {
    seen.iter()
        .find(|command| command.method == method)
        .cloned()
        .unwrap_or_else(|| panic!("{method} was not sent; sent: {:?}", describe(seen)))
}

async fn one_tab(kind: ConnectionKind) -> Scripted {
    Boot::new(kind, vec![page_target("T1", "http://127.0.0.1/", "One")])
        .start()
        .await
}

fn attach_iframe(scripted: &Scripted, parent: &str, session: &str, frame: &str) {
    lock(&scripted.fake)
        .child_frames
        .insert(session.to_owned(), (frame.to_owned(), "T1".to_owned()));
    scripted.control.emit(
        "Target.attachedToTarget",
        Some(parent),
        json!({
            "sessionId": session,
            "targetInfo": {"targetId": frame, "type": "iframe", "url": "", "attached": true, "parentFrameId": "T1"},
            "waitingForDebugger": true,
        }),
    );
}

fn attached_targets(seen: &[SentCommand]) -> Vec<Value> {
    seen.iter()
        .filter(|command| command.method == "Target.attachToTarget")
        .map(|command| command.params["targetId"].clone())
        .collect()
}

fn detached_sessions(seen: &[SentCommand]) -> usize {
    seen.iter()
        .filter(|command| command.method == "Target.detachFromTarget")
        .count()
}

async fn wait_closed(page: &Page) {
    let deadline = Instant::now() + SETTLE;
    while !page.is_closed() {
        assert!(
            Instant::now() < deadline,
            "the page was never marked closed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_discovery_replay_seeds_the_registry_and_get_targets_fills_only_missing_tabs() {
    let replay = vec![
        page_target("T2", "http://127.0.0.1/two", "Two"),
        page_target("T1", "http://127.0.0.1/one", "One"),
    ];
    let mut boot = Boot::new(ConnectionKind::Launched, replay);
    boot.listed = vec![
        page_target("T3", "http://127.0.0.1/three", "Three"),
        page_target("T1", "http://127.0.0.1/one", "One"),
        prerender_target("P1", "http://127.0.0.1/next"),
        json!({"targetId": "W1", "type": "service_worker", "url": "http://127.0.0.1/sw.js"}),
        json!({"targetId": "U1", "type": "browser_ui", "url": "chrome://omnibox-popup.top-chrome/"}),
        page_target(
            "D1",
            "devtools://devtools/bundled/inspector.html",
            "DevTools",
        ),
        page_target("T2", "http://127.0.0.1/two", "Two"),
    ];
    let scripted = boot.start().await;

    assert_eq!(tab_ids(&scripted.browser), ["T2", "T1", "T3"]);
    let seen = scripted.seen();
    assert!(no_browser_level_auto_attach(&seen));
    assert_eq!(
        sent(&seen, "Target.attachToTarget"),
        0,
        "tabs are attached lazily"
    );
    let discover = seen
        .iter()
        .position(|command| command.method == "Target.setDiscoverTargets")
        .expect("discovery is on");
    let listed = seen
        .iter()
        .position(|command| command.method == "Target.getTargets")
        .expect("targets are listed");
    assert!(discover < listed);
    assert_eq!(
        scripted.browser.version().product,
        "HeadlessChrome/154.0.8037.92"
    );
}

#[tokio::test]
async fn popups_are_appended_in_first_seen_order_and_never_attached() {
    let scripted = Boot::new(
        ConnectionKind::AttachedPort,
        vec![page_target("T1", "http://127.0.0.1/", "Opener")],
    )
    .start()
    .await;
    let mut popup = page_target("T9", "http://127.0.0.1/popup", "Popup");
    popup["openerId"] = json!("T1");
    scripted
        .control
        .emit("Target.targetCreated", None, json!({"targetInfo": popup}));
    scripted.control.emit(
        "Target.targetCreated",
        None,
        json!({"targetInfo": prerender_target("P2", "http://127.0.0.1/prefetched")}),
    );
    let deadline = Instant::now() + SETTLE;
    while scripted.browser.pages().len() < 2 {
        assert!(Instant::now() < deadline, "the popup was never registered");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pages = scripted.browser.pages();
    assert_eq!(tab_ids(&scripted.browser), ["T1", "T9"]);
    assert_eq!(pages[1].opener_id, Some(TargetId::from("T1")));
    assert_eq!(pages[1].title, "Popup");
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 0);
}

#[tokio::test]
async fn a_lazy_attach_enqueues_the_whole_setup_before_resuming() {
    let scripted = one_tab(ConnectionKind::Launched).await;
    let page = scripted.page("T1").await;
    let session = session_of(&page);
    scripted
        .eventually("the utility world request", |seen| {
            methods_on(seen, &session).contains(&"Page.createIsolatedWorld".to_owned())
        })
        .await;

    let methods = methods_on(&scripted.seen(), &session);
    assert_eq!(methods[..8], PAGE_SETUP);
    let seen = scripted.seen();
    assert_eq!(
        first(&seen, "Target.setAutoAttach").params,
        json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true})
    );
    let world = first(&seen, "Page.createIsolatedWorld");
    assert_eq!(world.params["frameId"], "T1");
    assert_eq!(world.params["grantUniveralAccess"], true);
    assert!(no_browser_level_auto_attach(&seen));
    assert_eq!(page.target_id().as_str(), "T1");
    assert_eq!(
        page.main_frame()
            .loader_id()
            .map(|loader| loader.to_string()),
        Some("L-T1".to_owned()),
        "the getFrameTree reply seeds the main frame"
    );
    assert!(
        position(&methods, "Runtime.evaluate")
            > position(&methods, "Runtime.runIfWaitingForDebugger"),
        "the readyState probe runs after the resume"
    );
}

#[tokio::test]
async fn attached_kinds_resume_and_probe_before_enabling_any_domain() {
    let scripted = one_tab(ConnectionKind::AttachedApproval).await;
    let page = scripted.page("T1").await;
    let methods = methods_on(&scripted.seen(), &session_of(&page));
    assert_eq!(
        methods[..2],
        ["Runtime.runIfWaitingForDebugger", "Runtime.evaluate"]
    );
    assert!(position(&methods, "Runtime.evaluate") < position(&methods, "Page.enable"));
    assert_eq!(sent(&scripted.seen(), "Target.activateTarget"), 0);
}

#[tokio::test(start_paused = true)]
async fn a_silent_tab_is_revived_with_activate_target_before_setup() {
    let scripted = one_tab(ConnectionKind::AttachedPort).await;
    let probes = Arc::new(Mutex::new(0_usize));
    let counted = probes.clone();
    scripted.hold(move |command| {
        if command.method != "Runtime.evaluate" || command.params["expression"] != "1" {
            return false;
        }
        let mut probes = counted.lock().unwrap_or_else(PoisonError::into_inner);
        *probes += 1;
        *probes == 1
    });
    let page = tokio::time::timeout(
        Duration::from_secs(30),
        scripted.browser.page(&TargetId::from("T1")),
    )
    .await
    .expect("revival finishes")
    .expect("the revived tab attaches");
    let seen = scripted.seen();
    let order: Vec<String> = seen.iter().map(|command| command.method.clone()).collect();
    assert!(position(&order, "Target.activateTarget") < position(&order, "Page.enable"));
    assert_eq!(*probes.lock().unwrap_or_else(PoisonError::into_inner), 2);
    assert!(!page.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_tab_that_never_answers_is_reported_and_detached_without_enabling_domains() {
    let scripted = one_tab(ConnectionKind::AttachedApproval).await;
    scripted.hold(|command| {
        (command.method == "Runtime.evaluate" && command.params["expression"] == "1")
            || command.method == "Page.enable"
    });
    let started = Instant::now();
    let error = scripted
        .browser
        .page(&TargetId::from("T1"))
        .await
        .err()
        .expect("a silent tab cannot be attached");
    assert_eq!(
        error.to_string(),
        "The tab is not responding (it may be discarded, frozen, or blocked by a dialog)"
    );
    assert!(started.elapsed() >= Duration::from_secs(10));
    let seen = scripted.seen();
    assert_eq!(
        sent(&seen, "Page.enable"),
        0,
        "no domain is enabled before the probe"
    );
    assert_eq!(sent(&seen, "Target.activateTarget"), 1);
    scripted
        .eventually("the release of the probe session", |seen| {
            seen.iter().any(|command| {
                command.method == "Target.detachFromTarget"
                    && command.params["sessionId"] == "S1-T1"
            })
        })
        .await;
}

#[tokio::test]
async fn a_policy_refusal_is_reported_and_not_cached() {
    let mut scripted = Boot::new(
        ConnectionKind::AttachedPort,
        vec![page_target(
            "T1",
            "chrome-extension://blocked/page.html",
            "Blocked",
        )],
    )
    .start()
    .await;
    scripted.hold(|command| command.method == "Target.attachToTarget");
    let browser = scripted.browser.clone();
    let first = tokio::spawn(async move { browser.page(&TargetId::from("T1")).await });
    let attach = scripted
        .control
        .wait_for("Target.attachToTarget", None)
        .await;
    scripted.control.reply_error(&attach, -32000, "Not allowed");
    let error = first.await.expect("join").err().expect("refused");
    assert!(matches!(error, BrowserError::NoSuchPage { .. }));
    assert_eq!(
        error.to_string(),
        "This tab is protected by browser policy; choose another tab"
    );
    assert_eq!(tab_ids(&scripted.browser), ["T1"], "the tab stays listed");

    scripted.hold(|_| false);
    let page = scripted.page("T1").await;
    assert_eq!(page.target_id().as_str(), "T1");
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 2);
}

#[tokio::test]
async fn concurrent_page_calls_attach_once() {
    let scripted = one_tab(ConnectionKind::Launched).await;
    let target = TargetId::from("T1");
    let (first, second) = tokio::join!(
        scripted.browser.page(&target),
        scripted.browser.page(&target)
    );
    let (first, second) = (first.expect("first"), second.expect("second"));
    assert_eq!(session_of(&first), session_of(&second));
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 1);
    let again = scripted.page("T1").await;
    assert_eq!(session_of(&again), session_of(&first));
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 1);
}

#[tokio::test]
async fn a_cancelled_page_call_finishes_the_attach_in_the_background() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    scripted.hold(|command| command.method == "Runtime.enable");
    let cancelled = tokio::time::timeout(
        Duration::from_millis(200),
        scripted.browser.page(&TargetId::from("T1")),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "the held Runtime.enable keeps page() waiting"
    );
    let held = scripted
        .control
        .wait_for("Runtime.enable", Some("S1-T1"))
        .await;
    scripted.hold(|_| false);
    scripted.control.reply(&held, json!({}));

    let page = scripted.page("T1").await;
    assert_eq!(session_of(&page), "S1-T1");
    let seen = scripted.seen();
    assert_eq!(sent(&seen, "Target.attachToTarget"), 1);
    assert_eq!(detached_sessions(&seen), 0);

    attach_iframe(&scripted, "S1-T1", "S9-F1", "F1");
    scripted
        .eventually("the child setup on the kept session", |seen| {
            methods_on(seen, "S9-F1").contains(&"Runtime.runIfWaitingForDebugger".to_owned())
        })
        .await;
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 1);
}

#[tokio::test]
async fn a_session_lost_during_the_ready_state_probe_fails_the_attach() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    scripted.hold(|command| {
        command.method == "Runtime.evaluate"
            && command.params["expression"]
                .as_str()
                .is_some_and(|expression| expression.starts_with("({s:"))
    });
    let browser = scripted.browser.clone();
    let attaching = tokio::spawn(async move { browser.page(&TargetId::from("T1")).await });
    let probe = scripted
        .control
        .wait_for("Runtime.evaluate", Some("S1-T1"))
        .await;
    scripted.control.emit(
        "Target.detachedFromTarget",
        None,
        json!({"sessionId": "S1-T1", "targetId": "T1"}),
    );
    scripted.control.reply(
        &probe,
        json!({"result": {"type": "object", "value": {"s": "complete", "u": "http://127.0.0.1/", "b": "http://127.0.0.1/"}}}),
    );
    let error = tokio::time::timeout(SETTLE, attaching)
        .await
        .expect("page() finishes")
        .expect("join")
        .err()
        .expect("a page whose session died is never returned");
    assert_eq!(
        error.to_string(),
        "The current browser tab was closed; use Select Tab or New Page"
    );

    scripted.hold(|_| false);
    let again = scripted.page("T1").await;
    assert!(!again.is_closed());
    assert_eq!(session_of(&again), "S2-T1");
}

#[tokio::test(start_paused = true)]
async fn a_dialog_during_setup_still_seeds_the_frames_once_the_replies_drain() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    scripted.hold(|command| command.method == "Runtime.enable");
    let browser = scripted.browser.clone();
    let attaching = tokio::spawn(async move { browser.page(&TargetId::from("T1")).await });
    scripted
        .control
        .wait_for("Runtime.enable", Some("S1-T1"))
        .await;
    scripted.control.emit(
        "Page.javascriptDialogOpening",
        Some("S1-T1"),
        json!({"url": "http://127.0.0.1/", "message": "Hold on", "type": "alert", "hasBrowserHandler": false, "defaultPrompt": ""}),
    );
    let page = attaching
        .await
        .expect("join")
        .expect("the dialog completes the attach");
    assert!(page.pending_dialog().is_some());

    tokio::time::sleep(Duration::from_secs(40)).await;
    assert_eq!(
        page.main_frame()
            .loader_id()
            .map(|loader| loader.to_string()),
        Some("L-T1".to_owned()),
        "the drained getFrameTree reply seeds the main frame"
    );
    assert!(methods_on(&scripted.seen(), "S1-T1").contains(&"Page.createIsolatedWorld".to_owned()));
    assert!(!page.is_closed());
}

#[tokio::test]
async fn a_root_detach_of_the_page_session_closes_the_page_and_the_next_call_reattaches() {
    let scripted = one_tab(ConnectionKind::AttachedPort).await;
    let page = scripted.page("T1").await;
    let session = session_of(&page);
    scripted.control.emit(
        "Target.detachedFromTarget",
        None,
        json!({"sessionId": session, "targetId": "T1"}),
    );
    wait_closed(&page).await;
    assert_eq!(tab_ids(&scripted.browser), ["T1"]);

    let again = scripted.page("T1").await;
    assert!(!again.is_closed());
    assert_ne!(session_of(&again), session);
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 2);
}

#[tokio::test]
async fn a_destroyed_page_reports_the_closed_tab() {
    let scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/", "One"),
            page_target("T2", "http://127.0.0.1/two", "Two"),
        ],
    )
    .start()
    .await;
    let page = scripted.page("T1").await;
    scripted
        .control
        .emit("Target.targetDestroyed", None, json!({"targetId": "T1"}));
    wait_closed(&page).await;
    assert_eq!(tab_ids(&scripted.browser), ["T2"]);
    let error = scripted
        .browser
        .page(&TargetId::from("T1"))
        .await
        .err()
        .expect("the tab is gone");
    assert_eq!(
        error.to_string(),
        "The current browser tab was closed; use Select Tab or New Page"
    );
}

#[tokio::test]
async fn iframe_children_are_set_up_and_resumed_without_focus_emulation() {
    let scripted = one_tab(ConnectionKind::Launched).await;
    let page = scripted.page("T1").await;
    let parent = session_of(&page);
    attach_iframe(&scripted, &parent, "S2", "F2");
    scripted
        .eventually("the child resume", |seen| {
            methods_on(seen, "S2").contains(&"Runtime.runIfWaitingForDebugger".to_owned())
        })
        .await;
    let methods = methods_on(&scripted.seen(), "S2");
    assert_eq!(methods[..7], IFRAME_SETUP);
    assert!(!methods.contains(&"Emulation.setFocusEmulationEnabled".to_owned()));
    let child = page
        .frame(&FrameId::from("F2"))
        .expect("the OOPIF frame is known");
    assert!(!child.is_main());
    scripted
        .eventually("the child frame seed", |_| child.loader_id().is_some())
        .await;
}

#[tokio::test]
async fn service_workers_and_idle_dedicated_workers_are_resumed_and_detached() {
    let scripted = one_tab(ConnectionKind::Launched).await;
    let page = scripted.page("T1").await;
    let parent = session_of(&page);
    for (session, kind) in [
        ("SW1", "service_worker"),
        ("W1", "worker"),
        ("B1", "browser_ui"),
    ] {
        scripted.control.emit(
            "Target.attachedToTarget",
            Some(&parent),
            json!({
                "sessionId": session,
                "targetInfo": {"targetId": format!("X{session}"), "type": kind, "url": "http://127.0.0.1/w.js", "attached": true},
                "waitingForDebugger": true,
            }),
        );
    }
    let detached = |seen: &[SentCommand], child: &str| {
        seen.iter().any(|command| {
            command.method == "Target.detachFromTarget"
                && command.session.as_deref() == Some(parent.as_str())
                && command.params["sessionId"] == child
        })
    };
    scripted
        .eventually("every worker detach", |seen| {
            ["SW1", "W1", "B1"]
                .iter()
                .all(|child| detached(seen, child))
        })
        .await;
    let seen = scripted.seen();
    for child in ["SW1", "W1", "B1"] {
        assert_eq!(
            methods_on(&seen, child),
            ["Runtime.runIfWaitingForDebugger"]
        );
    }
}

#[tokio::test]
async fn a_detach_first_swap_round_trips_before_the_frame_is_resolved() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    let page = scripted.page("T1").await;
    let parent = session_of(&page);
    attach_iframe(&scripted, &parent, "S2", "F2");
    scripted
        .eventually("the child setup", |seen| {
            methods_on(seen, "S2").contains(&"Page.createIsolatedWorld".to_owned())
        })
        .await;
    let round_trip_parent = parent.clone();
    scripted.hold(move |command| {
        command.method == "Page.enable"
            && command.session.as_deref() == Some(round_trip_parent.as_str())
    });
    scripted.control.emit(
        "Target.detachedFromTarget",
        Some(&parent),
        json!({"sessionId": "S2", "targetId": "F2"}),
    );
    let round_trip = scripted
        .control
        .wait_for("Page.enable", Some(&parent))
        .await;
    scripted.control.emit(
        "Page.frameAttached",
        Some(&parent),
        json!({"frameId": "F2", "parentFrameId": "T1"}),
    );
    scripted.control.reply(&round_trip, json!({}));
    let frame = FrameId::from("F2");
    let deadline = Instant::now() + SETTLE;
    loop {
        let swapped = page
            .frame(&frame)
            .is_some_and(|child| child.loader_id().is_none());
        if swapped {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the swapped-back frame never settled"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(page.frame(&frame).is_some(), "a swap back keeps the frame");
}

#[tokio::test]
async fn a_prerender_activation_binds_a_successor_at_the_old_position() {
    let scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/", "One"),
            page_target("T2", "http://127.0.0.1/two", "Two"),
            prerender_target("R1", "http://127.0.0.1/next"),
        ],
    )
    .start()
    .await;
    assert_eq!(tab_ids(&scripted.browser), ["T1", "T2"]);
    let page = scripted.page("T1").await;
    scripted.control.emit(
        "Target.detachedFromTarget",
        None,
        json!({"sessionId": session_of(&page), "targetId": "T1"}),
    );
    wait_closed(&page).await;
    let browser = scripted.browser.clone();
    let waiting = tokio::spawn(async move { browser.page(&TargetId::from("T1")).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    scripted.control.emit(
        "Target.targetInfoChanged",
        None,
        json!({"targetInfo": page_target("R1", "http://127.0.0.1/next", "Next")}),
    );
    let successor = tokio::time::timeout(SETTLE, waiting)
        .await
        .expect("the binding resolves within the window")
        .expect("join")
        .expect("page(T1) follows the successor");
    assert_eq!(successor.target_id().as_str(), "R1");
    assert_eq!(tab_ids(&scripted.browser), ["R1", "T2"]);
    assert_eq!(
        attached_targets(&scripted.seen()),
        [json!("T1"), json!("R1")]
    );
}

#[tokio::test]
async fn a_destroyed_page_binds_a_prerender_that_activated_just_before() {
    let scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/", "One"),
            prerender_target("R1", "http://127.0.0.1/next"),
            page_target("T2", "http://127.0.0.1/two", "Two"),
        ],
    )
    .start()
    .await;
    let page = scripted.page("T1").await;
    scripted.control.emit(
        "Target.targetInfoChanged",
        None,
        json!({"targetInfo": page_target("R1", "http://127.0.0.1/next", "Next")}),
    );
    scripted
        .control
        .emit("Target.targetDestroyed", None, json!({"targetId": "T1"}));
    wait_closed(&page).await;
    let successor = scripted.page("T1").await;
    assert_eq!(successor.target_id().as_str(), "R1");
    assert_eq!(tab_ids(&scripted.browser), ["R1", "T2"]);
}

#[tokio::test]
async fn an_ambiguous_prerender_replacement_is_reported() {
    let scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/", "One"),
            prerender_target("R1", "http://127.0.0.1/a"),
            prerender_target("R2", "http://127.0.0.1/b"),
        ],
    )
    .start()
    .await;
    let page = scripted.page("T1").await;
    for (id, url) in [("R1", "http://127.0.0.1/a"), ("R2", "http://127.0.0.1/b")] {
        scripted.control.emit(
            "Target.targetInfoChanged",
            None,
            json!({"targetInfo": page_target(id, url, "")}),
        );
    }
    scripted
        .control
        .emit("Target.targetDestroyed", None, json!({"targetId": "T1"}));
    wait_closed(&page).await;
    let error = tokio::time::timeout(SETTLE, scripted.browser.page(&TargetId::from("T1")))
        .await
        .expect("the window closes")
        .err()
        .expect("no unique successor");
    assert_eq!(
        error.to_string(),
        "The current tab was replaced by a prerendered page; use Select Tab"
    );
}

#[tokio::test]
async fn new_page_creates_a_blank_target_and_attaches_it() {
    let scripted = one_tab(ConnectionKind::Launched).await;
    let page = tokio::time::timeout(SETTLE, scripted.browser.new_page())
        .await
        .expect("new_page finishes")
        .expect("new_page attaches");
    assert_eq!(page.target_id().as_str(), "N1");
    let create = scripted
        .seen()
        .into_iter()
        .find(|command| command.method == "Target.createTarget")
        .expect("createTarget");
    assert_eq!(create.params, json!({"url": "about:blank"}));
    assert_eq!(tab_ids(&scripted.browser), ["T1", "N1"]);
    scripted.control.emit(
        "Target.targetCreated",
        None,
        json!({"targetInfo": page_target("N1", "about:blank", "")}),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        tab_ids(&scripted.browser),
        ["T1", "N1"],
        "no duplicate entry"
    );
}

async fn close_and_confirm(scripted: &mut Scripted, target: &str) -> ClosePageOutcome {
    scripted.hold(|command| command.method == "Target.closeTarget");
    let browser = scripted.browser.clone();
    let id = TargetId::from(target);
    let closing = tokio::spawn(async move { browser.close_page(&id).await });
    let close = scripted.control.wait_for("Target.closeTarget", None).await;
    assert_eq!(close.params["targetId"], target);
    scripted.control.reply(&close, json!({"success": true}));
    scripted
        .control
        .emit("Target.targetDestroyed", None, json!({"targetId": target}));
    tokio::time::timeout(SETTLE, closing)
        .await
        .expect("close_page finishes")
        .expect("join")
        .expect("close_page succeeds")
}

#[tokio::test]
async fn closing_a_page_falls_back_to_the_oldest_remaining_page() {
    let mut scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/1", "One"),
            page_target("T2", "http://127.0.0.1/2", "Two"),
            page_target("T3", "http://127.0.0.1/3", "Three"),
        ],
    )
    .start()
    .await;
    let page = scripted.page("T2").await;
    let outcome = close_and_confirm(&mut scripted, "T2").await;
    assert_eq!(
        outcome,
        ClosePageOutcome::Remaining {
            next: TargetId::from("T1")
        }
    );
    assert!(page.is_closed());
    assert_eq!(tab_ids(&scripted.browser), ["T1", "T3"]);
    assert!(scripted.browser.is_alive());
}

#[tokio::test]
async fn a_refused_close_keeps_the_tab_listed_and_attachable() {
    let mut scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "http://127.0.0.1/1", "One"),
            page_target("T2", "http://127.0.0.1/2", "Two"),
        ],
    )
    .start()
    .await;
    let page = scripted.page("T2").await;
    scripted.hold(|command| command.method == "Target.closeTarget");
    let browser = scripted.browser.clone();
    let closing = tokio::spawn(async move { browser.close_page(&TargetId::from("T2")).await });
    let close = scripted.control.wait_for("Target.closeTarget", None).await;
    scripted
        .control
        .reply_error(&close, -32000, "Failed to close target");
    let error = tokio::time::timeout(SETTLE, closing)
        .await
        .expect("close_page finishes")
        .expect("join")
        .expect_err("the refusal is reported");
    assert!(matches!(error, BrowserError::Protocol { .. }), "{error:?}");
    assert_eq!(tab_ids(&scripted.browser), ["T1", "T2"]);
    assert!(!page.is_closed());
    scripted.hold(|_| false);
    let again = scripted.page("T2").await;
    assert_eq!(session_of(&again), session_of(&page));
}

#[tokio::test]
async fn closing_the_last_page_of_a_launched_browser_closes_the_browser() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    let outcome = close_and_confirm(&mut scripted, "T1").await;
    assert_eq!(
        outcome,
        ClosePageOutcome::LastPageClosed {
            browser_closed: true
        }
    );
    assert_eq!(sent(&scripted.seen(), "Browser.close"), 1);
    assert!(!scripted.browser.is_alive());
    assert!(
        scripted.browser.close().await.is_ok(),
        "close is idempotent"
    );
    assert_eq!(sent(&scripted.seen(), "Browser.close"), 1);
}

#[tokio::test]
async fn a_browser_that_exits_with_its_last_tab_reports_the_close() {
    let mut scripted = one_tab(ConnectionKind::Launched).await;
    scripted.hold(|command| command.method == "Target.closeTarget");
    let browser = scripted.browser.clone();
    let closing = tokio::spawn(async move { browser.close_page(&TargetId::from("T1")).await });
    scripted.control.wait_for("Target.closeTarget", None).await;
    scripted
        .control
        .close("the browser exited with its last window");
    let outcome = tokio::time::timeout(SETTLE, closing)
        .await
        .expect("close_page finishes")
        .expect("join")
        .expect("an exited browser still reports the close");
    assert_eq!(
        outcome,
        ClosePageOutcome::LastPageClosed {
            browser_closed: true
        }
    );
    assert!(!scripted.browser.is_alive());
}

#[tokio::test]
async fn closing_the_last_page_of_an_attached_browser_only_disconnects() {
    let mut scripted = one_tab(ConnectionKind::AttachedPort).await;
    let outcome = close_and_confirm(&mut scripted, "T1").await;
    assert_eq!(
        outcome,
        ClosePageOutcome::LastPageClosed {
            browser_closed: false
        }
    );
    assert_eq!(sent(&scripted.seen(), "Browser.close"), 0);
    assert!(!scripted.browser.is_alive());
    assert!(scripted.browser.connection().is_closed());
}

async fn titled_tabs() -> Scripted {
    let scripted = Boot::new(
        ConnectionKind::Launched,
        vec![
            page_target("T1", "about:blank", "about:blank"),
            page_target("T2", "http://127.0.0.1/held", "held target title"),
            page_target("T3", "http://127.0.0.1/cold", "cold target title"),
            page_target("T4", "chrome://settings/", "Settings"),
            page_target("T5", "http://127.0.0.1/slow", "slow target title"),
        ],
    )
    .start()
    .await;
    {
        let mut fake = lock(&scripted.fake);
        fake.titles
            .insert("T2".to_owned(), "Held document".to_owned());
        fake.titles
            .insert("T3".to_owned(), "Cold document".to_owned());
    }
    scripted
}

#[tokio::test]
async fn launched_titles_use_temporary_attaches_within_one_budget() {
    let scripted = titled_tabs().await;
    let held = scripted.page("T2").await;
    let budget = Duration::from_millis(500);
    let started = Instant::now();
    let pages = scripted.browser.list_pages(budget).await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < budget + Duration::from_millis(400),
        "one budget for all probes: {elapsed:?}"
    );
    let titles: Vec<&str> = pages.iter().map(|info| info.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "",
            "Held document",
            "Cold document",
            "Settings",
            "slow target title"
        ]
    );
    assert_eq!(
        attached_targets(&scripted.seen()),
        [json!("T2"), json!("T3"), json!("T5")],
        "T2 once for the held page, T3 and T5 temporarily, never the internal T4"
    );
    assert!(!held.is_closed());
    scripted
        .eventually("the temporary sessions are released", |seen| {
            detached_sessions(seen) == 2
        })
        .await;
}

#[tokio::test]
async fn attached_titles_never_attach_tabs() {
    let scripted = Boot::new(
        ConnectionKind::AttachedApproval,
        vec![
            page_target("T1", "http://127.0.0.1/a", "Target title A"),
            page_target("T2", "http://127.0.0.1/b", "Target title B"),
        ],
    )
    .start()
    .await;
    lock(&scripted.fake)
        .titles
        .insert("T2".to_owned(), "Document B".to_owned());
    scripted.page("T2").await;
    let pages = scripted.browser.list_pages(Duration::from_secs(2)).await;
    let titles: Vec<&str> = pages.iter().map(|info| info.title.as_str()).collect();
    assert_eq!(titles, ["Target title A", "Document B"]);
    assert_eq!(sent(&scripted.seen(), "Target.attachToTarget"), 1);
}

#[tokio::test]
async fn owned_setup_attaches_the_first_page_during_construction() {
    let mut boot = Boot::new(
        ConnectionKind::Launched,
        vec![page_target("T1", "data:,", "")],
    );
    boot.run_owned_setup = true;
    let scripted = boot.start().await;
    let seen = scripted.seen();
    let attach = seen
        .iter()
        .find(|command| command.method == "Target.attachToTarget")
        .expect("the first page is attached");
    assert_eq!(attach.params["targetId"], "T1");
    assert!(scripted.browser.is_owned());
    assert!(scripted.browser.is_alive());
}

#[tokio::test]
async fn owned_setup_configures_downloads_before_attaching_the_first_page() {
    let mut boot = Boot::new(
        ConnectionKind::Launched,
        vec![page_target("T1", "data:,", "")],
    );
    boot.run_owned_setup = true;
    let scripted = boot.start().await;
    let order: Vec<String> = scripted
        .seen()
        .iter()
        .filter(|command| command.session.is_none())
        .map(|command| command.method.clone())
        .collect();
    assert!(
        position(&order, "Browser.setDownloadBehavior") < position(&order, "Target.attachToTarget")
    );
    assert_eq!(
        first(&scripted.seen(), "Browser.setDownloadBehavior").params["behavior"],
        "allowAndName"
    );
}

#[tokio::test(start_paused = true)]
async fn a_browser_that_dies_before_its_first_page_reports_the_disconnect() {
    let (transport, control) = InMemoryTransport::new();
    control.set_auto_reply(|command| match command.method.as_str() {
        "Browser.getVersion" => Some(json!({"product": "HeadlessChrome/154.0.8037.92"})),
        "Target.getTargets" => Some(json!({"targetInfos": []})),
        "Target.setDiscoverTargets" | "Browser.setDownloadBehavior" => Some(json!({})),
        _ => default_auto_reply(command),
    });
    let staging = tempfile::tempdir().expect("staging dir");
    let setup = TestSetup {
        kind: ConnectionKind::Launched,
        headless: true,
        page_load_timeout: PAGE_LOAD_TIMEOUT,
        process: None,
        staging: Some(staging.path().to_path_buf()),
        run_owned_setup: true,
    };
    let connecting = tokio::spawn(Browser::connect_transport(Box::new(transport), setup));
    let started = Instant::now();
    while !control
        .commands_seen()
        .iter()
        .any(|command| command.method == "Target.getTargets")
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    control.close("the browser exited during startup");
    let error = connecting
        .await
        .expect("join")
        .err()
        .expect("construction fails");
    assert!(
        matches!(error, BrowserError::Disconnected { .. }),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn aborting_an_attached_browser_disconnects_once() {
    let scripted = one_tab(ConnectionKind::AttachedPort).await;
    scripted.browser.abort();
    scripted.browser.abort();
    assert!(!scripted.browser.is_alive());
    assert!(scripted.browser.connection().is_closed());
    assert_eq!(sent(&scripted.seen(), "Browser.close"), 0);
    assert!(scripted.browser.close().await.is_ok());
}

#[cfg(unix)]
mod process {
    use super::*;
    use flow_like_browser::test_hooks::{BrowserProcess, Profile, adopt_process};

    fn running(pid: u32) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        !state.is_empty() && !state.starts_with('Z')
    }

    struct Adopted {
        scripted: Scripted,
        pid: u32,
        profile: std::path::PathBuf,
        staging: std::path::PathBuf,
        _root: tempfile::TempDir,
    }

    fn sleeper() -> tokio::process::Child {
        tokio::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("sleep starts")
    }

    async fn launched_with(process: BrowserProcess, staging: std::path::PathBuf) -> Scripted {
        let (transport, control) = InMemoryTransport::new();
        let fake: SharedFake = Arc::new(Mutex::new(Fake {
            targets: vec![page_target("T1", "http://127.0.0.1/", "One")],
            ..Fake::default()
        }));
        let replies = fake.clone();
        control.set_auto_reply(move |command| scripted_reply(&replies, command));
        let setup = TestSetup {
            kind: ConnectionKind::Launched,
            headless: true,
            page_load_timeout: PAGE_LOAD_TIMEOUT,
            process: Some(process),
            staging: Some(staging),
            run_owned_setup: false,
        };
        let connecting = Browser::connect_transport(Box::new(transport), setup);
        let browser = tokio::time::timeout(SETTLE, connecting)
            .await
            .expect("construction finishes")
            .expect("construction succeeds");
        Scripted {
            browser,
            control,
            fake,
            _staging: None,
        }
    }

    async fn adopted(temporary: bool) -> Adopted {
        let root = tempfile::tempdir().expect("temp dir");
        let profile = root.path().join("profile");
        let staging = root.path().join("staging");
        std::fs::create_dir_all(&profile).expect("profile dir");
        std::fs::create_dir_all(&staging).expect("staging dir");
        let child = sleeper();
        let pid = child.id().expect("pid");
        let owned = Profile {
            dir: profile.clone(),
            temporary,
        };
        let process = adopt_process(child, owned, Some(staging.clone()));
        Adopted {
            scripted: launched_with(process, staging.clone()).await,
            pid,
            profile,
            staging,
            _root: root,
        }
    }

    async fn wait_gone(pid: u32, within: Duration) {
        let deadline = Instant::now() + within;
        while running(pid) {
            assert!(Instant::now() < deadline, "process {pid} is still running");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn closing_an_already_disconnected_browser_still_cleans_up() {
        let adopted = adopted(true).await;
        adopted.scripted.control.close("the browser went away");
        let deadline = Instant::now() + SETTLE;
        while !adopted.scripted.browser.connection().is_closed() {
            assert!(
                Instant::now() < deadline,
                "the disconnect was never noticed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!adopted.scripted.browser.is_alive());
        let closed =
            tokio::time::timeout(Duration::from_secs(15), adopted.scripted.browser.close())
                .await
                .expect("close is bounded");
        assert!(
            closed.is_ok(),
            "an exited browser closes cleanly: {closed:?}"
        );
        wait_gone(adopted.pid, SETTLE).await;
        assert!(
            !adopted.profile.exists(),
            "the temporary profile is removed"
        );
        assert!(!adopted.staging.exists(), "the staging dir is removed");
    }

    async fn wait_removed(dir: &std::path::Path, within: Duration) {
        let deadline = Instant::now() + within;
        while dir.exists() {
            assert!(
                Instant::now() < deadline,
                "{} is still there",
                dir.display()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn aborting_a_temporary_profile_kills_at_once_and_the_drop_removes_the_profile() {
        let Adopted {
            scripted,
            pid,
            profile,
            staging,
            _root,
        } = adopted(true).await;
        scripted.browser.abort();
        wait_gone(pid, Duration::from_secs(2)).await;
        assert!(!scripted.browser.is_alive());
        assert_eq!(sent(&scripted.seen(), "Browser.close"), 0);
        let Scripted { browser, .. } = scripted;
        drop(browser);
        assert!(!profile.exists());
        assert!(!staging.exists());
    }

    #[tokio::test]
    async fn aborting_a_persistent_profile_asks_the_browser_to_close_and_arms_the_watchdog() {
        let adopted = adopted(false).await;
        adopted.scripted.browser.abort();
        adopted
            .scripted
            .eventually("the best-effort Browser.close", |seen| {
                sent(seen, "Browser.close") == 1
            })
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(running(adopted.pid), "the browser gets its grace period");
        wait_gone(adopted.pid, Duration::from_secs(6)).await;
        assert!(
            adopted.profile.exists(),
            "a persistent profile is never removed"
        );
    }

    #[tokio::test]
    async fn a_dropped_browser_keeps_the_abort_grace_of_a_persistent_profile() {
        let Adopted {
            scripted,
            pid,
            profile,
            staging,
            _root,
        } = adopted(false).await;
        scripted.browser.abort();
        let Scripted {
            browser, control, ..
        } = scripted;
        drop(browser);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(running(pid), "dropping the browser keeps the grace period");
        assert_eq!(sent(&control.commands_seen(), "Browser.close"), 1);
        wait_gone(pid, Duration::from_secs(6)).await;
        wait_removed(&staging, SETTLE).await;
        assert!(profile.exists(), "a persistent profile is never removed");
    }

    #[tokio::test]
    async fn a_cancelled_close_keeps_the_shutdown_grace_of_a_persistent_profile() {
        let Adopted {
            scripted,
            pid,
            profile,
            staging,
            _root,
        } = adopted(false).await;
        let Scripted {
            browser, control, ..
        } = scripted;
        let connection = browser.connection().clone();
        let closing = tokio::time::timeout(Duration::from_millis(300), browser.close()).await;
        assert!(closing.is_err(), "close waits for the browser to exit");
        assert!(sent(&control.commands_seen(), "Browser.close") >= 1);
        drop(browser);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(running(pid), "a cancelled close keeps the grace period");
        assert!(connection.is_closed(), "a cancelled close disconnects");
        wait_gone(pid, Duration::from_secs(6)).await;
        wait_removed(&staging, SETTLE).await;
        assert!(profile.exists(), "a persistent profile is never removed");
    }

    #[tokio::test]
    async fn a_cancelled_close_of_a_temporary_profile_kills_at_once() {
        let adopted = adopted(true).await;
        let browser = &adopted.scripted.browser;
        let closing = tokio::time::timeout(Duration::from_millis(300), browser.close()).await;
        assert!(closing.is_err(), "close waits for the browser to exit");
        wait_gone(adopted.pid, Duration::from_secs(2)).await;
        assert!(browser.connection().is_closed());
    }
}

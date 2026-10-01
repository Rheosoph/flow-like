// Derived from agent-browser cli/src/native/cdp/client.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like_browser::connection::{
    Connection, ConnectionOptions, EventHook, MethodMatch, PendingReply, RouteFilter,
    RouteReceiver, SessionScope,
};
use flow_like_browser::event_log::{Event, EventCursor, EventLogLimits};
use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport};
use flow_like_browser::transport::{MAX_OUTBOUND_BYTES, WriteStatus};
use flow_like_browser::types::{SessionId, TargetType};
use flow_like_browser::{BrowserError, ErrorClass};
use futures_util::FutureExt;
use serde_json::json;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(5);

const ALLOWED_AFTER_CRASH: [&str; 9] = [
    "Page.reload",
    "Page.navigate",
    "Page.getNavigationHistory",
    "Page.navigateToHistoryEntry",
    "Page.stopLoading",
    "Page.handleJavaScriptDialog",
    "Target.getTargetInfo",
    "Runtime.runIfWaitingForDebugger",
    "Inspector.enable",
];

fn connect() -> (Connection, InMemoryControl) {
    connect_with(ConnectionOptions::default())
}

fn connect_with(options: ConnectionOptions) -> (Connection, InMemoryControl) {
    let (transport, control) = InMemoryTransport::new();
    (Connection::start(Box::new(transport), options), control)
}

fn connect_with_log(max_events: usize) -> (Connection, InMemoryControl) {
    connect_with(ConnectionOptions {
        default_timeout: Duration::from_secs(30),
        event_log: EventLogLimits {
            max_events,
            max_bytes: 1024 * 1024,
        },
    })
}

fn sid(id: &str) -> SessionId {
    SessionId::from(id)
}

fn enqueue(connection: &Connection, session: Option<&str>, method: &str) -> PendingReply {
    connection
        .session(session.map(sid))
        .enqueue(method, json!({}), None)
        .unwrap_or_else(|error| panic!("enqueue {method}: {error}"))
}

fn attach(
    control: &InMemoryControl,
    parent: Option<&str>,
    session: &str,
    target: &str,
    kind: &str,
) {
    let target_info = json!({"targetId": target, "type": kind, "url": "about:blank"});
    control.emit(
        "Target.attachedToTarget",
        parent,
        json!({"sessionId": session, "targetInfo": target_info, "waitingForDebugger": true}),
    );
}

// The reader handles frames in arrival order, so once this reply resolves every earlier frame
// has been dispatched.
async fn barrier(connection: &Connection, control: &mut InMemoryControl) {
    let pending = enqueue(connection, None, "Test.barrier");
    let command = control.expect("Test.barrier").await;
    control.reply(&command, json!({}));
    pending.await.unwrap();
}

fn route(
    connection: &Connection,
    sessions: SessionScope,
    methods: Vec<MethodMatch>,
) -> RouteReceiver {
    connection.route(RouteFilter { sessions, methods })
}

async fn next_routed(route: &mut RouteReceiver) -> Event {
    timeout(WAIT, route.recv())
        .await
        .expect("the route received nothing within 5 s")
        .expect("the route closed")
}

fn assert_refused(connection: &Connection, session: &str, method: &str) -> BrowserError {
    connection
        .send_nowait(method, json!({}), Some(&sid(session)))
        .err()
        .unwrap_or_else(|| panic!("{method} on {session} must be refused"))
}

#[tokio::test]
async fn replies_resolve_the_command_with_their_id() {
    let (connection, mut control) = connect();
    let version = enqueue(&connection, None, "Browser.getVersion");
    let evaluate = enqueue(&connection, Some("S1"), "Runtime.evaluate");
    let tree = enqueue(&connection, Some("S1"), "Page.getFrameTree");
    let commands = [
        control.expect("Browser.getVersion").await,
        control.expect("Runtime.evaluate").await,
        control.expect("Page.getFrameTree").await,
    ];
    assert!(commands[0].id >= 1);
    assert!(commands.windows(2).all(|pair| pair[0].id < pair[1].id));
    for command in commands.iter().rev() {
        control.reply(command, json!({"answered": command.method}));
    }
    assert_eq!(tree.await.unwrap().result["answered"], "Page.getFrameTree");
    assert_eq!(
        version.await.unwrap().result["answered"],
        "Browser.getVersion"
    );
    assert_eq!(
        evaluate.await.unwrap().result["answered"],
        "Runtime.evaluate"
    );
}

#[tokio::test]
async fn events_and_responses_take_separate_paths() {
    let (connection, mut control) = connect();
    let mut pending = enqueue(&connection, Some("S1"), "DOM.getDocument");
    let command = control.expect("DOM.getDocument").await;
    let cursor = connection.events().cursor();
    control.emit("Page.loadEventFired", Some("S1"), json!({"id": command.id}));
    barrier(&connection, &mut control).await;
    assert!((&mut pending).now_or_never().is_none());
    let event = connection
        .events()
        .wait_for(cursor, Instant::now() + WAIT, |event| {
            &*event.method == "Page.loadEventFired"
        })
        .await
        .unwrap()
        .expect("the event reached the log");
    assert_eq!(event.session, Some(sid("S1")));
    assert_eq!(event.params["id"], command.id);
    let reply = json!({"id": command.id, "sessionId": "S1", "result": {"root": {"nodeId": 1}}});
    control.send_text(&reply.to_string());
    assert_eq!(pending.await.unwrap().result["root"]["nodeId"], 1);
}

#[tokio::test]
async fn lone_surrogates_in_replies_and_events_are_repaired() {
    let (connection, mut control) = connect();
    let mut fetch = route(
        &connection,
        SessionScope::Exactly(sid("s1")),
        vec![MethodMatch::Prefix("Fetch.")],
    );
    let first = enqueue(&connection, None, "Accessibility.getFullAXTree");
    let command = control.expect("Accessibility.getFullAXTree").await;
    control.send_text(&format!(
        r#"{{"id":{},"result":{{"value":"\ud800"}}}}"#,
        command.id
    ));
    control.send_text(
        r#"{"method":"Fetch.requestPaused","params":{"requestId":"r1","request":{"url":"https://example.test/\udfff"}},"sessionId":"s1"}"#,
    );
    let first = timeout(Duration::from_secs(1), first)
        .await
        .expect("the surrogate reply completes")
        .unwrap();
    assert_eq!(first.result["value"], "\u{fffd}");
    let event = next_routed(&mut fetch).await;
    assert_eq!(&*event.method, "Fetch.requestPaused");
    assert_eq!(
        event.params["request"]["url"],
        "https://example.test/\u{fffd}"
    );
    let second = enqueue(&connection, None, "Browser.getVersion");
    let command = control.expect("Browser.getVersion").await;
    control.reply(&command, json!({"value": "ok"}));
    assert_eq!(second.await.unwrap().result["value"], "ok");
}

#[tokio::test]
async fn a_malformed_reply_fails_only_its_command() {
    let (connection, mut control) = connect();
    let broken = enqueue(&connection, None, "Broken.response");
    let command = control.expect("Broken.response").await;
    control.send_text(&format!(
        r#"{{"id":{},"result":{{"value":"\ud800"}},"method":42}}"#,
        command.id
    ));
    let error = timeout(Duration::from_secs(1), broken)
        .await
        .expect("a malformed reply must not time out")
        .unwrap_err();
    assert!(matches!(
        &error,
        BrowserError::Protocol { method, code: -32700, .. } if method == "Broken.response"
    ));
    let second = enqueue(&connection, None, "Browser.getVersion");
    let command = control.expect("Browser.getVersion").await;
    control.send_text(&format!(
        r#"{{"result":{{"id":{},"value":"\ud800"}},"method":42}}"#,
        command.id
    ));
    control.reply(&command, json!({"value": "still-open"}));
    assert_eq!(second.await.unwrap().result["value"], "still-open");
    assert!(!connection.is_closed());
}

#[tokio::test]
async fn an_invalid_utf8_frame_fails_the_command_its_id_names() {
    let (connection, mut control) = connect();
    let survivor = enqueue(&connection, None, "Page.enable");
    let victim = enqueue(&connection, None, "Page.getFrameTree");
    let survivor_command = control.expect("Page.enable").await;
    let victim_command = control.expect("Page.getFrameTree").await;
    control.send_invalid_utf8(b"{\"method\":\"Page.x\",\"params\":{\"v\":\"\xff\"}}");
    let mut frame = format!("{{\"result\":{{\"id\":{},\"v\":\"", survivor_command.id).into_bytes();
    frame.extend_from_slice(&[0xff, 0xfe]);
    frame.extend_from_slice(format!("\"}},\"id\":{}}}", victim_command.id).as_bytes());
    control.send_invalid_utf8(&frame);
    assert!(matches!(
        victim.await,
        Err(BrowserError::Protocol { method, code: -32700, message })
            if method == "Page.getFrameTree" && message.contains("not valid UTF-8")
    ));
    let valid_binary = json!({"id": survivor_command.id, "result": {"ok": true}}).to_string();
    control.send_invalid_utf8(valid_binary.as_bytes());
    assert_eq!(survivor.await.unwrap().result["ok"], true);
    assert!(!connection.is_closed());
}

#[tokio::test]
async fn commands_to_unknown_sessions_are_sent_and_late_replies_ignored() {
    let (connection, mut control) = connect();
    assert!(connection.session_info(&sid("S404")).is_none());
    let pending = enqueue(&connection, Some("S404"), "Runtime.evaluate");
    let command = control.expect("Runtime.evaluate").await;
    assert_eq!(command.session.as_deref(), Some("S404"));
    control.reply_error(&command, -32001, "Session with given id not found.");
    let error = pending.await.unwrap_err();
    assert!(matches!(
        &error,
        BrowserError::Protocol { method, code: -32001, .. } if method == "Runtime.evaluate"
    ));
    assert_eq!(error.class(), ErrorClass::SessionGone);
    control.send_text(r#"{"id":999999,"result":{}}"#);
    control.send_text(
        r#"{"id":999998,"sessionId":"S404","error":{"code":-32001,"message":"Session with given id not found."}}"#,
    );
    control.send_text(&json!({"id": command.id, "result": {}}).to_string());
    barrier(&connection, &mut control).await;
    assert!(!connection.is_closed());
}

#[tokio::test]
async fn protocol_error_data_is_part_of_the_message() {
    let (connection, mut control) = connect();
    let pending = enqueue(&connection, None, "Target.attachToTarget");
    let command = control.expect("Target.attachToTarget").await;
    let data =
        "Failed to deserialize params.targetId - BINDINGS: mandatory field missing at position 8";
    let error = json!({"code": -32602, "message": "Invalid parameters", "data": data});
    control.send_text(&json!({"id": command.id, "error": error}).to_string());
    assert_eq!(
        pending.await.unwrap_err().to_string(),
        format!(
            "Chrome DevTools command Target.attachToTarget failed (-32602): Invalid parameters: {data}"
        )
    );
}

#[derive(Default)]
struct DetachProbe {
    seen: Mutex<Vec<(String, bool, bool)>>,
}

impl EventHook for DetachProbe {
    fn on_event(&self, connection: &Connection, event: &Event) {
        if &*event.method != "Target.detachedFromTarget" {
            return;
        }
        let mut seen = self.seen.lock().unwrap();
        for id in ["S2", "S3"] {
            let info = connection.session_info(&sid(id));
            let detached = info.as_ref().is_some_and(|info| info.detached);
            seen.push((id.to_owned(), info.is_some(), detached));
        }
    }
}

async fn attach_page_frame_worker(connection: &Connection, control: &mut InMemoryControl) {
    attach(control, None, "S1", "T1", "page");
    attach(control, Some("S1"), "S2", "F2", "iframe");
    attach(control, Some("S2"), "S3", "W3", "worker");
    barrier(connection, control).await;
    let mut below = connection.descendants(&sid("S1"));
    below.sort();
    assert_eq!(below, [sid("S2"), sid("S3")]);
    let worker = connection.session_info(&sid("S3")).unwrap();
    assert_eq!(worker.parent, Some(sid("S2")));
    assert_eq!(worker.target.target_id.as_str(), "W3");
}

#[tokio::test]
async fn detach_fails_the_session_subtree_after_hooks_saw_it() {
    let (connection, mut control) = connect();
    let probe = Arc::new(DetachProbe::default());
    connection.add_hook(probe.clone());
    attach_page_frame_worker(&connection, &mut control).await;
    let page = enqueue(&connection, Some("S1"), "Page.enable");
    let frame = enqueue(&connection, Some("S2"), "DOM.describeNode");
    let nested = enqueue(&connection, Some("S3"), "Runtime.evaluate");
    let page_command = control.expect("Page.enable").await;
    control.expect("DOM.describeNode").await;
    control.expect("Runtime.evaluate").await;
    let detached = json!({"sessionId": "S2", "targetId": "F2"});
    control.emit("Target.detachedFromTarget", Some("S1"), detached);
    assert!(
        matches!(frame.await, Err(BrowserError::TargetClosed { method }) if method == "DOM.describeNode")
    );
    assert!(
        matches!(nested.await, Err(BrowserError::TargetClosed { method }) if method == "Runtime.evaluate")
    );
    barrier(&connection, &mut control).await;
    let expected = [("S2".to_owned(), true, true), ("S3".to_owned(), true, true)];
    assert_eq!(*probe.seen.lock().unwrap(), expected);
    assert!(connection.session_info(&sid("S2")).is_none());
    assert!(connection.session_info(&sid("S3")).is_none());
    assert!(connection.descendants(&sid("S1")).is_empty());
    assert!(matches!(
        assert_refused(&connection, "S2", "Page.enable"),
        BrowserError::TargetClosed { method } if method == "Page.enable"
    ));
    assert!(matches!(
        connection
            .session(Some(sid("S3")))
            .enqueue("Runtime.enable", json!({}), None),
        Err(BrowserError::TargetClosed { .. })
    ));
    control.reply(&page_command, json!({}));
    page.await.unwrap();
}

#[tokio::test]
async fn inspector_detached_closes_only_that_session() {
    let (connection, mut control) = connect();
    attach(&control, None, "S1", "T1", "page");
    attach(&control, None, "S4", "T4", "page");
    barrier(&connection, &mut control).await;
    let other = enqueue(&connection, Some("S1"), "Page.enable");
    let closing = enqueue(&connection, Some("S4"), "Page.enable");
    let other_command = control.expect("Page.enable").await;
    control.expect("Page.enable").await;
    control.emit(
        "Inspector.detached",
        Some("S4"),
        json!({"reason": "target_closed"}),
    );
    assert!(matches!(
        closing.await,
        Err(BrowserError::TargetClosed { method }) if method == "Page.enable"
    ));
    barrier(&connection, &mut control).await;
    assert!(connection.session_info(&sid("S4")).is_none());
    assert!(matches!(
        assert_refused(&connection, "S4", "Page.enable"),
        BrowserError::TargetClosed { .. }
    ));
    control.reply(&other_command, json!({}));
    other.await.unwrap();
    connection
        .send_nowait("Runtime.enable", json!({}), Some(&sid("S1")))
        .unwrap();
    let resumed = control.expect("Runtime.enable").await;
    assert_eq!(resumed.session.as_deref(), Some("S1"));
}

fn assert_crashed(connection: &Connection, method: &str) {
    assert!(
        matches!(
            assert_refused(connection, "S1", method),
            BrowserError::TargetCrashed { ref target_id } if target_id == "T1"
        ),
        "{method} must fail fast on a crashed page"
    );
}

async fn crash_page(connection: &Connection, control: &mut InMemoryControl) {
    attach(control, None, "S1", "T1", "page");
    attach(control, Some("S1"), "S2", "F2", "iframe");
    barrier(connection, control).await;
    control.emit("Inspector.targetCrashed", Some("S1"), json!({}));
    barrier(connection, control).await;
    assert!(connection.session_info(&sid("S1")).unwrap().crashed);
}

#[tokio::test]
async fn a_crash_fails_pending_of_the_session_and_its_children() {
    let (connection, mut control) = connect();
    attach(&control, None, "S1", "T1", "page");
    attach(&control, Some("S1"), "S2", "F2", "iframe");
    barrier(&connection, &mut control).await;
    let page = enqueue(&connection, Some("S1"), "Runtime.evaluate");
    let frame = enqueue(&connection, Some("S2"), "DOM.getDocument");
    control.expect("Runtime.evaluate").await;
    control.expect("DOM.getDocument").await;
    control.emit("Inspector.targetCrashed", Some("S1"), json!({}));
    for pending in [page, frame] {
        assert!(matches!(
            pending.await,
            Err(BrowserError::TargetCrashed { target_id }) if target_id == "T1"
        ));
    }
    connection
        .send_nowait("DOM.getDocument", json!({}), Some(&sid("S2")))
        .unwrap();
    control.expect("DOM.getDocument").await;
}

#[tokio::test]
async fn crashed_sessions_only_send_the_allowlist_until_reloaded() {
    let (connection, mut control) = connect();
    crash_page(&connection, &mut control).await;
    for method in [
        "Runtime.evaluate",
        "DOM.getDocument",
        "Page.enable",
        "Input.dispatchKeyEvent",
    ] {
        assert_crashed(&connection, method);
    }
    for method in ALLOWED_AFTER_CRASH {
        connection
            .send_nowait(method, json!({}), Some(&sid("S1")))
            .unwrap_or_else(|error| panic!("{method} is allowed after a crash: {error}"));
        control.expect(method).await;
    }
    control.emit("Inspector.targetReloadedAfterCrash", Some("S1"), json!({}));
    barrier(&connection, &mut control).await;
    assert!(!connection.session_info(&sid("S1")).unwrap().crashed);
    connection
        .send_nowait("Runtime.evaluate", json!({}), Some(&sid("S1")))
        .unwrap();
    control.expect("Runtime.evaluate").await;
}

#[tokio::test]
async fn a_root_target_crash_marks_every_session_of_that_target() {
    let (connection, mut control) = connect();
    attach(&control, None, "S1", "T1", "page");
    attach(&control, None, "S9", "T9", "page");
    barrier(&connection, &mut control).await;
    let crashed = json!({"targetId": "T1", "status": "crashed", "errorCode": 11});
    control.emit("Target.targetCrashed", None, crashed);
    barrier(&connection, &mut control).await;
    assert!(connection.session_info(&sid("S1")).unwrap().crashed);
    assert!(!connection.session_info(&sid("S9")).unwrap().crashed);
    assert_crashed(&connection, "Runtime.evaluate");
    connection
        .send_nowait("Runtime.evaluate", json!({}), Some(&sid("S9")))
        .unwrap();
    control.expect("Runtime.evaluate").await;
}

#[tokio::test]
async fn an_undecodable_target_info_keeps_its_target_id_for_crash_matching() {
    let (connection, mut control) = connect();
    let target_info = json!({"targetId": "T1", "type": "page", "title": null, "attached": "yes"});
    control.emit(
        "Target.attachedToTarget",
        None,
        json!({"sessionId": "S1", "targetInfo": target_info}),
    );
    barrier(&connection, &mut control).await;
    let info = connection.session_info(&sid("S1")).unwrap();
    assert_eq!(info.target.target_id.as_str(), "T1");
    assert_eq!(info.target.type_, TargetType::Page);
    let pending = enqueue(&connection, Some("S1"), "Runtime.evaluate");
    control.expect("Runtime.evaluate").await;
    control.emit("Target.targetCrashed", None, json!({"targetId": "T1"}));
    assert!(matches!(
        pending.await,
        Err(BrowserError::TargetCrashed { target_id }) if target_id == "T1"
    ));
    assert_crashed(&connection, "DOM.getDocument");
}

fn enqueue_until_refused(connection: &Connection) -> Vec<PendingReply> {
    let session = connection.session(Some(sid("S1")));
    let mut accepted = Vec::new();
    loop {
        match session.enqueue("Runtime.evaluate", json!({}), None) {
            Ok(pending) => accepted.push(pending),
            Err(BrowserError::TargetCrashed { .. } | BrowserError::TargetClosed { .. }) => {
                return accepted;
            }
            Err(error) => panic!("Runtime.evaluate on S1 failed unexpectedly: {error}"),
        }
    }
}

async fn race_senders_against(event: &str, expected: fn(&BrowserError) -> bool) {
    let (connection, mut control) = connect();
    attach(&control, None, "S1", "T1", "page");
    barrier(&connection, &mut control).await;
    let cursor = connection.events().cursor();
    let senders: Vec<JoinHandle<Vec<PendingReply>>> = (0..3)
        .map(|_| {
            let connection = connection.clone();
            tokio::task::spawn_blocking(move || enqueue_until_refused(&connection))
        })
        .collect();
    control.emit(event, Some("S1"), json!({}));
    let mut accepted = Vec::new();
    for sender in senders {
        accepted.extend(sender.await.unwrap());
    }
    connection
        .events()
        .wait_for(cursor, Instant::now() + WAIT, |logged| {
            &*logged.method == event
        })
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{event} was not dispatched within 5 s"));
    // Unconstrained, because once the task exhausts its tokio coop budget a resolved oneshot polls
    // as pending.
    for pending in accepted {
        let result = tokio::task::unconstrained(pending)
            .now_or_never()
            .unwrap_or_else(|| panic!("a command accepted while {event} arrived was left waiting"));
        assert!(
            matches!(result, Err(ref error) if expected(error)),
            "{event}: {result:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sends_racing_a_crash_or_detach_fail_fast_instead_of_waiting() {
    for _ in 0..40 {
        race_senders_against(
            "Inspector.targetCrashed",
            |error| matches!(error, BrowserError::TargetCrashed { target_id } if target_id == "T1"),
        )
        .await;
        race_senders_against("Inspector.detached", |error| {
            matches!(error, BrowserError::TargetClosed { method } if method == "Runtime.evaluate")
        })
        .await;
    }
}

#[derive(Default)]
struct Counter(AtomicUsize);

impl EventHook for Counter {
    fn on_event(&self, _connection: &Connection, _event: &Event) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn eof_fails_pending_commands_and_resolves_closed() {
    let (connection, mut control) = connect();
    let root = enqueue(&connection, None, "Browser.getVersion");
    let page = enqueue(&connection, Some("S1"), "Page.navigate");
    control.expect("Browser.getVersion").await;
    control.expect("Page.navigate").await;
    let cursor = connection.events().cursor();
    control.close("Chrome exited");
    for pending in [root, page] {
        assert!(matches!(
            pending.await,
            Err(BrowserError::Disconnected { reason }) if reason == "Chrome exited"
        ));
    }
    timeout(WAIT, connection.closed()).await.unwrap();
    assert!(connection.is_closed());
    assert_eq!(connection.closed_reason().as_deref(), Some("Chrome exited"));
    let waited = connection
        .events()
        .wait_for(cursor, Instant::now() + WAIT, |_| true)
        .await;
    assert!(matches!(
        waited,
        Err(BrowserError::Disconnected { reason }) if reason == "Chrome exited"
    ));
}

#[tokio::test]
async fn a_closed_connection_clears_hooks_and_routes_and_refuses_commands() {
    let (connection, mut control) = connect();
    let hook = Arc::new(Counter::default());
    connection.add_hook(hook.clone());
    attach(&control, None, "S1", "T1", "page");
    barrier(&connection, &mut control).await;
    assert_eq!(hook.0.load(Ordering::SeqCst), 1);
    let mut page_events = route(
        &connection,
        SessionScope::Exactly(sid("S1")),
        vec![MethodMatch::Prefix("Page.")],
    );
    control.close("Chrome exited");
    timeout(WAIT, connection.closed()).await.unwrap();
    assert_eq!(Arc::strong_count(&hook), 1);
    assert!(page_events.recv().await.is_none());
    assert!(matches!(
        connection.send_nowait("Browser.getVersion", json!({}), None),
        Err(BrowserError::Disconnected { .. })
    ));
    assert!(matches!(
        connection
            .session(Some(sid("S1")))
            .enqueue("Page.enable", json!({}), None),
        Err(BrowserError::Disconnected { .. })
    ));
    connection.add_hook(hook.clone());
    assert_eq!(Arc::strong_count(&hook), 1);
    let scope = SessionScope::WithDescendants(sid("S1"));
    let mut late = route(&connection, scope, vec![MethodMatch::Prefix("")]);
    assert!(late.recv().await.is_none());
}

#[derive(Default)]
struct Resumer {
    resumed: AtomicUsize,
    detached_visible: AtomicUsize,
}

impl EventHook for Resumer {
    fn on_event(&self, connection: &Connection, event: &Event) {
        let Some(child) = event.params["sessionId"].as_str().map(SessionId::from) else {
            return;
        };
        let info = connection.session_info(&child);
        let _ = connection.descendants(&child);
        match &*event.method {
            "Target.attachedToTarget" if info.is_some() => {
                let resume = connection.send_nowait(
                    "Runtime.runIfWaitingForDebugger",
                    json!({}),
                    Some(&child),
                );
                if resume.is_ok() {
                    self.resumed.fetch_add(1, Ordering::SeqCst);
                }
            }
            "Target.detachedFromTarget" if info.is_some_and(|info| info.detached) => {
                self.detached_visible.fetch_add(1, Ordering::SeqCst);
                let _ = connection.send_nowait("Target.getTargets", json!({}), None);
            }
            _ => {}
        }
    }
}

fn spawn_registry_poller(connection: &Connection) -> JoinHandle<()> {
    let connection = connection.clone();
    tokio::spawn(async move {
        for round in 0..2_000 {
            let _ = connection.session_info(&sid("S0"));
            let _ = connection.descendants(&sid("S0"));
            if round % 100 == 0 {
                tokio::task::yield_now().await;
            }
        }
    })
}

async fn count_resumes_and_listings(control: &mut InMemoryControl, total: usize) -> (usize, usize) {
    let (mut resumed, mut listed) = (0, 0);
    while resumed + listed < total {
        match control.next_command().await.method.as_str() {
            "Runtime.runIfWaitingForDebugger" => resumed += 1,
            "Target.getTargets" => listed += 1,
            other => panic!("unexpected command {other}"),
        }
    }
    (resumed, listed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hooks_that_send_and_read_the_registry_do_not_deadlock() {
    const SESSIONS: usize = 50;
    let (connection, mut control) = connect();
    let hook = Arc::new(Resumer::default());
    connection.add_hook(hook.clone());
    let poller = spawn_registry_poller(&connection);
    for index in 0..SESSIONS {
        attach(
            &control,
            None,
            &format!("S{index}"),
            &format!("T{index}"),
            "page",
        );
    }
    for index in 0..SESSIONS {
        let detached = json!({"sessionId": format!("S{index}")});
        control.emit("Target.detachedFromTarget", None, detached);
    }
    let counts = timeout(WAIT, count_resumes_and_listings(&mut control, 2 * SESSIONS))
        .await
        .expect("a hook deadlocked the reader");
    assert_eq!(counts, (SESSIONS, SESSIONS));
    poller.await.unwrap();
    assert_eq!(hook.resumed.load(Ordering::SeqCst), SESSIONS);
    assert_eq!(hook.detached_visible.load(Ordering::SeqCst), SESSIONS);
}

#[tokio::test]
async fn enqueue_order_is_wire_order() {
    let (connection, mut control) = connect();
    let first = enqueue(&connection, None, "Target.setAutoAttach");
    let second = enqueue(&connection, Some("S1"), "Page.getFrameTree");
    connection
        .session(Some(sid("S1")))
        .send_nowait("Runtime.runIfWaitingForDebugger", json!({}))
        .unwrap();
    let mut wire = Vec::new();
    for _ in 0..3 {
        let command = control.next_command().await;
        wire.push((command.method, command.id));
    }
    let methods: Vec<&str> = wire.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(
        methods,
        [
            "Target.setAutoAttach",
            "Page.getFrameTree",
            "Runtime.runIfWaitingForDebugger"
        ]
    );
    assert!(wire.windows(2).all(|pair| pair[0].1 < pair[1].1));
    drop((first, second));
}

#[tokio::test]
async fn default_and_per_command_timeouts_apply() {
    let (connection, mut control) = connect_with(ConnectionOptions {
        default_timeout: Duration::from_millis(50),
        event_log: EventLogLimits::default(),
    });
    let page = connection.session(Some(sid("S1")));
    let started = Instant::now();
    assert!(matches!(
        page.send("Page.enable", json!({})).await,
        Err(BrowserError::Timeout { method, timeout_ms: 50 }) if method == "Page.enable"
    ));
    assert!(started.elapsed() >= Duration::from_millis(50));
    let short = Duration::from_millis(20);
    assert!(matches!(
        page.send_with_timeout("Runtime.enable", json!({}), short).await,
        Err(BrowserError::Timeout { method, timeout_ms: 20 }) if method == "Runtime.enable"
    ));
    let long = Some(Duration::from_millis(80));
    assert!(matches!(
        page.request("DOM.enable", json!({}), long).await,
        Err(BrowserError::Timeout { timeout_ms: 80, .. })
    ));
    for method in ["Page.enable", "Runtime.enable", "DOM.enable"] {
        let late = control.expect(method).await;
        control.reply(&late, json!({}));
    }
    drop(enqueue(&connection, Some("S1"), "Page.getFrameTree"));
    let abandoned = control.expect("Page.getFrameTree").await;
    control.reply(&abandoned, json!({}));
    barrier(&connection, &mut control).await;
    assert!(!connection.is_closed());
}

#[tokio::test(start_paused = true)]
async fn the_writer_skips_commands_whose_deadline_passed() {
    let (connection, mut control) = connect();
    let root = connection.session(None);
    let stale = root
        .enqueue("Page.reload", json!({}), Some(Duration::from_millis(10)))
        .unwrap();
    let stale_ticket = stale.ticket().clone();
    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(matches!(
        stale.await,
        Err(BrowserError::Timeout { method, timeout_ms: 10 }) if method == "Page.reload"
    ));
    let fresh = enqueue(&connection, None, "Page.enable");
    let command = control.expect("Page.enable").await;
    assert_eq!(fresh.ticket().status(), WriteStatus::Written);
    assert_eq!(stale_ticket.status(), WriteStatus::NotWritten);
    let seen = control.commands_seen();
    assert!(seen.iter().all(|command| command.method != "Page.reload"));
    control.reply(&command, json!({}));
    fresh.await.unwrap();
}

#[tokio::test]
async fn oversized_commands_are_refused_before_they_reach_the_wire() {
    let (connection, mut control) = connect();
    let expression = "x".repeat(MAX_OUTBOUND_BYTES);
    let refused = connection
        .send_raw("Runtime.evaluate", json!({"expression": expression}), None)
        .await;
    assert!(matches!(
        refused,
        Err(BrowserError::InvalidArgument { message })
            if message.starts_with("Runtime.evaluate message is ")
                && message.ends_with("bytes; Chrome resets the connection above ~100 MiB")
    ));
    barrier(&connection, &mut control).await;
    let seen = control.commands_seen();
    assert!(
        seen.iter()
            .all(|command| command.method != "Runtime.evaluate")
    );
}

fn emit_network_load(control: &InMemoryControl, events: u64) {
    let sessions = ["S1", "S2", "S3"];
    for index in 0..events {
        let session = sessions[(index % 3) as usize];
        control.emit(
            "Network.requestWillBeSent",
            Some(session),
            json!({"index": index}),
        );
        if index % 1_000 == 0 {
            control.emit("Page.frameNavigated", Some("S1"), json!({"index": index}));
        }
    }
}

async fn assert_routed_indices(route: &mut RouteReceiver, indices: impl Iterator<Item = u64>) {
    let mut last_seq = 0;
    for index in indices {
        let event = next_routed(route).await;
        assert_eq!(&*event.method, "Network.requestWillBeSent");
        assert_eq!(event.params["index"], index);
        let expected = if index % 3 == 0 { "S1" } else { "S2" };
        assert_eq!(event.session, Some(sid(expected)));
        assert!(event.seq > last_seq);
        last_seq = event.seq;
    }
}

#[tokio::test]
async fn lossless_routes_receive_every_event_in_order_under_load() {
    const EVENTS: u64 = 10_000;
    let (connection, mut control) = connect_with_log(16);
    attach(&control, None, "S1", "T1", "page");
    attach(&control, Some("S1"), "S2", "F2", "iframe");
    attach(&control, None, "S3", "T3", "page");
    barrier(&connection, &mut control).await;
    let mut page_only = route(
        &connection,
        SessionScope::Exactly(sid("S1")),
        vec![MethodMatch::Prefix("Network.")],
    );
    let mut with_frames = route(
        &connection,
        SessionScope::WithDescendants(sid("S1")),
        vec![
            MethodMatch::Exact("Network.requestWillBeSent"),
            MethodMatch::Prefix("Fetch."),
        ],
    );
    let cursor = connection.events().cursor();
    emit_network_load(&control, EVENTS);
    assert_routed_indices(&mut page_only, (0..EVENTS).filter(|index| index % 3 == 0)).await;
    assert_routed_indices(&mut with_frames, (0..EVENTS).filter(|index| index % 3 != 2)).await;
    barrier(&connection, &mut control).await;
    assert!(page_only.recv().now_or_never().is_none());
    let network_logged = connection
        .events()
        .wait_for(cursor, Instant::now(), |event| {
            event.method.starts_with("Network.")
        })
        .await
        .unwrap();
    assert!(network_logged.is_none());
}

#[tokio::test]
async fn only_state_events_enter_the_log_but_every_event_gets_a_sequence_number() {
    let (connection, mut control) = connect();
    let cursor = connection.events().cursor();
    let methods = [
        "Network.requestWillBeSent",
        "Runtime.consoleAPICalled",
        "Fetch.requestPaused",
        "Log.entryAdded",
        "Runtime.executionContextCreated",
        "Page.frameNavigated",
        "Target.targetCreated",
        "Browser.downloadProgress",
        "Inspector.targetReloadedAfterCrash",
    ];
    for method in methods {
        control.emit(method, None, json!({}));
    }
    barrier(&connection, &mut control).await;
    let mut logged = Vec::new();
    let mut from = cursor;
    while let Some(event) = connection
        .events()
        .wait_for(from, Instant::now(), |_| true)
        .await
        .unwrap()
    {
        from = EventCursor(event.seq + 1);
        logged.push((event.method.to_string(), event.seq - cursor.0));
    }
    let expected: Vec<(String, u64)> = (4..methods.len())
        .map(|at| (methods[at].to_owned(), at as u64))
        .collect();
    assert_eq!(logged, expected);
}

fn emit_lifecycle(control: &InMemoryControl, name: &str) {
    let params = json!({"frameId": "T1", "loaderId": "L1", "name": name});
    control.emit("Page.lifecycleEvent", Some("S1"), params);
}

#[tokio::test]
async fn evicted_events_are_reported_to_lagging_cursors() {
    let (connection, mut control) = connect_with_log(4);
    attach(&control, None, "S1", "T1", "page");
    barrier(&connection, &mut control).await;
    let lagging = connection.events().cursor();
    for index in 0..10 {
        emit_lifecycle(&control, &format!("step{index}"));
    }
    barrier(&connection, &mut control).await;
    let deadline = Instant::now() + Duration::from_millis(50);
    assert!(matches!(
        connection
            .events()
            .wait_for(lagging, deadline, |_| false)
            .await,
        Err(BrowserError::EventsLost)
    ));
    let current = connection.events().cursor();
    let waiter = tokio::spawn({
        let connection = connection.clone();
        async move {
            let deadline = Instant::now() + WAIT;
            let is_load = |event: &Event| event.params["name"] == "load";
            connection
                .events()
                .wait_for(current, deadline, is_load)
                .await
        }
    });
    emit_lifecycle(&control, "load");
    let found = waiter
        .await
        .unwrap()
        .unwrap()
        .expect("the load event was logged");
    assert!(found.seq >= current.0);
    assert_eq!(found.session, Some(sid("S1")));
}

struct OrderProbe {
    route: Mutex<RouteReceiver>,
    observed: Mutex<Vec<(bool, bool, bool)>>,
}

impl EventHook for OrderProbe {
    fn on_event(&self, connection: &Connection, event: &Event) {
        if &*event.method != "Target.attachedToTarget" || event.session.is_none() {
            return;
        }
        let child = sid(event.params["sessionId"].as_str().unwrap_or_default());
        let registered = connection
            .session_info(&child)
            .is_some_and(|info| info.parent == event.session);
        let routed = self.route.lock().unwrap().recv().now_or_never().is_some();
        let seq = event.seq;
        let logged = connection
            .events()
            .wait_for(EventCursor(seq), Instant::now() + WAIT, |event| {
                event.seq == seq
            })
            .now_or_never()
            .is_some();
        let observation = (registered, routed, logged);
        self.observed.lock().unwrap().push(observation);
    }
}

#[tokio::test]
async fn hooks_run_after_the_registry_and_before_routes_and_the_log() {
    let (connection, mut control) = connect();
    let probe = Arc::new(OrderProbe {
        route: Mutex::new(route(
            &connection,
            SessionScope::WithDescendants(sid("S1")),
            vec![MethodMatch::Exact("Target.attachedToTarget")],
        )),
        observed: Mutex::new(Vec::new()),
    });
    connection.add_hook(probe.clone());
    attach(&control, None, "S1", "T1", "page");
    let cursor = connection.events().cursor();
    attach(&control, Some("S1"), "S2", "F2", "iframe");
    barrier(&connection, &mut control).await;
    assert_eq!(*probe.observed.lock().unwrap(), [(true, false, false)]);
    let routed = probe.route.lock().unwrap().recv().now_or_never().flatten();
    let routed = routed.expect("the route received the child attach after the hook");
    assert_eq!(routed.params["sessionId"], "S2");
    let logged = connection
        .events()
        .wait_for(cursor, Instant::now(), |event| event.seq == routed.seq)
        .await
        .unwrap();
    assert!(logged.is_some());
}

#[tokio::test]
async fn dropped_routes_stop_receiving_and_others_continue() {
    let (connection, mut control) = connect();
    let dropped = route(
        &connection,
        SessionScope::Exactly(sid("S1")),
        vec![MethodMatch::Prefix("Runtime.")],
    );
    let mut kept = route(
        &connection,
        SessionScope::Exactly(sid("S1")),
        vec![MethodMatch::Exact("Runtime.consoleAPICalled")],
    );
    drop(dropped);
    control.emit(
        "Runtime.consoleAPICalled",
        Some("S1"),
        json!({"type": "log"}),
    );
    control.emit("Runtime.exceptionThrown", Some("S1"), json!({}));
    control.emit(
        "Runtime.consoleAPICalled",
        Some("S2"),
        json!({"type": "log"}),
    );
    barrier(&connection, &mut control).await;
    let event = next_routed(&mut kept).await;
    assert_eq!(event.params["type"], "log");
    assert_eq!(event.session, Some(sid("S1")));
    assert!(kept.recv().now_or_never().is_none());
}

use std::time::Duration;

use flow_like_browser::event_log::EventCursor;
use flow_like_browser::testing::{
    CommandOutcome, OpClass, PageHarness, default_auto_reply, element, run_command,
};
use flow_like_browser::types::{DialogType, FrameId};
use flow_like_browser::{BrowserError, NavigationOutcome, Page};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::Instant;

const MAIN: &str = "T1";
const PAGE_SESSION: &str = "S1";
const START_URL: &str = "http://127.0.0.1/";
const NEXT_URL: &str = "http://127.0.0.1/next";
const STILL_WAITING: Duration = Duration::from_millis(150);
const FINISHES_WITHIN: Duration = Duration::from_secs(5);
const LONG_PAGE_LOAD: Duration = Duration::from_secs(60);

fn spawn_command(
    page: &Page,
    frame: &'static str,
    class: OpClass,
    method: &'static str,
) -> JoinHandle<flow_like_browser::Result<CommandOutcome>> {
    let page = page.clone();
    tokio::spawn(async move { run_command(&page, frame, class, method, json!({})).await })
}

async fn finished<T>(op: JoinHandle<T>) -> T {
    tokio::time::timeout(FINISHES_WITHIN, op)
        .await
        .expect("the op did not finish")
        .expect("the op task panicked")
}

async fn still_waiting<T>(op: &JoinHandle<T>, what: &str) {
    tokio::time::sleep(STILL_WAITING).await;
    assert!(!op.is_finished(), "the op returned before {what}");
}

async fn processed(harness: &PageHarness, cursor: EventCursor, method: &str) {
    let deadline = Instant::now() + FINISHES_WITHIN;
    let event = harness
        .connection
        .events()
        .wait_for(cursor, deadline, |event| &*event.method == method)
        .await
        .expect("the event log closed");
    assert!(event.is_some(), "{method} was never processed");
}

fn count(harness: &PageHarness, method: &str) -> usize {
    harness
        .control
        .commands_seen()
        .iter()
        .filter(|command| command.method == method)
        .count()
}

fn requested(harness: &PageHarness, frame: &str, url: &str) {
    harness.emit(
        "Page.frameRequestedNavigation",
        json!({"frameId": frame, "reason": "anchorClick", "url": url, "disposition": "currentTab"}),
    );
}

fn started(harness: &PageHarness, frame: &str, loader: &str, url: &str) {
    harness.emit(
        "Page.frameStartedNavigating",
        json!({"frameId": frame, "url": url, "loaderId": loader, "navigationType": "differentDocument"}),
    );
    harness.emit("Page.frameStartedLoading", json!({"frameId": frame}));
}

fn committed(harness: &PageHarness, frame: &str, loader: &str, url: &str) {
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": frame, "loaderId": loader, "url": url}, "type": "Navigation"}),
    );
}

fn stopped(harness: &PageHarness, frame: &str) {
    harness.emit("Page.frameStoppedLoading", json!({"frameId": frame}));
}

fn navigation_error(error: Option<&str>, loader: Option<&str>, download: bool) -> Value {
    json!({"frameId": MAIN, "loaderId": loader, "errorText": error, "isDownload": download})
}

async fn document_reply(harness: &mut PageHarness, session: &str) {
    let command = harness
        .control
        .wait_for("DOM.getDocument", Some(session))
        .await;
    harness
        .control
        .reply(&command, json!({"root": {"nodeId": 1, "backendNodeId": 1}}));
}

async fn fail_document(harness: &mut PageHarness, session: &str, message: &str) {
    let command = harness
        .control
        .wait_for("DOM.getDocument", Some(session))
        .await;
    harness.control.reply_error(&command, -32000, message);
}

async fn crash(harness: &PageHarness) {
    let cursor = harness.connection.events().cursor();
    harness.emit("Inspector.targetCrashed", json!({}));
    processed(harness, cursor, "Inspector.targetCrashed").await;
}

async fn goto_awaiting_load(
    harness: &mut PageHarness,
) -> JoinHandle<flow_like_browser::Result<NavigationOutcome>> {
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.goto(NEXT_URL).await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    started(harness, MAIN, "L2", NEXT_URL);
    harness
        .control
        .reply(&navigate, json!({"frameId": MAIN, "loaderId": "L2"}));
    op
}

async fn reply_history(harness: &mut PageHarness, current_index: i64) {
    let history = harness
        .control
        .wait_for("Page.getNavigationHistory", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &history,
        json!({"currentIndex": current_index, "entries": [
            {"id": 1, "url": START_URL, "title": ""},
            {"id": 4, "url": NEXT_URL, "title": "Next"}
        ]}),
    );
}

fn lose_connection(harness: &PageHarness) {
    harness.control.close("the browser went away");
}

fn assert_disconnected(error: BrowserError) {
    assert!(
        matches!(error, BrowserError::Disconnected { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_anchor_click_waits_until_the_navigation_stops() {
    let mut harness = PageHarness::new().await;
    let op = spawn_command(
        &harness.page,
        MAIN,
        OpClass::Input,
        "Input.dispatchMouseEvent",
    );
    let click = harness
        .control
        .wait_for("Input.dispatchMouseEvent", Some(PAGE_SESSION))
        .await;
    requested(&harness, MAIN, NEXT_URL);
    started(&harness, MAIN, "L2", NEXT_URL);
    harness.control.reply(&click, json!({}));
    still_waiting(&op, "the navigation stopped").await;
    committed(&harness, MAIN, "L2", NEXT_URL);
    stopped(&harness, MAIN);
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));
    assert_eq!(harness.page.url().await.unwrap(), NEXT_URL);
}

#[tokio::test]
async fn a_form_request_before_the_start_is_waited_for() {
    let mut harness = PageHarness::new().await;
    let op = spawn_command(
        &harness.page,
        MAIN,
        OpClass::Input,
        "Input.dispatchMouseEvent",
    );
    let submit = harness
        .control
        .wait_for("Input.dispatchMouseEvent", Some(PAGE_SESSION))
        .await;
    requested(&harness, MAIN, NEXT_URL);
    harness.control.reply(&submit, json!({}));
    still_waiting(&op, "the requested navigation started").await;
    started(&harness, MAIN, "L2", NEXT_URL);
    still_waiting(&op, "the started navigation stopped").await;
    committed(&harness, MAIN, "L2", NEXT_URL);
    stopped(&harness, MAIN);
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));
}

#[tokio::test]
async fn a_timer_navigation_is_caught_by_the_next_pre_wait() {
    let mut harness = PageHarness::new().await;
    let first = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(finished(first).await.is_ok());

    let cursor = harness.connection.events().cursor();
    requested(&harness, MAIN, NEXT_URL);
    started(&harness, MAIN, "L2", NEXT_URL);
    processed(&harness, cursor, "Page.frameStartedLoading").await;
    let second = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    still_waiting(&second, "the timer navigation stopped").await;
    assert!(
        harness.control.try_next_command().is_none(),
        "the op was sent while the navigation was pending"
    );
    committed(&harness, MAIN, "L2", NEXT_URL);
    stopped(&harness, MAIN);
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(finished(second).await.is_ok());
}

#[tokio::test]
async fn a_navigation_past_the_page_load_timeout_is_stopped() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_millis(300));
    let cursor = harness.connection.events().cursor();
    started(&harness, MAIN, "L2", NEXT_URL);
    processed(&harness, cursor, "Page.frameStartedLoading").await;
    let op = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    harness
        .control
        .wait_for("Page.stopLoading", Some(PAGE_SESSION))
        .await;
    stopped(&harness, MAIN);
    let error = finished(op).await.unwrap_err();
    assert!(
        matches!(error, BrowserError::RendererTimeout { seconds } if (seconds - 0.3).abs() < 1e-9),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "Timed out receiving message from renderer: 0.300"
    );
    assert_eq!(count(&harness, "DOM.getDocument"), 0);
}

#[tokio::test]
async fn a_flush_held_at_the_deadline_ends_in_a_renderer_timeout() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_millis(300));
    harness.control.set_auto_reply(|command| {
        let flush = command.method == "Runtime.evaluate" && command.params["expression"] == "1";
        if flush {
            None
        } else {
            default_auto_reply(command)
        }
    });
    let op = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .wait_for("Page.stopLoading", Some(PAGE_SESSION))
        .await;
    assert!(matches!(
        finished(op).await,
        Err(BrowserError::RendererTimeout { .. })
    ));
    assert_eq!(count(&harness, "DOM.getDocument"), 0);
}

#[tokio::test]
async fn a_user_script_interrupted_by_a_navigation_is_not_retried() {
    let mut harness = PageHarness::new().await;
    let op = spawn_command(
        &harness.page,
        MAIN,
        OpClass::Script,
        "Runtime.callFunctionOn",
    );
    let script = harness
        .control
        .wait_for("Runtime.callFunctionOn", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply_error(&script, -32000, "Inspected target navigated or closed");
    assert!(matches!(
        finished(op).await,
        Err(BrowserError::NavigationInterrupted)
    ));
    assert_eq!(count(&harness, "Runtime.callFunctionOn"), 1);
}

#[tokio::test]
async fn an_internal_probe_is_retried_after_a_navigation() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.title().await });
    let first = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    assert_eq!(first.params["expression"], "document.title");
    harness
        .control
        .reply_error(&first, -32000, "Execution context was destroyed.");
    let second = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &second,
        json!({"result": {"type": "string", "value": "Checkout"}}),
    );
    assert_eq!(finished(op).await.unwrap(), "Checkout");
}

#[tokio::test]
async fn the_third_attempt_of_a_frame_scoped_op_uses_the_top_frame_once() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let op = spawn_command(&harness.page, "F2", OpClass::FrameRead, "DOM.getDocument");
    fail_document(&mut harness, "S2", "Execution context was destroyed.").await;
    fail_document(&mut harness, "S2", "Cannot find context with specified id").await;
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));

    let next = spawn_command(&harness.page, "F2", OpClass::FrameRead, "DOM.getDocument");
    document_reply(&mut harness, "S2").await;
    assert!(finished(next).await.is_ok());
}

#[tokio::test]
async fn element_bound_ops_never_fall_back_to_the_top_frame() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let op = spawn_command(&harness.page, "F2", OpClass::Read, "DOM.getDocument");
    for _ in 0..3 {
        fail_document(&mut harness, "S2", "Execution context was destroyed.").await;
    }
    assert!(matches!(
        finished(op).await,
        Err(BrowserError::RendererTimeout { .. })
    ));
    let on_page = harness
        .control
        .commands_seen()
        .iter()
        .filter(|command| {
            command.method == "DOM.getDocument" && command.session.as_deref() == Some(PAGE_SESSION)
        })
        .count();
    assert_eq!(on_page, 0);
}

#[tokio::test]
async fn a_child_session_gone_during_a_detach_first_swap_is_retried() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let op = spawn_command(&harness.page, "F2", OpClass::FrameRead, "DOM.getDocument");
    let first = harness
        .control
        .wait_for("DOM.getDocument", Some("S2"))
        .await;
    harness
        .control
        .reply_error(&first, -32001, "Session with given id not found");
    harness.detach_child("S2");
    still_waiting(&op, "the frame was re-reported by its parent").await;
    harness.emit(
        "Page.frameAttached",
        json!({"frameId": "F2", "parentFrameId": MAIN}),
    );
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L3", "url": "http://127.0.0.1/child"}, "type": "Navigation"}),
    );
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));
}

#[tokio::test]
async fn a_child_session_failed_before_its_frames_are_parked_is_retried() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let op = spawn_command(&harness.page, "F2", OpClass::FrameRead, "DOM.getDocument");
    harness
        .control
        .wait_for("DOM.getDocument", Some("S2"))
        .await;
    let cursor = harness.connection.events().cursor();
    harness.emit_child_detached("S2");
    processed(&harness, cursor, "Target.detachedFromTarget").await;
    still_waiting(&op, "the frames of the detached session were parked").await;
    harness.park_child_frames("S2");
    harness.emit(
        "Page.frameAttached",
        json!({"frameId": "F2", "parentFrameId": MAIN}),
    );
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L3", "url": "http://127.0.0.1/child"}, "type": "Navigation"}),
    );
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));
    assert_eq!(count(&harness, "DOM.getDocument"), 2);
}

#[tokio::test]
async fn a_frame_that_never_reports_a_document_holds_ops_only_briefly() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_secs(20));
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Page.frameAttached",
        json!({"frameId": "F3", "parentFrameId": MAIN}),
    );
    processed(&harness, cursor, "Page.frameAttached").await;
    let begun = Instant::now();
    let op = spawn_command(&harness.page, "F3", OpClass::FrameRead, "DOM.getDocument");
    still_waiting(&op, "its document was given a moment to be reported").await;
    assert_eq!(count(&harness, "DOM.getDocument"), 0);
    document_reply(&mut harness, PAGE_SESSION).await;
    assert!(matches!(finished(op).await, Ok(CommandOutcome::Done(_))));
    assert!(begun.elapsed() < FINISHES_WITHIN, "{:?}", begun.elapsed());
    assert_eq!(count(&harness, "Page.stopLoading"), 0);
}

#[tokio::test]
async fn the_page_session_detaching_during_a_pre_wait_fails_fast() {
    let harness = PageHarness::new().await;
    let cursor = harness.connection.events().cursor();
    started(&harness, MAIN, "L2", NEXT_URL);
    processed(&harness, cursor, "Page.frameStartedLoading").await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.url().await });
    still_waiting(&op, "the page session detached").await;
    let detached_at = Instant::now();
    harness.control.emit(
        "Target.detachedFromTarget",
        None,
        json!({"sessionId": PAGE_SESSION, "targetId": MAIN}),
    );
    let error = finished(op).await.unwrap_err();
    assert!(detached_at.elapsed() < Duration::from_secs(1));
    assert!(
        matches!(error, BrowserError::TargetClosed { .. }),
        "{error:?}"
    );
    assert!(matches!(
        harness.page.url().await,
        Err(BrowserError::NoSuchPage { .. })
    ));
}

#[tokio::test]
async fn goto_fails_on_connection_errors_only() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.goto("http://nope.invalid/").await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    assert_eq!(navigate.params["url"], "http://nope.invalid/");
    harness.control.reply(
        &navigate,
        navigation_error(Some("net::ERR_NAME_NOT_RESOLVED"), Some("L2"), false),
    );
    let error = finished(op).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Navigation to http://nope.invalid/ failed: net::ERR_NAME_NOT_RESOLVED"
    );
    assert_eq!(count(&harness, "Page.navigate"), 1);
}

#[tokio::test]
async fn goto_lands_after_other_navigation_errors_once_the_frame_is_idle() {
    for error_text in [
        "net::ERR_ABORTED",
        "net::ERR_BLOCKED_BY_CLIENT",
        "net::ERR_CERT_AUTHORITY_INVALID",
    ] {
        let mut harness = PageHarness::new().await;
        let page = harness.page.clone();
        let op = tokio::spawn(async move { page.goto("http://127.0.0.1/no-content").await });
        let navigate = harness
            .control
            .wait_for("Page.navigate", Some(PAGE_SESSION))
            .await;
        started(&harness, MAIN, "L2", "http://127.0.0.1/no-content");
        harness
            .control
            .reply(&navigate, navigation_error(Some(error_text), None, false));
        still_waiting(&op, "the frame stopped loading").await;
        stopped(&harness, MAIN);
        assert_eq!(
            finished(op).await.unwrap(),
            NavigationOutcome::Loaded {
                url: START_URL.to_owned()
            },
            "{error_text}"
        );
    }
}

#[tokio::test]
async fn goto_reports_downloads_and_same_document_navigations() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let download = tokio::spawn(async move { page.goto("http://127.0.0.1/report.pdf").await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &navigate,
        navigation_error(Some("net::ERR_ABORTED"), Some("L2"), true),
    );
    assert_eq!(
        finished(download).await.unwrap(),
        NavigationOutcome::Download {
            url: "http://127.0.0.1/report.pdf".to_owned()
        }
    );

    let page = harness.page.clone();
    let fragment = tokio::spawn(async move { page.goto("http://127.0.0.1/#details").await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    harness.emit(
        "Page.navigatedWithinDocument",
        json!({"frameId": MAIN, "url": "http://127.0.0.1/#details", "navigationType": "fragment"}),
    );
    harness
        .control
        .reply(&navigate, json!({"frameId": MAIN, "isDownload": false}));
    assert_eq!(
        finished(fragment).await.unwrap(),
        NavigationOutcome::SameDocument {
            url: "http://127.0.0.1/#details".to_owned()
        }
    );
    assert_eq!(
        harness.page.url().await.unwrap(),
        "http://127.0.0.1/#details"
    );
}

#[tokio::test]
async fn goto_waits_for_the_load_of_the_committed_document() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.goto(NEXT_URL).await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    started(&harness, MAIN, "L2", NEXT_URL);
    harness.emit(
        "Page.lifecycleEvent",
        json!({"frameId": MAIN, "loaderId": "L2", "name": "init"}),
    );
    committed(&harness, MAIN, "L2", NEXT_URL);
    harness
        .control
        .reply(&navigate, json!({"frameId": MAIN, "loaderId": "L2"}));
    still_waiting(&op, "the document loaded").await;
    harness.emit(
        "Page.lifecycleEvent",
        json!({"frameId": MAIN, "loaderId": "L2", "name": "load"}),
    );
    stopped(&harness, MAIN);
    assert_eq!(
        finished(op).await.unwrap(),
        NavigationOutcome::Loaded {
            url: NEXT_URL.to_owned()
        }
    );
}

#[tokio::test]
async fn losing_the_connection_during_the_load_wait_fails_goto_at_once() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(LONG_PAGE_LOAD);
    let op = goto_awaiting_load(&mut harness).await;
    still_waiting(&op, "the document loaded").await;
    lose_connection(&harness);
    assert_disconnected(finished(op).await.unwrap_err());
}

#[tokio::test]
async fn losing_the_connection_while_goto_stops_loading_skips_the_stop_wait() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_millis(300));
    let op = goto_awaiting_load(&mut harness).await;
    harness
        .control
        .wait_for("Page.stopLoading", Some(PAGE_SESSION))
        .await;
    lose_connection(&harness);
    assert_disconnected(finished(op).await.unwrap_err());
}

#[tokio::test]
async fn a_connection_lost_before_the_page_load_timeout_is_not_waited_out_at_the_deadline() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_millis(300));
    harness.control.set_auto_reply(|command| {
        let flush = command.method == "Runtime.evaluate" && command.params["expression"] == "1";
        if flush {
            None
        } else {
            default_auto_reply(command)
        }
    });
    let cursor = harness.connection.events().cursor();
    started(&harness, MAIN, "L2", NEXT_URL);
    processed(&harness, cursor, "Page.frameStartedLoading").await;
    let op = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    let flush = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply(&flush, json!({"result": {"type": "number", "value": 1}}));
    lose_connection(&harness);
    assert_disconnected(finished(op).await.unwrap_err());
    assert_eq!(count(&harness, "DOM.getDocument"), 0);
}

#[tokio::test]
async fn goto_refuses_javascript_urls_without_sending_anything() {
    let harness = PageHarness::new().await;
    let before = harness.control.commands_seen().len();
    let error = harness.page.goto("JavaScript:alert(1)").await.unwrap_err();
    assert!(
        matches!(error, BrowserError::InvalidArgument { ref message } if message.contains("Execute JavaScript"))
    );
    assert_eq!(harness.control.commands_seen().len(), before);
}

#[tokio::test]
async fn a_dialog_during_goto_is_an_outcome_not_an_error() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.goto(NEXT_URL).await });
    harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": START_URL, "frameId": MAIN, "message": "", "type": "beforeunload", "defaultPrompt": ""}),
    );
    assert_eq!(finished(op).await.unwrap(), NavigationOutcome::DialogOpened);
    let dialog = harness.page.pending_dialog().unwrap();
    assert_eq!(dialog.kind, DialogType::BeforeUnload);
}

#[tokio::test]
async fn reloading_a_crashed_page_waits_for_the_renderer_to_come_back() {
    let mut harness = PageHarness::new().await;
    let cursor = harness.connection.events().cursor();
    harness.emit("Inspector.targetCrashed", json!({}));
    processed(&harness, cursor, "Inspector.targetCrashed").await;
    assert!(matches!(
        harness.page.title().await,
        Err(BrowserError::TargetCrashed { .. })
    ));
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.reload().await });
    let reload = harness
        .control
        .wait_for("Page.reload", Some(PAGE_SESSION))
        .await;
    harness.control.reply(&reload, json!({}));
    still_waiting(&op, "Inspector.targetReloadedAfterCrash").await;
    harness.emit("Inspector.targetReloadedAfterCrash", json!({}));
    started(&harness, MAIN, "L2", START_URL);
    committed(&harness, MAIN, "L2", START_URL);
    still_waiting(&op, "the reloaded document stopped loading").await;
    stopped(&harness, MAIN);
    finished(op).await.unwrap();
    assert_eq!(harness.page.url().await.unwrap(), START_URL);
}

#[tokio::test]
async fn back_and_forward_follow_the_history_and_ignore_its_ends() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let back = tokio::spawn(async move { page.back().await });
    let history = harness
        .control
        .wait_for("Page.getNavigationHistory", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &history,
        json!({"currentIndex": 0, "entries": [{"id": 1, "url": START_URL, "title": ""}]}),
    );
    finished(back).await.unwrap();
    assert_eq!(count(&harness, "Page.navigateToHistoryEntry"), 0);

    let page = harness.page.clone();
    let forward = tokio::spawn(async move { page.forward().await });
    let history = harness
        .control
        .wait_for("Page.getNavigationHistory", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &history,
        json!({"currentIndex": 0, "entries": [
            {"id": 1, "url": START_URL, "title": ""},
            {"id": 4, "url": NEXT_URL, "title": "Next"}
        ]}),
    );
    let entry = harness
        .control
        .wait_for("Page.navigateToHistoryEntry", Some(PAGE_SESSION))
        .await;
    assert_eq!(entry.params["entryId"], 4);
    harness.control.reply(&entry, json!({}));
    finished(forward).await.unwrap();
}

#[tokio::test]
async fn stepping_past_the_history_ends_of_a_crashed_page_fails_at_once() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(LONG_PAGE_LOAD);
    crash(&harness).await;
    let page = harness.page.clone();
    let back = tokio::spawn(async move { page.back().await });
    reply_history(&mut harness, 0).await;
    let error = finished(back).await.unwrap_err();
    assert!(
        matches!(error, BrowserError::TargetCrashed { .. }),
        "{error:?}"
    );

    let page = harness.page.clone();
    let forward = tokio::spawn(async move { page.forward().await });
    reply_history(&mut harness, 1).await;
    let error = finished(forward).await.unwrap_err();
    assert!(
        matches!(error, BrowserError::TargetCrashed { .. }),
        "{error:?}"
    );
    assert_eq!(count(&harness, "Page.navigateToHistoryEntry"), 0);
}

#[tokio::test]
async fn losing_the_connection_while_a_crashed_page_reloads_fails_at_once() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(LONG_PAGE_LOAD);
    crash(&harness).await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.reload().await });
    let reload = harness
        .control
        .wait_for("Page.reload", Some(PAGE_SESSION))
        .await;
    harness.control.reply(&reload, json!({}));
    still_waiting(&op, "the renderer came back").await;
    lose_connection(&harness);
    assert_disconnected(finished(op).await.unwrap_err());
}

#[tokio::test]
async fn cdp_passes_commands_through_on_the_page_session() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.cdp("Page.getLayoutMetrics", json!({"x": 1})).await });
    let command = harness
        .control
        .wait_for("Page.getLayoutMetrics", Some(PAGE_SESSION))
        .await;
    assert_eq!(command.params, json!({"x": 1}));
    harness
        .control
        .reply(&command, json!({"cssVisualViewport": {"clientWidth": 800}}));
    assert_eq!(
        finished(op).await.unwrap()["cssVisualViewport"]["clientWidth"],
        800
    );
    let page = harness.page.clone();
    let failing = tokio::spawn(async move { page.cdp("Page.bogus", json!({})).await });
    let command = harness
        .control
        .wait_for("Page.bogus", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply_error(&command, -32601, "'Page.bogus' wasn't found");
    assert_eq!(
        finished(failing).await.unwrap_err().to_string(),
        "Chrome DevTools command Page.bogus failed (-32601): 'Page.bogus' wasn't found"
    );
}

#[tokio::test]
async fn cdp_replays_emulation_on_iframe_sessions() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let page = harness.page.clone();
    let op = tokio::spawn(async move {
        page.cdp(
            "Emulation.setTimezoneOverride",
            json!({"timezoneId": "Europe/Berlin"}),
        )
        .await
    });
    let on_page = harness
        .control
        .wait_for("Emulation.setTimezoneOverride", Some(PAGE_SESSION))
        .await;
    harness.control.reply(&on_page, json!({}));
    let on_child = harness
        .control
        .wait_for("Emulation.setTimezoneOverride", Some("S2"))
        .await;
    assert_eq!(on_child.params["timezoneId"], "Europe/Berlin");
    harness
        .control
        .reply_error(&on_child, -32000, "Timezone override is already in effect");
    assert_eq!(finished(op).await.unwrap(), json!({}));
}

#[tokio::test]
async fn child_frame_resolves_the_owner_frame_id() {
    let mut harness = PageHarness::new().await;
    harness.attach_child("S2", "F2", MAIN, "http://127.0.0.1/child", "L2");
    let owner = element(&harness.page, MAIN, 7).unwrap();
    let main = harness.page.main_frame();
    let op = tokio::spawn(async move { main.child_frame(&owner).await });
    let describe = harness
        .control
        .wait_for("DOM.describeNode", Some(PAGE_SESSION))
        .await;
    assert_eq!(describe.params["backendNodeId"], 7);
    harness.control.reply(
        &describe,
        json!({"node": {"nodeId": 0, "backendNodeId": 7, "nodeName": "IFRAME", "frameId": "F2"}}),
    );
    let child = finished(op).await.unwrap();
    assert_eq!(child.id(), &FrameId::from("F2"));
    assert!(!child.is_main());
    assert_eq!(child.parent().unwrap().id(), &FrameId::from(MAIN));
    assert!(harness.page.main_frame().parent().is_none());
}

#[tokio::test]
async fn child_frame_rejects_non_owners_and_frames_that_never_load() {
    let mut harness = PageHarness::new().await;
    for (node, message) in [
        (
            json!({"nodeId": 0, "backendNodeId": 7, "nodeName": "DIV"}),
            "The element is not an iframe or frame",
        ),
        (
            json!({"nodeId": 0, "backendNodeId": 7, "nodeName": "IFRAME", "frameId": "F9"}),
            "The frame has not loaded yet",
        ),
    ] {
        let owner = element(&harness.page, MAIN, 7).unwrap();
        let main = harness.page.main_frame();
        let op = tokio::spawn(async move { main.child_frame(&owner).await });
        let describe = harness
            .control
            .wait_for("DOM.describeNode", Some(PAGE_SESSION))
            .await;
        harness.control.reply(&describe, json!({"node": node}));
        let error = finished(op).await.err().expect("child_frame found a frame");
        assert!(
            matches!(error, BrowserError::NoSuchFrame { message: ref text } if text == message),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn child_frame_waits_briefly_for_a_frame_being_attached() {
    let mut harness = PageHarness::new().await;
    let owner = element(&harness.page, MAIN, 8).unwrap();
    let main = harness.page.main_frame();
    let op = tokio::spawn(async move { main.child_frame(&owner).await });
    let describe = harness
        .control
        .wait_for("DOM.describeNode", Some(PAGE_SESSION))
        .await;
    harness.control.reply(
        &describe,
        json!({"node": {"nodeId": 0, "backendNodeId": 8, "nodeName": "IFRAME", "frameId": "F3"}}),
    );
    harness.emit(
        "Page.frameAttached",
        json!({"frameId": "F3", "parentFrameId": MAIN}),
    );
    assert_eq!(finished(op).await.unwrap().id(), &FrameId::from("F3"));
}

#[tokio::test]
async fn child_frame_of_an_owner_from_an_old_document_is_stale() {
    let harness = PageHarness::new().await;
    let owner = element(&harness.page, MAIN, 7).unwrap();
    let cursor = harness.connection.events().cursor();
    committed(&harness, MAIN, "L2", NEXT_URL);
    processed(&harness, cursor, "Page.frameNavigated").await;
    let error = harness
        .page
        .main_frame()
        .child_frame(&owner)
        .await
        .err()
        .expect("child_frame used a stale owner");
    assert!(matches!(error, BrowserError::StaleElement), "{error:?}");
    assert_eq!(count(&harness, "DOM.describeNode"), 0);
}

#[tokio::test]
async fn output_ops_are_not_retried_on_timeouts() {
    let mut harness = PageHarness::new().await;
    harness.set_page_load_timeout(Duration::from_millis(300));
    let op = spawn_command(&harness.page, MAIN, OpClass::Output, "Page.printToPDF");
    harness
        .control
        .wait_for("Page.printToPDF", Some(PAGE_SESSION))
        .await;
    let error = finished(op).await.unwrap_err();
    assert!(
        matches!(error, BrowserError::Timeout { ref method, .. } if method == "Page.printToPDF"),
        "{error:?}"
    );
    assert_eq!(count(&harness, "Page.printToPDF"), 1);
}

#[tokio::test]
async fn goto_on_a_crashed_page_waits_for_the_new_renderer() {
    let mut harness = PageHarness::new().await;
    let cursor = harness.connection.events().cursor();
    harness.emit("Inspector.targetCrashed", json!({}));
    processed(&harness, cursor, "Inspector.targetCrashed").await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.goto(NEXT_URL).await });
    let navigate = harness
        .control
        .wait_for("Page.navigate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply(&navigate, json!({"frameId": MAIN, "loaderId": "L2"}));
    still_waiting(&op, "the renderer came back").await;
    harness.emit("Inspector.targetReloadedAfterCrash", json!({}));
    started(&harness, MAIN, "L2", NEXT_URL);
    committed(&harness, MAIN, "L2", NEXT_URL);
    stopped(&harness, MAIN);
    assert_eq!(
        finished(op).await.unwrap(),
        NavigationOutcome::Loaded {
            url: NEXT_URL.to_owned()
        }
    );
}

#[tokio::test]
async fn a_flush_failing_on_a_missing_context_waits_for_the_next_event() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        let flush = command.method == "Runtime.evaluate" && command.params["expression"] == "1";
        if flush {
            None
        } else {
            default_auto_reply(command)
        }
    });
    let op = spawn_command(&harness.page, MAIN, OpClass::Read, "DOM.getDocument");
    let flush = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply_error(&flush, -32000, "Cannot find default execution context");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        harness.control.try_next_command().is_none(),
        "the flush was repeated without any page event"
    );
    harness.context_created(PAGE_SESSION, MAIN, 3, "S1-main-2", false);
    let flush = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply(&flush, json!({"result": {"type": "number", "value": 1}}));
    document_reply(&mut harness, PAGE_SESSION).await;
    let flush = harness
        .control
        .wait_for("Runtime.evaluate", Some(PAGE_SESSION))
        .await;
    harness
        .control
        .reply(&flush, json!({"result": {"type": "number", "value": 1}}));
    assert!(finished(op).await.is_ok());
}

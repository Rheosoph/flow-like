use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use flow_like_browser::output::{Clip, ScreenshotOptions};
use flow_like_browser::refs::NodeRef;
use flow_like_browser::test_hooks::TestSetup;
use flow_like_browser::testing::{PageHarness, default_auto_reply};
use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport, SentCommand};
use flow_like_browser::types::{LoaderId, TargetId};
use flow_like_browser::{Browser, BrowserError, ConnectionKind, Page};
use serde_json::{Value, json};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nflow-like";
const WINDOW: i64 = 3;
const PAGE_URL: &str = "http://127.0.0.1/";

fn sent(control: &InMemoryControl, method: &str) -> Vec<SentCommand> {
    control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method == method)
        .collect()
}

fn capture_reply(command: &SentCommand) -> Option<Value> {
    match command.method.as_str() {
        "Page.captureScreenshot" => Some(json!({"data": BASE64.encode(PNG)})),
        _ => default_auto_reply(command),
    }
}

#[tokio::test]
async fn screenshots_capture_png_on_the_page_session_with_the_requested_clip() {
    let harness = PageHarness::new().await;
    harness.control.set_auto_reply(capture_reply);
    let clip = Clip {
        x: 10.5,
        y: 20.0,
        width: 300.0,
        height: 150.0,
        scale: 2.0,
    };
    let clipped = ScreenshotOptions {
        clip: Some(clip),
        capture_beyond_viewport: true,
    };
    assert_eq!(harness.page.screenshot(clipped).await.unwrap(), PNG);
    let viewport = harness.page.screenshot(ScreenshotOptions::default());
    assert_eq!(viewport.await.unwrap(), PNG);

    let captures = sent(&harness.control, "Page.captureScreenshot");
    assert_eq!(captures.len(), 2);
    assert!(
        captures
            .iter()
            .all(|capture| capture.session.as_deref() == Some("S1"))
    );
    assert_eq!(
        captures[0].params,
        json!({
            "format": "png",
            "captureBeyondViewport": true,
            "clip": {"x": 10.5, "y": 20.0, "width": 300.0, "height": 150.0, "scale": 2.0},
        })
    );
    assert_eq!(
        captures[1].params,
        json!({"format": "png", "captureBeyondViewport": false})
    );
}

#[tokio::test]
async fn a_screenshot_with_undecodable_data_names_the_command() {
    let harness = PageHarness::new().await;
    harness
        .control
        .set_auto_reply(|command| match command.method.as_str() {
            "Page.captureScreenshot" => Some(json!({"data": "not base64!"})),
            _ => default_auto_reply(command),
        });
    let error = harness
        .page
        .screenshot(ScreenshotOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, BrowserError::Protocol { method, .. } if method == "Page.captureScreenshot"),
        "{error}"
    );
}

fn pdf_reply(chunks: Arc<Mutex<VecDeque<Value>>>) -> impl Fn(&SentCommand) -> Option<Value> {
    move |command| match command.method.as_str() {
        "Page.printToPDF" => Some(json!({"stream": "7", "data": ""})),
        "IO.read" => chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front(),
        "IO.close" => None,
        _ => default_auto_reply(command),
    }
}

#[tokio::test]
async fn pdf_streams_are_read_on_the_page_session_until_eof_then_closed() {
    let PageHarness {
        page,
        mut control,
        connection: _connection,
    } = PageHarness::new().await;
    let chunks = VecDeque::from([
        json!({"data": BASE64.encode("%PDF-1.4\n"), "base64Encoded": true, "eof": false}),
        json!({"data": "1 0 obj\n", "base64Encoded": false, "eof": false}),
        json!({"data": BASE64.encode("%%EOF"), "base64Encoded": true, "eof": false}),
        json!({"data": "", "base64Encoded": true, "eof": true}),
    ]);
    control.set_auto_reply(pdf_reply(Arc::new(Mutex::new(chunks))));

    let pdf = page
        .print_pdf(json!({"landscape": true, "transferMode": "ReturnAsBase64"}))
        .await
        .unwrap();
    assert_eq!(pdf, b"%PDF-1.4\n1 0 obj\n%%EOF");
    let close = control.wait_for("IO.close", Some("S1")).await;
    assert_eq!(close.params, json!({"handle": "7"}));

    let prints = sent(&control, "Page.printToPDF");
    assert_eq!(prints.len(), 1);
    assert_eq!(prints[0].session.as_deref(), Some("S1"));
    assert_eq!(
        prints[0].params,
        json!({"landscape": true, "transferMode": "ReturnAsStream"})
    );
    let reads = sent(&control, "IO.read");
    assert_eq!(reads.len(), 4);
    for read in &reads {
        assert_eq!(read.session.as_deref(), Some("S1"));
        assert_eq!(read.params, json!({"handle": "7", "size": 1_048_576}));
    }
    let seen = control.commands_seen();
    let last_read = seen.iter().rposition(|command| command.method == "IO.read");
    let closed = seen.iter().position(|command| command.method == "IO.close");
    assert!(last_read < closed, "IO.close must follow the last IO.read");
}

#[tokio::test]
async fn the_guard_closes_the_stream_when_a_read_fails() {
    let PageHarness {
        page,
        mut control,
        connection: _connection,
    } = PageHarness::new().await;
    control.set_auto_reply(pdf_reply(Arc::new(Mutex::new(VecDeque::new()))));

    let (printed, close) = tokio::join!(page.print_pdf(Value::Null), async {
        let read = control.wait_for("IO.read", Some("S1")).await;
        control.reply_error(&read, -32602, "Invalid stream handle");
        control.wait_for("IO.close", Some("S1")).await
    });
    assert_eq!(close.params, json!({"handle": "7"}));
    let error = printed.unwrap_err();
    assert!(
        matches!(&error, BrowserError::Protocol { method, code: -32602, .. } if method == "IO.read"),
        "{error}"
    );
}

#[tokio::test]
async fn a_stream_that_opens_after_the_op_was_dropped_is_still_closed() {
    let PageHarness {
        page,
        mut control,
        connection: _connection,
    } = PageHarness::new().await;
    control.set_auto_reply(|command| match command.method.as_str() {
        "Page.printToPDF" | "IO.close" => None,
        _ => default_auto_reply(command),
    });

    let print = tokio::select! {
        printed = page.print_pdf(Value::Null) => panic!("the print ended before Chrome replied: {printed:?}"),
        print = control.wait_for("Page.printToPDF", Some("S1")) => print,
    };
    control.reply(&print, json!({"stream": "9", "data": ""}));

    let close = control.wait_for("IO.close", Some("S1")).await;
    assert_eq!(close.params, json!({"handle": "9"}));
    assert!(sent(&control, "IO.read").is_empty());
}

#[tokio::test(start_paused = true)]
async fn print_to_pdf_is_not_retried_after_a_timeout() {
    let harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        (command.method != "Page.printToPDF")
            .then(|| default_auto_reply(command))
            .flatten()
    });
    let error = harness.page.print_pdf(json!({})).await.unwrap_err();
    assert!(
        matches!(&error, BrowserError::Timeout { method, .. } if method == "Page.printToPDF"),
        "{error}"
    );
    assert_eq!(sent(&harness.control, "Page.printToPDF").len(), 1);
    assert!(sent(&harness.control, "IO.close").is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_opens_after_the_op_timed_out_is_still_closed() {
    let PageHarness {
        page,
        mut control,
        connection: _connection,
    } = PageHarness::new().await;
    control.set_auto_reply(|command| match command.method.as_str() {
        "Page.printToPDF" | "IO.close" => None,
        _ => default_auto_reply(command),
    });

    let error = page.print_pdf(Value::Null).await.unwrap_err();
    assert!(
        matches!(&error, BrowserError::Timeout { method, timeout_ms } if method == "Page.printToPDF" && *timeout_ms > 0),
        "{error}"
    );
    let print = control.wait_for("Page.printToPDF", Some("S1")).await;
    control.reply(&print, json!({"stream": "9", "data": ""}));

    let close = control.wait_for("IO.close", Some("S1")).await;
    assert_eq!(close.params, json!({"handle": "9"}));
    assert!(sent(&control, "IO.read").is_empty());
    assert_eq!(sent(&control, "Page.printToPDF").len(), 1);
}

#[tokio::test]
async fn print_options_must_be_an_object() {
    let harness = PageHarness::new().await;
    let error = harness.page.print_pdf(json!("A4")).await.unwrap_err();
    assert!(
        matches!(error, BrowserError::InvalidArgument { .. }),
        "{error}"
    );
    assert!(sent(&harness.control, "Page.printToPDF").is_empty());
}

struct FakeWindow {
    bounds: Value,
    pending: Option<Value>,
    frame_height: i64,
    state_locked: bool,
    screen_height: Option<i64>,
}

impl FakeWindow {
    fn new(bounds: Value, frame_height: i64) -> Arc<Mutex<FakeWindow>> {
        Arc::new(Mutex::new(FakeWindow {
            bounds,
            pending: None,
            frame_height,
            state_locked: false,
            screen_height: None,
        }))
    }

    fn reply(&mut self, command: &SentCommand) -> Option<Value> {
        match command.method.as_str() {
            "Browser.getWindowForTarget" => {
                Some(json!({"windowId": WINDOW, "bounds": self.bounds}))
            }
            "Browser.getWindowBounds" => {
                let current = json!({"bounds": self.bounds});
                if let Some(applied) = self.pending.take() {
                    self.bounds = applied;
                }
                Some(current)
            }
            "Browser.setWindowBounds" => {
                self.request(&command.params["bounds"]);
                Some(json!({}))
            }
            "Runtime.evaluate" if is_viewport_probe(command) => {
                let width = self.bounds["width"].as_i64()?;
                let height = self.bounds["height"].as_i64()? - self.frame_height;
                Some(
                    json!({"result": {"type": "object", "subtype": "array", "value": [width, height]}}),
                )
            }
            _ => default_auto_reply(command),
        }
    }

    fn request(&mut self, bounds: &Value) {
        if self.state_locked && bounds.get("windowState").is_some() {
            return;
        }
        let mut next = self.pending.clone().unwrap_or_else(|| self.bounds.clone());
        for (key, value) in bounds.as_object().into_iter().flatten() {
            next[key] = value.clone();
        }
        if let (Some(screen), Some(height)) = (self.screen_height, next["height"].as_i64()) {
            next["height"] = height.min(screen).into();
        }
        self.pending = Some(next);
    }
}

fn is_viewport_probe(command: &SentCommand) -> bool {
    command.params["expression"]
        .as_str()
        .is_some_and(|expression| expression.contains("innerWidth"))
}

fn serve_window(control: &InMemoryControl, window: &Arc<Mutex<FakeWindow>>) {
    let window = window.clone();
    control.set_auto_reply(move |command| {
        window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reply(command)
    });
}

fn window_bounds(window: &Arc<Mutex<FakeWindow>>) -> Value {
    window
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .bounds
        .clone()
}

fn requested_bounds(control: &InMemoryControl) -> Vec<Value> {
    sent(control, "Browser.setWindowBounds")
        .into_iter()
        .map(|command| {
            assert_eq!(
                command.session, None,
                "window commands go to the browser session"
            );
            assert_eq!(command.params["windowId"], WINDOW);
            command.params["bounds"].clone()
        })
        .collect()
}

#[tokio::test]
async fn the_window_is_restored_resized_and_grown_once_by_the_browser_frame() {
    let harness = PageHarness::new().await;
    let window = FakeWindow::new(
        json!({"left": 40, "top": 40, "width": 800, "height": 600, "windowState": "maximized"}),
        143,
    );
    serve_window(&harness.control, &window);

    harness.page.fit_viewport(1920, 1080).await.unwrap();

    assert_eq!(
        requested_bounds(&harness.control),
        [
            json!({"windowState": "normal"}),
            json!({"left": 0, "top": 0, "width": 1920, "height": 1080}),
            json!({"left": 0, "top": 0, "width": 1920, "height": 1223}),
        ]
    );
    assert_eq!(
        window_bounds(&window),
        json!({"left": 0, "top": 0, "width": 1920, "height": 1223, "windowState": "normal"})
    );
    let lookup = &sent(&harness.control, "Browser.getWindowForTarget")[0];
    assert_eq!(lookup.params, json!({"targetId": "T1"}));
    assert_eq!(lookup.session, None);
    let probes: Vec<_> = sent(&harness.control, "Runtime.evaluate")
        .into_iter()
        .filter(is_viewport_probe)
        .collect();
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].session.as_deref(), Some("S1"));
}

#[tokio::test]
async fn a_window_that_already_fits_is_neither_polled_nor_grown() {
    let harness = PageHarness::new().await;
    let window = FakeWindow::new(
        json!({"left": 0, "top": 0, "width": 1280, "height": 720, "windowState": "normal"}),
        0,
    );
    serve_window(&harness.control, &window);

    harness.page.fit_viewport(1280, 720).await.unwrap();

    assert_eq!(
        requested_bounds(&harness.control),
        [json!({"left": 0, "top": 0, "width": 1280, "height": 720})]
    );
    assert!(sent(&harness.control, "Browser.getWindowBounds").is_empty());
}

#[tokio::test]
async fn a_window_stuck_outside_the_normal_state_fails_the_fit() {
    let harness = PageHarness::new().await;
    let window = FakeWindow::new(
        json!({"left": 0, "top": 0, "width": 800, "height": 600, "windowState": "fullscreen"}),
        0,
    );
    window
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .state_locked = true;
    serve_window(&harness.control, &window);

    let error = harness.page.fit_viewport(1280, 720).await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("failed to change window state to 'normal', current state is 'fullscreen'"),
        "{error}"
    );
    assert_eq!(
        requested_bounds(&harness.control),
        [json!({"windowState": "normal"})]
    );
}

#[tokio::test]
async fn a_window_the_screen_clamps_is_not_grown_by_a_viewport_measured_mid_resize() {
    let harness = PageHarness::new().await;
    let window = FakeWindow::new(
        json!({"left": 0, "top": 0, "width": 800, "height": 600, "windowState": "normal"}),
        143,
    );
    window
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .screen_height = Some(900);
    serve_window(&harness.control, &window);

    harness.page.fit_viewport(1920, 1080).await.unwrap();

    assert_eq!(
        requested_bounds(&harness.control),
        [json!({"left": 0, "top": 0, "width": 1920, "height": 1080})]
    );
    assert_eq!(sent(&harness.control, "Browser.getWindowBounds").len(), 5);
    assert!(
        !harness
            .control
            .commands_seen()
            .iter()
            .any(is_viewport_probe)
    );
}

async fn fit_without(method: &str) -> Vec<SentCommand> {
    let PageHarness {
        page,
        mut control,
        connection: _connection,
    } = PageHarness::new().await;
    let window = FakeWindow::new(
        json!({"left": 0, "top": 0, "width": 800, "height": 600, "windowState": "normal"}),
        0,
    );
    let withheld = method.to_owned();
    control.set_auto_reply(move |command| {
        (command.method != withheld)
            .then(|| {
                window
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .reply(command)
            })
            .flatten()
    });
    let (fitted, ()) = tokio::join!(page.fit_viewport(1280, 720), async {
        let unsupported = control.wait_for(method, None).await;
        control.reply_error(&unsupported, -32601, &format!("'{method}' wasn't found"));
    });
    fitted.unwrap_or_else(|error| panic!("a missing {method} must be tolerated: {error}"));
    control.commands_seen()
}

#[tokio::test]
async fn browsers_without_window_commands_keep_their_viewport() {
    let seen = fit_without("Browser.getWindowForTarget").await;
    assert!(
        !seen
            .iter()
            .any(|command| command.method == "Browser.setWindowBounds")
    );
    let seen = fit_without("Browser.setWindowBounds").await;
    assert!(!seen.iter().any(is_viewport_probe));
}

fn target_info(attached: bool) -> Value {
    json!({"targetId": "T1", "type": "page", "title": "", "url": PAGE_URL, "attached": attached, "browserContextId": "C1"})
}

fn page_script_value() -> Value {
    json!({
        "s": "complete", "u": PAGE_URL, "b": PAGE_URL,
        "x": 8, "y": 8, "left": 8, "top": 8, "width": 100, "height": 50,
        "pageXOffset": 0, "pageYOffset": 0, "clientWidth": 1280, "clientHeight": 720,
    })
}

fn headful_browser_reply(command: &SentCommand) -> Option<Value> {
    let script_result =
        || Some(json!({"result": {"type": "object", "value": page_script_value()}}));
    match command.method.as_str() {
        "Target.setDiscoverTargets"
        | "Target.attachToTarget"
        | "Runtime.enable"
        | "Page.createIsolatedWorld" => None,
        "Browser.getVersion" => Some(json!({
            "protocolVersion": "1.3", "product": "Chrome/154.0.8037.92", "revision": "@1",
            "userAgent": "Mozilla/5.0 Chrome/154.0.8037.92", "jsVersion": "15.4",
        })),
        "Target.getTargets" => Some(json!({"targetInfos": [target_info(false)]})),
        "Page.getFrameTree" => Some(json!({"frameTree": {
            "frame": {"id": "T1", "loaderId": "L1", "url": PAGE_URL, "securityOrigin": "http://127.0.0.1", "mimeType": "text/html"},
            "childFrames": [],
        }})),
        "Page.captureScreenshot" => Some(json!({"data": BASE64.encode(PNG)})),
        "DOM.resolveNode" => Some(
            json!({"object": {"type": "object", "subtype": "node", "className": "HTMLDivElement", "objectId": "node-7"}}),
        ),
        "Runtime.callFunctionOn"
            if command.params["functionDeclaration"]
                .as_str()
                .is_some_and(|declaration| declaration.contains("isConnected")) =>
        {
            Some(json!({"result": {"type": "boolean", "value": true}}))
        }
        "Runtime.callFunctionOn" => script_result(),
        "Runtime.evaluate" if command.params["expression"] != "1" => script_result(),
        _ => default_auto_reply(command).or_else(|| Some(json!({}))),
    }
}

fn context_created(control: &InMemoryControl, id: i64, name: &str, default: bool) {
    let kind = if default { "default" } else { "isolated" };
    control.emit(
        "Runtime.executionContextCreated",
        Some("S1"),
        json!({"context": {
            "id": id, "origin": "http://127.0.0.1", "name": name, "uniqueId": format!("S1-{id}"),
            "auxData": {"isDefault": default, "type": kind, "frameId": "T1"},
        }}),
    );
}

async fn play_chrome_for_the_attach(control: &mut InMemoryControl) {
    let discover = control.wait_for("Target.setDiscoverTargets", None).await;
    control.emit(
        "Target.targetCreated",
        None,
        json!({"targetInfo": target_info(false)}),
    );
    control.reply(&discover, json!({}));
    let attach = control.wait_for("Target.attachToTarget", None).await;
    control.emit(
        "Target.attachedToTarget",
        None,
        json!({"sessionId": "S1", "targetInfo": target_info(true), "waitingForDebugger": false}),
    );
    control.reply(&attach, json!({"sessionId": "S1"}));
    let runtime = control.wait_for("Runtime.enable", Some("S1")).await;
    context_created(control, 1, "", true);
    control.reply(&runtime, json!({}));
    let isolated = control
        .wait_for("Page.createIsolatedWorld", Some("S1"))
        .await;
    let world = isolated.params["worldName"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    context_created(control, 2, &world, false);
    control.reply(&isolated, json!({"executionContextId": 2}));
}

async fn headful_page(
    control: &mut InMemoryControl,
    transport: InMemoryTransport,
) -> (Browser, Page) {
    let setup = TestSetup {
        kind: ConnectionKind::Direct,
        headless: false,
        page_load_timeout: Duration::from_secs(30),
        process: None,
        staging: None,
        run_owned_setup: false,
    };
    let (opened, ()) = tokio::join!(
        async {
            let browser = Browser::connect_transport(Box::new(transport), setup).await?;
            let page = browser.page(&TargetId::from("T1")).await?;
            Ok::<_, BrowserError>((browser, page))
        },
        play_chrome_for_the_attach(control)
    );
    opened.unwrap_or_else(|error| panic!("the headful page did not open: {error}"))
}

fn captures_after_activation(seen: &[SentCommand]) -> usize {
    let mut activated = false;
    let mut captures = 0;
    for command in seen {
        match command.method.as_str() {
            "Target.activateTarget" => {
                assert_eq!(
                    command.session, None,
                    "activation goes to the browser session"
                );
                assert_eq!(command.params, json!({"targetId": "T1"}));
                activated = true;
            }
            "Page.captureScreenshot" => {
                assert!(
                    activated,
                    "capture {captures} was not preceded by Target.activateTarget"
                );
                assert_eq!(command.session.as_deref(), Some("S1"));
                activated = false;
                captures += 1;
            }
            _ => {}
        }
    }
    captures
}

#[tokio::test]
async fn a_headful_tab_is_activated_before_every_capture_path() {
    let (transport, mut control) = InMemoryTransport::new();
    control.set_auto_reply(headful_browser_reply);
    let (_browser, page) = headful_page(&mut control, transport).await;

    assert_eq!(
        page.screenshot(ScreenshotOptions::default()).await.unwrap(),
        PNG
    );
    let captured = page
        .cdp("Page.captureScreenshot", json!({"format": "png"}))
        .await
        .unwrap();
    assert_eq!(captured["data"], BASE64.encode(PNG));
    let node = NodeRef {
        page: TargetId::from("T1"),
        local_root: TargetId::from("T1"),
        frame_id: "T1".into(),
        loader_id: LoaderId::from("L1"),
        backend_node_id: 7,
    };
    let element = page
        .resolve_node(&node, "e1", Some(&LoaderId::from("L1")))
        .await
        .unwrap();
    assert_eq!(element.screenshot_png().await.unwrap(), PNG);

    assert_eq!(captures_after_activation(&control.commands_seen()), 3);
}

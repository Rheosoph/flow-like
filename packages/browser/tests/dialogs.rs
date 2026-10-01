use std::time::Duration;

use flow_like_browser::input::{ActionOutcome, ClickOptions};
use flow_like_browser::testing::{
    CommandOutcome, OpClass, PageHarness, default_auto_reply, run_command,
};
use flow_like_browser::transport::memory::{InMemoryControl, SentCommand};
use flow_like_browser::types::DialogType;
use flow_like_browser::{BrowserError, DialogAction};
use serde_json::json;
use tokio::task::JoinHandle;
use tokio::time::Instant;

const MAIN: &str = "T1";
const PAGE_SESSION: &str = "S1";
const FAIL_FAST: Duration = Duration::from_secs(1);
const FINISHES_WITHIN: Duration = Duration::from_secs(5);

async fn finished<T>(op: JoinHandle<T>) -> T {
    tokio::time::timeout(FINISHES_WITHIN, op)
        .await
        .expect("the op did not finish")
        .expect("the op task panicked")
}

async fn open_dialog(harness: &PageHarness, kind: &str, message: &str) {
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "frameId": MAIN, "message": message, "type": kind, "defaultPrompt": "guest"}),
    );
    let deadline = Instant::now() + FINISHES_WITHIN;
    let seen = harness
        .connection
        .events()
        .wait_for(cursor, deadline, |event| {
            &*event.method == "Page.javascriptDialogOpening"
        })
        .await
        .unwrap();
    assert!(seen.is_some(), "the dialog never opened");
}

async fn handle(
    harness: &mut PageHarness,
    action: DialogAction,
) -> (
    SentCommand,
    JoinHandle<flow_like_browser::Result<flow_like_browser::Dialog>>,
) {
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.handle_dialog(action).await });
    let command = harness
        .control
        .wait_for("Page.handleJavaScriptDialog", Some(PAGE_SESSION))
        .await;
    (command, op)
}

async fn next_mouse_event(control: &mut InMemoryControl, kind: &str) -> SentCommand {
    loop {
        let command = control
            .wait_for("Input.dispatchMouseEvent", Some(PAGE_SESSION))
            .await;
        if command.params["type"] == kind {
            return command;
        }
        control.reply(&command, json!({}));
    }
}

#[tokio::test]
async fn an_open_dialog_fails_the_next_op_without_cdp_traffic() {
    let harness = PageHarness::new().await;
    open_dialog(&harness, "alert", "Saved").await;
    let before = harness.control.commands_seen().len();
    let started = Instant::now();
    let url = harness.page.url().await.unwrap_err();
    let title = harness.page.title().await.unwrap_err();
    let cdp = harness
        .page
        .cdp("DOM.getDocument", json!({}))
        .await
        .unwrap_err();
    let goto = harness
        .page
        .goto("http://127.0.0.1/next")
        .await
        .unwrap_err();
    assert!(started.elapsed() < FAIL_FAST);
    assert_eq!(harness.control.commands_seen().len(), before);
    for error in [url, title, cdp, goto] {
        assert!(
            matches!(error, BrowserError::DialogOpen { kind: DialogType::Alert, ref message } if message == "Saved"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "unexpected alert open: {Alert text : Saved}"
        );
    }
}

#[tokio::test]
async fn a_dialog_opening_while_a_read_waits_for_its_reply_fails_it_fast() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move { page.cdp("DOM.getDocument", json!({})).await });
    harness
        .control
        .wait_for("DOM.getDocument", Some(PAGE_SESSION))
        .await;
    let opened_at = Instant::now();
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "frameId": MAIN, "message": "Leave?", "type": "confirm", "defaultPrompt": ""}),
    );
    let error = tokio::time::timeout(FAIL_FAST, op)
        .await
        .expect("the read kept waiting behind the dialog")
        .unwrap()
        .unwrap_err();
    assert!(opened_at.elapsed() < FAIL_FAST);
    assert!(
        matches!(error, BrowserError::DialogOpen { kind: DialogType::Confirm, ref message } if message == "Leave?"),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_op_that_opens_a_dialog_reports_it_as_its_outcome() {
    let mut harness = PageHarness::new().await;
    let page = harness.page.clone();
    let op = tokio::spawn(async move {
        run_command(
            &page,
            MAIN,
            OpClass::Script,
            "Runtime.callFunctionOn",
            json!({}),
        )
        .await
    });
    harness
        .control
        .wait_for("Runtime.callFunctionOn", Some(PAGE_SESSION))
        .await;
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "frameId": MAIN, "message": "Hi", "type": "alert", "defaultPrompt": ""}),
    );
    let outcome = finished(op).await.unwrap();
    assert!(
        matches!(outcome, CommandOutcome::DialogOpened(ref dialog) if dialog.message == "Hi"),
        "{outcome:?}"
    );
    assert_eq!(harness.page.pending_dialog().unwrap().message, "Hi");
}

#[tokio::test]
async fn accepting_a_prompt_sends_the_text_and_clears_the_dialog() {
    let mut harness = PageHarness::new().await;
    open_dialog(&harness, "prompt", "Name?").await;
    let (command, op) = handle(
        &mut harness,
        DialogAction::Accept {
            prompt_text: Some("Felix".to_owned()),
        },
    )
    .await;
    assert_eq!(
        command.params,
        json!({"accept": true, "promptText": "Felix"})
    );
    harness.control.reply(&command, json!({}));
    let dialog = finished(op).await.unwrap();
    assert_eq!(dialog.kind, DialogType::Prompt);
    assert_eq!(dialog.message, "Name?");
    assert_eq!(dialog.default_prompt, "guest");
    assert!(harness.page.pending_dialog().is_none());
    assert_eq!(
        harness.page.url().await.unwrap(),
        "http://127.0.0.1/",
        "ops run again once the dialog is handled"
    );
}

#[tokio::test]
async fn dismissing_and_accepting_without_text() {
    let mut harness = PageHarness::new().await;
    for (action, params) in [
        (DialogAction::Dismiss, json!({"accept": false})),
        (
            DialogAction::Accept { prompt_text: None },
            json!({"accept": true}),
        ),
    ] {
        open_dialog(&harness, "confirm", "Delete?").await;
        let (command, op) = handle(&mut harness, action).await;
        assert_eq!(command.params, params);
        harness.control.reply(&command, json!({}));
        assert_eq!(finished(op).await.unwrap().kind, DialogType::Confirm);
        harness.emit(
            "Page.javascriptDialogClosed",
            json!({"result": false, "userInput": ""}),
        );
    }
}

#[tokio::test]
async fn handling_without_a_dialog_is_no_such_alert() {
    let mut harness = PageHarness::new().await;
    let before = harness.control.commands_seen().len();
    let error = harness
        .page
        .handle_dialog(DialogAction::Dismiss)
        .await
        .unwrap_err();
    assert!(matches!(error, BrowserError::NoDialog));
    assert_eq!(harness.control.commands_seen().len(), before);

    open_dialog(&harness, "alert", "Gone already").await;
    let (command, op) = handle(&mut harness, DialogAction::Dismiss).await;
    harness
        .control
        .reply_error(&command, -32602, "No dialog is showing");
    assert!(matches!(finished(op).await, Err(BrowserError::NoDialog)));
    assert!(harness.page.pending_dialog().is_none());
}

#[tokio::test]
async fn a_held_mouse_button_is_released_when_someone_else_closes_the_dialog() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        let moved =
            command.method == "Input.dispatchMouseEvent" && command.params["type"] == "mouseMoved";
        if moved {
            Some(json!({}))
        } else {
            default_auto_reply(command)
        }
    });
    let page = harness.page.clone();
    let click = tokio::spawn(async move {
        page.mouse_click_at(10.0, 20.0, ClickOptions::default())
            .await
    });
    let pressed = next_mouse_event(&mut harness.control, "mousePressed").await;
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "frameId": MAIN, "message": "Down", "type": "alert", "defaultPrompt": ""}),
    );
    assert!(matches!(
        finished(click).await,
        Ok(ActionOutcome::DialogOpened { ref message, .. }) if message == "Down"
    ));
    harness.emit(
        "Page.javascriptDialogClosed",
        json!({"result": true, "userInput": ""}),
    );
    harness.control.reply(&pressed, json!({}));
    let released = next_mouse_event(&mut harness.control, "mouseReleased").await;
    assert_eq!(released.params["x"], 10.0);
    assert_eq!(released.params["y"], 20.0);
    assert_eq!(released.params["button"], "left");
}

use std::time::Duration;

use flow_like_browser::DialogAction;
use flow_like_browser::input::keys::{Key, Modifiers, NamedKey};
use flow_like_browser::input::{ActionOutcome, ClickOptions, MouseButton};
use flow_like_browser::testing::{PageHarness, default_auto_reply};
use flow_like_browser::transport::memory::{InMemoryControl, SentCommand};
use flow_like_browser::types::DialogType;
use serde_json::{Value, json};

const MOUSE: &str = "Input.dispatchMouseEvent";
const KEY: &str = "Input.dispatchKeyEvent";

fn answer_input(command: &SentCommand) -> Option<Value> {
    if command.method.starts_with("Input.") || command.method == "Page.handleJavaScriptDialog" {
        return Some(json!({}));
    }
    default_auto_reply(command)
}

fn withhold(kind: &'static str) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync {
    move |command| {
        (command.params["type"] != kind)
            .then(|| answer_input(command))
            .flatten()
    }
}

fn input_events(control: &InMemoryControl) -> Vec<SentCommand> {
    control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method.starts_with("Input."))
        .collect()
}

fn summary(control: &InMemoryControl) -> Vec<(String, String, Value)> {
    input_events(control)
        .iter()
        .map(|command| {
            (
                command.params["type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                command.params["key"]
                    .as_str()
                    .or_else(|| command.params["button"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
                command.params["modifiers"].clone(),
            )
        })
        .collect()
}

fn row(kind: &str, what: &str, modifiers: i64) -> (String, String, Value) {
    (kind.to_owned(), what.to_owned(), json!(modifiers))
}

async fn wait_seen(control: &InMemoryControl, what: &str, matches: impl Fn(&SentCommand) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !control.commands_seen().iter().any(&matches) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} was never sent; input seen: {:?}",
            summary(control)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn count(control: &InMemoryControl, kind: &str) -> usize {
    input_events(control)
        .iter()
        .filter(|command| command.params["type"] == kind)
        .count()
}

fn open_dialog(harness: &PageHarness, message: &str) {
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "message": message, "type": "alert", "defaultPrompt": ""}),
    );
}

fn ctrl_click() -> ClickOptions {
    ClickOptions {
        modifiers: Modifiers::CTRL,
        ..ClickOptions::default()
    }
}

async fn click_into_a_dialog(harness: &mut PageHarness, options: ClickOptions) -> ActionOutcome {
    harness.control.set_auto_reply(withhold("mousePressed"));
    let page = harness.page.clone();
    let click = tokio::spawn(async move { page.mouse_click_at(40.0, 30.0, options).await });
    let pressed = harness.control.wait_for(MOUSE, Some("S1")).await;
    assert_eq!(pressed.params["type"], "mousePressed");
    open_dialog(harness, "pressed");
    click.await.unwrap().unwrap()
}

fn close_dialog(harness: &PageHarness) {
    harness.emit(
        "Page.javascriptDialogClosed",
        json!({"result": true, "userInput": ""}),
    );
}

async fn wait_dialog_closed(harness: &PageHarness) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while harness.page.pending_dialog().is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the dialog never closed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn mouse_click_at_dispatches_on_the_page_session_with_modifiers_and_click_count() {
    let harness = PageHarness::new().await;
    harness.control.set_auto_reply(answer_input);
    let options = ClickOptions {
        button: MouseButton::Right,
        modifiers: Modifiers(Modifiers::CTRL.0 | Modifiers::SHIFT.0),
        click_count: 2,
    };
    let clicked = harness.page.mouse_click_at(12.5, 7.0, options).await;
    assert_eq!(clicked.unwrap(), ActionOutcome::Completed);
    assert_eq!(
        summary(&harness.control),
        vec![
            row("rawKeyDown", "Shift", 8),
            row("rawKeyDown", "Control", 10),
            row("mouseMoved", "none", 10),
            row("mousePressed", "right", 10),
            row("mouseReleased", "right", 10),
            row("mousePressed", "right", 10),
            row("mouseReleased", "right", 10),
            row("keyUp", "Control", 8),
            row("keyUp", "Shift", 0),
        ]
    );
    let events = input_events(&harness.control);
    assert!(
        events
            .iter()
            .all(|command| command.session.as_deref() == Some("S1"))
    );
    assert_eq!(
        counts_and_buttons(&events[3..7]),
        vec![
            (json!(1), json!(2)),
            (json!(1), json!(0)),
            (json!(2), json!(2)),
            (json!(2), json!(0)),
        ]
    );
    assert_eq!(
        (
            events[2].params["x"].as_f64(),
            events[2].params["y"].as_f64()
        ),
        (Some(12.5), Some(7.0))
    );
}

fn counts_and_buttons(events: &[SentCommand]) -> Vec<(Value, Value)> {
    events
        .iter()
        .map(|command| {
            (
                command.params["clickCount"].clone(),
                command.params["buttons"].clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn key_chord_pipelines_its_events_and_awaits_only_the_last() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        (command.method != KEY)
            .then(|| answer_input(command))
            .flatten()
    });
    let page = harness.page.clone();
    let chord = tokio::spawn(async move { page.key_chord(Key::Char('a'), Modifiers::CTRL).await });
    let mut sent = Vec::new();
    for _ in 0..4 {
        let command = harness.control.next_command().await;
        assert_eq!(
            (command.method.as_str(), command.session.as_deref()),
            (KEY, Some("S1"))
        );
        sent.push(command);
    }
    assert!(!chord.is_finished(), "only the last key event is awaited");
    let kinds: Vec<_> = sent
        .iter()
        .map(|command| {
            (
                command.params["type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                command.params["key"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("rawKeyDown".to_owned(), "Control".to_owned()),
            ("rawKeyDown".to_owned(), "a".to_owned()),
            ("keyUp".to_owned(), "a".to_owned()),
            ("keyUp".to_owned(), "Control".to_owned()),
        ]
    );
    harness.control.reply(&sent[3], json!({}));
    assert_eq!(chord.await.unwrap().unwrap(), ActionOutcome::Completed);
}

#[tokio::test]
async fn a_dialog_on_the_last_chord_key_is_reported_and_nothing_stays_held() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(|command| {
        (command.method != KEY)
            .then(|| answer_input(command))
            .flatten()
    });
    let page = harness.page.clone();
    let chord = tokio::spawn(async move {
        page.key_chord(Key::Named(NamedKey::Enter), Modifiers::NONE)
            .await
    });
    for _ in 0..3 {
        harness.control.next_command().await;
    }
    open_dialog(&harness, "submitted");
    assert_eq!(
        chord.await.unwrap().unwrap(),
        ActionOutcome::DialogOpened {
            kind: DialogType::Alert,
            message: "submitted".to_owned()
        }
    );
    harness.page.release_input().await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        count(&harness.control, "keyUp"),
        1,
        "only the chord's own keyUp"
    );
}

#[tokio::test]
async fn a_dialog_during_mouse_pressed_is_released_when_another_closer_closes_it() {
    let mut harness = PageHarness::new().await;
    let outcome = click_into_a_dialog(&mut harness, ClickOptions::default()).await;
    assert_eq!(
        outcome,
        ActionOutcome::DialogOpened {
            kind: DialogType::Alert,
            message: "pressed".to_owned()
        }
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(count(&harness.control, "mouseReleased"), 0);
    close_dialog(&harness);
    wait_seen(&harness.control, "the pending release", |command| {
        command.params["type"] == "mouseReleased"
    })
    .await;
    let released = input_events(&harness.control)
        .into_iter()
        .find(|command| command.params["type"] == "mouseReleased")
        .unwrap();
    assert_eq!(released.session.as_deref(), Some("S1"));
    assert_eq!(released.params["x"], 40.0);
    assert_eq!(released.params["y"], 30.0);
    assert_eq!(released.params["button"], "left");
}

#[tokio::test]
async fn handle_dialog_releases_the_pressed_button_once() {
    let mut harness = PageHarness::new().await;
    click_into_a_dialog(&mut harness, ClickOptions::default()).await;
    harness
        .page
        .handle_dialog(DialogAction::Accept { prompt_text: None })
        .await
        .unwrap();
    wait_seen(&harness.control, "the pending release", |command| {
        command.params["type"] == "mouseReleased"
    })
    .await;
    close_dialog(&harness);
    wait_dialog_closed(&harness).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(count(&harness.control, "mouseReleased"), 1);
}

#[tokio::test]
async fn a_failed_click_still_releases_ctrl_and_the_button() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(withhold("mousePressed"));
    let page = harness.page.clone();
    let click = tokio::spawn(async move { page.mouse_click_at(5.0, 5.0, ctrl_click()).await });
    let pressed = harness.control.wait_for(MOUSE, Some("S1")).await;
    harness
        .control
        .reply_error(&pressed, -32000, "Internal error");
    let error = click.await.unwrap().unwrap_err();
    assert!(
        error.to_string().contains("Input.dispatchMouseEvent"),
        "{error}"
    );
    wait_seen(&harness.control, "the Ctrl keyUp", |command| {
        command.params["type"] == "keyUp" && command.params["key"] == "Control"
    })
    .await;
    wait_seen(&harness.control, "the button release", |command| {
        command.params["type"] == "mouseReleased"
    })
    .await;
}

#[tokio::test]
async fn a_dropped_click_future_still_releases_ctrl_and_the_button() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(withhold("mousePressed"));
    let page = harness.page.clone();
    let click = tokio::spawn(async move { page.mouse_click_at(5.0, 5.0, ctrl_click()).await });
    harness.control.wait_for(MOUSE, Some("S1")).await;
    click.abort();
    assert!(click.await.unwrap_err().is_cancelled());
    wait_seen(&harness.control, "the Ctrl keyUp", |command| {
        command.params["type"] == "keyUp" && command.params["key"] == "Control"
    })
    .await;
    wait_seen(&harness.control, "the button release", |command| {
        command.params["type"] == "mouseReleased"
    })
    .await;
}

#[tokio::test]
async fn a_target_closed_after_commit_is_completed() {
    let mut harness = PageHarness::new().await;
    harness.control.set_auto_reply(withhold("mouseReleased"));
    let page = harness.page.clone();
    let click =
        tokio::spawn(async move { page.mouse_click_at(5.0, 5.0, ClickOptions::default()).await });
    let released = harness.control.wait_for(MOUSE, Some("S1")).await;
    assert_eq!(released.params["type"], "mouseReleased");
    harness
        .control
        .reply_error(&released, -32001, "Session with given id not found.");
    assert_eq!(click.await.unwrap().unwrap(), ActionOutcome::Completed);
}

#[tokio::test]
async fn release_input_releases_the_pending_button_once() {
    let mut harness = PageHarness::new().await;
    click_into_a_dialog(&mut harness, ctrl_click()).await;
    wait_seen(&harness.control, "the Ctrl keyUp", |command| {
        command.params["type"] == "keyUp"
    })
    .await;
    assert_eq!(count(&harness.control, "mouseReleased"), 0);
    harness.page.release_input().await.unwrap();
    wait_seen(&harness.control, "the explicit release", |command| {
        command.params["type"] == "mouseReleased"
    })
    .await;
    close_dialog(&harness);
    wait_dialog_closed(&harness).await;
    harness.page.release_input().await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(count(&harness.control, "mouseReleased"), 1);
    assert_eq!(count(&harness.control, "keyUp"), 1);
}

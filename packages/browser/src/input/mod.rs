use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::connection::without_op_deadline;
use crate::dialogs::Dialog;
use crate::error::{BrowserError, ErrorClass};
use crate::page::{Page, PageEffect};
use crate::settle::OpSpec;
use crate::transport::{WriteStatus, WriteTicket};
use crate::types::{DialogType, SessionId};

pub(crate) mod keyboard;
pub mod keys;
pub(crate) mod mouse;
pub(crate) mod us_layout;

use keys::{Key, KeyEvent, KeyEventKind, Modifiers};
use mouse::PendingRelease;

const KEY_EVENT: &str = "Input.dispatchKeyEvent";
const MOUSE_EVENT: &str = "Input.dispatchMouseEvent";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn wire_name(&self) -> &str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
        }
    }

    fn mask(self) -> u8 {
        match self {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClickOptions {
    pub button: MouseButton,
    pub modifiers: Modifiers,
    pub click_count: u8,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            modifiers: Modifiers::NONE,
            click_count: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActionOutcome {
    Completed,
    DialogOpened { kind: DialogType, message: String },
}

pub(crate) fn dialog_outcome(dialog: Dialog) -> ActionOutcome {
    ActionOutcome::DialogOpened {
        kind: dialog.kind,
        message: dialog.message,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseEventKind {
    Moved,
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MouseEvent {
    pub kind: MouseEventKind,
    pub point: (f64, f64),
    pub button: Option<MouseButton>,
    pub buttons: u8,
    pub click_count: u8,
    pub modifiers: u32,
}

impl MouseEvent {
    pub(crate) fn moved(point: (f64, f64), held: Option<MouseButton>, modifiers: u32) -> Self {
        Self {
            kind: MouseEventKind::Moved,
            point,
            button: held,
            buttons: held.map_or(0, MouseButton::mask),
            click_count: 0,
            modifiers,
        }
    }

    pub(crate) fn pressed(
        point: (f64, f64),
        button: MouseButton,
        click_count: u8,
        modifiers: u32,
    ) -> Self {
        Self {
            kind: MouseEventKind::Pressed,
            point,
            button: Some(button),
            buttons: button.mask(),
            click_count,
            modifiers,
        }
    }

    pub(crate) fn released(
        point: (f64, f64),
        button: MouseButton,
        click_count: u8,
        modifiers: u32,
    ) -> Self {
        Self {
            kind: MouseEventKind::Released,
            point,
            button: Some(button),
            buttons: 0,
            click_count,
            modifiers,
        }
    }

    pub(crate) fn params(&self) -> Value {
        let kind = match self.kind {
            MouseEventKind::Moved => "mouseMoved",
            MouseEventKind::Pressed => "mousePressed",
            MouseEventKind::Released => "mouseReleased",
        };
        json!({
            "type": kind,
            "x": self.point.0,
            "y": self.point.1,
            "modifiers": self.modifiers,
            "button": self.button.as_ref().map_or("none", MouseButton::wire_name),
            "buttons": self.buttons,
            "clickCount": self.click_count,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum InputEvent {
    Mouse(MouseEvent),
    Key(KeyEvent),
}

impl InputEvent {
    pub(crate) fn method(&self) -> &str {
        match self {
            InputEvent::Mouse(_) => MOUSE_EVENT,
            InputEvent::Key(_) => KEY_EVENT,
        }
    }

    pub(crate) fn params(&self) -> Value {
        match self {
            InputEvent::Mouse(event) => event.params(),
            InputEvent::Key(event) => event.to_params(),
        }
    }
}

static MODIFIER_KEY_CODES: LazyLock<Vec<(i64, Modifiers)>> = LazyLock::new(|| {
    [
        Modifiers::SHIFT,
        Modifiers::CTRL,
        Modifiers::ALT,
        Modifiers::META,
    ]
    .into_iter()
    .flat_map(|modifier| {
        keys::modifier_events(modifier, true)
            .into_iter()
            .map(move |down| (down.windows_virtual_key_code, modifier))
    })
    .collect()
});

fn modifier_of(event: &KeyEvent) -> Option<Modifiers> {
    MODIFIER_KEY_CODES
        .iter()
        .find(|(code, _)| *code == event.windows_virtual_key_code)
        .map(|(_, modifier)| *modifier)
}

struct PressedButton {
    release: PendingRelease,
    ticket: WriteTicket,
    deadline: tokio::time::Instant,
}

impl PressedButton {
    fn may_reach_chrome(&self, now: tokio::time::Instant) -> bool {
        self.ticket.status() != WriteStatus::NotWritten || now < self.deadline
    }
}

pub(crate) struct InputState {
    modifiers: Modifiers,
    buttons: Vec<PressedButton>,
    pending_release: Option<PendingRelease>,
}

impl InputState {
    pub(crate) fn new() -> Self {
        Self {
            modifiers: Modifiers::NONE,
            buttons: Vec::new(),
            pending_release: None,
        }
    }

    pub(crate) fn on_dialog_closed(&mut self) -> Vec<PageEffect> {
        self.pending_release
            .take()
            .map(|release| release.effect())
            .into_iter()
            .collect()
    }

    fn record(
        &mut self,
        session: &SessionId,
        event: &InputEvent,
        ticket: &WriteTicket,
        deadline: tokio::time::Instant,
    ) {
        match event {
            InputEvent::Key(key) => self.record_key(key),
            InputEvent::Mouse(mouse) => self.record_mouse(session, mouse, ticket, deadline),
        }
    }

    fn record_key(&mut self, event: &KeyEvent) {
        let Some(modifier) = modifier_of(event) else {
            return;
        };
        match event.kind {
            KeyEventKind::RawKeyDown | KeyEventKind::KeyDown => self.modifiers.insert(modifier),
            KeyEventKind::KeyUp => self.modifiers = Modifiers(self.modifiers.0 & !modifier.0),
            KeyEventKind::Char => {}
        }
    }

    fn record_mouse(
        &mut self,
        session: &SessionId,
        event: &MouseEvent,
        ticket: &WriteTicket,
        deadline: tokio::time::Instant,
    ) {
        let Some(button) = event.button else {
            return;
        };
        match event.kind {
            MouseEventKind::Pressed => self.buttons.push(PressedButton {
                release: PendingRelease {
                    session: session.clone(),
                    x: event.point.0,
                    y: event.point.1,
                    button,
                    click_count: event.click_count,
                },
                ticket: ticket.clone(),
                deadline,
            }),
            MouseEventKind::Released => self
                .buttons
                .retain(|pressed| !pressed.release.is_held(session, button)),
            MouseEventKind::Moved => {
                for pressed in &mut self.buttons {
                    if pressed.release.is_held(session, button) {
                        (pressed.release.x, pressed.release.y) = event.point;
                    }
                }
            }
        }
    }

    fn take_held(
        &mut self,
        page_session: &SessionId,
        dialog_open: bool,
        now: tokio::time::Instant,
    ) -> Vec<PageEffect> {
        let mut effects = Vec::new();
        for pressed in std::mem::take(&mut self.buttons) {
            if !pressed.may_reach_chrome(now) {
                continue;
            }
            if dialog_open {
                self.pending_release = Some(pressed.release);
            } else {
                effects.push(pressed.release.effect());
            }
        }
        effects.extend(self.take_modifiers(page_session));
        effects
    }

    fn take_all(&mut self, page_session: &SessionId, now: tokio::time::Instant) -> Vec<PageEffect> {
        let mut effects = self.take_held(page_session, false, now);
        effects.extend(self.on_dialog_closed());
        effects
    }

    fn take_modifiers(&mut self, page_session: &SessionId) -> Vec<PageEffect> {
        let held = std::mem::replace(&mut self.modifiers, Modifiers::NONE);
        keys::modifier_events(held, false)
            .iter()
            .map(|up| PageEffect::SendNowait {
                session: page_session.clone(),
                method: KEY_EVENT,
                params: up.to_params(),
            })
            .collect()
    }
}

pub(crate) fn record_sent(
    page: &Page,
    session: &SessionId,
    event: &InputEvent,
    ticket: &WriteTicket,
    deadline: tokio::time::Instant,
) {
    page.inner
        .lock_state()
        .input
        .record(session, event, ticket, deadline);
}

fn send_effects(page: &Page, effects: Vec<PageEffect>) -> crate::Result<()> {
    let mut failure = None;
    for effect in effects {
        let PageEffect::SendNowait {
            session,
            method,
            params,
        } = effect;
        let sent = without_op_deadline(|| {
            page.inner
                .connection
                .send_nowait(method, params, Some(&session))
        });
        match sent {
            Ok(_) => {}
            Err(error) if error.class() == ErrorClass::SessionGone => {
                tracing::debug!(method, %session, %error, "input release skipped: session is gone");
            }
            Err(error) => {
                tracing::debug!(method, %session, %error, "input release could not be sent");
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

pub(crate) fn release_pending(page: &Page) -> crate::Result<()> {
    let effects = page.inner.lock_state().input.on_dialog_closed();
    send_effects(page, effects)
}

pub(crate) struct InputGuard {
    page: Page,
}

impl InputGuard {
    pub(crate) fn new(page: &Page) -> Self {
        Self { page: page.clone() }
    }
}

impl Drop for InputGuard {
    fn drop(&mut self) {
        let effects = {
            let mut state = self.page.inner.lock_state();
            let dialog_open = state.dialog.open().is_some();
            state.input.take_held(
                &self.page.inner.session,
                dialog_open,
                tokio::time::Instant::now(),
            )
        };
        if let Err(error) = send_effects(&self.page, effects) {
            tracing::debug!(target = %self.page.target_id(), %error, "held input was not released");
        }
    }
}

impl Page {
    pub async fn mouse_click_at(
        &self,
        x: f64,
        y: f64,
        options: ClickOptions,
    ) -> crate::Result<ActionOutcome> {
        let _guard = InputGuard::new(self);
        let main = self.main_frame().id;
        let outcome = self
            .run_op(&main, OpSpec::INPUT, |attempt| async move {
                mouse::w3c_click(self, &attempt, (x, y), options).await
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub async fn key_chord(&self, key: Key, modifiers: Modifiers) -> crate::Result<ActionOutcome> {
        let events = keys::chord_events(key, modifiers)?;
        let _guard = InputGuard::new(self);
        let main = self.main_frame().id;
        let outcome = self
            .run_op(&main, OpSpec::INPUT, |attempt| {
                let events = events.clone();
                async move { keyboard::dispatch_key_events(self, &attempt, events).await }
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub async fn release_input(&self) -> crate::Result<()> {
        let effects = self
            .inner
            .lock_state()
            .input
            .take_all(&self.inner.session, tokio::time::Instant::now());
        send_effects(self, effects)
    }
}

pub(crate) fn unexpected_reply(method: &str, what: &str, reply: &Value) -> BrowserError {
    BrowserError::protocol(method, -32000, format!("{what} missing in reply {reply}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::PageHarness;
    use std::time::Duration;

    fn release_of(effect: &PageEffect) -> (&str, &str, &Value) {
        let PageEffect::SendNowait {
            session,
            method,
            params,
        } = effect;
        (session.as_str(), method, params)
    }

    fn state() -> (InputState, SessionId, tokio::time::Instant) {
        (
            InputState::new(),
            SessionId::from("S1"),
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
    }

    #[tokio::test]
    async fn modifiers_are_held_between_their_down_and_up_events() {
        let (mut input, session, deadline) = state();
        let ticket = WriteTicket::default();
        let downs = keys::modifier_events(Modifiers(Modifiers::CTRL.0 | Modifiers::SHIFT.0), true);
        for down in &downs {
            input.record(&session, &InputEvent::Key(down.clone()), &ticket, deadline);
        }
        let shift_up = keys::modifier_events(Modifiers::SHIFT, false);
        input.record(
            &session,
            &InputEvent::Key(shift_up[0].clone()),
            &ticket,
            deadline,
        );
        assert_eq!(input.modifiers, Modifiers::CTRL);
        let effects = input.take_held(&session, false, tokio::time::Instant::now());
        assert_eq!(effects.len(), 1);
        let (to, method, params) = release_of(&effects[0]);
        assert_eq!((to, method), ("S1", KEY_EVENT));
        assert_eq!(params["type"], "keyUp");
        assert_eq!(params["key"], "Control");
        assert_eq!(params["modifiers"], 0);
        assert_eq!(input.modifiers, Modifiers::NONE);
    }

    #[tokio::test]
    async fn pressed_buttons_are_released_unless_they_never_reached_chrome() {
        let (mut input, session, deadline) = state();
        let written = WriteTicket::default();
        written.set(WriteStatus::Written);
        let press = MouseEvent::pressed((3.0, 4.0), MouseButton::Right, 2, 0);
        input.record(&session, &InputEvent::Mouse(press), &written, deadline);
        let skipped = WriteTicket::default();
        let expired = tokio::time::Instant::now();
        let left = MouseEvent::pressed((1.0, 1.0), MouseButton::Left, 1, 0);
        input.record(&session, &InputEvent::Mouse(left), &skipped, expired);
        let effects = input.take_held(&session, false, expired + Duration::from_millis(1));
        assert_eq!(effects.len(), 1);
        let (_, method, params) = release_of(&effects[0]);
        assert_eq!(method, MOUSE_EVENT);
        assert_eq!(params["type"], "mouseReleased");
        assert_eq!(params["button"], "right");
        assert_eq!(params["buttons"], 0);
        assert_eq!(params["clickCount"], 2);
        assert_eq!(
            (params["x"].as_f64(), params["y"].as_f64()),
            (Some(3.0), Some(4.0))
        );
    }

    #[tokio::test]
    async fn a_queued_press_is_still_released() {
        let (mut input, session, deadline) = state();
        let queued = WriteTicket::default();
        let press = MouseEvent::pressed((1.0, 1.0), MouseButton::Left, 1, 0);
        input.record(&session, &InputEvent::Mouse(press), &queued, deadline);
        assert_eq!(
            input
                .take_held(&session, false, tokio::time::Instant::now())
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn an_open_dialog_turns_the_pressed_button_into_the_pending_release() {
        let (mut input, session, deadline) = state();
        let ticket = WriteTicket::default();
        ticket.set(WriteStatus::Written);
        let ctrl = keys::modifier_events(Modifiers::CTRL, true);
        input.record(
            &session,
            &InputEvent::Key(ctrl[0].clone()),
            &ticket,
            deadline,
        );
        let press = MouseEvent::pressed((5.0, 6.0), MouseButton::Left, 1, 2);
        input.record(&session, &InputEvent::Mouse(press), &ticket, deadline);
        let effects = input.take_held(&session, true, tokio::time::Instant::now());
        assert_eq!(effects.len(), 1, "only the Ctrl keyUp goes out now");
        assert_eq!(release_of(&effects[0]).2["type"], "keyUp");
        let on_close = input.on_dialog_closed();
        assert_eq!(on_close.len(), 1);
        let (to, _, params) = release_of(&on_close[0]);
        assert_eq!(to, "S1");
        assert_eq!(params["type"], "mouseReleased");
        assert!(input.on_dialog_closed().is_empty(), "taken exactly once");
    }

    #[tokio::test]
    async fn a_release_clears_the_matching_press_only() {
        let (mut input, session, deadline) = state();
        let ticket = WriteTicket::default();
        ticket.set(WriteStatus::Written);
        let other = SessionId::from("S2");
        let left = MouseEvent::pressed((1.0, 1.0), MouseButton::Left, 1, 0);
        input.record(&session, &InputEvent::Mouse(left), &ticket, deadline);
        input.record(&other, &InputEvent::Mouse(left), &ticket, deadline);
        let up = MouseEvent::released((1.0, 1.0), MouseButton::Left, 1, 0);
        input.record(&session, &InputEvent::Mouse(up), &ticket, deadline);
        let effects = input.take_held(&session, false, tokio::time::Instant::now());
        assert_eq!(effects.len(), 1);
        assert_eq!(release_of(&effects[0]).0, "S2");
    }

    #[tokio::test]
    async fn a_held_button_is_released_where_the_pointer_moved_it() {
        let (mut input, session, deadline) = state();
        let ticket = WriteTicket::default();
        ticket.set(WriteStatus::Written);
        let press = MouseEvent::pressed((1.0, 2.0), MouseButton::Left, 1, 0);
        input.record(&session, &InputEvent::Mouse(press), &ticket, deadline);
        let hover = MouseEvent::moved((50.0, 60.0), None, 0);
        input.record(&session, &InputEvent::Mouse(hover), &ticket, deadline);
        let other = SessionId::from("S2");
        let elsewhere = MouseEvent::moved((70.0, 80.0), Some(MouseButton::Left), 0);
        input.record(&other, &InputEvent::Mouse(elsewhere), &ticket, deadline);
        let drag = MouseEvent::moved((30.0, 40.0), Some(MouseButton::Left), 0);
        input.record(&session, &InputEvent::Mouse(drag), &ticket, deadline);
        let effects = input.take_held(&session, false, tokio::time::Instant::now());
        let params = release_of(&effects[0]).2;
        assert_eq!(
            (params["x"].as_f64(), params["y"].as_f64()),
            (Some(30.0), Some(40.0))
        );
    }

    #[tokio::test]
    async fn release_pending_sends_the_pending_release_once() {
        let harness = PageHarness::new().await;
        harness.page.inner.lock_state().input.pending_release = Some(PendingRelease {
            session: SessionId::from("S1"),
            x: 7.0,
            y: 8.0,
            button: MouseButton::Left,
            click_count: 1,
        });
        release_pending(&harness.page).unwrap();
        release_pending(&harness.page).unwrap();
        let mut control = harness.control;
        let released = control.wait_for(MOUSE_EVENT, Some("S1")).await;
        assert_eq!(released.params["type"], "mouseReleased");
        assert_eq!(released.params["x"], 7.0);
        tokio::task::yield_now().await;
        let releases = control
            .commands_seen()
            .iter()
            .filter(|command| command.method == MOUSE_EVENT)
            .count();
        assert_eq!(releases, 1);
    }

    #[tokio::test]
    async fn release_to_a_detached_session_is_ignored() {
        let harness = PageHarness::new().await;
        harness.control.emit(
            "Target.attachedToTarget",
            Some("S1"),
            json!({"sessionId": "S9", "targetInfo": {"targetId": "F9", "type": "iframe"}, "waitingForDebugger": false}),
        );
        harness.control.emit(
            "Target.detachedFromTarget",
            Some("S1"),
            json!({"sessionId": "S9"}),
        );
        let detached = harness
            .connection
            .events()
            .wait_for(
                crate::event_log::EventCursor(1),
                tokio::time::Instant::now() + Duration::from_secs(5),
                |event| {
                    &*event.method == "Target.detachedFromTarget"
                        && event.params["sessionId"] == "S9"
                },
            )
            .await
            .unwrap();
        assert!(detached.is_some());
        harness.page.inner.lock_state().input.pending_release = Some(PendingRelease {
            session: SessionId::from("S9"),
            x: 1.0,
            y: 1.0,
            button: MouseButton::Left,
            click_count: 1,
        });
        assert!(release_pending(&harness.page).is_ok());
    }
}

#[cfg(test)]
pub(crate) mod scene {
    use serde_json::{Value, json};
    use std::time::Duration;

    use crate::element::Element;
    use crate::testing::{PageHarness, default_auto_reply};
    use crate::transport::memory::{InMemoryControl, SentCommand};
    use crate::types::FrameId;

    pub(crate) const IS_CONNECTED: &str = "function(){return this.isConnected}";
    pub(crate) const OOPIF_OWNER: i64 = 7;
    pub(crate) const NESTED_OWNER: i64 = 8;
    pub(crate) const TARGET: i64 = 42;

    pub(crate) fn value(value: Value) -> Option<Value> {
        Some(json!({"result": {"type": "object", "value": value}}))
    }

    pub(crate) fn declaration(command: &SentCommand) -> &str {
        command.params["functionDeclaration"]
            .as_str()
            .unwrap_or_default()
    }

    pub(crate) fn calls_atom(command: &SentCommand, name: &str) -> bool {
        command.method == "Runtime.callFunctionOn"
            && declaration(command).contains(&format!("__flowlike.{name}(this"))
    }

    fn quad(session: &str) -> Value {
        match session {
            "S1" => json!([100, 200, 140, 200, 140, 220, 100, 220]),
            "S2" => json!([20, 50, 108.5, 50, 108.5, 71, 20, 71]),
            _ => json!([20, 50, 40, 50, 40, 70, 20, 70]),
        }
    }

    fn viewport(session: &str) -> Value {
        let (width, height) = match session {
            "S1" => (1280, 720),
            "S2" => (300, 150),
            _ => (200, 100),
        };
        json!({"cssLayoutViewport": {"pageX": 0, "pageY": 0, "clientWidth": width, "clientHeight": height}})
    }

    fn frame_owner(frame: &Value) -> Option<Value> {
        let owner = match frame.as_str()? {
            "F2" => OOPIF_OWNER,
            "F3" => NESTED_OWNER,
            _ => return None,
        };
        Some(json!({"backendNodeId": owner}))
    }

    fn box_model(owner: &Value) -> Option<Value> {
        let origin = match owner.as_i64()? {
            OOPIF_OWNER => [51.0, 297.5],
            NESTED_OWNER => [0.0, 18.0],
            _ => return None,
        };
        let [x, y] = origin;
        Some(
            json!({"model": {"content": [x, y, x + 300.0, y, x + 300.0, y + 150.0, x, y + 150.0]}}),
        )
    }

    pub(crate) fn reply(command: &SentCommand) -> Option<Value> {
        let session = command.session.as_deref().unwrap_or_default();
        let params = &command.params;
        match command.method.as_str() {
            "DOM.resolveNode" => Some(json!({"object": {"type": "object", "subtype": "node",
                "objectId": format!("obj-{}", params["backendNodeId"])}})),
            "Runtime.callFunctionOn" if declaration(command) == IS_CONNECTED => value(json!(true)),
            "Runtime.evaluate" if params.get("contextId").is_some() => {
                Some(json!({"result": {"type": "undefined"}}))
            }
            "DOM.scrollIntoViewIfNeeded"
            | "DOM.setFileInputFiles"
            | "Input.dispatchMouseEvent"
            | "Input.dispatchKeyEvent" => Some(json!({})),
            "DOM.getContentQuads" => Some(json!({"quads": [quad(session)]})),
            "Page.getLayoutMetrics" => Some(viewport(session)),
            "DOM.getFrameOwner" => frame_owner(&params["frameId"]),
            "DOM.getBoxModel" => box_model(&params["backendNodeId"]),
            "DOM.getNodeForLocation" => {
                let (hit, frame) = match session {
                    "S1" => (OOPIF_OWNER, "T1"),
                    "S2" => (TARGET, "F2"),
                    _ => (TARGET, "F3"),
                };
                Some(json!({"backendNodeId": hit, "frameId": frame}))
            }
            _ => default_auto_reply(command),
        }
    }

    pub(crate) async fn page_with_oopif() -> PageHarness {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(reply);
        harness.attach_child("S2", "F2", "T1", "http://127.0.0.1/child", "L2");
        harness
    }

    pub(crate) async fn page_with_nested_oopif() -> PageHarness {
        let harness = page_with_oopif().await;
        harness.attach_child("S3", "F3", "F2", "http://127.0.0.1/leaf", "L3");
        harness
    }

    pub(crate) fn element(harness: &PageHarness, frame: &str, backend_node_id: i64) -> Element {
        let frame = harness
            .page
            .frame(&FrameId::from(frame))
            .expect("the scene frame exists");
        let stamp = frame.stamp().expect("the scene frame has a document");
        Element::new(frame, &stamp, backend_node_id, None)
    }

    pub(crate) fn input_events(control: &InMemoryControl) -> Vec<SentCommand> {
        control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method.starts_with("Input."))
            .collect()
    }

    pub(crate) async fn wait_seen(
        control: &InMemoryControl,
        what: &str,
        matches: impl Fn(&SentCommand) -> bool,
    ) -> SentCommand {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(found) = control.commands_seen().into_iter().find(&matches) {
                return found;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what} was never sent; commands seen: {:?}",
                control
                    .commands_seen()
                    .iter()
                    .map(|command| format!(
                        "{} {:?} {}",
                        command.method, command.session, command.params
                    ))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub(crate) fn mouse(command: &SentCommand) -> (&str, f64, f64) {
        (
            command.params["type"].as_str().unwrap_or_default(),
            command.params["x"].as_f64().unwrap_or(f64::NAN),
            command.params["y"].as_f64().unwrap_or(f64::NAN),
        )
    }
}

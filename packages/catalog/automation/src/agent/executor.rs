use super::computer_use::AgentAction;
use super::history::ToolCallRequest;
use super::observe::{Observation, ZoomView, zoom_box, zoom_view};
use super::tools::{Action, ForbiddenText, MouseButton, ScrollDirection, parse_action};
use crate::computer::keyboard::{ChordModifier, KeyChord, parse_key};
use crate::computer::mouse::{check_cancellation, click_at, interruptible_sleep, perform_drag};
use crate::computer::native::input::DesktopInput;
use crate::computer::ocr::InputPoint;
use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{async_trait, tokio_util::sync::CancellationToken};

pub(crate) const EARLIER_FAILED: &str = "an earlier action in this turn failed";
const TASK_ENDED: &str = "the task already ended in this turn";
const DRAG_MS: u64 = 500;
const SCROLL_TICK_MS: u64 = 15;
const KEY_REPEAT_MS: u64 = 40;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Terminal {
    Done { success: bool, answer: String },
    AskUser { question: String },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Performed {
    pub message: String,
    pub input_points: Vec<InputPoint>,
}

impl Performed {
    fn plain(message: String) -> Self {
        Self {
            message,
            input_points: Vec::new(),
        }
    }

    fn at(message: String, points: &[(i32, i32)]) -> Self {
        Self {
            message,
            input_points: points.iter().map(|&(x, y)| InputPoint { x, y }).collect(),
        }
    }
}

#[async_trait]
pub(crate) trait ActionRunner: Send {
    async fn perform(&mut self, action: &Action) -> Result<Performed, String>;
}

pub(crate) struct BatchOutcome {
    pub actions: Vec<AgentAction>,
    pub terminal: Option<Terminal>,
    /// Whether at least one mouse or keyboard action ran successfully.
    pub acted: bool,
}

fn record(call: &ToolCallRequest, skipped: bool, ok: bool, message: String) -> AgentAction {
    AgentAction {
        tool: call.name.clone(),
        args: call.arguments.clone(),
        input_points: Vec::new(),
        skipped,
        ok,
        message,
    }
}

fn terminal_of(action: &Action) -> Option<Terminal> {
    match action {
        Action::Done { success, answer } => Some(Terminal::Done {
            success: *success,
            answer: answer.clone(),
        }),
        Action::AskUser { question } => Some(Terminal::AskUser {
            question: question.clone(),
        }),
        _ => None,
    }
}

enum CallResult {
    Performed { record: AgentAction, input: bool },
    Failed(AgentAction),
    Ended(AgentAction, Terminal),
}

async fn run_call<R: ActionRunner>(
    call: &ToolCallRequest,
    marks: bool,
    runner: &mut R,
) -> CallResult {
    let action = match parse_action(&call.name, &call.arguments, marks) {
        Ok(action) => action,
        Err(error) => return CallResult::Failed(record(call, false, false, error)),
    };
    if let Some(terminal) = terminal_of(&action) {
        return CallResult::Ended(record(call, false, true, "Recorded".to_string()), terminal);
    }
    match runner.perform(&action).await {
        Ok(performed) => CallResult::Performed {
            record: AgentAction {
                input_points: performed.input_points,
                ..record(call, false, true, performed.message)
            },
            input: action.is_input(),
        },
        Err(error) => CallResult::Failed(record(call, false, false, error)),
    }
}

/// Runs a turn's tool calls in order: at most `max_actions`, stopping at the first failure or
/// at done/ask_user; every call gets a record so every call gets a tool result.
pub(crate) async fn run_batch<R: ActionRunner>(
    calls: &[ToolCallRequest],
    max_actions: usize,
    marks: bool,
    runner: &mut R,
) -> BatchOutcome {
    let mut outcome = BatchOutcome {
        actions: Vec::with_capacity(calls.len()),
        terminal: None,
        acted: false,
    };
    let mut skip: Option<String> = None;
    for (index, call) in calls.iter().enumerate() {
        let reason = skip.clone().or_else(|| {
            (index >= max_actions).then(|| {
                format!("only {max_actions} actions run per turn; call it again if still needed")
            })
        });
        if let Some(reason) = reason {
            outcome.actions.push(record(call, true, false, reason));
            continue;
        }
        match run_call(call, marks, runner).await {
            CallResult::Performed { record, input } => {
                outcome.acted |= input;
                outcome.actions.push(record);
            }
            CallResult::Failed(record) => {
                outcome.actions.push(record);
                skip = Some(EARLIER_FAILED.to_string());
            }
            CallResult::Ended(record, terminal) => {
                outcome.actions.push(record);
                outcome.terminal = Some(terminal);
                skip = Some(TASK_ENDED.to_string());
            }
        }
    }
    outcome
}

/// Desktop input coordinates of a point in the model's image; off-image points are errors.
pub(crate) fn map_point(model_frame: &ScreenFrame, x: f64, y: f64) -> Result<(i32, i32), String> {
    model_frame
        .pixel_to_input(x, y)
        .map_err(|e| format!("{e}; use coordinates of the latest screenshot"))
}

fn enigo_button(button: MouseButton) -> enigo::Button {
    match button {
        MouseButton::Left => enigo::Button::Left,
        MouseButton::Right => enigo::Button::Right,
        MouseButton::Middle => enigo::Button::Middle,
    }
}

fn modifier_list(modifiers: &[ChordModifier]) -> String {
    modifiers
        .iter()
        .map(|modifier| match modifier {
            ChordModifier::Control => "ctrl",
            ChordModifier::Shift => "shift",
            ChordModifier::Alt => "alt",
            ChordModifier::Meta => "meta",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn chord_keys(chord: &KeyChord) -> flow_like_types::Result<(Vec<enigo::Key>, enigo::Key)> {
    let modifiers = chord
        .modifiers
        .iter()
        .map(|modifier| match modifier {
            ChordModifier::Control => enigo::Key::Control,
            ChordModifier::Shift => enigo::Key::Shift,
            ChordModifier::Alt => enigo::Key::Alt,
            ChordModifier::Meta => enigo::Key::Meta,
        })
        .collect();
    Ok((modifiers, parse_key(&chord.key)?))
}

/// Presses `modifiers` in order, runs `body`, then releases them in reverse, even on failure.
fn with_modifiers(
    input: &mut DesktopInput,
    modifiers: &[enigo::Key],
    body: impl FnOnce(&mut DesktopInput) -> flow_like_types::Result<()>,
) -> flow_like_types::Result<()> {
    use enigo::{Direction, Keyboard};
    let mut pressed = Vec::new();
    let mut result = Ok(());
    for modifier in modifiers {
        if let Err(error) = input.key(*modifier, Direction::Press) {
            result = Err(flow_like_types::anyhow!(
                "Failed to press modifier {:?}: {}",
                modifier,
                error
            ));
            break;
        }
        pressed.push(*modifier);
    }
    if result.is_ok() {
        result = body(input);
    }
    for modifier in pressed.into_iter().rev() {
        if let Err(error) = input.key(modifier, Direction::Release)
            && result.is_ok()
        {
            result = Err(flow_like_types::anyhow!(
                "Failed to release modifier {:?}: {}",
                modifier,
                error
            ));
        }
    }
    result
}

fn error_text(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

/// Performs actions on the real desktop against the observation the model saw.
pub(crate) struct DesktopRunner<'a> {
    context: &'a ExecutionContext,
    session: &'a AutomationSession,
    observation: &'a Observation,
    forbidden: &'a ForbiddenText,
    pub zoom: Option<ZoomView>,
}

impl<'a> DesktopRunner<'a> {
    pub(crate) fn new(
        context: &'a ExecutionContext,
        session: &'a AutomationSession,
        observation: &'a Observation,
        forbidden: &'a ForbiddenText,
    ) -> Self {
        Self {
            context,
            session,
            observation,
            forbidden,
            zoom: None,
        }
    }

    fn point(&self, x: f64, y: f64) -> Result<(i32, i32), String> {
        map_point(&self.observation.model_frame, x, y)
    }

    async fn with_input<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut DesktopInput, Option<&CancellationToken>) -> flow_like_types::Result<T>
        + Send
        + 'static,
    ) -> Result<T, String> {
        let mut input = self
            .session
            .create_enigo(self.context)
            .await
            .map_err(error_text)?;
        let cancellation = self.context.get_cancellation_token();
        tokio::task::spawn_blocking(move || job(&mut input, cancellation.as_ref()))
            .await
            .map_err(error_text)?
            .map_err(error_text)
    }

    async fn click(
        &self,
        point: (i32, i32),
        button: MouseButton,
        count: u32,
        modifiers: &[ChordModifier],
    ) -> Result<(), String> {
        click_at(
            self.context,
            self.session,
            point,
            enigo_button(button),
            count,
            &modifier_list(modifiers),
        )
        .await
        .map_err(error_text)
    }

    async fn click_element(
        &self,
        id: u32,
        button: MouseButton,
        count: u32,
    ) -> Result<Performed, String> {
        let element = self
            .observation
            .elements
            .iter()
            .find(|element| element.id == id)
            .ok_or_else(|| {
                format!(
                    "No element [{id}] in the latest screenshot; its ids run from 1 to {}",
                    self.observation.elements.len()
                )
            })?;
        let point = (element.center.x, element.center.y);
        self.click(point, button, count, &[]).await?;
        Ok(Performed::at(
            format!(
                "Clicked {} x{count} on element [{id}] {}",
                button.name(),
                element.role
            ),
            &[point],
        ))
    }

    async fn move_to(&self, x: f64, y: f64) -> Result<Performed, String> {
        use enigo::{Coordinate, Mouse};
        let point = self.point(x, y)?;
        self.with_input(move |input, _| {
            input
                .move_mouse(point.0, point.1, Coordinate::Abs)
                .map_err(|e| {
                    flow_like_types::anyhow!("Failed to move the mouse to {:?}: {}", point, e)
                })
        })
        .await?;
        Ok(Performed::at(
            format!("Moved the pointer to ({x}, {y})"),
            &[point],
        ))
    }

    async fn drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<Performed, String> {
        let start = self.point(from.0, from.1)?;
        let end = self.point(to.0, to.1)?;
        self.with_input(move |input, cancellation| {
            perform_drag(
                input,
                start,
                end,
                enigo::Button::Left,
                DRAG_MS,
                cancellation,
            )
        })
        .await?;
        Ok(Performed::at(
            format!(
                "Dragged from ({}, {}) to ({}, {})",
                from.0, from.1, to.0, to.1
            ),
            &[start, end],
        ))
    }

    async fn scroll(
        &self,
        x: f64,
        y: f64,
        direction: ScrollDirection,
        ticks: u32,
    ) -> Result<Performed, String> {
        use enigo::{Axis, Coordinate, Mouse};
        let point = self.point(x, y)?;
        let (axis, step) = match direction {
            ScrollDirection::Up => (Axis::Vertical, -1),
            ScrollDirection::Down => (Axis::Vertical, 1),
            ScrollDirection::Left => (Axis::Horizontal, -1),
            ScrollDirection::Right => (Axis::Horizontal, 1),
        };
        self.with_input(move |input, cancellation| {
            input
                .move_mouse(point.0, point.1, Coordinate::Abs)
                .map_err(|e| {
                    flow_like_types::anyhow!("Failed to move the mouse to {:?}: {}", point, e)
                })?;
            interruptible_sleep(50, cancellation)?;
            for _ in 0..ticks {
                check_cancellation(cancellation)?;
                input
                    .scroll(step, axis)
                    .map_err(|e| flow_like_types::anyhow!("Failed to scroll: {}", e))?;
                interruptible_sleep(SCROLL_TICK_MS, cancellation)?;
            }
            Ok(())
        })
        .await?;
        Ok(Performed::at(
            format!("Scrolled {} {ticks} ticks at ({x}, {y})", direction.name()),
            &[point],
        ))
    }

    async fn type_text(&self, text: &str, press_enter: bool) -> Result<Performed, String> {
        use enigo::{Direction, Keyboard};
        self.forbidden.check(text)?;
        let typed = text.to_string();
        self.with_input(move |input, cancellation| {
            check_cancellation(cancellation)?;
            input
                .text(&typed)
                .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;
            if press_enter {
                input
                    .key(enigo::Key::Return, Direction::Click)
                    .map_err(|e| flow_like_types::anyhow!("Failed to press Enter: {}", e))?;
            }
            Ok(())
        })
        .await?;
        let enter = if press_enter {
            " and pressed Enter"
        } else {
            ""
        };
        Ok(Performed::plain(format!(
            "Typed {} characters{enter}",
            text.chars().count()
        )))
    }

    async fn press(&self, chord: &KeyChord, repeat: u32) -> Result<Performed, String> {
        use enigo::{Direction, Keyboard};
        let (modifiers, key) = chord_keys(chord).map_err(error_text)?;
        self.with_input(move |input, cancellation| {
            for index in 0..repeat {
                check_cancellation(cancellation)?;
                if index > 0 {
                    interruptible_sleep(KEY_REPEAT_MS, cancellation)?;
                }
                with_modifiers(input, &modifiers, |input| {
                    input
                        .key(key, Direction::Click)
                        .map_err(|e| flow_like_types::anyhow!("Failed to press {:?}: {}", key, e))
                })?;
            }
            Ok(())
        })
        .await?;
        Ok(Performed::plain(format!(
            "Pressed {} x{repeat}",
            describe_chord(chord)
        )))
    }

    async fn hold(&self, chord: &KeyChord, duration_ms: u64) -> Result<Performed, String> {
        use enigo::{Direction, Keyboard};
        let (modifiers, key) = chord_keys(chord).map_err(error_text)?;
        self.with_input(move |input, cancellation| {
            with_modifiers(input, &modifiers, |input| {
                input
                    .key(key, Direction::Press)
                    .map_err(|e| flow_like_types::anyhow!("Failed to press {:?}: {}", key, e))?;
                let held = interruptible_sleep(duration_ms, cancellation);
                let released = input
                    .key(key, Direction::Release)
                    .map_err(|e| flow_like_types::anyhow!("Failed to release {:?}: {}", key, e));
                held.and(released)
            })
        })
        .await?;
        Ok(Performed::plain(format!(
            "Held {} for {duration_ms} ms",
            describe_chord(chord)
        )))
    }

    async fn zoom_in(&mut self, corners: (f64, f64, f64, f64)) -> Result<Performed, String> {
        let (x0, y0, x1, y1) = corners;
        let shot = &self.observation.shot;
        let crop = zoom_box(
            corners,
            self.observation.model_size(),
            (shot.image.width(), shot.image.height()),
        )?;
        let label = format!(
            "Enlarged view of screenshot region ({x0}, {y0})-({x1}, {y1}); coordinates still refer to the full screenshot:"
        );
        self.zoom = Some(
            zoom_view(&shot.image, crop, label)
                .await
                .map_err(error_text)?,
        );
        Ok(Performed::plain(format!(
            "The enlarged view of ({x0}, {y0})-({x1}, {y1}) follows the next screenshot"
        )))
    }

    async fn act(&mut self, action: &Action) -> Result<Performed, String> {
        match action {
            Action::Click {
                x,
                y,
                button,
                count,
                modifiers,
            } => {
                let point = self.point(*x, *y)?;
                self.click(point, *button, *count, modifiers).await?;
                Ok(Performed::at(
                    format!("Clicked {} x{count} at ({x}, {y})", button.name()),
                    &[point],
                ))
            }
            Action::ClickElement { id, button, count } => {
                self.click_element(*id, *button, *count).await
            }
            Action::Move { x, y } => self.move_to(*x, *y).await,
            Action::Drag { from, to } => self.drag(*from, *to).await,
            Action::Scroll {
                x,
                y,
                direction,
                amount,
            } => self.scroll(*x, *y, *direction, *amount).await,
            Action::Type { text, press_enter } => self.type_text(text, *press_enter).await,
            Action::Key { chord, repeat } => self.press(chord, *repeat).await,
            Action::HoldKey { chord, duration_ms } => self.hold(chord, *duration_ms).await,
            Action::Wait { ms } => {
                crate::rpa::branch::delay(self.context, std::time::Duration::from_millis(*ms))
                    .await
                    .map_err(error_text)?;
                Ok(Performed::plain(format!("Waited {ms} ms")))
            }
            Action::Zoom { x0, y0, x1, y1 } => self.zoom_in((*x0, *y0, *x1, *y1)).await,
            Action::Done { .. } | Action::AskUser { .. } => {
                Err("done and ask_user end the task and are not desktop actions".to_string())
            }
        }
    }
}

fn describe_chord(chord: &KeyChord) -> String {
    let mut parts: Vec<&str> = chord
        .modifiers
        .iter()
        .map(|modifier| match modifier {
            ChordModifier::Control => "ctrl",
            ChordModifier::Shift => "shift",
            ChordModifier::Alt => "alt",
            ChordModifier::Meta => "cmd",
        })
        .collect();
    parts.push(&chord.key);
    parts.join("+")
}

#[async_trait]
impl<'a> ActionRunner for DesktopRunner<'a> {
    async fn perform(&mut self, action: &Action) -> Result<Performed, String> {
        let performed = self.act(action).await?;
        if action.is_input() {
            self.session
                .apply_delay(self.context)
                .await
                .map_err(error_text)?;
        }
        Ok(performed)
    }
}

pub(crate) fn describe_for_log(actions: &[AgentAction]) -> String {
    actions
        .iter()
        .map(|action| {
            let state = match (action.skipped, action.ok) {
                (true, _) => "skipped",
                (false, true) => "ok",
                (false, false) => "failed",
            };
            format!("{} {state}", action.tool)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{CLICK, DONE, TYPE, WAIT};
    use crate::types::screen_frame::ModelImageBudget;
    use flow_like_types::{Value, json::json};

    #[derive(Default)]
    struct FakeRunner {
        performed: Vec<Action>,
        fail_on: Option<usize>,
    }

    #[async_trait]
    impl ActionRunner for FakeRunner {
        async fn perform(&mut self, action: &Action) -> Result<Performed, String> {
            let index = self.performed.len();
            self.performed.push(action.clone());
            if self.fail_on == Some(index) {
                return Err("window went away".into());
            }
            Ok(Performed::at("ok".into(), &[(index as i32, 0)]))
        }
    }

    fn call(id: &str, name: &str, arguments: Value) -> ToolCallRequest {
        ToolCallRequest {
            id: id.into(),
            call_id: None,
            name: name.into(),
            arguments,
        }
    }

    fn click(id: &str) -> ToolCallRequest {
        call(id, CLICK, json!({"x": 1, "y": 2}))
    }

    fn summary(outcome: &BatchOutcome) -> Vec<(bool, bool)> {
        outcome.actions.iter().map(|a| (a.skipped, a.ok)).collect()
    }

    #[tokio::test]
    async fn a_failure_skips_the_rest_of_the_turn() {
        let mut runner = FakeRunner {
            fail_on: Some(1),
            ..Default::default()
        };
        let calls = [click("a"), click("b"), click("c")];
        let outcome = run_batch(&calls, 5, false, &mut runner).await;
        assert_eq!(runner.performed.len(), 2);
        assert_eq!(
            summary(&outcome),
            [(false, true), (false, false), (true, false)]
        );
        assert_eq!(outcome.actions[1].message, "window went away");
        assert_eq!(outcome.actions[2].message, EARLIER_FAILED);
        assert_eq!(
            outcome.actions[0].input_points,
            vec![InputPoint { x: 0, y: 0 }]
        );
        assert!(outcome.acted && outcome.terminal.is_none());
    }

    #[tokio::test]
    async fn invalid_arguments_stop_the_turn_without_running_anything() {
        let mut runner = FakeRunner::default();
        let calls = [call("a", TYPE, json!({"text": ""})), click("b")];
        let outcome = run_batch(&calls, 5, false, &mut runner).await;
        assert!(runner.performed.is_empty());
        assert_eq!(summary(&outcome), [(false, false), (true, false)]);
        assert!(!outcome.acted);
    }

    #[tokio::test]
    async fn only_the_allowed_number_of_actions_runs() {
        let mut runner = FakeRunner::default();
        let calls = [click("a"), click("b"), click("c")];
        let outcome = run_batch(&calls, 2, false, &mut runner).await;
        assert_eq!(runner.performed.len(), 2);
        assert_eq!(outcome.actions.len(), 3);
        assert!(outcome.actions[2].skipped);
        assert!(outcome.actions[2].message.contains("only 2 actions"));
    }

    #[tokio::test]
    async fn done_ends_the_turn_after_earlier_actions() {
        let mut runner = FakeRunner::default();
        let calls = [
            call("a", WAIT, json!({"ms": 10})),
            call("b", DONE, json!({"success": true, "answer": "Saved"})),
            click("c"),
        ];
        let outcome = run_batch(&calls, 5, false, &mut runner).await;
        assert_eq!(runner.performed, vec![Action::Wait { ms: 10 }]);
        assert_eq!(
            outcome.terminal,
            Some(Terminal::Done {
                success: true,
                answer: "Saved".into()
            })
        );
        assert!(!outcome.acted);
        assert_eq!(
            summary(&outcome),
            [(false, true), (false, true), (true, false)]
        );
    }

    #[tokio::test]
    async fn done_after_a_failure_is_not_accepted() {
        let mut runner = FakeRunner {
            fail_on: Some(0),
            ..Default::default()
        };
        let calls = [
            click("a"),
            call("b", DONE, json!({"success": true, "answer": "x"})),
        ];
        let outcome = run_batch(&calls, 5, false, &mut runner).await;
        assert!(outcome.terminal.is_none());
        assert!(outcome.actions[1].skipped);
    }

    #[test]
    fn model_points_map_to_desktop_input_and_off_image_points_fail() {
        let retina = ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (2880, 1800)).unwrap();
        let (width, height) = ModelImageBudget::default().fit(2880, 1800);
        let model = retina.resized(width, height).unwrap();
        let (x, y) = map_point(&model, width as f64 / 2.0, height as f64 / 2.0).unwrap();
        assert!((x + 720).abs() <= 1 && (y - 250).abs() <= 1);
        assert_eq!(map_point(&model, 0.0, 0.0).unwrap(), (-1440, -200));
        let error = map_point(&model, width as f64, 10.0).unwrap_err();
        assert!(error.contains("outside") && error.contains("latest screenshot"));
        assert!(map_point(&model, -0.5, 10.0).is_err());

        let uhd = ScreenFrame::new(Some(0), (0, 0, 3840, 2160), (3840, 2160)).unwrap();
        let (width, height) = ModelImageBudget::default().fit(3840, 2160);
        let model = uhd.resized(width, height).unwrap();
        let (x, y) = map_point(&model, 100.0, 50.0).unwrap();
        let scale = 3840.0 / width as f64;
        assert!(
            (x as f64 - 100.0 * scale).abs() <= 1.0
                && (y as f64 - 50.0 * 2160.0 / height as f64).abs() <= 1.0
        );
    }

    #[test]
    fn modifiers_and_chords_render_for_input_and_logs() {
        assert_eq!(
            modifier_list(&[ChordModifier::Control, ChordModifier::Meta]),
            "ctrl,meta"
        );
        let chord = KeyChord {
            modifiers: vec![ChordModifier::Meta, ChordModifier::Shift],
            key: "t".into(),
        };
        assert_eq!(describe_chord(&chord), "cmd+shift+t");
        let (modifiers, key) = chord_keys(&chord).unwrap();
        assert_eq!(modifiers, vec![enigo::Key::Meta, enigo::Key::Shift]);
        assert_eq!(key, enigo::Key::Unicode('t'));
        assert!(
            chord_keys(&KeyChord {
                modifiers: vec![],
                key: "PageDown".into()
            })
            .is_ok()
        );
    }

    #[test]
    fn log_lines_name_tools_without_arguments() {
        let actions = vec![AgentAction {
            tool: TYPE.into(),
            args: json!({"text": "secret"}),
            input_points: vec![],
            skipped: false,
            ok: true,
            message: "Typed 6 characters".into(),
        }];
        assert_eq!(describe_for_log(&actions), "type ok");
    }
}

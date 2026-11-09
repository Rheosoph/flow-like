use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use enigo::Axis;
use flow_like_types::interaction_observation::{
    InteractionAction, InteractionFocus, InteractionMapping, InteractionObservation,
    InteractionPhase, InteractionPoint, InteractionPointer, InteractionRect, PointerSource,
};
use flow_like_types::tokio_util::sync::CancellationToken;
use image::{DynamicImage, ImageFormat};

use crate::computer::keyboard::{ChordModifier, parse_key_chord};
use crate::computer::native::input::{DesktopInput, observed_cursor_position};
use crate::computer::native::{self, gestures};
use crate::computer::window::WindowInfo;
use crate::types::screen_frame::{
    ModelImageBudget, ScreenFrame, capture_display, fit_image_for_model, monitor_frame,
    select_monitor,
};

const CLICK_DELAY_MS: u64 = 80;
const PRIMARY_DISPLAY: i64 = -1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotPointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

pub struct BotScreen {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub observation: Option<InteractionObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WindowState {
    id: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    minimized: bool,
}

impl From<&WindowInfo> for WindowState {
    fn from(window: &WindowInfo) -> Self {
        Self {
            id: window.id.clone(),
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            minimized: window.is_minimized,
        }
    }
}

#[derive(Clone)]
pub(super) enum CapturedSurface {
    Display(ScreenFrame),
    Window(WindowState),
}

#[derive(Clone)]
pub(super) struct CapturedDesktop {
    pub frame: ScreenFrame,
    pub surface: CapturedSurface,
    pub focus: Option<WindowState>,
    pub focus_known: bool,
    pub stable: bool,
}

impl CapturedDesktop {
    pub(super) fn new(
        frame: ScreenFrame,
        surface: CapturedSurface,
        windows: Option<&[WindowInfo]>,
    ) -> Self {
        Self {
            frame,
            surface,
            focus: windows.and_then(focused).map(WindowState::from),
            focus_known: windows.is_some(),
            stable: true,
        }
    }

    pub(super) fn validate(
        &self,
        display: Option<&ScreenFrame>,
        windows: Option<&[WindowInfo]>,
    ) -> flow_like_types::Result<()> {
        let stale = |why: &str| {
            flow_like_types::anyhow!("{why}; take a fresh computer_screenshot before acting")
        };
        if !self.stable {
            return Err(stale(
                "The desktop changed while the screenshot was captured",
            ));
        }
        match &self.surface {
            CapturedSurface::Display(expected) if display != Some(expected) => {
                return Err(stale("The captured display geometry changed"));
            }
            CapturedSurface::Window(expected) => {
                let current = windows
                    .and_then(|windows| windows.iter().find(|window| window.id == expected.id));
                if current.map(WindowState::from).as_ref() != Some(expected) {
                    return Err(stale(
                        "The captured window moved, resized, minimized or closed",
                    ));
                }
                if !current.is_some_and(|window| window.is_focused) {
                    return Err(stale(
                        "Focus the captured window with computer_window first",
                    ));
                }
            }
            _ => {}
        }
        if self.focus_known {
            let current =
                windows.ok_or_else(|| stale("The focused window could not be checked"))?;
            if focused(current).map(WindowState::from) != self.focus {
                return Err(stale("The focused window changed since the screenshot"));
            }
        }
        Ok(())
    }

    fn validate_current(&self) -> flow_like_types::Result<()> {
        let display = match self.surface {
            CapturedSurface::Display(_) => Some(current_display_frame()?),
            CapturedSurface::Window(_) => None,
        };
        self.validate(display.as_ref(), native::windows().ok().as_deref())
    }
}

#[derive(Clone)]
struct ActionRecord {
    kind: &'static str,
    phase: InteractionPhase,
    at_ms: u64,
    target: Option<(i32, i32)>,
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn current_display_frame() -> flow_like_types::Result<ScreenFrame> {
    let monitors = xcap::Monitor::all()?;
    let (index, monitor) = select_monitor(&monitors, PRIMARY_DISPLAY)?;
    monitor_frame(monitor, Some(index as u32))
}

fn focused(windows: &[WindowInfo]) -> Option<&WindowInfo> {
    windows
        .iter()
        .find(|window| window.is_focused && !window.is_minimized)
}

const FOCUS_LABEL_CHARS: usize = 160;

fn focus_of(frame: &ScreenFrame, window: &WindowInfo) -> Option<InteractionFocus> {
    let bounds = window_bounds(frame, window)?;
    let label = match &window.app_name {
        Some(app) if !window.title.is_empty() => format!("{app}: {}", window.title),
        Some(app) => app.clone(),
        None => window.title.clone(),
    };
    Some(InteractionFocus {
        label: label.chars().take(FOCUS_LABEL_CHARS).collect(),
        bounds: Some(bounds),
    })
}

fn normalized_point(frame: &ScreenFrame, point: (i32, i32)) -> Option<InteractionPoint> {
    frame.validate().ok()?;
    frame
        .contains_input(point.0, point.1)
        .then(|| InteractionPoint {
            x: (point.0 as f64 - frame.x as f64) / frame.width as f64,
            y: (point.1 as f64 - frame.y as f64) / frame.height as f64,
        })
}

fn window_bounds(frame: &ScreenFrame, window: &WindowInfo) -> Option<InteractionRect> {
    frame.validate().ok()?;
    let x0 = (window.x as i64).max(frame.x as i64);
    let y0 = (window.y as i64).max(frame.y as i64);
    let x1 = (window.x as i64 + window.width as i64).min(frame.x as i64 + frame.width as i64);
    let y1 = (window.y as i64 + window.height as i64).min(frame.y as i64 + frame.height as i64);
    (x1 > x0 && y1 > y0).then(|| InteractionRect {
        x: (x0 - frame.x as i64) as f64 / frame.width as f64,
        y: (y0 - frame.y as i64) as f64 / frame.height as f64,
        width: (x1 - x0) as f64 / frame.width as f64,
        height: (y1 - y0) as f64 / frame.height as f64,
    })
}

fn observation(
    frame: &ScreenFrame,
    windows: Option<&[WindowInfo]>,
    pointer: Option<(i32, i32)>,
    action: Option<&ActionRecord>,
) -> InteractionObservation {
    InteractionObservation {
        captured_at_ms: now_ms(),
        width: frame.pixel_width,
        height: frame.pixel_height,
        pointer: pointer
            .and_then(|point| normalized_point(frame, point))
            .map(|position| InteractionPointer {
                position,
                source: PointerSource::Observed,
            }),
        focus: windows
            .and_then(focused)
            .and_then(|window| focus_of(frame, window)),
        action: action.map(|action| InteractionAction {
            kind: action.kind.into(),
            phase: action.phase,
            at_ms: action.at_ms,
            target: action
                .target
                .and_then(|point| normalized_point(frame, point)),
            target_bounds: None,
        }),
        mapping: Some(InteractionMapping {
            x: frame.x,
            y: frame.y,
            width: frame.width,
            height: frame.height,
        }),
    }
}

/// The primary display, mouse and keyboard. Coordinates are pixels of the latest screenshot,
/// which is scaled to what a vision model takes; `stop` ends the action in flight.
pub struct BotComputer {
    pub(super) captured: Mutex<Option<CapturedDesktop>>,
    pub(super) operation: tokio::sync::Mutex<()>,
    action: Mutex<Option<ActionRecord>>,
    cancellation: Mutex<CancellationToken>,
    active: Arc<AtomicBool>,
}

impl Default for BotComputer {
    fn default() -> Self {
        Self {
            captured: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            action: Mutex::new(None),
            cancellation: Mutex::new(CancellationToken::new()),
            active: Arc::new(AtomicBool::new(true)),
        }
    }
}

fn modifier_names(modifiers: &[ChordModifier]) -> String {
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

impl BotComputer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stop(&self) {
        let mut token = self.cancellation.lock().unwrap_or_else(|e| e.into_inner());
        token.cancel();
        *token = CancellationToken::new();
    }

    pub async fn screenshot(&self) -> flow_like_types::Result<BotScreen> {
        let _operation = self.operation.lock().await;
        let (screen, captured) = self.capture().await?;
        *self.captured.lock().unwrap_or_else(|e| e.into_inner()) = Some(captured);
        Ok(screen)
    }

    /// A picture of the screen for a viewer: it leaves the coordinates of the latest
    /// `screenshot` untouched.
    pub async fn view(&self) -> flow_like_types::Result<BotScreen> {
        let _operation = self.operation.lock().await;
        Ok(self.capture().await?.0)
    }

    async fn capture(&self) -> flow_like_types::Result<(BotScreen, CapturedDesktop)> {
        let ((image, original), before, windows, pointer, display_after, at_ms) =
            tokio::task::spawn_blocking(|| {
                let before = native::windows().ok();
                let at_ms = now_ms();
                let captured = capture_display(PRIMARY_DISPLAY)?;
                Ok::<_, flow_like_types::Error>((
                    captured,
                    before,
                    native::windows().ok(),
                    observed_cursor_position(),
                    current_display_frame().ok(),
                    at_ms,
                ))
            })
            .await??;
        let (image, frame) = fit_image_for_model(
            &DynamicImage::ImageRgba8(image),
            &original,
            ModelImageBudget::default(),
        )?;
        let mut png = Vec::new();
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        let mut captured = CapturedDesktop::new(
            frame.clone(),
            CapturedSurface::Display(original),
            before.as_deref(),
        );
        captured.stable = captured
            .validate(display_after.as_ref(), windows.as_deref())
            .is_ok();
        let mut observation = self.describe(&frame, windows.as_deref(), pointer);
        observation.captured_at_ms = at_ms;
        if !captured.stable {
            observation.focus = None;
            observation.pointer = None;
        }
        let screen = BotScreen {
            png,
            width: image.width(),
            height: image.height(),
            observation: Some(observation),
        };
        Ok((screen, captured))
    }

    pub(super) fn describe(
        &self,
        frame: &ScreenFrame,
        windows: Option<&[WindowInfo]>,
        pointer: Option<(i32, i32)>,
    ) -> InteractionObservation {
        observation(
            frame,
            windows,
            pointer,
            self.action
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref(),
        )
    }

    pub(super) fn clear_capture(&self) {
        *self.captured.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(super) fn begin_action(&self, kind: &'static str, target: Option<(i32, i32)>) {
        *self.action.lock().unwrap_or_else(|e| e.into_inner()) = Some(ActionRecord {
            kind,
            phase: InteractionPhase::Running,
            at_ms: now_ms(),
            target,
        });
    }

    pub(super) fn finish_action<T>(&self, result: &flow_like_types::Result<T>) {
        if let Some(action) = self
            .action
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            action.phase = if result.is_ok() {
                InteractionPhase::Completed
            } else {
                InteractionPhase::Failed
            };
            action.at_ms = now_ms();
        }
    }

    fn latest_capture(&self) -> flow_like_types::Result<CapturedDesktop> {
        self.captured
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Take a fresh computer_screenshot before acting")
            })
    }

    fn prepare_action(
        &self,
        kind: &'static str,
        point: Option<(f64, f64)>,
    ) -> flow_like_types::Result<(CapturedDesktop, Option<(i32, i32)>)> {
        self.begin_action(kind, None);
        let prepared = (|| {
            let captured = self.latest_capture()?;
            let point = point
                .map(|(x, y)| captured.frame.pixel_to_input(x, y))
                .transpose()?;
            if let Some(action) = self
                .action
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                action.target = point;
            }
            Ok((captured, point))
        })();
        if prepared.is_err() {
            self.finish_action(&prepared);
        }
        prepared
    }

    async fn with_checked_input<T: Send + 'static>(
        &self,
        captured: CapturedDesktop,
        job: impl FnOnce(&mut DesktopInput, Option<&CancellationToken>) -> flow_like_types::Result<T>
        + Send
        + 'static,
    ) -> flow_like_types::Result<T> {
        self.with_input(move |input, cancellation| {
            captured.validate_current()?;
            job(input, cancellation)
        })
        .await
    }

    pub(super) async fn with_input<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut DesktopInput, Option<&CancellationToken>) -> flow_like_types::Result<T>
        + Send
        + 'static,
    ) -> flow_like_types::Result<T> {
        let cancellation = self
            .cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut input = DesktopInput::new(self.active.clone(), Some(cancellation.clone())).await?;
        tokio::task::spawn_blocking(move || job(&mut input, Some(&cancellation))).await?
    }

    pub async fn click(
        &self,
        x: f64,
        y: f64,
        button: BotPointerButton,
        count: u32,
    ) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let (captured, point) = self.prepare_action("click", Some((x, y)))?;
        let point = point.expect("pointer action has a point");
        let enigo_button = match button {
            BotPointerButton::Left => enigo::Button::Left,
            BotPointerButton::Right => enigo::Button::Right,
            BotPointerButton::Middle => enigo::Button::Middle,
        };
        let result = self
            .with_checked_input(captured, move |input, cancel| {
                gestures::click(
                    input,
                    point,
                    enigo_button,
                    count,
                    "",
                    CLICK_DELAY_MS,
                    cancel,
                )
            })
            .await;
        self.finish_action(&result);
        result?;
        Ok(format!("Clicked {button:?} x{count} at ({x}, {y})"))
    }

    pub async fn move_to(&self, x: f64, y: f64) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let (captured, point) = self.prepare_action("move", Some((x, y)))?;
        let point = point.expect("pointer action has a point");
        let result = self
            .with_checked_input(captured, move |input, _| gestures::move_to(input, point))
            .await;
        self.finish_action(&result);
        result?;
        Ok(format!("Moved the pointer to ({x}, {y})"))
    }

    pub async fn scroll(
        &self,
        x: f64,
        y: f64,
        direction: BotScrollDirection,
        ticks: u32,
    ) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let (captured, point) = self.prepare_action("scroll", Some((x, y)))?;
        let point = point.expect("pointer action has a point");
        let (axis, step) = match direction {
            BotScrollDirection::Up => (Axis::Vertical, -1),
            BotScrollDirection::Down => (Axis::Vertical, 1),
            BotScrollDirection::Left => (Axis::Horizontal, -1),
            BotScrollDirection::Right => (Axis::Horizontal, 1),
        };
        let result = self
            .with_checked_input(captured, move |input, cancel| {
                gestures::scroll(input, point, axis, step, ticks, cancel)
            })
            .await;
        self.finish_action(&result);
        result?;
        Ok(format!(
            "Scrolled {direction:?} {ticks} ticks at ({x}, {y})"
        ))
    }

    pub async fn type_text(&self, text: &str) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let (captured, _) = self.prepare_action("type", None)?;
        let typed = text.to_string();
        let count = text.chars().count();
        let result = self
            .with_checked_input(captured, move |input, cancel| {
                gestures::type_text(input, &typed, false, cancel)
            })
            .await;
        self.finish_action(&result);
        result?;
        Ok(format!("Typed {count} characters"))
    }

    pub async fn press(&self, chord: &str) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let (captured, _) = self.prepare_action("key", None)?;
        let parsed = parse_key_chord(chord)
            .and_then(|parsed| gestures::chord_keys(&parsed).map(|keys| (parsed, keys)));
        if parsed.is_err() {
            self.finish_action(&parsed);
        }
        let (parsed, (modifiers, key)) = parsed?;
        let shown = format!("{}{}", modifier_names(&parsed.modifiers), parsed.key);
        let result = self
            .with_checked_input(captured, move |input, cancel| {
                gestures::press(input, &modifiers, key, 1, cancel)
            })
            .await;
        self.finish_action(&result);
        result?;
        Ok(format!("Pressed {shown}"))
    }
}

impl Drop for BotComputer {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> ScreenFrame {
        ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (1440, 900)).unwrap()
    }

    fn window() -> WindowInfo {
        WindowInfo {
            id: "editor".into(),
            title: "Draft".into(),
            app_name: Some("Editor".into()),
            x: -1440,
            y: -200,
            width: 1000,
            height: 800,
            is_focused: true,
            is_minimized: false,
        }
    }

    #[test]
    fn pointer_readback_and_intended_click_stay_distinct_after_resizing() {
        let action = ActionRecord {
            kind: "click",
            phase: InteractionPhase::Completed,
            at_ms: 123,
            target: Some((-720, 250)),
        };
        let original = frame();
        let smaller = original.resized(720, 450).unwrap();
        let observed = observation(&smaller, None, Some((-800, 200)), Some(&action));
        assert_eq!((observed.width, observed.height), (720, 450));
        let pointer = observed.pointer.unwrap();
        assert_eq!(pointer.source, PointerSource::Observed);
        assert_eq!(
            pointer.position,
            InteractionPoint {
                x: 640.0 / 1440.0,
                y: 400.0 / 900.0
            }
        );
        assert_eq!(
            observed.action.unwrap().target,
            Some(InteractionPoint { x: 0.5, y: 0.5 })
        );
        assert_eq!(observed.mapping.unwrap().x, -1440);
        assert_eq!(
            normalized_point(&original, (-800, 200)),
            Some(pointer.position)
        );
    }

    #[test]
    fn missing_or_offscreen_cursor_is_not_replaced_with_the_requested_location() {
        let action = ActionRecord {
            kind: "click",
            phase: InteractionPhase::Completed,
            at_ms: 0,
            target: Some((-720, 250)),
        };
        for point in [None, Some((2000, 2000))] {
            let observed = observation(&frame(), None, point, Some(&action));
            assert!(observed.pointer.is_none());
            assert!(observed.action.unwrap().target.is_some());
        }
    }

    #[test]
    fn focus_label_is_capped_and_omitted_outside_the_captured_surface() {
        let mut long = window();
        long.title = "é".repeat(400);
        let focus = observation(&frame(), Some(&[long.clone()]), None, None)
            .focus
            .unwrap();
        assert_eq!(focus.label.chars().count(), 160);

        long.x = 100_000;
        long.y = 100_000;
        assert!(
            observation(&frame(), Some(&[long]), None, None)
                .focus
                .is_none()
        );
    }

    #[test]
    fn focus_outline_is_clipped_to_the_image_and_has_no_input_value() {
        let mut focused = window();
        focused.x = -1600;
        focused.y = -300;
        let observed = observation(&frame(), Some(&[focused]), None, None);
        let focus = observed.focus.unwrap();
        assert_eq!(focus.label, "Editor: Draft");
        assert_eq!(
            focus.bounds,
            Some(InteractionRect {
                x: 0.0,
                y: 0.0,
                width: 840.0 / 1440.0,
                height: 700.0 / 900.0,
            })
        );
    }

    #[test]
    fn display_actions_refuse_focus_and_geometry_changes() {
        let original = frame();
        let window = window();
        let captured = CapturedDesktop::new(
            original.clone(),
            CapturedSurface::Display(original.clone()),
            Some(&[window.clone()]),
        );
        assert!(
            captured
                .validate(Some(&original), Some(&[window.clone()]))
                .is_ok()
        );
        let mut moved = window.clone();
        moved.x += 25;
        assert!(captured.validate(Some(&original), Some(&[moved])).is_err());
        let mut replacement = window.clone();
        replacement.id = "other".into();
        assert!(
            captured
                .validate(Some(&original), Some(&[replacement]))
                .is_err()
        );
        assert!(captured.validate(Some(&original), Some(&[])).is_err());
        let resized = original.resized(2880, 1800).unwrap();
        assert!(captured.validate(Some(&resized), Some(&[window])).is_err());
    }

    #[test]
    fn window_actions_refuse_moved_closed_or_unfocused_targets() {
        let window = window();
        let captured = CapturedDesktop::new(
            frame(),
            CapturedSurface::Window(WindowState::from(&window)),
            Some(&[window.clone()]),
        );
        assert!(captured.validate(None, Some(&[window.clone()])).is_ok());
        let mut resized = window.clone();
        resized.width += 1;
        assert!(captured.validate(None, Some(&[resized])).is_err());
        let mut hidden = window.clone();
        hidden.is_minimized = true;
        assert!(captured.validate(None, Some(&[hidden])).is_err());
        let mut blurred = window;
        blurred.is_focused = false;
        assert!(captured.validate(None, Some(&[blurred])).is_err());
        assert!(captured.validate(None, Some(&[])).is_err());
    }

    #[test]
    fn unavailable_focus_is_best_effort_but_known_focus_cannot_silently_disappear() {
        let original = frame();
        let unknown = CapturedDesktop::new(
            original.clone(),
            CapturedSurface::Display(original.clone()),
            None,
        );
        assert!(unknown.validate(Some(&original), None).is_ok());
        let known = CapturedDesktop::new(
            original.clone(),
            CapturedSurface::Display(original.clone()),
            Some(&[window()]),
        );
        assert!(known.validate(Some(&original), None).is_err());
    }

    #[test]
    fn view_metadata_does_not_replace_the_models_coordinate_frame() {
        let computer = BotComputer::new();
        let model_frame = frame();
        *computer.captured.lock().unwrap() = Some(CapturedDesktop::new(
            model_frame.clone(),
            CapturedSurface::Display(model_frame.clone()),
            None,
        ));
        let other = ScreenFrame::new(Some(0), (0, 0, 1920, 1080), (1280, 720)).unwrap();
        computer.describe(&other, None, Some((500, 500)));
        assert_eq!(
            computer.captured.lock().unwrap().as_ref().unwrap().frame,
            model_frame
        );
        computer.clear_capture();
        assert!(computer.captured.lock().unwrap().is_none());
    }

    #[test]
    fn action_metadata_tracks_failure_without_containing_typed_text() {
        let computer = BotComputer::new();
        computer.begin_action("type", None);
        let failed: flow_like_types::Result<()> =
            Err(flow_like_types::anyhow!("typing secret failed"));
        computer.finish_action(&failed);
        let observed = computer.describe(&frame(), None, None);
        let json = flow_like_types::json::to_string(&observed).unwrap();
        assert!(!json.contains("secret"));
        assert_eq!(observed.action.unwrap().phase, InteractionPhase::Failed);
    }

    #[test]
    fn preflight_failure_replaces_the_previous_completed_action() {
        let computer = BotComputer::new();
        computer.begin_action("click", Some((-720, 250)));
        computer.finish_action(&Ok::<_, flow_like_types::Error>(()));
        assert!(computer.prepare_action("type", None).is_err());
        let action = computer.describe(&frame(), None, None).action.unwrap();
        assert_eq!(action.kind, "type");
        assert_eq!(action.phase, InteractionPhase::Failed);
        assert!(action.target.is_none());
    }

    #[test]
    fn a_capture_that_changed_mid_frame_cannot_be_used_even_after_focus_returns() {
        let original = frame();
        let windows = [window()];
        let mut captured = CapturedDesktop::new(
            original.clone(),
            CapturedSurface::Display(original.clone()),
            Some(&windows),
        );
        captured.stable = false;
        assert!(
            captured
                .validate(Some(&original), Some(&windows))
                .unwrap_err()
                .to_string()
                .contains("while the screenshot")
        );
    }
}

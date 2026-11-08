use std::io::Cursor;

use enigo::{Axis, Button, Coordinate, Direction, Mouse};
use image::{DynamicImage, ImageFormat};

use super::computer::{
    BotComputer, BotPointerButton, BotScreen, CapturedDesktop, CapturedSurface, WindowState, now_ms,
};
use crate::computer::keyboard::parse_key_chord;
use crate::computer::native::{self, gestures};
use crate::computer::window::{WindowInfo, capture_window, launch_application, pick_window};
use crate::types::screen_frame::{
    ModelImageBudget, fit_image_for_model, monitor_frame, select_monitor,
};

const LAUNCH_SETTLE_MS: u64 = 800;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotWindowOp {
    Focus,
    Close,
    Minimize,
    Maximize,
    Move,
    Resize,
}

/// One raw event of the owner driving the screen. Positions are fractions (0..1) of the
/// primary display.
#[derive(Clone, PartialEq, Debug)]
pub enum BotInput {
    Move {
        fx: f64,
        fy: f64,
    },
    Button {
        fx: f64,
        fy: f64,
        button: BotPointerButton,
        down: bool,
    },
    Scroll {
        fx: f64,
        fy: f64,
        dx: i32,
        dy: i32,
    },
    Chord(String),
    Text(String),
}

fn enigo_button(button: BotPointerButton) -> Button {
    match button {
        BotPointerButton::Left => Button::Left,
        BotPointerButton::Right => Button::Right,
        BotPointerButton::Middle => Button::Middle,
    }
}

fn resolve(windows: Vec<WindowInfo>, query: &str) -> flow_like_types::Result<WindowInfo> {
    let query = query.trim();
    if let Some(window) = windows.iter().find(|window| window.id == query) {
        return Ok(window.clone());
    }
    pick_window(windows, query, "", false).ok_or_else(|| {
        flow_like_types::anyhow!(
            "No window matches '{query}'; call computer_windows and pass an id or part of a title"
        )
    })
}

async fn primary_point(fx: f64, fy: f64) -> flow_like_types::Result<(i32, i32)> {
    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return Err(flow_like_types::anyhow!(
            "Pointer position ({fx}, {fy}) is outside the screen"
        ));
    }
    let frame = tokio::task::spawn_blocking(|| {
        let monitors = xcap::Monitor::all()
            .map_err(|e| flow_like_types::anyhow!("Failed to enumerate displays: {e}"))?;
        let (position, monitor) = select_monitor(&monitors, -1)?;
        monitor_frame(monitor, Some(position as u32))
    })
    .await??;
    Ok((
        frame.x + (fx * (frame.width as f64 - 1.0)).round() as i32,
        frame.y + (fy * (frame.height as f64 - 1.0)).round() as i32,
    ))
}

impl BotComputer {
    pub async fn windows(&self) -> flow_like_types::Result<Vec<WindowInfo>> {
        native::windows_async().await
    }

    pub async fn screenshot_window(&self, query: &str) -> flow_like_types::Result<BotScreen> {
        let _operation = self.operation.lock().await;
        let before = self.windows().await?;
        let window = resolve(before.clone(), query)?;
        let id = window.id.clone();
        let at_ms = now_ms();
        let captured = tokio::task::spawn_blocking(move || capture_window(&id))
            .await??
            .ok_or_else(|| {
                flow_like_types::anyhow!("Window {} closed before it was captured", window.id)
            })?;
        let (image, _, frame) = captured;
        let frame = frame.ok_or_else(|| {
            flow_like_types::anyhow!(
                "Window '{}' has no usable geometry (minimized?); restore it with computer_window first",
                window.title
            )
        })?;
        let (image, frame) = fit_image_for_model(
            &DynamicImage::ImageRgba8(image),
            &frame,
            ModelImageBudget::default(),
        )?;
        let mut png = Vec::new();
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        let (windows, pointer) = tokio::task::spawn_blocking(|| {
            (
                native::windows().ok(),
                native::input::observed_cursor_position(),
            )
        })
        .await?;
        let mut captured = CapturedDesktop::new(
            frame.clone(),
            CapturedSurface::Window(WindowState::from(&window)),
            Some(&before),
        );
        captured.stable = captured.validate(None, windows.as_deref()).is_ok();
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
        *self.captured.lock().unwrap_or_else(|e| e.into_inner()) = Some(captured);
        Ok(screen)
    }

    pub async fn window_action(
        &self,
        query: &str,
        operation: BotWindowOp,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<i32>,
        height: Option<i32>,
    ) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        let name = match operation {
            BotWindowOp::Focus => "focus",
            BotWindowOp::Close => "close",
            BotWindowOp::Minimize => "minimize",
            BotWindowOp::Maximize => "maximize",
            BotWindowOp::Move | BotWindowOp::Resize => "move_resize",
        };
        self.begin_action(name, None);
        let result = async {
            let window = resolve(self.windows().await?, query)?;
            let bounds = match operation {
                BotWindowOp::Move | BotWindowOp::Resize => {
                    let (width, height) = (
                        width.unwrap_or(window.width as i32),
                        height.unwrap_or(window.height as i32),
                    );
                    if width <= 0 || height <= 0 {
                        return Err(flow_like_types::anyhow!("Window size must be positive"));
                    }
                    Some((x.unwrap_or(window.x), y.unwrap_or(window.y), width, height))
                }
                _ => None,
            };
            let id = window.id.clone();
            self.clear_capture();
            tokio::task::spawn_blocking(move || native::operate_window(&id, name, bounds))
                .await??;
            Ok(format!(
                "{operation:?} applied to window '{}' ({})",
                window.title, window.id
            ))
        }
        .await;
        self.finish_action(&result);
        result
    }

    pub async fn launch(&self, program: &str, args: &[String]) -> flow_like_types::Result<String> {
        let _operation = self.operation.lock().await;
        self.begin_action("launch", None);
        self.clear_capture();
        let (path, arguments) = (program.to_string(), args.to_vec());
        let result = tokio::task::spawn_blocking(move || launch_application(&path, &arguments))
            .await
            .map_err(flow_like_types::Error::from)
            .and_then(|result| result);
        self.finish_action(&result);
        let pid = result?;
        tokio::time::sleep(std::time::Duration::from_millis(LAUNCH_SETTLE_MS)).await;
        Ok(if pid >= 0 {
            format!("Started {program} (pid {pid})")
        } else {
            format!("Started {program}")
        })
    }

    pub async fn inject(&self, input: BotInput) -> flow_like_types::Result<()> {
        let _operation = self.operation.lock().await;
        self.clear_capture();
        match input {
            BotInput::Move { fx, fy } => {
                let point = primary_point(fx, fy).await?;
                self.with_input(move |input, _| gestures::move_to(input, point))
                    .await
            }
            BotInput::Button {
                fx,
                fy,
                button,
                down,
            } => {
                let point = primary_point(fx, fy).await?;
                let button = enigo_button(button);
                self.with_input(move |input, _| {
                    input
                        .move_mouse(point.0, point.1, Coordinate::Abs)
                        .map_err(|e| flow_like_types::anyhow!("Failed to move the mouse: {e}"))?;
                    let direction = if down {
                        Direction::Press
                    } else {
                        Direction::Release
                    };
                    input.button(button, direction).map_err(|e| {
                        flow_like_types::anyhow!("Failed to {direction:?} {button:?}: {e}")
                    })?;
                    if down {
                        input.latch_button(button);
                    } else {
                        input.unlatch_button(button);
                    }
                    Ok(())
                })
                .await
            }
            BotInput::Scroll { fx, fy, dx, dy } => {
                let point = primary_point(fx, fy).await?;
                self.with_input(move |input, cancel| {
                    gestures::move_to(input, point)?;
                    for (amount, axis) in [(dy, Axis::Vertical), (dx, Axis::Horizontal)] {
                        if amount != 0 {
                            gestures::scroll(
                                input,
                                point,
                                axis,
                                amount.signum(),
                                amount.unsigned_abs().min(30),
                                cancel,
                            )?;
                        }
                    }
                    Ok(())
                })
                .await
            }
            BotInput::Chord(chord) => {
                let parsed = parse_key_chord(&chord)?;
                let (modifiers, key) = gestures::chord_keys(&parsed)?;
                self.with_input(move |input, cancel| {
                    gestures::press(input, &modifiers, key, 1, cancel)
                })
                .await
            }
            BotInput::Text(text) => {
                self.with_input(move |input, cancel| {
                    gestures::type_text(input, &text, false, cancel)
                })
                .await
            }
        }
    }
}

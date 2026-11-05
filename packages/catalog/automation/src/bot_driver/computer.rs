use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use enigo::Axis;
use flow_like_types::tokio_util::sync::CancellationToken;
use image::{DynamicImage, ImageFormat};

use crate::computer::keyboard::{ChordModifier, parse_key_chord};
use crate::computer::native::gestures;
use crate::computer::native::input::DesktopInput;
use crate::types::screen_frame::{
    ModelImageBudget, ScreenFrame, capture_display, fit_image_for_model,
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
}

/// The primary display, mouse and keyboard. Coordinates are pixels of the latest screenshot,
/// which is scaled to what a vision model takes; `stop` ends the action in flight.
pub struct BotComputer {
    pub(super) frame: Mutex<Option<ScreenFrame>>,
    cancellation: Mutex<CancellationToken>,
    active: Arc<AtomicBool>,
}

impl Default for BotComputer {
    fn default() -> Self {
        Self {
            frame: Mutex::new(None),
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
        let (screen, frame) = self.capture().await?;
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
        Ok(screen)
    }

    /// A picture of the screen for a viewer: it leaves the coordinates of the latest
    /// `screenshot` untouched.
    pub async fn view(&self) -> flow_like_types::Result<BotScreen> {
        Ok(self.capture().await?.0)
    }

    async fn capture(&self) -> flow_like_types::Result<(BotScreen, ScreenFrame)> {
        let (image, frame) =
            tokio::task::spawn_blocking(|| capture_display(PRIMARY_DISPLAY)).await??;
        let (image, frame) = fit_image_for_model(
            &DynamicImage::ImageRgba8(image),
            &frame,
            ModelImageBudget::default(),
        )?;
        let mut png = Vec::new();
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
        let screen = BotScreen {
            png,
            width: image.width(),
            height: image.height(),
        };
        Ok((screen, frame))
    }

    pub(super) fn point(&self, x: f64, y: f64) -> flow_like_types::Result<(i32, i32)> {
        let frame = self.frame.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let frame = frame.ok_or_else(|| {
            flow_like_types::anyhow!("Take a screenshot first: coordinates are pixels of it")
        })?;
        frame
            .pixel_to_input(x, y)
            .map_err(|e| flow_like_types::anyhow!("{e}; use coordinates of the latest screenshot"))
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
        let point = self.point(x, y)?;
        let enigo_button = match button {
            BotPointerButton::Left => enigo::Button::Left,
            BotPointerButton::Right => enigo::Button::Right,
            BotPointerButton::Middle => enigo::Button::Middle,
        };
        self.with_input(move |input, cancel| {
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
        .await?;
        Ok(format!("Clicked {button:?} x{count} at ({x}, {y})"))
    }

    pub async fn move_to(&self, x: f64, y: f64) -> flow_like_types::Result<String> {
        let point = self.point(x, y)?;
        self.with_input(move |input, _| gestures::move_to(input, point))
            .await?;
        Ok(format!("Moved the pointer to ({x}, {y})"))
    }

    pub async fn scroll(
        &self,
        x: f64,
        y: f64,
        direction: BotScrollDirection,
        ticks: u32,
    ) -> flow_like_types::Result<String> {
        let point = self.point(x, y)?;
        let (axis, step) = match direction {
            BotScrollDirection::Up => (Axis::Vertical, -1),
            BotScrollDirection::Down => (Axis::Vertical, 1),
            BotScrollDirection::Left => (Axis::Horizontal, -1),
            BotScrollDirection::Right => (Axis::Horizontal, 1),
        };
        self.with_input(move |input, cancel| {
            gestures::scroll(input, point, axis, step, ticks, cancel)
        })
        .await?;
        Ok(format!(
            "Scrolled {direction:?} {ticks} ticks at ({x}, {y})"
        ))
    }

    pub async fn type_text(&self, text: &str) -> flow_like_types::Result<String> {
        let typed = text.to_string();
        let count = text.chars().count();
        self.with_input(move |input, cancel| gestures::type_text(input, &typed, false, cancel))
            .await?;
        Ok(format!("Typed {count} characters"))
    }

    pub async fn press(&self, chord: &str) -> flow_like_types::Result<String> {
        let parsed = parse_key_chord(chord)?;
        let (modifiers, key) = gestures::chord_keys(&parsed)?;
        let shown = format!("{}{}", modifier_names(&parsed.modifiers), parsed.key);
        self.with_input(move |input, cancel| gestures::press(input, &modifiers, key, 1, cancel))
            .await?;
        Ok(format!("Pressed {shown}"))
    }
}

impl Drop for BotComputer {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

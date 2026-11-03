//! What the bot panel's Computer tab shows: the latest picture of the bot's browser or screen,
//! and the Stop button. Pictures are pulled by the panel; there is no video.

use std::io::Cursor;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, imageops::FilterType};
use serde::Serialize;

const ACTIVE_FOR: Duration = Duration::from_secs(180);
const FRESH_FOR: Duration = Duration::from_millis(1500);
const VIEW_WIDTH: u32 = 960;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    Browser,
    Computer,
}

#[derive(Clone)]
struct Frame {
    png: Vec<u8>,
    kind: FrameKind,
    caption: String,
    at: Instant,
}

static FRAMES: LazyLock<DashMap<String, Frame>> = LazyLock::new(DashMap::new);

pub(crate) fn publish(bot_id: &str, kind: FrameKind, caption: &str, png: Vec<u8>) {
    FRAMES.insert(
        bot_id.to_string(),
        Frame {
            png,
            kind,
            caption: caption.to_string(),
            at: Instant::now(),
        },
    );
}

pub(crate) fn forget(bot_id: &str) {
    FRAMES.remove(bot_id);
}

pub(crate) fn is_fresh(bot_id: &str) -> bool {
    FRAMES
        .get(bot_id)
        .is_some_and(|frame| frame.at.elapsed() < FRESH_FOR)
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ViewState {
    Asleep,
    Active,
}

#[derive(Serialize, Debug)]
pub struct ComputerView {
    state: ViewState,
    kind: Option<FrameKind>,
    caption: Option<String>,
    /// A `data:` URL of the latest picture.
    image: Option<String>,
    age_ms: Option<u64>,
}

impl ComputerView {
    fn asleep() -> Self {
        Self {
            state: ViewState::Asleep,
            kind: None,
            caption: None,
            image: None,
            age_ms: None,
        }
    }
}

fn data_url(png: &[u8]) -> Option<String> {
    let image = image::load_from_memory(png).ok()?;
    let image = if image.width() > VIEW_WIDTH {
        let height = (image.height() as f64 * VIEW_WIDTH as f64 / image.width() as f64) as u32;
        image.resize_exact(VIEW_WIDTH, height.max(1), FilterType::Triangle)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image.to_rgb8())
        .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .ok()?;
    Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)))
}

fn view_of(bot_id: &str) -> ComputerView {
    let Some(frame) = FRAMES.get(bot_id).map(|frame| frame.clone()) else {
        return ComputerView::asleep();
    };
    if frame.at.elapsed() > ACTIVE_FOR {
        return ComputerView::asleep();
    }
    ComputerView {
        state: ViewState::Active,
        kind: Some(frame.kind),
        caption: Some(frame.caption),
        image: data_url(&frame.png),
        age_ms: Some(frame.at.elapsed().as_millis() as u64),
    }
}

/// The latest picture of the bot's browser or screen, or "asleep" when it has done nothing for
/// a while or nothing is running.
#[tauri::command(async)]
pub async fn bot_computer_view(bot_id: String) -> Result<ComputerView, String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    super::browser::refresh_view(&bot_id).await;
    Ok(view_of(&bot_id))
}

/// Ends the bot's browser sessions and any mouse or keyboard action in flight.
#[tauri::command(async)]
pub async fn bot_computer_stop(bot_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        super::browser::stop(&bot_id).await;
        super::computer::stop(&bot_id);
    }
    forget(&bot_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn a_bot_without_a_frame_is_asleep() {
        let view = view_of("view-test-none");
        assert_eq!(view.state, ViewState::Asleep);
        assert!(view.image.is_none());
    }

    #[test]
    fn a_published_frame_is_active_and_a_stopped_one_asleep() {
        publish(
            "view-test-a",
            FrameKind::Browser,
            "https://shop.example/",
            png(1600, 900),
        );
        let view = view_of("view-test-a");
        assert_eq!(view.state, ViewState::Active);
        assert_eq!(view.kind, Some(FrameKind::Browser));
        assert!(
            view.image
                .as_deref()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        forget("view-test-a");
        assert_eq!(view_of("view-test-a").state, ViewState::Asleep);
    }

    #[test]
    fn an_old_frame_is_asleep() {
        FRAMES.insert(
            "view-test-old".into(),
            Frame {
                png: png(10, 10),
                kind: FrameKind::Computer,
                caption: String::new(),
                at: Instant::now() - ACTIVE_FOR - Duration::from_secs(1),
            },
        );
        assert_eq!(view_of("view-test-old").state, ViewState::Asleep);
    }
}

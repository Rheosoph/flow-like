use std::io::Cursor;
use std::time::{Duration, Instant};

use flow_like_types::Bytes;
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

pub const FRAME_MAX_WIDTH: u32 = 1280;
pub const FRESH_FOR: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    Browser,
    Computer,
}

/// A picture of the bot's browser or screen as the playground took it.
#[derive(Clone, Debug)]
pub struct Shot {
    pub png: Vec<u8>,
    pub kind: FrameKind,
    pub caption: String,
}

/// A `Shot` ready to show: a JPEG of at most [`FRAME_MAX_WIDTH`] pixels.
#[derive(Clone, Debug)]
pub struct Frame {
    pub jpeg: Bytes,
    pub width: u32,
    pub height: u32,
    pub kind: FrameKind,
    pub caption: String,
    pub at: Instant,
}

impl Frame {
    pub fn from_shot(shot: &Shot) -> Option<Self> {
        let image = image::load_from_memory(&shot.png).ok()?;
        let image = if image.width() > FRAME_MAX_WIDTH {
            let height = (image.height() as f64 * FRAME_MAX_WIDTH as f64 / image.width() as f64)
                .round() as u32;
            image.resize_exact(FRAME_MAX_WIDTH, height.max(1), FilterType::Triangle)
        } else {
            image
        };
        let rgb = DynamicImage::ImageRgb8(image.to_rgb8());
        let mut jpeg = Vec::new();
        rgb.write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .ok()?;
        Some(Self {
            jpeg: Bytes::from(jpeg),
            width: rgb.width(),
            height: rgb.height(),
            kind: shot.kind,
            caption: shot.caption.clone(),
            at: Instant::now(),
        })
    }

    pub fn is_fresh(&self) -> bool {
        self.at.elapsed() < FRESH_FOR
    }
}

/// The latest [`Frame`] of one bot; `changed` resolves when a newer one arrives.
#[derive(Clone)]
pub struct FrameStream(watch::Receiver<Option<Frame>>);

impl FrameStream {
    pub fn latest(&self) -> Option<Frame> {
        self.0.borrow().clone()
    }

    pub async fn changed(&mut self) -> Option<Frame> {
        self.0.changed().await.ok()?;
        self.latest()
    }
}

pub(crate) struct FrameStore(watch::Sender<Option<Frame>>);

impl Default for FrameStore {
    fn default() -> Self {
        Self(watch::channel(None).0)
    }
}

impl FrameStore {
    pub(crate) fn publish(&self, shot: &Shot) -> Option<Frame> {
        let frame = Frame::from_shot(shot)?;
        self.0.send_replace(Some(frame.clone()));
        Some(frame)
    }

    pub(crate) fn latest(&self) -> Option<Frame> {
        self.0.borrow().clone()
    }

    pub(crate) fn stream(&self) -> FrameStream {
        FrameStream(self.0.subscribe())
    }

    pub(crate) fn clear(&self) {
        self.0.send_replace(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn shot(width: u32, height: u32, kind: FrameKind) -> Shot {
        Shot {
            png: png(width, height),
            kind,
            caption: "caption".into(),
        }
    }

    #[test]
    fn wide_pictures_shrink_to_the_frame_width_keeping_the_ratio() {
        let frame = Frame::from_shot(&shot(2560, 1600, FrameKind::Computer)).unwrap();
        assert_eq!((frame.width, frame.height), (1280, 800));
        assert_eq!(&frame.jpeg[..2], &[0xFF, 0xD8]);
        let small = Frame::from_shot(&shot(640, 480, FrameKind::Browser)).unwrap();
        assert_eq!((small.width, small.height), (640, 480));
    }

    #[test]
    fn a_broken_picture_is_not_a_frame() {
        let broken = Shot {
            png: vec![1, 2, 3],
            kind: FrameKind::Browser,
            caption: String::new(),
        };
        assert!(Frame::from_shot(&broken).is_none());
    }

    #[tokio::test]
    async fn a_stream_sees_the_latest_frame_and_wakes_on_a_new_one() {
        let store = FrameStore::default();
        let mut stream = store.stream();
        assert!(stream.latest().is_none());
        store.publish(&shot(10, 10, FrameKind::Browser));
        let frame = stream.changed().await.unwrap();
        assert_eq!(frame.kind, FrameKind::Browser);
        assert!(frame.is_fresh());
        store.clear();
        assert!(store.latest().is_none());
    }
}

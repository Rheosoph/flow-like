//! Barcode detection for the chat composer's live camera view, with the decoder of the Read
//! Barcodes node (rxing).

use std::sync::atomic::{AtomicU32, Ordering};

use flow_like_catalog::image::content::read_barcodes::{
    Barcode, BarcodePreprocessingOptions, ReadBarcodeOptions, read_barcodes_in_luma,
};
use tauri::ipc::{InvokeBody, Request};

const MAX_EDGE: u32 = 4096;
/// Light codes on dark are rare, and looking for them doubles the work on every frame without
/// a code. One live frame in this many looks. The scanner keeps an outline for 900 ms, which
/// has to cover the frames in between on a slow phone.
const INVERTED_EVERY: u32 = 3;

static LIVE_FRAMES: AtomicU32 = AtomicU32::new(0);

fn header(request: &Request<'_>, name: &str) -> Option<u32> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

/// The captured photo is read like a live frame, on four times the rows and in both
/// polarities. The node's whole default search would hold the shutter for seconds on a phone
/// in dim light; a bot that needs it has `barcode_read` for the attached photo.
fn capture_options() -> ReadBarcodeOptions {
    ReadBarcodeOptions {
        try_harder: true,
        also_inverted: true,
        ..live_options(0)
    }
}

/// Live frame number `frame` is read as it is and once more sharpened, which is what a
/// handheld camera in room light needs, upright and turned for the linear codes.
fn live_options(frame: u32) -> ReadBarcodeOptions {
    ReadBarcodeOptions {
        also_inverted: frame.is_multiple_of(INVERTED_EVERY),
        // The mode that takes the switches below as they are set.
        preprocess: "Aggressive".to_string(),
        preprocessing: BarcodePreprocessingOptions {
            contrast_stretch: false,
            otsu_threshold: false,
            sharpen: true,
            max_variants: 4,
            max_decode_attempts: 4,
            ..BarcodePreprocessingOptions::default()
        },
        ..ReadBarcodeOptions::default()
    }
}

/// The codes in one camera frame. The body is the frame's 8-bit grayscale pixels row by row,
/// `x-frame-width` and `x-frame-height` give its size, `x-frame-thorough: 1` reads it thoroughly.
#[tauri::command]
pub async fn detect_barcodes(request: Request<'_>) -> Result<Vec<Barcode>, String> {
    let InvokeBody::Raw(luma) = request.body() else {
        return Err("Send the frame as raw grayscale bytes".into());
    };
    let (Some(width), Some(height)) = (
        header(&request, "x-frame-width"),
        header(&request, "x-frame-height"),
    ) else {
        return Err("The frame size headers are missing".into());
    };
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(format!(
            "A frame of {width}×{height} pixels is outside 1–{MAX_EDGE} per edge"
        ));
    }
    if luma.len() != width as usize * height as usize {
        return Err(format!(
            "The frame has {} bytes, but {width}×{height} grayscale pixels need {}",
            luma.len(),
            width as usize * height as usize
        ));
    }
    let options = if header(&request, "x-frame-thorough") == Some(1) {
        capture_options()
    } else {
        live_options(LIVE_FRAMES.fetch_add(1, Ordering::Relaxed))
    };
    let luma = luma.clone();
    tokio::task::spawn_blocking(move || read_barcodes_in_luma(luma, width, height, &options))
        .await
        .map_err(|error| format!("The barcode reader stopped: {error}"))?
        .map_err(|error| format!("Reading barcodes failed: {error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_frames_get_four_passes_and_captures_more_rows() {
        let live = live_options(1);
        assert!(!live.try_harder && !live.also_inverted);
        assert!(live.preprocessing.sharpen);
        assert_eq!(live.preprocessing.max_decode_attempts, 4);
        let capture = capture_options();
        assert!(capture.try_harder && capture.also_inverted);
        assert_eq!(capture.preprocessing.max_decode_attempts, 4);
    }

    #[test]
    fn one_live_frame_in_three_looks_for_light_codes_on_dark() {
        let looking: Vec<_> = (0..6)
            .map(|frame| live_options(frame).also_inverted)
            .collect();
        assert_eq!(looking, [true, false, false, true, false, false]);
    }
}

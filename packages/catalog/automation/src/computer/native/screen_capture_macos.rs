//! Display capture through ScreenCaptureKit's `SCScreenshotManager` (macOS 14+). The caller keeps
//! the CGWindowList capture of `xcap` as the fallback for every error returned here.

use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::Duration;

use block2::RcBlock;
use flow_like_types::{Error, Result, anyhow};
use objc2::{AnyThread, MainThreadMarker};
use objc2_core_graphics::{CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

type Captured = Result<image::RgbaImage>;

/// Captures the display `display_id` (a `CGDirectDisplayID`) at `width`x`height` pixels, without
/// the cursor, like `CGWindowListCreateImage` of the display bounds. Blocking.
pub fn capture_display(display_id: u32, width: u32, height: u32) -> Captured {
    if !objc2::available!(macos = 14.0) {
        return Err(anyhow!("SCScreenshotManager needs macOS 14"));
    }
    if MainThreadMarker::new().is_some() {
        return Err(anyhow!(
            "ScreenCaptureKit is not awaited on the main thread, whose queue it may need"
        ));
    }
    let (sender, receiver) = mpsc::sync_channel::<Captured>(1);
    let on_content = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let started = match unsafe { content.as_ref() } {
                Some(content) => screenshot(content, display_id, width, height, sender.clone()),
                None => Err(failure("list the shareable displays", error)),
            };
            if let Err(error) = started {
                let _ = sender.try_send(Err(error));
            }
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            true,
            &on_content,
        );
    }
    match receiver.recv_timeout(TIMEOUT) {
        Ok(captured) => captured,
        Err(RecvTimeoutError::Timeout) => Err(anyhow!(
            "ScreenCaptureKit did not capture display {} within {:?}",
            display_id,
            TIMEOUT
        )),
        Err(RecvTimeoutError::Disconnected) => Err(anyhow!(
            "ScreenCaptureKit dropped the capture of display {} without an answer",
            display_id
        )),
    }
}

fn screenshot(
    content: &SCShareableContent,
    display_id: u32,
    width: u32,
    height: u32,
    sender: SyncSender<Captured>,
) -> Result<()> {
    let displays = unsafe { content.displays() };
    let display = displays
        .iter()
        .find(|display| unsafe { display.displayID() } == display_id)
        .ok_or_else(|| {
            anyhow!(
                "ScreenCaptureKit lists no display {} among {} shareable displays",
                display_id,
                displays.count()
            )
        })?;
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &NSArray::<SCWindow>::new(),
        )
    };
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(width as usize);
        config.setHeight(height as usize);
        config.setShowsCursor(false);
        config.setPixelFormat(PIXEL_FORMAT_BGRA);
    }
    let on_image = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let captured = match unsafe { image.as_ref() } {
            Some(image) => rgba(image),
            None => Err(failure("capture the display", error)),
        };
        let _ = sender.try_send(captured);
    });
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&on_image),
        );
    }
    Ok(())
}

fn failure(operation: &str, error: *mut NSError) -> Error {
    match unsafe { error.as_ref() } {
        Some(error) => anyhow!(
            "ScreenCaptureKit could not {}: {} (code {})",
            operation,
            error.localizedDescription(),
            error.code()
        ),
        None => anyhow!("ScreenCaptureKit could not {} and gave no error", operation),
    }
}

/// Byte offsets of red, green, blue and alpha inside one 32-bit pixel, and whether the alpha byte
/// is padding.
fn channel_offsets(
    alpha: CGImageAlphaInfo,
    order: CGImageByteOrderInfo,
) -> Option<([usize; 4], bool)> {
    let (alpha_first, padding) = match alpha {
        CGImageAlphaInfo::PremultipliedFirst | CGImageAlphaInfo::First => (true, false),
        CGImageAlphaInfo::NoneSkipFirst => (true, true),
        CGImageAlphaInfo::PremultipliedLast | CGImageAlphaInfo::Last => (false, false),
        CGImageAlphaInfo::NoneSkipLast => (false, true),
        _ => return None,
    };
    let big_endian = if alpha_first {
        [1, 2, 3, 0]
    } else {
        [0, 1, 2, 3]
    };
    let offsets = match order {
        CGImageByteOrderInfo::OrderDefault | CGImageByteOrderInfo::Order32Big => big_endian,
        CGImageByteOrderInfo::Order32Little => big_endian.map(|offset| 3 - offset),
        _ => return None,
    };
    Some((offsets, padding))
}

fn rgba(image: &CGImage) -> Captured {
    let image = Some(image);
    let (width, height) = (CGImage::width(image), CGImage::height(image));
    let bytes_per_row = CGImage::bytes_per_row(image);
    let (component, pixel) = (
        CGImage::bits_per_component(image),
        CGImage::bits_per_pixel(image),
    );
    let (alpha_info, order) = (CGImage::alpha_info(image), CGImage::byte_order_info(image));
    let (offsets, padding) = channel_offsets(alpha_info, order)
        .filter(|_| component == 8 && pixel == 32)
        .ok_or_else(|| {
            anyhow!(
                "ScreenCaptureKit returned an unsupported pixel layout: {} bits per component, {} per pixel, alpha {:?}, byte order {:?}",
                component,
                pixel,
                alpha_info,
                order
            )
        })?;
    let data = CGDataProvider::data(CGImage::data_provider(image).as_deref())
        .ok_or_else(|| {
            anyhow!(
                "ScreenCaptureKit image of {}x{} has no pixel data",
                width,
                height
            )
        })?
        .to_vec();
    if width == 0 || height == 0 || bytes_per_row < width * 4 || data.len() < bytes_per_row * height
    {
        return Err(anyhow!(
            "ScreenCaptureKit image of {}x{} has {} bytes in rows of {}",
            width,
            height,
            data.len(),
            bytes_per_row
        ));
    }
    let [red, green, blue, alpha] = offsets;
    let mut buffer = Vec::with_capacity(width * height * 4);
    for row in data.chunks_exact(bytes_per_row).take(height) {
        for pixel in row[..width * 4].chunks_exact(4) {
            buffer.extend_from_slice(&[
                pixel[red],
                pixel[green],
                pixel[blue],
                if padding { u8::MAX } else { pixel[alpha] },
            ]);
        }
    }
    image::RgbaImage::from_raw(width as u32, height as u32, buffer).ok_or_else(|| {
        anyhow!(
            "ScreenCaptureKit image of {}x{} does not fit RGBA",
            width,
            height
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_little_endian_and_rgba_big_endian_map_to_rgba() {
        assert_eq!(
            channel_offsets(
                CGImageAlphaInfo::PremultipliedFirst,
                CGImageByteOrderInfo::Order32Little
            ),
            Some(([2, 1, 0, 3], false))
        );
        assert_eq!(
            channel_offsets(
                CGImageAlphaInfo::NoneSkipLast,
                CGImageByteOrderInfo::OrderDefault
            ),
            Some(([0, 1, 2, 3], true))
        );
        assert_eq!(
            channel_offsets(CGImageAlphaInfo::First, CGImageByteOrderInfo::Order32Big),
            Some(([1, 2, 3, 0], false))
        );
        assert_eq!(
            channel_offsets(CGImageAlphaInfo::Only, CGImageByteOrderInfo::OrderDefault),
            None
        );
        assert_eq!(
            channel_offsets(
                CGImageAlphaInfo::PremultipliedLast,
                CGImageByteOrderInfo::Order16Little
            ),
            None
        );
    }

    fn distinct_colors(image: &image::RgbaImage) -> usize {
        image
            .pixels()
            .step_by(97)
            .map(|pixel| pixel.0)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    #[test]
    #[ignore = "captures the real primary display; run with --ignored --nocapture"]
    fn probe_primary_display_through_screen_capture_kit() {
        use crate::capabilities::{AutomationCapability, capability_status};
        let grant = capability_status(AutomationCapability::ScreenCapture);
        println!(
            "screen recording preflight: {:?} ({})",
            grant.state, grant.detail
        );
        let monitors = xcap::Monitor::all().expect("displays");
        let (_, monitor) =
            crate::types::screen_frame::select_monitor(&monitors, -1).expect("primary display");
        let (width, height) =
            crate::types::screen_frame::monitor_pixel_size(monitor).expect("pixel size");
        let started = std::time::Instant::now();
        let sck = capture_display(monitor.id().expect("display id"), width, height);
        let sck_time = started.elapsed();
        let repeat = std::time::Instant::now();
        let again = capture_display(monitor.id().expect("display id"), width, height).is_ok();
        println!(
            "ScreenCaptureKit second capture ok: {} in {:?}",
            again,
            repeat.elapsed()
        );
        let started = std::time::Instant::now();
        let legacy = monitor.capture_image().expect("CGWindowList capture");
        let legacy_time = started.elapsed();
        println!(
            "CGWindowList: {}x{} in {:?}, {} distinct sampled colors",
            legacy.width(),
            legacy.height(),
            legacy_time,
            distinct_colors(&legacy)
        );
        println!("ScreenCaptureKit first capture: {:?}", sck.as_ref().err());
        let sck = sck.expect("ScreenCaptureKit capture");
        let colors = distinct_colors(&sck);
        println!(
            "ScreenCaptureKit: {}x{} in {:?}, {} distinct sampled colors, non-uniform: {}",
            sck.width(),
            sck.height(),
            sck_time,
            colors,
            colors > 1
        );
        assert_eq!(sck.dimensions(), legacy.dimensions());
        assert!(colors > 1, "ScreenCaptureKit returned a uniform image");
    }
}

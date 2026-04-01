//! Screen capture and template matching that bypasses rustautogui's broken
//! macOS screen capture.
//!
//! **Problem:** rustautogui's `capture_screen()` on macOS ignores CGImage
//! `bytes_per_row` (row stride), so padding bytes corrupt the pixel data,
//! producing garbled screenshots. Since `find_image_on_screen` uses this
//! same broken capture, template matching fails on macOS Retina displays.
//!
//! **Solution:** Capture the screen correctly via `xcap`, convert to
//! grayscale, then call rustautogui's segmented NCC algorithm directly
//! (exposed through the `dev` feature). Mouse/keyboard still use rustautogui.

#[cfg(feature = "execute")]
use image::GrayImage;

/// Capture the primary monitor and return a grayscale image.
///
/// Uses `xcap::Monitor` which handles CGImage row stride correctly,
/// then converts to Luma8 using the standard Rec. 601 formula that
/// [`to_grayscale`] also uses — ensuring screen and template match.
#[cfg(feature = "execute")]
pub fn capture_screen_grayscale() -> Option<GrayImage> {
    let monitors = xcap::Monitor::all().ok()?;
    let monitor = monitors.into_iter().next()?;
    let rgba = monitor.capture_image().ok()?;
    Some(image::DynamicImage::ImageRgba8(rgba).to_luma8())
}

/// Capture the primary monitor and return it as PNG bytes (for debug saving).
#[cfg(feature = "execute")]
pub fn capture_screen_png() -> Option<Vec<u8>> {
    use image::ImageEncoder;

    let monitors = xcap::Monitor::all().ok()?;
    let monitor = monitors.into_iter().next()?;
    let rgba = monitor.capture_image().ok()?;
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(buf)
}

/// Convert template image bytes (PNG/JPEG/etc.) to grayscale using the
/// standard Rec. 601 luminance formula (same as `image`'s `to_luma8()`).
///
/// Now that we capture the screen ourselves (bypassing rustautogui's buggy
/// capture), both screen and template use the same standard formula —
/// no need for per-platform R/B channel swaps.
#[cfg(feature = "execute")]
pub fn to_grayscale(template_bytes: &[u8]) -> Option<GrayImage> {
    let img = image::load_from_memory(template_bytes).ok()?;
    Some(img.to_luma8())
}

/// Get the logical screen dimensions (width, height) from the primary monitor.
#[cfg(feature = "execute")]
pub fn screen_dimensions() -> (u32, u32) {
    xcap::Monitor::all()
        .ok()
        .and_then(|m| m.into_iter().next())
        .and_then(|m| Some((m.width().ok()?, m.height().ok()?)))
        .unwrap_or((0, 0))
}

/// Find a grayscale template in a grayscale screen image using rustautogui's
/// segmented NCC algorithm.
///
/// Returns a list of `(x, y, confidence)` matches above the given precision.
#[cfg(feature = "execute")]
pub fn find_template_in_image(
    screen: &GrayImage,
    template: &GrayImage,
    precision: f32,
) -> Vec<(u32, u32, f32)> {
    use rustautogui::core::template_match::segmented_ncc;
    use rustautogui::data::PreparedData;

    let (tw, th) = template.dimensions();
    let (sw, sh) = screen.dimensions();

    // Template must fit within screen
    if tw > sw || th > sh {
        tracing::warn!(
            "find_template_in_image: template {}x{} larger than screen {}x{}",
            tw,
            th,
            sw,
            sh
        );
        return Vec::new();
    }

    let prepared = segmented_ncc::prepare_template_picture(template, &false, None);
    let data = match prepared {
        PreparedData::Segmented(data) => data,
        _ => return Vec::new(),
    };

    segmented_ncc::fast_ncc_template_match(screen, precision, &data, &false)
}

/// High-level: capture screen, find template, return matches.
///
/// This is the primary entry point for template matching that replaces
/// `gui.prepare_template_from_imagebuffer()` + `gui.find_image_on_screen()`.
#[cfg(feature = "execute")]
pub fn find_template_on_screen(
    template_bytes: &[u8],
    precision: f32,
) -> Option<(Vec<(u32, u32, f32)>, GrayImage, GrayImage)> {
    let gray_template = to_grayscale(template_bytes)?;
    let gray_screen = capture_screen_grayscale()?;

    let (tw, th) = gray_template.dimensions();
    let (sw, sh) = gray_screen.dimensions();
    tracing::debug!(
        "find_template_on_screen: template {}x{}, screen {}x{}, precision {}",
        tw,
        th,
        sw,
        sh,
        precision
    );

    let matches = find_template_in_image(&gray_screen, &gray_template, precision);
    tracing::debug!("find_template_on_screen: {} matches found", matches.len());
    if let Some(&(x, y, c)) = matches.first() {
        tracing::debug!("Best match at ({}, {}) confidence {:.4}", x, y, c);
    }

    Some((matches, gray_template, gray_screen))
}

/// Result of a template matching attempt with full diagnostic info.
#[cfg(feature = "execute")]
pub struct TemplateMatchResult {
    /// Best match position and confidence, if any match was found at all
    pub best_match: Option<(u32, u32, f32)>,
    /// Template dimensions (width, height) after grayscale conversion
    pub template_dims: (u32, u32),
    /// Screen dimensions
    pub screen_dims: (u32, u32),
    /// The grayscale template image (for debug saving)
    pub gray_template: GrayImage,
    /// PNG bytes of the screen capture taken during the match attempt
    pub screen_capture_png: Option<Vec<u8>>,
}

/// Full diagnostic template matching: try at requested confidence, then
/// retry at 0.01 to report the best achievable score for debugging.
#[cfg(feature = "execute")]
pub fn try_template_match(
    template_bytes: &[u8],
    min_confidence: f32,
) -> Option<TemplateMatchResult> {
    use rustautogui::core::template_match::segmented_ncc;
    use rustautogui::data::PreparedData;

    let gray_template = to_grayscale(template_bytes)?;
    let template_dims = gray_template.dimensions();

    let screen_capture_png = capture_screen_png();
    let gray_screen = capture_screen_grayscale()?;
    let screen_dims = gray_screen.dimensions();

    tracing::debug!(
        "try_template_match: template {}x{}, screen {}x{}, confidence threshold {}",
        template_dims.0,
        template_dims.1,
        screen_dims.0,
        screen_dims.1,
        min_confidence
    );

    // Bounds check: template must be smaller than screen
    if template_dims.0 > screen_dims.0 || template_dims.1 > screen_dims.1 {
        tracing::warn!(
            "Template {}x{} is larger than screen {}x{} — cannot search",
            template_dims.0,
            template_dims.1,
            screen_dims.0,
            screen_dims.1
        );
        return Some(TemplateMatchResult {
            best_match: None,
            template_dims,
            screen_dims,
            gray_template,
            screen_capture_png,
        });
    }

    let prepared = segmented_ncc::prepare_template_picture(&gray_template, &false, None);
    let data = match prepared {
        PreparedData::Segmented(data) => {
            tracing::debug!(
                "Template prepared: {} fast segments, {} slow segments, expected_corr_fast={:.4}, expected_corr_slow={:.4}",
                data.template_segments_fast.len(),
                data.template_segments_slow.len(),
                data.expected_corr_fast,
                data.expected_corr_slow
            );
            data
        }
        _ => {
            tracing::warn!("Template preparation failed (not segmented)");
            return Some(TemplateMatchResult {
                best_match: None,
                template_dims,
                screen_dims,
                gray_template,
                screen_capture_png,
            });
        }
    };

    // First try at desired confidence
    let matches =
        segmented_ncc::fast_ncc_template_match(&gray_screen, min_confidence, &data, &false);
    tracing::debug!(
        "NCC at confidence {}: {} matches found",
        min_confidence,
        matches.len()
    );
    if !matches.is_empty() {
        tracing::debug!(
            "Best match at ({}, {}) with confidence {:.4}",
            matches[0].0,
            matches[0].1,
            matches[0].2
        );
        return Some(TemplateMatchResult {
            best_match: Some(matches[0]),
            template_dims,
            screen_dims,
            gray_template,
            screen_capture_png,
        });
    }

    // Re-search at minimum threshold to get the best achievable score
    let fallback = segmented_ncc::fast_ncc_template_match(&gray_screen, 0.01, &data, &false);
    tracing::debug!("NCC fallback at 0.01: {} matches found", fallback.len());
    let best = fallback.into_iter().next();
    if let Some((x, y, c)) = best {
        tracing::debug!("Fallback best at ({}, {}) with confidence {:.4}", x, y, c);
    } else {
        tracing::warn!(
            "No matches found even at 0.01 threshold — template may not be visible on screen"
        );
    }

    Some(TemplateMatchResult {
        best_match: best,
        template_dims,
        screen_dims,
        gray_template,
        screen_capture_png,
    })
}

/// Adjust match coordinates from physical (Retina) to logical screen points.
///
/// xcap captures at physical resolution (e.g. 2880×1800 on a 2× Retina)
/// but mouse coordinates use logical resolution (1440×900). We need to
/// scale the match position down by the display scale factor.
#[cfg(feature = "execute")]
pub fn physical_to_logical(x: u32, y: u32) -> (i32, i32) {
    let (phys_w, phys_h) = xcap::Monitor::all()
        .ok()
        .and_then(|m| m.into_iter().next())
        .and_then(|m| Some((m.width().ok()?, m.height().ok()?)))
        .unwrap_or((1, 1));

    // rustautogui uses get_screen_size() which returns logical dimensions
    let gui = rustautogui::RustAutoGui::new(false).ok();
    let (logical_w, logical_h) = gui
        .map(|mut g| g.get_screen_size())
        .unwrap_or((phys_w as i32, phys_h as i32));

    let scale_x = logical_w as f64 / phys_w as f64;
    let scale_y = logical_h as f64 / phys_h as f64;

    ((x as f64 * scale_x) as i32, (y as f64 * scale_y) as i32)
}

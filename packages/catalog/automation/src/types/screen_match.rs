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
//! (exposed through the `dev` feature). Native input uses a shared Enigo connection.

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
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())?;
    let rgba = capture_monitor(monitor).ok()?;
    Some(image::DynamicImage::ImageRgba8(rgba).to_luma8())
}

/// Capture the primary monitor and return it as PNG bytes (for debug saving).
#[cfg(feature = "execute")]
pub fn capture_screen_png() -> Option<Vec<u8>> {
    use image::ImageEncoder;

    let monitors = xcap::Monitor::all().ok()?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())?;
    let rgba = capture_monitor(monitor).ok()?;
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
        .and_then(|m| {
            m.iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .or_else(|| m.first())
                .cloned()
        })
        .and_then(|m| monitor_input_bounds(&m).ok().map(|(_, _, w, h)| (w, h)))
        .unwrap_or((0, 0))
}

/// Find a grayscale template in a grayscale screen image using rustautogui's
/// segmented NCC algorithm.
///
/// Returns a list of `(x, y, confidence)` matches above the given precision.
#[cfg(all(feature = "execute", not(windows)))]
pub fn find_template_in_image(
    screen: &GrayImage,
    template: &GrayImage,
    precision: f32,
) -> Vec<(u32, u32, f32)> {
    use rustautogui::core::template_match::segmented_ncc;
    use rustautogui::data::PreparedData;

    if !precision.is_finite()
        || !(0.0..=1.0).contains(&precision)
        || template.width() == 0
        || template.height() == 0
    {
        return Vec::new();
    }
    let (tw, th) = template.dimensions();
    let (sw, sh) = screen.dimensions();
    if template
        .as_raw()
        .iter()
        .all(|pixel| *pixel == template.as_raw()[0])
    {
        return Vec::new();
    }

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

    let expected = data.expected_corr_slow;
    if !expected.is_finite() || expected <= 0.0 {
        return Vec::new();
    }
    let candidates = segmented_ncc::fast_ncc_template_match(screen, precision, &data, &false)
        .into_iter()
        .filter_map(|(x, y, score)| {
            let confidence = (score / expected).clamp(0.0, 1.0);
            (confidence.is_finite() && confidence >= precision).then_some((
                x + tw / 2,
                y + th / 2,
                confidence,
            ))
        })
        .collect();
    distinct_matches(candidates, tw, th)
}

#[cfg(all(feature = "execute", windows))]
pub fn find_template_in_image(
    screen: &GrayImage,
    template: &GrayImage,
    precision: f32,
) -> Vec<(u32, u32, f32)> {
    if !precision.is_finite()
        || !(0.0..=1.0).contains(&precision)
        || template.width() == 0
        || template.height() == 0
        || template.width() > screen.width()
        || template.height() > screen.height()
    {
        return vec![];
    }
    use imageproc::template_matching::{MatchTemplateMethod, match_template_parallel};
    let scores = match_template_parallel(
        screen,
        template,
        MatchTemplateMethod::SumOfSquaredErrorsNormalized,
    );
    let matches: Vec<_> = scores
        .enumerate_pixels()
        .filter_map(|(x, y, p)| {
            let score = (1.0 - p[0]).clamp(0.0, 1.0);
            (score.is_finite() && score >= precision).then_some((
                x + template.width() / 2,
                y + template.height() / 2,
                score,
            ))
        })
        .collect();
    distinct_matches(matches, template.width(), template.height())
}

#[cfg(feature = "execute")]
fn distinct_matches(
    mut matches: Vec<(u32, u32, f32)>,
    width: u32,
    height: u32,
) -> Vec<(u32, u32, f32)> {
    matches.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut distinct: Vec<(u32, u32, f32)> = Vec::new();
    for candidate in matches {
        if distinct.iter().all(|m| {
            m.0.abs_diff(candidate.0) > width / 2 || m.1.abs_diff(candidate.1) > height / 2
        }) {
            distinct.push(candidate);
        }
        if distinct.len() >= 1000 {
            break;
        }
    }
    distinct
}

/// Matches found on screen plus the grayscale screen and template they came from.
#[cfg(feature = "execute")]
pub type ScreenTemplateMatch = (Vec<(u32, u32, f32)>, GrayImage, GrayImage);

/// High-level: capture screen, find template, return matches.
///
/// This is the primary entry point for template matching that replaces
/// `gui.prepare_template_from_imagebuffer()` + `gui.find_image_on_screen()`.
#[cfg(feature = "execute")]
pub fn find_template_on_screen(
    template_bytes: &[u8],
    precision: f32,
) -> Option<ScreenTemplateMatch> {
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

/// Match the exact frame included in diagnostic output.
#[cfg(feature = "execute")]
pub fn try_template_match(
    template_bytes: &[u8],
    min_confidence: f32,
) -> Option<TemplateMatchResult> {
    let screen_capture_png = capture_screen_png()?;
    let gray_screen = to_grayscale(&screen_capture_png)?;
    let gray_template = to_grayscale(template_bytes)?;
    let matches = find_template_in_image(&gray_screen, &gray_template, min_confidence);
    Some(TemplateMatchResult {
        best_match: matches.first().copied(),
        template_dims: gray_template.dimensions(),
        screen_dims: gray_screen.dimensions(),
        gray_template,
        screen_capture_png: Some(screen_capture_png),
    })
}

/// Converts screenshot pixels into the desktop coordinate system used by input.
#[cfg(feature = "execute")]
pub fn physical_to_logical(x: u32, y: u32) -> flow_like_types::Result<(i32, i32)> {
    let monitors = xcap::Monitor::all()?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| flow_like_types::anyhow!("No monitor available"))?;
    let scale = monitor.scale_factor()? as f64;
    #[cfg(target_os = "macos")]
    let input_scale = scale;
    #[cfg(target_os = "windows")]
    let input_scale = 1.0;
    #[cfg(target_os = "linux")]
    let input_scale = if crate::computer::native::input::wayland() {
        scale
    } else {
        1.0
    };
    #[cfg(target_os = "linux")]
    let origin_scale = if input_scale == 1.0 { scale } else { 1.0 };
    #[cfg(not(target_os = "linux"))]
    let origin_scale = 1.0;
    map_capture_point(
        x,
        y,
        (
            monitor.x()? as f64 * origin_scale,
            monitor.y()? as f64 * origin_scale,
        ),
        input_scale,
    )
}

pub fn map_capture_point(
    x: u32,
    y: u32,
    origin: (f64, f64),
    scale: f64,
) -> flow_like_types::Result<(i32, i32)> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(flow_like_types::anyhow!("Invalid display scale"));
    }
    let x = origin.0 + x as f64 / scale;
    let y = origin.1 + y as f64 / scale;
    if !x.is_finite()
        || !y.is_finite()
        || x < i32::MIN as f64
        || x > i32::MAX as f64
        || y < i32::MIN as f64
        || y > i32::MAX as f64
    {
        return Err(flow_like_types::anyhow!(
            "Screen coordinate is outside the desktop range"
        ));
    }
    Ok((x.round() as i32, y.round() as i32))
}

#[cfg(test)]
mod geometry_tests {
    use super::map_capture_point;
    #[test]
    fn retina_pixels_map_to_points_and_negative_monitor_origin() {
        assert_eq!(
            map_capture_point(1000, 600, (0.0, 0.0), 2.0).unwrap(),
            (500, 300)
        );
        assert_eq!(
            map_capture_point(1000, 600, (-1440.0, -200.0), 2.0).unwrap(),
            (-940, 100)
        );
        assert!(map_capture_point(1, 1, (0.0, 0.0), 0.0).is_err());
    }
}

#[cfg(feature = "execute")]
pub fn monitor_scale_factor(monitor: &xcap::Monitor) -> flow_like_types::Result<f32> {
    #[cfg(target_os = "linux")]
    if crate::computer::native::input::wayland() {
        let connection = libwayshot_xcap::WayshotConnection::new()?;
        let name = monitor.name()?;
        let output = connection
            .get_all_outputs()
            .iter()
            .find(|output| output.name == name)
            .ok_or_else(|| {
                flow_like_types::anyhow!("No native Wayland display matches {}", name)
            })?;
        let height = output.logical_region.inner.size.height;
        if height == 0 {
            return Err(flow_like_types::anyhow!(
                "Wayland display has no logical height"
            ));
        }
        return Ok(output.physical_size.height as f32 / height as f32);
    }
    Ok(monitor.scale_factor()?)
}

#[cfg(feature = "execute")]
pub fn monitor_input_bounds(m: &xcap::Monitor) -> flow_like_types::Result<(i32, i32, u32, u32)> {
    #[cfg(target_os = "linux")]
    if crate::computer::native::input::wayland() {
        let connection = libwayshot_xcap::WayshotConnection::new()?;
        let name = m.name()?;
        let output = connection
            .get_all_outputs()
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| {
                flow_like_types::anyhow!("No native Wayland display matches {}", name)
            })?;
        let region = output.logical_region.inner;
        return Ok((
            region.position.x,
            region.position.y,
            region.size.width,
            region.size.height,
        ));
    }

    #[cfg(target_os = "linux")]
    let factor = if crate::computer::native::input::wayland() {
        1.0
    } else {
        m.scale_factor()? as f64
    };
    #[cfg(not(target_os = "linux"))]
    let factor = 1.0;
    Ok((
        (m.x()? as f64 * factor).round() as i32,
        (m.y()? as f64 * factor).round() as i32,
        (m.width()? as f64 * factor).round() as u32,
        (m.height()? as f64 * factor).round() as u32,
    ))
}
#[cfg(feature = "execute")]
pub fn capture_pixel(x: i64, y: i64) -> flow_like_types::Result<[u8; 3]> {
    let x = i32::try_from(x)?;
    let y = i32::try_from(y)?;
    for monitor in xcap::Monitor::all()? {
        let (ox, oy, width, height) = monitor_input_bounds(&monitor)?;
        let (dx, dy) = (x as i64 - ox as i64, y as i64 - oy as i64);
        if dx >= 0 && dy >= 0 && dx < width as i64 && dy < height as i64 {
            let image = capture_monitor(&monitor)?;
            let px = (dx as u64 * image.width() as u64 / width as u64) as u32;
            let py = (dy as u64 * image.height() as u64 / height as u64) as u32;
            let p = image.get_pixel(px, py);
            return Ok([p[0], p[1], p[2]]);
        }
    }
    Err(flow_like_types::anyhow!(
        "Desktop coordinate is outside connected displays"
    ))
}
#[cfg(feature = "execute")]
pub async fn load_template(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
) -> flow_like_types::Result<Vec<u8>> {
    if let Ok(template) = context
        .evaluate_pin::<flow_like_catalog_core::FlowPath>("template")
        .await
    {
        return template.get(context, false).await;
    }
    let path: String = context.evaluate_pin("template_path").await?;
    if path.is_empty() {
        return Err(flow_like_types::anyhow!("Template path is required"));
    }
    Ok(tokio::fs::read(path).await?)
}

/// Search selected displays and return centers in native desktop coordinates.
#[cfg(feature = "execute")]
pub fn find_template_on_desktop(
    bytes: &[u8],
    confidence: f64,
    monitor_index: i64,
) -> flow_like_types::Result<Vec<(i32, i32, f32)>> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(flow_like_types::anyhow!(
            "Confidence must be between 0 and 1"
        ));
    }
    let template = to_grayscale(bytes)
        .ok_or_else(|| flow_like_types::anyhow!("Could not decode template image"))?;
    let monitors = xcap::Monitor::all()?;
    if monitors.is_empty() {
        return Err(flow_like_types::anyhow!("No display is available"));
    }
    let selected: Vec<&xcap::Monitor> = match monitor_index {
        -2 => monitors.iter().collect(),
        -1 => vec![
            monitors
                .iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .unwrap_or(&monitors[0]),
        ],
        index => vec![
            monitors
                .get(usize::try_from(index)?)
                .ok_or_else(|| flow_like_types::anyhow!("Monitor index out of range"))?,
        ],
    };
    let mut result = Vec::new();
    for monitor in selected {
        let rgba = capture_monitor(monitor)?;
        let (ox, oy, width, height) = monitor_input_bounds(monitor)?;
        if width == 0 || height == 0 || rgba.width() == 0 || rgba.height() == 0 {
            return Err(flow_like_types::anyhow!("Display has empty capture bounds"));
        }
        let scale_x = rgba.width() as f64 / width as f64;
        let scale_y = rgba.height() as f64 / height as f64;
        let gray = image::DynamicImage::ImageRgba8(rgba).to_luma8();
        for (px, py, score) in find_template_in_image(&gray, &template, confidence as f32) {
            let (x, _) = map_capture_point(px, 0, (ox as f64, 0.0), scale_x)?;
            let (_, y) = map_capture_point(0, py, (0.0, oy as f64), scale_y)?;
            result.push((x, y, score));
        }
    }
    result.sort_by(|a, b| b.2.total_cmp(&a.2));
    Ok(result)
}

#[cfg(all(test, feature = "execute"))]
mod matching_tests {
    use super::*;
    #[test]
    fn template_coordinates_are_centers_of_the_supplied_frame() {
        let template = GrayImage::from_fn(12, 12, |x, y| {
            image::Luma([((x * 41 + y * 17 + x * y * 7) % 251) as u8])
        });
        let mut screen =
            GrayImage::from_fn(48, 32, |x, y| image::Luma([((x * 7 + y * 3) % 101) as u8]));
        image::imageops::replace(&mut screen, &template, 7, 9);
        let matches = find_template_in_image(&screen, &template, 0.95);
        let best = matches.first().expect("exact template in supplied image");
        assert_eq!((best.0, best.1), (13, 15));
        assert!(best.2 >= 0.95);
    }
    #[test]
    fn invalid_template_bounds_or_confidence_do_not_panic() {
        let image = GrayImage::new(8, 8);
        assert!(find_template_in_image(&image, &GrayImage::new(9, 9), 0.8).is_empty());
        assert!(find_template_in_image(&image, &image, f32::NAN).is_empty());
        assert!(find_template_in_image(&image, &GrayImage::new(0, 0), 0.8).is_empty());
    }
}

/// Capture one output without applying a global scale to differently scaled Wayland displays.
#[cfg(feature = "execute")]
pub fn capture_monitor(monitor: &xcap::Monitor) -> flow_like_types::Result<image::RgbaImage> {
    #[cfg(target_os = "linux")]
    if crate::computer::native::input::wayland() {
        let connection = libwayshot_xcap::WayshotConnection::new()?;
        let name = monitor.name()?;
        let outputs = connection.get_all_outputs();
        let output = outputs.iter().find(|o| o.name == name).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Native Wayland display geometry is unavailable for {}",
                name
            )
        })?;
        match connection.screenshot_single_output(output, false) {
            Ok(image) => {
                return image::RgbaImage::from_raw(
                    image.width(),
                    image.height(),
                    image.to_rgba8().into_vec(),
                )
                .ok_or_else(|| flow_like_types::anyhow!("Invalid Wayland capture buffer"));
            }
            Err(error) => {
                let scale = output.physical_size.height as f64
                    / output.logical_region.inner.size.height as f64;
                let uniform = outputs.iter().all(|other| {
                    let region = other.logical_region.inner;
                    region.position.x >= 0
                        && region.position.y >= 0
                        && region.size.height > 0
                        && ((other.physical_size.height as f64 / region.size.height as f64) - scale)
                            .abs()
                            < 0.001
                });
                if !uniform {
                    return Err(flow_like_types::anyhow!(
                        "This Wayland compositor cannot capture mixed-scale or negative-origin displays through the available screenshot backend: {}",
                        error
                    ));
                }
            }
        }
    }
    Ok(monitor.capture_image()?)
}

#[cfg(feature = "execute")]
pub async fn match_desktop_async(
    bytes: Vec<u8>,
    confidence: f64,
    monitor_index: i64,
) -> flow_like_types::Result<Vec<(i32, i32, f32)>> {
    tokio::task::spawn_blocking(move || find_template_on_desktop(&bytes, confidence, monitor_index))
        .await?
}

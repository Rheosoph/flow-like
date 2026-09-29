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
    zncc::find_template(screen, template, precision)
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(feature = "execute")]
pub mod zncc {
    use std::{borrow::Cow, collections::HashSet, ops::RangeInclusive};

    use image::GrayImage;

    const MIN_COARSE_SIDE: usize = 12;
    const MAX_LEVELS: usize = 4;
    const MAX_CANDIDATES: usize = 128;
    const REFINE_RADIUS: usize = 2;
    const CANDIDATE_MARGIN: f32 = 0.45;
    const MIN_CANDIDATE_SCORE: f32 = 0.2;
    const MIN_VARIANCE: f64 = 0.01;

    type Candidate = (usize, usize, f32);

    /// Zero-mean normalized cross-correlation, searched coarse-to-fine.
    ///
    /// Returns `(center_x, center_y, score)` in screen pixels, best first, with
    /// scores in `[0, 1]` at or above `precision`.
    pub fn find_template(
        screen: &GrayImage,
        template: &GrayImage,
        precision: f32,
    ) -> Vec<(u32, u32, f32)> {
        search(screen, template, precision).0
    }

    fn search(
        screen: &GrayImage,
        template: &GrayImage,
        precision: f32,
    ) -> (Vec<(u32, u32, f32)>, usize) {
        if !searchable(screen, template, precision) {
            return (Vec::new(), 0);
        }
        let (tw, th) = template.dimensions();
        let (candidates, evaluations) = descend(&pyramid(screen, template), precision);
        let matches = candidates
            .into_iter()
            .map(|(x, y, score)| (x as u32 + tw / 2, y as u32 + th / 2, score))
            .collect();
        (super::distinct_matches(matches, tw, th), evaluations)
    }

    fn searchable(screen: &GrayImage, template: &GrayImage, precision: f32) -> bool {
        let (tw, th) = template.dimensions();
        let (sw, sh) = screen.dimensions();
        precision.is_finite()
            && (0.0..=1.0).contains(&precision)
            && tw > 0
            && th > 0
            && tw <= sw
            && th <= sh
    }

    fn descend(levels: &[Level], precision: f32) -> (Vec<Candidate>, usize) {
        let Some((coarse, finer)) = levels.split_last() else {
            return (Vec::new(), 0);
        };
        let floor = (precision - CANDIDATE_MARGIN)
            .max(MIN_CANDIDATE_SCORE)
            .min(precision);
        let mut candidates = coarse.local_maxima(floor);
        if finer.is_empty() {
            let (scored, evaluations) = coarse.refine(&candidates, 1, 0, precision);
            return (scored, coarse.position_count() + evaluations);
        }
        let mut evaluations = 0;
        for (index, level) in finer.iter().enumerate().rev() {
            let threshold = if index == 0 { precision } else { floor };
            (candidates, evaluations) = level.refine(&candidates, 2, REFINE_RADIUS, threshold);
        }
        (candidates, evaluations)
    }

    fn pyramid<'a>(screen: &'a GrayImage, template: &'a GrayImage) -> Vec<Level<'a>> {
        let mut templates = vec![Plane::from_image(template)];
        while templates.len() < MAX_LEVELS {
            let last = &templates[templates.len() - 1];
            if last.width.min(last.height) / 2 < MIN_COARSE_SIDE {
                break;
            }
            let next = last.halve();
            templates.push(next);
        }
        let mut levels: Vec<Level<'a>> = Vec::with_capacity(templates.len());
        for kernel in templates.iter().map_while(Kernel::new) {
            let screen = match levels.last() {
                Some(finer) => finer.screen.halve(),
                None => Plane::from_image(screen),
            };
            levels.push(Level { screen, kernel });
        }
        levels
    }

    struct Plane<'a> {
        width: usize,
        height: usize,
        pixels: Cow<'a, [u8]>,
    }

    impl<'a> Plane<'a> {
        fn from_image(image: &'a GrayImage) -> Self {
            let (width, height) = (image.width() as usize, image.height() as usize);
            Plane {
                width,
                height,
                pixels: Cow::Borrowed(&image.as_raw()[..width * height]),
            }
        }

        fn row(&self, y: usize) -> &[u8] {
            &self.pixels[y * self.width..(y + 1) * self.width]
        }

        fn halve(&self) -> Plane<'static> {
            let (width, height) = (self.width / 2, self.height / 2);
            let mut pixels = Vec::with_capacity(width * height);
            for y in 0..height {
                let upper = self.row(2 * y).chunks_exact(2);
                let lower = self.row(2 * y + 1).chunks_exact(2);
                pixels.extend(upper.zip(lower).map(|(top, bottom)| {
                    let total: u16 = top.iter().chain(bottom).map(|&p| u16::from(p)).sum();
                    ((total + 2) / 4) as u8
                }));
            }
            Plane {
                width,
                height,
                pixels: Cow::Owned(pixels),
            }
        }
    }

    struct Kernel {
        width: usize,
        height: usize,
        weights: Vec<f32>,
        norm: f64,
    }

    impl Kernel {
        fn new(plane: &Plane<'_>) -> Option<Self> {
            let (sum, squares) = moments(&plane.pixels);
            let spread = spread(plane.pixels.len(), sum, squares)?;
            let mean = sum as f64 / plane.pixels.len() as f64;
            Some(Kernel {
                width: plane.width,
                height: plane.height,
                weights: plane
                    .pixels
                    .iter()
                    .map(|&p| (f64::from(p) - mean) as f32)
                    .collect(),
                norm: spread.sqrt(),
            })
        }

        fn rows(&self) -> impl Iterator<Item = &[f32]> {
            self.weights.chunks_exact(self.width)
        }

        fn score(&self, dot: f64, sum: u64, squares: u64) -> f32 {
            let Some(spread) = spread(self.weights.len(), sum, squares) else {
                return 0.0;
            };
            let score = dot / (spread.sqrt() * self.norm);
            if score.is_finite() {
                score.clamp(0.0, 1.0) as f32
            } else {
                0.0
            }
        }
    }

    struct Level<'a> {
        screen: Plane<'a>,
        kernel: Kernel,
    }

    impl Level<'_> {
        fn positions(&self) -> (usize, usize) {
            (
                self.screen.width - self.kernel.width + 1,
                self.screen.height - self.kernel.height + 1,
            )
        }

        fn position_count(&self) -> usize {
            let (across, down) = self.positions();
            across * down
        }

        fn score_at(&self, x: usize, y: usize) -> f32 {
            let (mut dot, mut sum, mut squares) = (0.0, 0, 0);
            for (offset, weights) in self.kernel.rows().enumerate() {
                let pixels = &self.screen.row(y + offset)[x..x + self.kernel.width];
                let (row_sum, row_squares) = moments(pixels);
                dot += f64::from(weighted_sum(weights, pixels));
                sum += row_sum;
                squares += row_squares;
            }
            self.kernel.score(dot, sum, squares)
        }

        fn score_map(&self) -> Vec<f32> {
            let (across, down) = self.positions();
            let mut columns = ColumnSums::new(self.screen.width);
            for y in 0..self.kernel.height {
                columns.add(self.screen.row(y));
            }
            let mut dots = vec![0.0f32; across];
            let mut scores = Vec::with_capacity(across * down);
            for y in 0..down {
                if y > 0 {
                    columns.slide(
                        self.screen.row(y - 1),
                        self.screen.row(y + self.kernel.height - 1),
                    );
                }
                columns.integrate();
                dots.fill(0.0);
                for (offset, weights) in self.kernel.rows().enumerate() {
                    let row = self.screen.row(y + offset);
                    for (shift, &weight) in weights.iter().enumerate() {
                        for (dot, &pixel) in dots.iter_mut().zip(&row[shift..shift + across]) {
                            *dot += weight * f32::from(pixel);
                        }
                    }
                }
                scores.extend(dots.iter().enumerate().map(|(x, &dot)| {
                    let (sum, squares) = columns.window(x, self.kernel.width);
                    self.kernel.score(f64::from(dot), sum, squares)
                }));
            }
            scores
        }

        fn local_maxima(&self, floor: f32) -> Vec<Candidate> {
            let (across, down) = self.positions();
            let scores = self.score_map();
            let at = |x: usize, y: usize| scores[y * across + x];
            let mut maxima: Vec<Candidate> = (0..down)
                .flat_map(|y| (0..across).map(move |x| (x, y)))
                .filter_map(|(x, y)| {
                    let score = at(x, y);
                    let peak = score > 0.0
                        && score >= floor
                        && span(y, 1, down)
                            .all(|ny| span(x, 1, across).all(|nx| at(nx, ny) <= score));
                    peak.then_some((x, y, score))
                })
                .collect();
            rank(&mut maxima);
            maxima.truncate(MAX_CANDIDATES);
            maxima
        }

        fn refine(
            &self,
            candidates: &[Candidate],
            scale: usize,
            radius: usize,
            threshold: f32,
        ) -> (Vec<Candidate>, usize) {
            let (across, down) = self.positions();
            let mut evaluations = 0;
            let mut refined: Vec<Candidate> = Vec::with_capacity(candidates.len());
            for &(x, y, _) in candidates {
                let mut best: Option<Candidate> = None;
                for ny in span(y * scale, radius, down) {
                    for nx in span(x * scale, radius, across) {
                        evaluations += 1;
                        let score = self.score_at(nx, ny);
                        if best.is_none_or(|(_, _, top)| score > top) {
                            best = Some((nx, ny, score));
                        }
                    }
                }
                refined.extend(best.filter(|&(_, _, score)| score >= threshold));
            }
            rank(&mut refined);
            let mut seen = HashSet::with_capacity(refined.len());
            refined.retain(|&(x, y, _)| seen.insert((x, y)));
            (refined, evaluations)
        }
    }

    struct ColumnSums {
        sums: Vec<u64>,
        squares: Vec<u64>,
        prefix_sums: Vec<u64>,
        prefix_squares: Vec<u64>,
    }

    impl ColumnSums {
        fn new(width: usize) -> Self {
            ColumnSums {
                sums: vec![0; width],
                squares: vec![0; width],
                prefix_sums: vec![0; width + 1],
                prefix_squares: vec![0; width + 1],
            }
        }

        fn add(&mut self, row: &[u8]) {
            for ((sum, square), &pixel) in self.sums.iter_mut().zip(&mut self.squares).zip(row) {
                let pixel = u64::from(pixel);
                *sum += pixel;
                *square += pixel * pixel;
            }
        }

        fn slide(&mut self, leaving: &[u8], entering: &[u8]) {
            let rows = leaving.iter().zip(entering);
            for ((sum, square), (&old, &new)) in
                self.sums.iter_mut().zip(&mut self.squares).zip(rows)
            {
                let (old, new) = (u64::from(old), u64::from(new));
                *sum = *sum + new - old;
                *square = *square + new * new - old * old;
            }
        }

        fn integrate(&mut self) {
            accumulate(&mut self.prefix_sums, &self.sums);
            accumulate(&mut self.prefix_squares, &self.squares);
        }

        fn window(&self, x: usize, width: usize) -> (u64, u64) {
            (
                self.prefix_sums[x + width] - self.prefix_sums[x],
                self.prefix_squares[x + width] - self.prefix_squares[x],
            )
        }
    }

    fn accumulate(prefix: &mut [u64], values: &[u64]) {
        let mut running = 0;
        for (slot, &value) in prefix[1..].iter_mut().zip(values) {
            running += value;
            *slot = running;
        }
    }

    fn moments(pixels: &[u8]) -> (u64, u64) {
        pixels.iter().fold((0, 0), |(sum, squares), &pixel| {
            let pixel = u64::from(pixel);
            (sum + pixel, squares + pixel * pixel)
        })
    }

    fn weighted_sum(weights: &[f32], pixels: &[u8]) -> f32 {
        let (weight_chunks, pixel_chunks) = (weights.chunks_exact(8), pixels.chunks_exact(8));
        let tail: f32 = weight_chunks
            .remainder()
            .iter()
            .zip(pixel_chunks.remainder())
            .map(|(&weight, &pixel)| weight * f32::from(pixel))
            .sum();
        let mut lanes = [0.0f32; 8];
        for (weight_chunk, pixel_chunk) in weight_chunks.zip(pixel_chunks) {
            for ((lane, &weight), &pixel) in lanes.iter_mut().zip(weight_chunk).zip(pixel_chunk) {
                *lane += weight * f32::from(pixel);
            }
        }
        lanes.iter().sum::<f32>() + tail
    }

    fn spread(count: usize, sum: u64, squares: u64) -> Option<f64> {
        let scaled = (count as u128 * u128::from(squares)).saturating_sub(u128::from(sum).pow(2));
        let spread = scaled as f64 / count as f64;
        (spread > count as f64 * MIN_VARIANCE).then_some(spread)
    }

    fn rank(candidates: &mut [Candidate]) {
        candidates.sort_by(|a, b| {
            b.2.total_cmp(&a.2)
                .then_with(|| (a.1, a.0).cmp(&(b.1, b.0)))
        });
    }

    fn span(center: usize, radius: usize, limit: usize) -> RangeInclusive<usize> {
        let last = limit - 1;
        center.saturating_sub(radius).min(last)..=(center + radius).min(last)
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, Instant};

        use image::{GrayImage, Luma, imageops::replace};

        use super::{find_template, search};

        fn hash(x: u32, y: u32, seed: u32) -> u32 {
            let mut h = x.wrapping_mul(0x9E37_79B1)
                ^ y.wrapping_mul(0x85EB_CA77)
                ^ seed.wrapping_mul(0xC2B2_AE3D);
            h ^= h >> 15;
            h = h.wrapping_mul(0x2C1B_3C6D);
            h ^= h >> 12;
            h = h.wrapping_mul(0x297A_2D39);
            h ^ (h >> 15)
        }

        fn noise(x: u32, y: u32, cell: u32, seed: u32) -> f32 {
            let (cx, cy) = (x / cell, y / cell);
            let fx = (x % cell) as f32 / cell as f32;
            let fy = (y % cell) as f32 / cell as f32;
            let corner = |dx: u32, dy: u32| (hash(cx + dx, cy + dy, seed) & 0xFF) as f32;
            let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * fx;
            let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * fx;
            top + (bottom - top) * fy
        }

        fn texture(width: u32, height: u32, seed: u32) -> GrayImage {
            GrayImage::from_fn(width, height, |x, y| {
                let value = 0.6 * noise(x, y, 11, seed) + 0.4 * noise(x, y, 3, seed ^ 0xA5A5);
                Luma([value.round() as u8])
            })
        }

        fn widget() -> GrayImage {
            GrayImage::from_fn(64, 64, |x, y| {
                let border = x < 2 || y < 2 || x > 61 || y > 61;
                let text = (10..54).contains(&x)
                    && (24..40).contains(&y)
                    && hash(x / 2, y / 3, 77) % 3 == 0;
                let fill = (8..56).contains(&x) && (12..52).contains(&y);
                Luma([match (border, text, fill) {
                    (true, _, _) => 40,
                    (_, true, _) => 20,
                    (_, _, true) => 200,
                    _ => 245,
                }])
            })
        }

        fn best(screen: &GrayImage, template: &GrayImage, precision: f32) -> (u32, u32, f32) {
            find_template(screen, template, precision)
                .first()
                .copied()
                .expect("template should be found")
        }

        #[test]
        fn exact_matches_report_exact_centers_at_odd_offsets() {
            let template = texture(48, 40, 7);
            for (x, y) in [(0, 0), (1, 1), (37, 23), (101, 5), (152, 110), (3, 109)] {
                let mut screen = texture(200, 150, 1);
                replace(&mut screen, &template, x, y);
                let (cx, cy, score) = best(&screen, &template, 0.9);
                assert_eq!(
                    (cx, cy),
                    (x as u32 + 24, y as u32 + 20),
                    "offset ({x}, {y})"
                );
                assert!(score >= 0.999, "offset ({x}, {y}) scored {score}");
            }
        }

        #[test]
        fn every_coarse_phase_is_recovered() {
            let background = texture(160, 128, 3);
            for template in [widget(), texture(64, 64, 11)] {
                for (dx, dy) in (0..8).flat_map(|dy| (0..8).map(move |dx| (dx, dy))) {
                    let mut screen = background.clone();
                    replace(&mut screen, &template, 40 + dx, 30 + dy);
                    let (cx, cy, score) = best(&screen, &template, 0.95);
                    assert_eq!(
                        (cx, cy),
                        (72 + dx as u32, 62 + dy as u32),
                        "phase ({dx}, {dy})"
                    );
                    assert!(score >= 0.999, "phase ({dx}, {dy}) scored {score}");
                }
            }
        }

        #[test]
        fn flat_backgrounds_and_flat_templates_never_match() {
            let checker = GrayImage::from_fn(32, 32, |x, y| {
                Luma([if (x / 4 + y / 4) % 2 == 0 { 30 } else { 220 }])
            });
            let screens = [
                GrayImage::from_pixel(200, 120, Luma([128])),
                GrayImage::from_fn(200, 120, |x, y| Luma([128 + (hash(x, y, 5) & 1) as u8])),
                GrayImage::from_fn(200, 120, |x, y| Luma([((x + y) / 2) as u8])),
            ];
            for screen in &screens {
                assert!(find_template(screen, &checker, 0.8).is_empty());
                assert!(
                    find_template(screen, &checker, 0.0)
                        .iter()
                        .all(|m| m.2.is_finite() && m.2 < 0.8)
                );
            }
            let flat = GrayImage::from_pixel(16, 16, Luma([90]));
            assert!(find_template(&texture(64, 64, 1), &flat, 0.0).is_empty());
        }

        #[test]
        fn affine_intensity_changes_match_but_inversion_does_not() {
            let template = texture(40, 40, 9);
            let dimmed = GrayImage::from_fn(40, 40, |x, y| {
                let value = 0.5 * f32::from(template.get_pixel(x, y)[0]) + 60.0;
                Luma([value.round().min(255.0) as u8])
            });
            let inverted =
                GrayImage::from_fn(40, 40, |x, y| Luma([255 - template.get_pixel(x, y)[0]]));
            let mut screen = texture(180, 140, 2);
            replace(&mut screen, &dimmed, 31, 17);
            let (cx, cy, score) = best(&screen, &template, 0.9);
            assert_eq!((cx, cy), (51, 37));
            assert!(score >= 0.98, "dimmed match scored {score}");
            let mut screen = texture(180, 140, 2);
            replace(&mut screen, &inverted, 31, 17);
            assert!(find_template(&screen, &template, 0.8).is_empty());
        }

        #[test]
        fn multiple_instances_are_reported_separately() {
            let template = widget();
            let mut screen = texture(400, 300, 4);
            let spots = [(10, 12), (171, 40), (300, 201), (97, 203)];
            for (x, y) in spots {
                replace(&mut screen, &template, x, y);
            }
            let mut found: Vec<(u32, u32)> = find_template(&screen, &template, 0.95)
                .iter()
                .map(|m| (m.0, m.1))
                .collect();
            let mut expected: Vec<(u32, u32)> = spots
                .iter()
                .map(|&(x, y)| (x as u32 + 32, y as u32 + 32))
                .collect();
            found.sort_unstable();
            expected.sort_unstable();
            assert_eq!(found, expected);
        }

        #[test]
        fn small_templates_are_searched_exhaustively() {
            let template = GrayImage::from_fn(12, 12, |x, y| {
                Luma([((x * 41 + y * 17 + x * y * 7) % 251) as u8])
            });
            let mut screen =
                GrayImage::from_fn(48, 32, |x, y| Luma([((x * 7 + y * 3) % 101) as u8]));
            replace(&mut screen, &template, 7, 9);
            let (matches, evaluations) = search(&screen, &template, 0.95);
            let (cx, cy, score) = matches[0];
            assert_eq!((cx, cy), (13, 15));
            assert!(score >= 0.999);
            assert!(evaluations >= 37 * 21);
        }

        #[test]
        fn invalid_inputs_return_nothing() {
            let screen = texture(48, 32, 6);
            let template = texture(12, 12, 8);
            for precision in [f32::NAN, f32::INFINITY, -0.1, 1.5] {
                assert!(find_template(&screen, &template, precision).is_empty());
            }
            assert!(find_template(&screen, &GrayImage::new(0, 0), 0.8).is_empty());
            assert!(find_template(&screen, &texture(49, 8, 8), 0.8).is_empty());
            assert!(find_template(&screen, &texture(8, 33, 8), 0.8).is_empty());
        }

        #[test]
        fn full_hd_search_touches_under_one_percent_of_positions() {
            let template = texture(64, 64, 21);
            let mut screen = texture(1920, 1080, 22);
            replace(&mut screen, &template, 1234, 567);
            let started = Instant::now();
            let (matches, evaluations) = search(&screen, &template, 0.9);
            let elapsed = started.elapsed();
            let exhaustive = (1920 - 64 + 1) * (1080 - 64 + 1);
            eprintln!(
                "zncc 1920x1080 / 64x64: {elapsed:?}, {evaluations} of {exhaustive} positions"
            );
            let (cx, cy, score) = matches.first().copied().expect("template should be found");
            assert_eq!((cx, cy), (1266, 599));
            assert!(score >= 0.999, "scored {score}");
            assert!(
                evaluations * 100 < exhaustive,
                "{evaluations} of {exhaustive}"
            );
            assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
        }
    }
}

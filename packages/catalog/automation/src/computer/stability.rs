use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};

#[cfg(any(feature = "execute", test))]
const THUMBNAIL_EDGE: u32 = 320;
#[cfg(any(feature = "execute", test))]
const PIXEL_TOLERANCE: u8 = 10;

/// Grayscale copy at most `max_edge` pixels on its long side; the averaging filter suppresses
/// sub-pixel noise such as antialiasing and dithering.
#[cfg(any(feature = "execute", test))]
pub(crate) fn diff_thumbnail(image: &image::RgbaImage, max_edge: u32) -> image::GrayImage {
    let gray = image::imageops::grayscale(image);
    let (width, height) = gray.dimensions();
    let long_edge = width.max(height).max(1);
    if long_edge <= max_edge {
        return gray;
    }
    let scale = max_edge as f64 / long_edge as f64;
    image::imageops::resize(
        &gray,
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
        image::imageops::FilterType::Triangle,
    )
}

/// Fraction (0–1) of pixels whose brightness differs by more than `tolerance`.
#[cfg(any(feature = "execute", test))]
pub(crate) fn changed_fraction(
    before: &image::GrayImage,
    after: &image::GrayImage,
    tolerance: u8,
) -> flow_like_types::Result<f64> {
    if before.dimensions() != after.dimensions() {
        return Err(flow_like_types::anyhow!(
            "Cannot compare a {}x{} frame with a {}x{} frame",
            before.width(),
            before.height(),
            after.width(),
            after.height()
        ));
    }
    let total = before.as_raw().len();
    if total == 0 {
        return Ok(0.0);
    }
    let changed = before
        .as_raw()
        .iter()
        .zip(after.as_raw())
        .filter(|(a, b)| a.abs_diff(**b) > tolerance)
        .count();
    Ok(changed as f64 / total as f64)
}

/// Tracks how long consecutive frames have stayed within `threshold` of each other.
#[cfg(any(feature = "execute", test))]
pub(crate) struct StabilityTracker {
    threshold: f64,
    stable_for: std::time::Duration,
    last_change: std::time::Duration,
}

#[cfg(any(feature = "execute", test))]
impl StabilityTracker {
    pub(crate) fn new(threshold: f64, stable_for: std::time::Duration) -> Self {
        Self {
            threshold,
            stable_for,
            last_change: std::time::Duration::ZERO,
        }
    }

    /// Records the change against the previous frame at `elapsed` since the wait started and
    /// reports whether nothing has changed for the required duration.
    pub(crate) fn observe(&mut self, change: f64, elapsed: std::time::Duration) -> bool {
        if change >= self.threshold {
            self.last_change = elapsed;
        }
        elapsed.saturating_sub(self.last_change) >= self.stable_for
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug)]
enum WatchTarget {
    Display(i64),
    Region(i32, i32, u32, u32),
}

#[cfg(feature = "execute")]
impl WatchTarget {
    async fn from_pins(context: &ExecutionContext) -> flow_like_types::Result<Self> {
        let display: i64 = context.evaluate_pin("display").await?;
        let x: i64 = context.evaluate_pin("region_x").await?;
        let y: i64 = context.evaluate_pin("region_y").await?;
        let width: i64 = context.evaluate_pin("region_width").await?;
        let height: i64 = context.evaluate_pin("region_height").await?;
        if width == 0 && height == 0 {
            return Ok(Self::Display(display));
        }
        if width <= 0 || height <= 0 {
            return Err(flow_like_types::anyhow!(
                "Region size {}x{} is invalid; use a positive width and height, or 0x0 for the whole display",
                width,
                height
            ));
        }
        Ok(Self::Region(
            i32::try_from(x)?,
            i32::try_from(y)?,
            u32::try_from(width)?,
            u32::try_from(height)?,
        ))
    }

    async fn capture(self) -> flow_like_types::Result<Capture> {
        tokio::task::spawn_blocking(move || {
            let (image, frame) = match self {
                Self::Display(index) => crate::types::screen_frame::capture_display(index)?,
                Self::Region(x, y, width, height) => {
                    crate::types::screen_frame::capture_input_region(None, x, y, width, height)?
                }
            };
            let thumbnail = diff_thumbnail(&image, THUMBNAIL_EDGE);
            flow_like_types::Ok(Capture {
                image,
                frame,
                thumbnail,
            })
        })
        .await?
    }
}

#[cfg(feature = "execute")]
struct Capture {
    image: image::RgbaImage,
    frame: ScreenFrame,
    thumbnail: image::GrayImage,
}

/// Returns whether the target became stable, the last frame-to-frame change and the last frame.
#[cfg(feature = "execute")]
async fn wait_until_stable(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    target: WatchTarget,
    tracker: &mut StabilityTracker,
    poll: std::time::Duration,
    timeout: std::time::Duration,
) -> flow_like_types::Result<(bool, f64, Capture)> {
    let started = std::time::Instant::now();
    let mut previous = target.capture().await?;
    let mut last_change = 0.0;
    while started.elapsed() < timeout {
        crate::rpa::branch::delay(context, poll).await?;
        session.ensure_active(context).await?;
        let current = target.capture().await?;
        last_change = changed_fraction(&previous.thumbnail, &current.thumbnail, PIXEL_TOLERANCE)
            .map_err(|e| flow_like_types::anyhow!("The watched area changed size: {}", e))?;
        previous = current;
        if tracker.observe(last_change, started.elapsed()) {
            return Ok((true, last_change, previous));
        }
    }
    Ok((false, last_change, previous))
}

/// The caller's Baseline image when connected, otherwise a capture taken now.
#[cfg(feature = "execute")]
async fn load_baseline(
    context: &mut ExecutionContext,
    target: WatchTarget,
) -> flow_like_types::Result<(NodeImage, image::GrayImage)> {
    if let Ok(before) = context.evaluate_pin::<NodeImage>("baseline").await {
        let image = before.get_image(context).await?.lock().await.to_rgba8();
        let thumbnail =
            tokio::task::spawn_blocking(move || diff_thumbnail(&image, THUMBNAIL_EDGE)).await?;
        return Ok((before, thumbnail));
    }
    let capture = target.capture().await?;
    let image = NodeImage::new(
        context,
        flow_like_types::image::DynamicImage::ImageRgba8(capture.image),
    )
    .await;
    Ok((image, capture.thumbnail))
}

fn add_watch_pins(node: &mut Node) {
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_input_pin(
        "session",
        "Session",
        "Computer session handle",
        VariableType::Struct,
    )
    .set_schema::<AutomationSession>();
    node.add_input_pin(
        "display",
        "Display",
        "Display to watch when no region is set: index from List Displays, -1 = primary",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(-1)));
    for (name, label, description) in [
        (
            "region_x",
            "Region X",
            "Left edge of the watched region in desktop input coordinates",
        ),
        (
            "region_y",
            "Region Y",
            "Top edge of the watched region in desktop input coordinates",
        ),
        (
            "region_width",
            "Region Width",
            "Region width in desktop input coordinates; 0 with height 0 watches the whole display. The region must lie on one display",
        ),
        (
            "region_height",
            "Region Height",
            "Region height in desktop input coordinates; 0 with width 0 watches the whole display",
        ),
    ] {
        node.add_input_pin(name, label, description, VariableType::Integer)
            .set_default_value(Some(json!(0)));
    }
}

fn add_timing_pin(node: &mut Node, name: &str, label: &str, description: &str, default: i64) {
    node.add_input_pin(name, label, description, VariableType::Integer)
        .set_default_value(Some(json!(default)));
}

fn add_threshold_pin(node: &mut Node, description: &str, default: f64) {
    node.add_input_pin("threshold", "Threshold", description, VariableType::Float)
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(default)));
}

fn watch_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(3)
        .set_security(5)
        .set_performance(5)
        .set_governance(5)
        .set_reliability(8)
        .set_cost(9)
        .build()
}

#[cfg(feature = "execute")]
async fn timing(
    context: &ExecutionContext,
    name: &str,
    range: std::ops::RangeInclusive<i64>,
) -> flow_like_types::Result<std::time::Duration> {
    let value: i64 = context.evaluate_pin(name).await?;
    if !range.contains(&value) {
        return Err(flow_like_types::anyhow!(
            "{} must be between {} and {} ms, got {}",
            name,
            range.start(),
            range.end(),
            value
        ));
    }
    Ok(std::time::Duration::from_millis(value as u64))
}

#[cfg(feature = "execute")]
async fn threshold(context: &ExecutionContext) -> flow_like_types::Result<f64> {
    let value: f64 = context.evaluate_pin("threshold").await?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(flow_like_types::anyhow!(
            "Threshold must be a fraction between 0 and 1, got {}",
            value
        ));
    }
    Ok(value)
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerWaitScreenStableNode {}

impl ComputerWaitScreenStableNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerWaitScreenStableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_wait_screen_stable",
            "Wait For Stable Screen",
            "Captures a display or desktop region repeatedly until consecutive frames stop changing for Stable (ms), e.g. after a page load or animation. Frames are compared on a downsampled grayscale copy",
            "Automation/Computer/Wait",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "waitScreenStable");
        node.add_icon("/flow/icons/computer.svg");
        node.set_scores(watch_scores());
        node.set_only_offline(true);

        add_watch_pins(&mut node);
        add_threshold_pin(
            &mut node,
            "Largest fraction of pixels (0-1) that may change between two frames while still counting as stable",
            0.002,
        );
        add_timing_pin(
            &mut node,
            "stable_ms",
            "Stable (ms)",
            "How long the screen must stay unchanged (0-600000 ms)",
            500,
        );
        add_timing_pin(
            &mut node,
            "poll_ms",
            "Poll (ms)",
            "Pause between captures (20-60000 ms)",
            100,
        );
        add_timing_pin(
            &mut node,
            "timeout_ms",
            "Timeout (ms)",
            "Maximum wait before taking the Timeout branch (0-3600000 ms)",
            10000,
        );

        node.add_output_pin(
            "exec_out",
            "▶",
            "The screen is stable",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "The screen kept changing until the timeout",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_output_pin(
            "image",
            "Image",
            "Last captured frame",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "frame",
            "Frame",
            "Desktop rectangle and pixel size of the image",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();
        node.add_output_pin(
            "stable",
            "Stable",
            "Whether the screen became stable before the timeout",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "last_change",
            "Last Change",
            "Fraction of pixels that changed between the last two frames",
            VariableType::Float,
        );
        node.add_output_pin(
            "waited_ms",
            "Waited (ms)",
            "Time spent waiting",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let target = WatchTarget::from_pins(context).await?;
        let threshold = threshold(context).await?;
        let stable_for = timing(context, "stable_ms", 0..=600_000).await?;
        let poll = timing(context, "poll_ms", 20..=60_000).await?;
        let timeout = timing(context, "timeout_ms", 0..=3_600_000).await?;

        let started = std::time::Instant::now();
        let mut tracker = StabilityTracker::new(threshold, stable_for);
        let (stable, last_change, last) =
            wait_until_stable(context, &session, target, &mut tracker, poll, timeout).await?;

        let waited_ms = started.elapsed().as_millis() as i64;
        let frame = last.frame.clone();
        let image = NodeImage::new(
            context,
            flow_like_types::image::DynamicImage::ImageRgba8(last.image),
        )
        .await;
        context.set_pin_value("image", json!(image)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        context.set_pin_value("stable", json!(stable)).await?;
        context.set_pin_value("last_change", json!(last_change)).await?;
        context.set_pin_value("waited_ms", json!(waited_ms)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context
            .activate_exec_pin(if stable { "exec_out" } else { "exec_timeout" })
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerWaitScreenChangeNode {}

impl ComputerWaitScreenChangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerWaitScreenChangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_wait_screen_change",
            "Wait For Screen Change",
            "Verifies that an action had a visible effect: compares a display or desktop region against a baseline until at least Threshold of it has changed, or times out. Connect Baseline to a screenshot of the same area taken before the action to also catch instant changes; otherwise the baseline is captured when this node starts",
            "Automation/Computer/Wait",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "waitScreenChange");
        node.add_icon("/flow/icons/computer.svg");
        node.set_scores(watch_scores());
        node.set_only_offline(true);

        add_watch_pins(&mut node);
        node.add_input_pin(
            "baseline",
            "Baseline",
            "Optional image of the same area captured before the action",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_options(PinOptions::new().set_optional(true).build());
        add_threshold_pin(
            &mut node,
            "Fraction of pixels (0-1) that must differ from the baseline to count as changed",
            0.01,
        );
        add_timing_pin(
            &mut node,
            "poll_ms",
            "Poll (ms)",
            "Pause between captures (20-60000 ms)",
            100,
        );
        add_timing_pin(
            &mut node,
            "timeout_ms",
            "Timeout (ms)",
            "Maximum wait before taking the Timeout branch (0-3600000 ms)",
            5000,
        );

        node.add_output_pin(
            "exec_out",
            "▶",
            "The area changed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "No change of at least Threshold before the timeout",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_output_pin(
            "changed",
            "Changed",
            "Whether the area changed before the timeout",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "change_ratio",
            "Change Ratio",
            "Fraction of pixels that differ from the baseline in the last frame",
            VariableType::Float,
        );
        node.add_output_pin(
            "before",
            "Before",
            "Baseline image",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "after",
            "After",
            "Last captured image",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "frame",
            "Frame",
            "Desktop rectangle and pixel size of the After image",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let target = WatchTarget::from_pins(context).await?;
        let threshold = threshold(context).await?;
        let poll = timing(context, "poll_ms", 20..=60_000).await?;
        let timeout = timing(context, "timeout_ms", 0..=3_600_000).await?;
        let started = std::time::Instant::now();
        let (before, baseline) = load_baseline(context, target).await?;
        let mut change_ratio;
        let latest = loop {
            session.ensure_active(context).await?;
            let current = target.capture().await?;
            let reference = matching_baseline(&baseline, &current.thumbnail)?;
            change_ratio = changed_fraction(&reference, &current.thumbnail, PIXEL_TOLERANCE)?;
            if change_ratio >= threshold || started.elapsed() >= timeout {
                break current;
            }
            crate::rpa::branch::delay(context, poll).await?;
        };
        let changed = change_ratio >= threshold;

        let after = NodeImage::new(
            context,
            flow_like_types::image::DynamicImage::ImageRgba8(latest.image),
        )
        .await;
        context.set_pin_value("changed", json!(changed)).await?;
        context
            .set_pin_value("change_ratio", json!(change_ratio))
            .await?;
        context.set_pin_value("before", json!(before)).await?;
        context.set_pin_value("after", json!(after)).await?;
        context.set_pin_value("frame", json!(latest.frame)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context
            .activate_exec_pin(if changed { "exec_out" } else { "exec_timeout" })
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

/// A caller-supplied baseline may have another resolution than the live capture; it is
/// rescaled when the aspect ratios agree.
#[cfg(any(feature = "execute", test))]
pub(crate) fn matching_baseline(
    baseline: &image::GrayImage,
    current: &image::GrayImage,
) -> flow_like_types::Result<image::GrayImage> {
    if baseline.dimensions() == current.dimensions() {
        return Ok(baseline.clone());
    }
    let ratio = |image: &image::GrayImage| image.width() as f64 / image.height().max(1) as f64;
    if (ratio(baseline) / ratio(current) - 1.0).abs() > 0.03 {
        return Err(flow_like_types::anyhow!(
            "Baseline image ({}x{}) does not have the shape of the watched area ({}x{} after downsampling); capture the baseline from the same region",
            baseline.width(),
            baseline.height(),
            current.width(),
            current.height()
        ));
    }
    Ok(image::imageops::resize(
        baseline,
        current.width(),
        current.height(),
        image::imageops::FilterType::Triangle,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn frame(width: u32, height: u32, value: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([value, value, value, 255]))
    }

    #[test]
    fn thumbnails_are_downsampled_to_the_long_edge() {
        let thumbnail = diff_thumbnail(&frame(1920, 1080, 40), THUMBNAIL_EDGE);
        assert_eq!(thumbnail.dimensions(), (320, 180));
        assert_eq!(diff_thumbnail(&frame(100, 50, 40), THUMBNAIL_EDGE).dimensions(), (100, 50));
    }

    #[test]
    fn changed_fraction_counts_pixels_beyond_tolerance() {
        let before = image::GrayImage::from_pixel(10, 10, image::Luma([100]));
        let mut after = before.clone();
        for x in 0..10 {
            after.put_pixel(x, 0, image::Luma([200]));
        }
        after.put_pixel(0, 5, image::Luma([105]));
        assert_eq!(changed_fraction(&before, &after, PIXEL_TOLERANCE).unwrap(), 0.1);
        assert_eq!(changed_fraction(&before, &before, 0).unwrap(), 0.0);
        assert!(changed_fraction(&before, &image::GrayImage::new(5, 5), 0).is_err());
    }

    #[test]
    fn small_moving_details_survive_downsampling_but_noise_does_not() {
        let base = frame(1280, 720, 30);
        let mut noisy = base.clone();
        for (x, y, pixel) in noisy.enumerate_pixels_mut() {
            if (x + y) % 7 == 0 {
                pixel.0 = [60, 60, 60, 255];
            }
        }
        let mut dialog = base.clone();
        for y in 200..400 {
            for x in 400..800 {
                dialog.put_pixel(x, y, image::Rgba([240, 240, 240, 255]));
            }
        }
        let reference = diff_thumbnail(&base, THUMBNAIL_EDGE);
        let noise = changed_fraction(
            &reference,
            &diff_thumbnail(&noisy, THUMBNAIL_EDGE),
            PIXEL_TOLERANCE,
        )
        .unwrap();
        let change = changed_fraction(
            &reference,
            &diff_thumbnail(&dialog, THUMBNAIL_EDGE),
            PIXEL_TOLERANCE,
        )
        .unwrap();
        assert_eq!(noise, 0.0);
        assert!((0.07..0.10).contains(&change), "{change}");
    }

    #[test]
    fn stability_requires_quiet_frames_for_the_whole_window() {
        let mut tracker = StabilityTracker::new(0.01, Duration::from_millis(300));
        assert!(!tracker.observe(0.0, Duration::from_millis(100)));
        assert!(!tracker.observe(0.2, Duration::from_millis(200)));
        assert!(!tracker.observe(0.0, Duration::from_millis(300)));
        assert!(!tracker.observe(0.009, Duration::from_millis(400)));
        assert!(tracker.observe(0.0, Duration::from_millis(500)));
        let mut immediate = StabilityTracker::new(0.01, Duration::ZERO);
        assert!(immediate.observe(0.0, Duration::from_millis(100)));
    }

    #[test]
    fn baselines_are_rescaled_only_when_shapes_agree() {
        let baseline = image::GrayImage::from_pixel(640, 360, image::Luma([10]));
        let current = image::GrayImage::from_pixel(320, 180, image::Luma([10]));
        assert_eq!(
            matching_baseline(&baseline, &current).unwrap().dimensions(),
            (320, 180)
        );
        assert!(matching_baseline(&image::GrayImage::new(100, 100), &current).is_err());
    }
}

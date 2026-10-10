use super::computer_use::{PERCEPTION_MARKS, PERCEPTION_MARKS_OCR, PERCEPTION_SCREENSHOT};
use super::history::ELEMENTS_HEADER;
use super::loop_guard::{fingerprint, screen_changed};
use crate::computer::accessibility::{flatten, load_window_tree, resolve_window};
use crate::computer::capture_state::{
    Candidate, ScreenElement, annotate, assemble, ax_candidates, dedup_ax, ocr_candidates,
    render_summary,
};
use crate::computer::ocr::{InputPoint, InputRect, OcrOptions, PixelBox, locate_lines, ocr_rgba};
use crate::computer::window::WindowInfo;
use crate::computer::zoom::{frame_rect, pixel_crop, window_rect};
use crate::llm::{ImageSource, ModelImage, prepare_screenshot};
use crate::types::screen_frame::{ScreenFrame, capture_display, display_frames};
use flow_like::flow::execution::context::ExecutionContext;
use std::time::{Duration, Instant};

const MAX_ELEMENTS: usize = 150;
const AX_DEPTH: usize = 32;
const SETTLE_POLL: Duration = Duration::from_millis(150);
const ZOOM_EDGE: u32 = 1024;
const MIN_ZOOM_PIXELS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Perception {
    Screenshot,
    Marks,
    MarksOcr,
}

impl Perception {
    pub(crate) fn parse(value: &str) -> flow_like_types::Result<Self> {
        match value.trim() {
            PERCEPTION_SCREENSHOT => Ok(Self::Screenshot),
            PERCEPTION_MARKS => Ok(Self::Marks),
            PERCEPTION_MARKS_OCR => Ok(Self::MarksOcr),
            other => Err(flow_like_types::anyhow!(
                "Unknown perception '{}'; use {}, {} or {}",
                other,
                PERCEPTION_SCREENSHOT,
                PERCEPTION_MARKS,
                PERCEPTION_MARKS_OCR
            )),
        }
    }

    pub(crate) fn marks(self) -> bool {
        self != Self::Screenshot
    }
}

/// A full-resolution display capture with its frame and change fingerprint.
pub(crate) struct Shot {
    pub image: image::RgbaImage,
    pub frame: ScreenFrame,
    pub fingerprint: image::GrayImage,
}

pub(crate) async fn capture(display_index: i64) -> flow_like_types::Result<Shot> {
    tokio::task::spawn_blocking(move || {
        let (image, frame) = capture_display(display_index)?;
        let fingerprint = fingerprint(&image);
        Ok(Shot {
            image,
            frame,
            fingerprint,
        })
    })
    .await?
}

/// Captures until two consecutive frames match or `max_wait` has passed; returns the last one.
pub(crate) async fn settle(
    context: &ExecutionContext,
    display_index: i64,
    max_wait: Duration,
) -> flow_like_types::Result<Shot> {
    let started = Instant::now();
    crate::rpa::branch::delay(context, max_wait.min(SETTLE_POLL)).await?;
    let mut shot = capture(display_index).await?;
    loop {
        let remaining = max_wait.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(shot);
        }
        crate::rpa::branch::delay(context, remaining.min(SETTLE_POLL)).await?;
        let next = capture(display_index).await?;
        let stable = !screen_changed(&shot.fingerprint, &next.fingerprint);
        shot = next;
        if stable {
            return Ok(shot);
        }
    }
}

pub(crate) fn display_label(index: i64) -> String {
    if index == -1 {
        "the primary display".to_string()
    } else {
        format!("display {index}")
    }
}

/// The display that shows most of `window`.
pub(crate) fn display_for(window: &InputRect, displays: &[ScreenFrame]) -> Option<usize> {
    displays
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| Some((index, frame_rect(frame).intersection(window)?.area())))
        .max_by_key(|(_, area)| *area)
        .map(|(index, _)| index)
}

/// What to observe: a display, or the display showing a window whose elements get marked.
pub(crate) struct Target {
    display_index: i64,
    window_title: Option<String>,
}

pub(crate) struct Located {
    pub display_index: i64,
    pub window: Option<WindowInfo>,
    pub warning: Option<String>,
}

impl Target {
    pub(crate) fn new(display_index: i64, window_title: &str) -> Self {
        let title = window_title.trim();
        Self {
            display_index,
            window_title: (!title.is_empty()).then(|| title.to_string()),
        }
    }

    pub(crate) async fn locate(&self) -> flow_like_types::Result<Located> {
        let fallback = Located {
            display_index: self.display_index,
            window: None,
            warning: None,
        };
        let Some(title) = &self.window_title else {
            return Ok(fallback);
        };
        let Some(window) = resolve_window(title).await? else {
            return Ok(Located {
                warning: Some(format!(
                    "Window \"{title}\" is not open; showing {}",
                    display_label(self.display_index)
                )),
                ..fallback
            });
        };
        let displays = tokio::task::spawn_blocking(display_frames).await??;
        let display_index = display_for(&window_rect(&window), &displays)
            .map_or(self.display_index, |index| index as i64);
        Ok(Located {
            display_index,
            window: Some(window),
            warning: None,
        })
    }
}

/// What the model sees in one turn, plus what is needed to act on it.
pub(crate) struct Observation {
    pub shot: Shot,
    /// Frame of the (downscaled) image the model sees; maps its coordinates to the desktop.
    pub model_frame: ScreenFrame,
    pub image: ModelImage,
    pub elements: Vec<ScreenElement>,
    /// Native locators in the same order as `elements`; OCR candidates have none.
    pub element_native_ids: Vec<Option<String>>,
    /// The window used for accessibility evidence, including the resolved default target.
    pub target_window: Option<WindowInfo>,
    pub element_text: Option<String>,
    pub source: String,
}

impl Observation {
    pub(crate) fn model_size(&self) -> (u32, u32) {
        (self.model_frame.pixel_width, self.model_frame.pixel_height)
    }

    pub(crate) fn summary(&self) -> String {
        let (width, height) = self.model_size();
        let elements = if self.element_text.is_some() {
            format!(", {} elements", self.elements.len())
        } else {
            String::new()
        };
        format!("{width}x{height} screenshot of {}{elements}", self.source)
    }
}

/// Whether this observation can support a decision that sends input to the foreground window.
pub(crate) fn decision_observation_ready(observation: &Observation) -> bool {
    observation.target_window.as_ref().is_some_and(|window| {
        !window.id.is_empty()
            && window.is_focused
            && !window.is_minimized
            && window.width > 0
            && window.height > 0
            && window_rect(window)
                .intersection(&frame_rect(&observation.shot.frame))
                .is_some()
    }) && !observation.elements.is_empty()
        && observation.element_native_ids.len() == observation.elements.len()
        && observation
            .element_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        && observation.shot.frame.validate().is_ok()
        && observation.model_frame.validate().is_ok()
        && observation.shot.image.dimensions()
            == (
                observation.shot.frame.pixel_width,
                observation.shot.frame.pixel_height,
            )
        && observation.shot.fingerprint.width() > 0
        && observation.shot.fingerprint.height() > 0
}

fn same_window(before: &WindowInfo, after: &WindowInfo) -> bool {
    before.id == after.id
        && before.title == after.title
        && before.app_name == after.app_name
        && window_rect(before) == window_rect(after)
        && before.is_focused == after.is_focused
        && before.is_minimized == after.is_minimized
}

/// Reject decisions whose target, controls or meaningful screen content changed while waiting.
/// Capture and accessibility reads are sequential, so the executor must still use the fresh state.
pub(crate) fn decision_observation_unchanged(before: &Observation, after: &Observation) -> bool {
    decision_observation_ready(before)
        && decision_observation_ready(after)
        && before
            .target_window
            .as_ref()
            .zip(after.target_window.as_ref())
            .is_some_and(|(before, after)| same_window(before, after))
        && before.source == after.source
        && before.shot.frame == after.shot.frame
        && before.model_frame == after.model_frame
        && before.elements == after.elements
        && before.element_native_ids == after.element_native_ids
        && before.element_text == after.element_text
        && !screen_changed(&before.shot.fingerprint, &after.shot.fingerprint)
}

/// Element centers in pixels of the model's image, for the list the model reads.
pub(crate) fn model_space(
    elements: &[ScreenElement],
    model_frame: &ScreenFrame,
) -> Vec<ScreenElement> {
    elements
        .iter()
        .filter_map(|element| {
            let (x, y) = model_frame.input_to_pixel(element.center.x, element.center.y)?;
            Some(ScreenElement {
                center: InputPoint {
                    x: x as i32,
                    y: y as i32,
                },
                ..element.clone()
            })
        })
        .collect()
}

fn describe_source(window: Option<&WindowInfo>, frame: &ScreenFrame) -> String {
    let display = frame.display_index.map_or_else(
        || "the display".to_string(),
        |index| format!("display {index}"),
    );
    match window {
        Some(window) => format!("window \"{}\" on {display}", window.title.replace('"', "'")),
        None => display,
    }
}

async fn accessibility_elements(
    window: Option<&WindowInfo>,
    frame: &ScreenFrame,
    warnings: &mut Vec<String>,
) -> (Vec<Candidate>, Option<WindowInfo>) {
    let window = match window {
        Some(window) => Ok(Some(window.clone())),
        None => resolve_window("").await,
    };
    let window = match window {
        Ok(Some(window)) => window,
        Ok(None) => {
            warnings.push("Accessibility elements unavailable: no application window outside Flow-Like is open".into());
            return (Vec::new(), None);
        }
        Err(error) => {
            warnings.push(format!("Accessibility elements unavailable: {error}"));
            return (Vec::new(), None);
        }
    };
    let elements = match load_window_tree(&window.id, AX_DEPTH).await {
        Ok(tree) => dedup_ax(ax_candidates(flatten(&tree), &frame_rect(frame), true)),
        Err(error) => {
            warnings.push(format!("Accessibility elements unavailable: {error}"));
            Vec::new()
        }
    };
    (elements, Some(window))
}

async fn text_elements(
    image: &image::RgbaImage,
    frame: &ScreenFrame,
    ax: &[Candidate],
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    match ocr_rgba(image.clone(), OcrOptions::default()).await {
        Ok(mut lines) => {
            locate_lines(&mut lines, frame);
            ocr_candidates(&lines, ax)
        }
        Err(error) => {
            warnings.push(format!("Text recognition failed: {error}"));
            Vec::new()
        }
    }
}

async fn marked_elements(
    shot: &Shot,
    located: &Located,
    perception: Perception,
    warnings: &mut Vec<String>,
) -> (Vec<Candidate>, Option<WindowInfo>) {
    if !perception.marks() {
        return (Vec::new(), located.window.clone());
    }
    let (ax, window) = accessibility_elements(located.window.as_ref(), &shot.frame, warnings).await;
    let ocr = if perception == Perception::MarksOcr {
        text_elements(&shot.image, &shot.frame, &ax, warnings).await
    } else {
        Vec::new()
    };
    (assemble(ax, ocr, MAX_ELEMENTS), window)
}

/// The capture with its marks drawn, downscaled and encoded for the model, and its frame.
async fn model_image(
    shot: Shot,
    marks: Vec<(u32, PixelBox)>,
) -> flow_like_types::Result<(Shot, ModelImage, ScreenFrame)> {
    tokio::task::spawn_blocking(move || {
        let picture = if marks.is_empty() {
            shot.image.clone()
        } else {
            annotate(shot.image.clone(), &marks)
        };
        let screenshot = prepare_screenshot(
            ImageSource::Decoded(image::DynamicImage::ImageRgba8(picture)),
            Some(shot.frame.clone()),
        )?;
        let (width, height) = screenshot.view.size();
        let model_frame = shot.frame.resized(width, height)?;
        Ok((shot, screenshot.image, model_frame))
    })
    .await?
}

fn element_list(
    elements: &[ScreenElement],
    warnings: &[String],
    model_frame: &ScreenFrame,
) -> String {
    let header = format!(
        "{ELEMENTS_HEADER} ({}; click_element takes the [id], @ (x,y) is the center in screenshot pixels):",
        elements.len()
    );
    render_summary(&header, warnings, &model_space(elements, model_frame))
}

pub(crate) async fn observe(
    shot: Shot,
    located: &Located,
    perception: Perception,
) -> flow_like_types::Result<Observation> {
    let mut warnings = Vec::new();
    let (candidates, target_window) =
        marked_elements(&shot, located, perception, &mut warnings).await;
    // The fallback display can help the vision model recover, but is not the requested window.
    let target_window = target_window.filter(|_| located.warning.is_none());
    let marks: Vec<(u32, PixelBox)> = candidates
        .iter()
        .filter_map(|c| Some((c.element.id, pixel_crop(&shot.frame, &c.element.bbox)?)))
        .collect();
    let element_native_ids = candidates.iter().map(|c| c.native_id.clone()).collect();
    let elements: Vec<ScreenElement> = candidates.into_iter().map(|c| c.element).collect();
    let (shot, image, model_frame) = model_image(shot, marks).await?;
    let element_text = perception
        .marks()
        .then(|| element_list(&elements, &warnings, &model_frame));
    let source = describe_source(located.window.as_ref(), &shot.frame);
    Ok(Observation {
        shot,
        model_frame,
        image,
        elements,
        element_native_ids,
        target_window,
        element_text,
        source,
    })
}

/// An enlarged region of the latest screenshot, shown with the next observation.
pub(crate) struct ZoomView {
    pub label: String,
    pub image: ModelImage,
}

fn spans(start: f64, end: f64, length: f64) -> bool {
    0.0 <= start && start < end && end <= length
}

/// Pixel box of the full-resolution capture covering a region of the model's image.
pub(crate) fn zoom_box(
    corners: (f64, f64, f64, f64),
    model: (u32, u32),
    capture: (u32, u32),
) -> Result<PixelBox, String> {
    let (x0, y0, x1, y1) = corners;
    let (width, height) = (model.0 as f64, model.1 as f64);
    if !(spans(x0, x1, width) && spans(y0, y1, height)) {
        return Err(format!(
            "Zoom region ({x0}, {y0})-({x1}, {y1}) must lie inside the {}x{} screenshot with x0 < x1 and y0 < y1",
            model.0, model.1
        ));
    }
    let (sx, sy) = (capture.0 as f64 / width, capture.1 as f64 / height);
    let left = (x0 * sx).floor() as u32;
    let top = (y0 * sy).floor() as u32;
    let right = ((x1 * sx).ceil() as u32).min(capture.0);
    let bottom = ((y1 * sy).ceil() as u32).min(capture.1);
    if right.saturating_sub(left) < MIN_ZOOM_PIXELS || bottom.saturating_sub(top) < MIN_ZOOM_PIXELS
    {
        return Err(format!(
            "Zoom region ({x0}, {y0})-({x1}, {y1}) is too small to enlarge"
        ));
    }
    Ok(PixelBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// Integer enlargement that brings a small crop towards a legible size.
pub(crate) fn zoom_factor(width: u32, height: u32) -> u32 {
    (ZOOM_EDGE / width.max(height).max(1)).clamp(1, 4)
}

pub(crate) async fn zoom_view(
    capture: &image::RgbaImage,
    crop: PixelBox,
    label: String,
) -> flow_like_types::Result<ZoomView> {
    let region =
        image::imageops::crop_imm(capture, crop.x, crop.y, crop.width, crop.height).to_image();
    tokio::task::spawn_blocking(move || {
        let factor = zoom_factor(region.width(), region.height());
        let enlarged = if factor == 1 {
            region
        } else {
            image::imageops::resize(
                &region,
                region.width() * factor,
                region.height() * factor,
                image::imageops::FilterType::Lanczos3,
            )
        };
        let screenshot = prepare_screenshot(
            ImageSource::Decoded(image::DynamicImage::ImageRgba8(enlarged)),
            None,
        )?;
        Ok(ZoomView {
            label,
            image: screenshot.image,
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::screen_frame::ModelImageBudget;

    fn frame(display: u32, x: i32, y: i32, width: u32, height: u32, scale: u32) -> ScreenFrame {
        ScreenFrame::new(
            Some(display),
            (x, y, width, height),
            (width * scale, height * scale),
        )
        .unwrap()
    }

    fn decision_observation() -> Observation {
        let frame = frame(0, 0, 0, 100, 100, 1);
        let image = image::RgbaImage::new(100, 100);
        let elements = ["Save", "Cancel"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let bbox = InputRect {
                    x: 10 + index as i32 * 30,
                    y: 20,
                    width: 20,
                    height: 10,
                };
                ScreenElement {
                    id: index as u32 + 1,
                    source: "ax".into(),
                    role: "button".into(),
                    name: Some(name.into()),
                    value: None,
                    states: vec!["enabled".into()],
                    bbox,
                    center: bbox.center(),
                }
            })
            .collect::<Vec<_>>();
        let element_text = Some(element_list(&elements, &[], &frame));
        Observation {
            shot: Shot {
                fingerprint: fingerprint(&image),
                image,
                frame: frame.clone(),
            },
            model_frame: frame,
            image: ModelImage {
                media_type: "image/png".into(),
                base64: String::new(),
            },
            elements,
            element_native_ids: vec![
                Some("window-1/path-0".into()),
                Some("window-1/path-1".into()),
            ],
            target_window: Some(WindowInfo {
                id: "window-1".into(),
                title: "Document".into(),
                app_name: Some("Editor".into()),
                x: 0,
                y: 0,
                width: 100,
                height: 100,
                is_focused: true,
                is_minimized: false,
            }),
            element_text,
            source: "window \"Document\" on display 0".into(),
        }
    }

    #[test]
    fn decision_guard_accepts_stable_observations_and_ignores_small_pixel_noise() {
        let before = decision_observation();
        let mut after = decision_observation();
        assert!(decision_observation_ready(&before));
        assert!(decision_observation_unchanged(&before, &after));
        after.shot.image.put_pixel(0, 0, image::Rgba([255; 4]));
        after.shot.fingerprint = fingerprint(&after.shot.image);
        assert!(decision_observation_unchanged(&before, &after));
        for x in 0..2 {
            for y in 0..2 {
                after.shot.image.put_pixel(x, y, image::Rgba([255; 4]));
            }
        }
        after.shot.fingerprint = fingerprint(&after.shot.image);
        assert!(!decision_observation_unchanged(&before, &after));
    }

    #[test]
    fn decision_guard_rejects_changed_or_missing_window_identity_and_geometry() {
        let before = decision_observation();
        let mutations: [fn(&mut WindowInfo); 8] = [
            |window| window.id = "replacement-window".into(),
            |window| window.title = "Another document".into(),
            |window| window.app_name = Some("Another editor".into()),
            |window| window.x += 1,
            |window| window.y += 1,
            |window| window.width -= 1,
            |window| window.height -= 1,
            |window| window.is_focused = false,
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut after = decision_observation();
            mutate(after.target_window.as_mut().unwrap());
            assert!(
                !decision_observation_unchanged(&before, &after),
                "change {index}"
            );
        }
        let mut after = decision_observation();
        after.target_window = None;
        assert!(!decision_observation_ready(&after));
        assert!(!decision_observation_unchanged(&before, &after));
    }

    #[test]
    fn decision_guard_requires_a_visible_focused_target_and_usable_evidence() {
        let mutations: [fn(&mut Observation); 8] = [
            |observation| observation.target_window.as_mut().unwrap().id.clear(),
            |observation| observation.target_window.as_mut().unwrap().is_focused = false,
            |observation| observation.target_window.as_mut().unwrap().is_minimized = true,
            |observation| observation.target_window.as_mut().unwrap().width = 0,
            |observation| observation.target_window.as_mut().unwrap().x = 200,
            |observation| observation.element_text = None,
            |observation| observation.elements.clear(),
            |observation| observation.element_native_ids.clear(),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut observation = decision_observation();
            mutate(&mut observation);
            assert!(!decision_observation_ready(&observation), "change {index}");
        }
    }

    #[test]
    fn decision_guard_rejects_control_changes_even_with_unchanged_pixels() {
        let before = decision_observation();
        let mutations: [fn(&mut Observation); 12] = [
            |observation| observation.elements[0].id = 3,
            |observation| observation.elements[0].source = "ocr".into(),
            |observation| observation.elements[0].role = "checkbox".into(),
            |observation| observation.elements[0].name = Some("Delete".into()),
            |observation| observation.elements[0].value = Some("changed".into()),
            |observation| observation.elements[0].states = vec!["disabled".into()],
            |observation| observation.elements[0].bbox.width += 1,
            |observation| observation.elements[0].center.x += 1,
            |observation| observation.element_native_ids[0] = Some("replacement-control".into()),
            |observation| observation.elements.swap(0, 1),
            |observation| {
                observation.elements.pop();
                observation.element_native_ids.pop();
            },
            |observation| observation.element_text = Some("Accessibility read failed".into()),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut after = decision_observation();
            mutate(&mut after);
            assert!(
                !decision_observation_unchanged(&before, &after),
                "change {index}"
            );
        }
    }

    #[test]
    fn decision_guard_rejects_changed_display_and_viewport_mapping() {
        let before = decision_observation();
        let mutations: [fn(&mut Observation); 6] = [
            |observation| observation.source = "another display".into(),
            |observation| observation.shot.frame.display_index = Some(1),
            |observation| observation.shot.frame.x += 1,
            |observation| observation.shot.frame.width += 1,
            |observation| {
                observation.model_frame = observation.model_frame.resized(50, 50).unwrap()
            },
            |observation| observation.shot.fingerprint = image::GrayImage::new(99, 100),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut after = decision_observation();
            mutate(&mut after);
            assert!(
                !decision_observation_unchanged(&before, &after),
                "change {index}"
            );
        }
    }

    #[test]
    fn perception_modes_parse() {
        assert_eq!(
            Perception::parse("screenshot").unwrap(),
            Perception::Screenshot
        );
        assert!(Perception::parse(" screenshot_marks ").unwrap().marks());
        assert_eq!(
            Perception::parse("screenshot_marks_ocr").unwrap(),
            Perception::MarksOcr
        );
        assert!(!Perception::Screenshot.marks());
        assert!(Perception::parse("video").is_err());
    }

    #[test]
    fn windows_belong_to_the_display_showing_most_of_them() {
        let displays = [
            frame(0, 0, 0, 1440, 900, 2),
            frame(1, 1440, 0, 1920, 1080, 1),
        ];
        let spanning = InputRect {
            x: 1200,
            y: 100,
            width: 800,
            height: 600,
        };
        assert_eq!(display_for(&spanning, &displays), Some(1));
        let left = InputRect {
            x: 10,
            y: 10,
            width: 300,
            height: 200,
        };
        assert_eq!(display_for(&left, &displays), Some(0));
        let offscreen = InputRect {
            x: -5000,
            y: 0,
            width: 10,
            height: 10,
        };
        assert_eq!(display_for(&offscreen, &displays), None);
        assert_eq!(display_label(-1), "the primary display");
    }

    #[test]
    fn element_list_uses_model_pixels_on_a_downscaled_retina_display() {
        let capture = frame(1, -1440, -200, 1440, 900, 2);
        let (width, height) = ModelImageBudget::default().fit(2880, 1800);
        let model = capture.resized(width, height).unwrap();
        let element = ScreenElement {
            id: 4,
            source: "ax".into(),
            role: "button".into(),
            name: Some("Save".into()),
            value: None,
            states: vec![],
            bbox: InputRect {
                x: -740,
                y: 240,
                width: 40,
                height: 20,
            },
            center: InputPoint { x: -720, y: 250 },
        };
        let listed = model_space(std::slice::from_ref(&element), &model);
        assert_eq!(listed.len(), 1);
        let (px, py) = (listed[0].center.x, listed[0].center.y);
        assert!(
            (px - width as i32 / 2).abs() <= 1 && (py - (height as i32 * 450 / 900)).abs() <= 1
        );
        let back = model.pixel_to_input(px as f64, py as f64).unwrap();
        assert!((back.0 + 720).abs() <= 1 && (back.1 - 250).abs() <= 1);
    }

    #[test]
    fn zoom_regions_map_to_native_pixels_and_reject_bad_corners() {
        let native = zoom_box((100.0, 50.0, 300.0, 150.0), (1440, 900), (2880, 1800)).unwrap();
        assert_eq!(
            native,
            PixelBox {
                x: 200,
                y: 100,
                width: 400,
                height: 200
            }
        );
        let edge = zoom_box((1400.0, 850.0, 1440.0, 900.0), (1440, 900), (2880, 1800)).unwrap();
        assert_eq!((edge.x + edge.width, edge.y + edge.height), (2880, 1800));
        assert!(zoom_box((300.0, 50.0, 100.0, 150.0), (1440, 900), (2880, 1800)).is_err());
        assert!(zoom_box((0.0, 0.0, 1441.0, 10.0), (1440, 900), (2880, 1800)).is_err());
        assert!(zoom_box((-1.0, 0.0, 10.0, 10.0), (1440, 900), (2880, 1800)).is_err());
        assert!(zoom_box((10.0, 10.0, 11.0, 11.0), (1440, 900), (1440, 900)).is_err());
        assert_eq!(zoom_factor(200, 100), 4);
        assert_eq!(zoom_factor(400, 300), 2);
        assert_eq!(zoom_factor(2880, 1800), 1);
    }
}

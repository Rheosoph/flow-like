#![cfg_attr(not(feature = "execute"), allow(dead_code))]

use super::accessibility::{AccessibilityNode, is_interactive};
use super::ocr::{InputPoint, InputRect, OcrLine, PixelBox, reading_order_key};
use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One numbered element of a screen state. Coordinates are desktop input coordinates.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ScreenElement {
    /// 1-based number drawn on the annotated image.
    pub id: u32,
    /// `ax` (accessibility tree) or `ocr` (recognized text).
    pub source: String,
    pub role: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub states: Vec<String>,
    /// Visible part of the element.
    pub bbox: InputRect,
    pub center: InputPoint,
}

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub element: ScreenElement,
    pub native_id: Option<String>,
    pub invokable: bool,
}

/// The element table of the latest capture, keyed by session in the run cache.
#[derive(Clone)]
pub(crate) struct ScreenState {
    pub generation: u64,
    pub elements: Vec<Candidate>,
}

impl flow_like_types::Cacheable for ScreenState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub(crate) fn state_key(session_ref: &str) -> String {
    format!("automation:screen_state:{}", session_ref)
}

fn clean_text(text: Option<&str>, limit: usize) -> Option<String> {
    let text = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > limit {
        format!("{}…", text.chars().take(limit).collect::<String>())
    } else {
        text
    })
}

fn loose(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Accessibility elements that are visible inside `visible`, clipped to it.
pub(crate) fn ax_candidates(
    nodes: Vec<AccessibilityNode>,
    visible: &InputRect,
    interactive_only: bool,
) -> Vec<Candidate> {
    nodes
        .into_iter()
        .filter_map(|node| {
            let bounds = node.bounds.as_ref()?;
            if bounds.width <= 0 || bounds.height <= 0 {
                return None;
            }
            if matches!(node.normalized_role.as_str(), "window" | "application")
                || node.normalized_states.iter().any(|s| s == "offscreen")
            {
                return None;
            }
            let interactive = is_interactive(&node);
            let name = clean_text(node.name.as_deref(), 120);
            let value = clean_text(node.value.as_deref(), 200);
            if (interactive_only && !interactive)
                || (!interactive && name.is_none() && value.is_none())
            {
                return None;
            }
            let bbox = InputRect {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width as u32,
                height: bounds.height as u32,
            }
            .intersection(visible)?;
            Some(Candidate {
                element: ScreenElement {
                    id: 0,
                    source: "ax".into(),
                    role: node.normalized_role.clone(),
                    name,
                    value,
                    states: node.normalized_states.clone(),
                    bbox,
                    center: bbox.center(),
                },
                invokable: node.normalized_actions.iter().any(|a| a == "invoke"),
                native_id: node.native_id,
            })
        })
        .collect()
}

/// Collapses accessibility elements that describe the same box (wrapper + control), keeping
/// the one that carries a name.
pub(crate) fn dedup_ax(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut kept: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if let Some(existing) = kept
            .iter_mut()
            .find(|k| k.element.bbox.iou(&candidate.element.bbox) >= 0.9)
        {
            if existing.element.name.is_none() && candidate.element.name.is_some() {
                *existing = candidate;
            }
            continue;
        }
        kept.push(candidate);
    }
    kept
}

/// Whether recognized text is already described by an accessibility element: nearly the same
/// box, or lying inside an element that is small relative to it or carries the same text.
pub(crate) fn ocr_covered(rect: &InputRect, text: &str, ax: &[(InputRect, String)]) -> bool {
    let key = loose(text);
    ax.iter().any(|(bbox, label)| {
        if rect.iou(bbox) >= 0.5 {
            return true;
        }
        let Some(overlap) = rect.intersection(bbox) else {
            return false;
        };
        let inside = overlap.area() as f64 / rect.area().max(1) as f64 >= 0.8;
        inside
            && (bbox.area() <= rect.area().saturating_mul(6)
                || (!key.is_empty() && label.contains(&key)))
    })
}

pub(crate) fn ocr_candidates(lines: &[OcrLine], ax: &[Candidate]) -> Vec<Candidate> {
    let labels: Vec<(InputRect, String)> = ax
        .iter()
        .map(|c| {
            let text = format!(
                "{} {}",
                c.element.name.as_deref().unwrap_or_default(),
                c.element.value.as_deref().unwrap_or_default()
            );
            (c.element.bbox, loose(&text))
        })
        .collect();
    lines
        .iter()
        .filter_map(|line| {
            let (bbox, center) = (line.bbox?, line.center?);
            if ocr_covered(&bbox, &line.text, &labels) {
                return None;
            }
            Some(Candidate {
                element: ScreenElement {
                    id: 0,
                    source: "ocr".into(),
                    role: "text".into(),
                    name: clean_text(Some(&line.text), 200),
                    value: None,
                    states: Vec::new(),
                    bbox,
                    center,
                },
                native_id: None,
                invokable: false,
            })
        })
        .collect()
}

/// Accessibility elements first (they can be acted on natively), recognized text fills the
/// remaining budget; ids follow reading order.
pub(crate) fn assemble(mut ax: Vec<Candidate>, ocr: Vec<Candidate>, max: usize) -> Vec<Candidate> {
    ax.truncate(max);
    let room = max - ax.len();
    ax.extend(ocr.into_iter().take(room));
    ax.sort_by_key(|c| {
        let b = c.element.bbox;
        reading_order_key(b.y as i64, b.height as i64, b.x as i64, 8)
    });
    for (index, candidate) in ax.iter_mut().enumerate() {
        candidate.element.id = index as u32 + 1;
    }
    ax
}

fn quote(text: &str) -> String {
    let text = clean_text(Some(text), 80).unwrap_or_default();
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn summary_line(element: &ScreenElement) -> String {
    const NOTABLE: [&str; 8] = [
        "focused",
        "disabled",
        "checked",
        "mixed",
        "selected",
        "expanded",
        "collapsed",
        "read_only",
    ];
    let mut line = format!("[{}] {}", element.id, element.role);
    if let Some(name) = &element.name {
        line.push_str(&format!(" \"{}\"", quote(name)));
    }
    if let Some(value) = element
        .value
        .as_ref()
        .filter(|v| Some(*v) != element.name.as_ref())
    {
        line.push_str(&format!(" value=\"{}\"", quote(value)));
    }
    let notable: Vec<&str> = element
        .states
        .iter()
        .map(String::as_str)
        .filter(|s| NOTABLE.contains(s))
        .collect();
    if !notable.is_empty() {
        line.push_str(&format!(" [{}]", notable.join(", ")));
    }
    line.push_str(&format!(" @ ({},{})", element.center.x, element.center.y));
    line
}

pub(crate) fn render_summary(
    header: &str,
    warnings: &[String],
    elements: &[ScreenElement],
) -> String {
    let mut summary = header.to_string();
    for warning in warnings {
        summary.push_str("\n! ");
        summary.push_str(warning);
    }
    for element in elements {
        summary.push('\n');
        summary.push_str(&summary_line(element));
    }
    summary
}

const PALETTE: [[u8; 3]; 8] = [
    [230, 25, 75],
    [0, 130, 200],
    [60, 180, 75],
    [245, 130, 48],
    [145, 30, 180],
    [0, 150, 150],
    [240, 50, 230],
    [128, 128, 0],
];

const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];

/// Glyph cell size so labels stay legible after the image is downscaled for a model.
pub(crate) fn glyph_scale(width: u32, height: u32) -> u32 {
    ((width.max(height) as f64 / 900.0).ceil() as u32).clamp(2, 8)
}

pub(crate) fn label_size(id: u32, scale: u32) -> (u32, u32) {
    let digits = id.max(1).ilog10() + 1;
    (digits * 6 * scale + scale, 9 * scale)
}

/// Label position next to a box: above it, inside its top-left corner, above-right, below,
/// left, right — the first that fits the image without covering an earlier label.
pub(crate) fn place_label(
    target: &PixelBox,
    size: (u32, u32),
    image: (u32, u32),
    placed: &[PixelBox],
) -> PixelBox {
    let (w, h) = (size.0 as i64, size.1 as i64);
    let (bx, by) = (target.x as i64, target.y as i64);
    let (bw, bh) = (target.width as i64, target.height as i64);
    let (iw, ih) = (image.0 as i64, image.1 as i64);
    let make = |x: i64, y: i64| PixelBox {
        x: x as u32,
        y: y as u32,
        width: size.0,
        height: size.1,
    };
    for (x, y) in [
        (bx, by - h),
        (bx, by),
        (bx + bw - w, by - h),
        (bx, by + bh),
        (bx - w, by),
        (bx + bw, by),
    ] {
        if x >= 0 && y >= 0 && x + w <= iw && y + h <= ih {
            let label = make(x, y);
            if !placed.iter().any(|p| p.intersects(&label)) {
                return label;
            }
        }
    }
    make(
        bx.clamp(0, (iw - w).max(0)),
        (by - h).clamp(0, (ih - h).max(0)),
    )
}

fn fill(image: &mut image::RgbaImage, area: &PixelBox, color: [u8; 3]) {
    let x_end = (area.x + area.width).min(image.width());
    let y_end = (area.y + area.height).min(image.height());
    for y in area.y..y_end {
        for x in area.x..x_end {
            image.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
        }
    }
}

fn outline(image: &mut image::RgbaImage, area: &PixelBox, thickness: u32, color: [u8; 3]) {
    let t = thickness.min(area.width).min(area.height).max(1);
    let right = area.x + area.width - t;
    let bottom = area.y + area.height - t;
    for edge in [
        PixelBox { height: t, ..*area },
        PixelBox {
            y: bottom,
            height: t,
            ..*area
        },
        PixelBox { width: t, ..*area },
        PixelBox {
            x: right,
            width: t,
            ..*area
        },
    ] {
        fill(image, &edge, color);
    }
}

pub(crate) fn draw_number(
    image: &mut image::RgbaImage,
    x: u32,
    y: u32,
    id: u32,
    scale: u32,
    color: [u8; 3],
) {
    for (index, digit) in id.to_string().bytes().enumerate() {
        let glyph = DIGITS[(digit - b'0') as usize];
        let left = x + index as u32 * 6 * scale;
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5u32 {
                if bits & (1 << (4 - column)) != 0 {
                    let cell = PixelBox {
                        x: left + column * scale,
                        y: y + row as u32 * scale,
                        width: scale,
                        height: scale,
                    };
                    fill(image, &cell, color);
                }
            }
        }
    }
}

/// Draws numbered set-of-marks boxes. Outlines first, labels on top of every outline.
pub(crate) fn annotate(mut image: image::RgbaImage, marks: &[(u32, PixelBox)]) -> image::RgbaImage {
    let dimensions = image.dimensions();
    let scale = glyph_scale(dimensions.0, dimensions.1);
    let thickness = scale.div_ceil(2);
    for (index, (_, area)) in marks.iter().enumerate() {
        outline(&mut image, area, thickness, PALETTE[index % PALETTE.len()]);
    }
    let mut placed = Vec::with_capacity(marks.len());
    for (index, (id, area)) in marks.iter().enumerate() {
        let color = PALETTE[index % PALETTE.len()];
        let label = place_label(area, label_size(*id, scale), dimensions, &placed);
        fill(&mut image, &label, color);
        let luminance = 0.299 * color[0] as f64 + 0.587 * color[1] as f64 + 0.114 * color[2] as f64;
        let ink = if luminance > 150.0 {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        };
        draw_number(
            &mut image,
            label.x + scale,
            label.y + scale,
            *id,
            scale,
            ink,
        );
        placed.push(label);
    }
    image
}

/// Clicks a desktop point with the same native input path as the mouse nodes.
#[cfg(feature = "execute")]
pub(crate) async fn click_point(
    context: &ExecutionContext,
    session: &AutomationSession,
    point: InputPoint,
    button: &str,
    double: bool,
) -> flow_like_types::Result<()> {
    use enigo::{Coordinate, Direction, Mouse};
    let button = match button.to_lowercase().as_str() {
        "left" => enigo::Button::Left,
        "right" => enigo::Button::Right,
        "middle" => enigo::Button::Middle,
        other => {
            return Err(flow_like_types::anyhow!("Unknown mouse button: {}", other));
        }
    };
    let mut input = session.create_enigo(context).await?;
    let cancellation = context.get_cancellation_token();
    let delay = session.click_delay_ms.min(5000);
    tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
        use super::mouse::{check_cancellation, interruptible_sleep};
        check_cancellation(cancellation.as_ref())?;
        input
            .move_mouse(point.x, point.y, Coordinate::Abs)
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to move mouse to ({}, {}): {}",
                    point.x,
                    point.y,
                    e
                )
            })?;
        interruptible_sleep(delay, cancellation.as_ref())?;
        let clicks = if double { 2 } else { 1 };
        for click in 0..clicks {
            if click > 0 {
                interruptible_sleep(80, cancellation.as_ref())?;
            }
            check_cancellation(cancellation.as_ref())?;
            input.button(button, Direction::Click).map_err(|e| {
                flow_like_types::anyhow!("Failed to click at ({}, {}): {}", point.x, point.y, e)
            })?;
        }
        Ok(())
    })
    .await?
}

#[cfg(feature = "execute")]
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(feature = "execute")]
async fn resolve_target(
    target: &str,
    window_title: &str,
) -> flow_like_types::Result<Option<super::window::WindowInfo>> {
    use super::accessibility::resolve_window;
    let title = window_title.trim();
    match target {
        "display" => Ok(None),
        "window" if title.is_empty() => Err(flow_like_types::anyhow!(
            "The window target needs a window title"
        )),
        "window" => Ok(Some(resolve_window(title).await?.ok_or_else(|| {
            flow_like_types::anyhow!("Window '{}' not found", title)
        })?)),
        "focused_window" => Ok(Some(resolve_window("").await?.ok_or_else(|| {
            flow_like_types::anyhow!(
                "No focused application window outside Flow-Like was found; use the display target"
            )
        })?)),
        other => Err(flow_like_types::anyhow!(
            "Unknown capture target '{}'; use focused_window, window or display",
            other
        )),
    }
}

#[cfg(feature = "execute")]
async fn capture_target(
    window: Option<&super::window::WindowInfo>,
    display_index: i64,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    match window {
        Some(window) => {
            super::zoom::capture_input_rect_async(super::zoom::window_rect(window)).await
        }
        None => {
            let index = u32::try_from(display_index).map_err(|_| {
                flow_like_types::anyhow!("Display index {} is invalid", display_index)
            })?;
            super::zoom::capture_display_async(index).await
        }
    }
}

/// Elements of the target window's accessibility tree (for a display target, the focused
/// window outside Flow-Like). Failures become warnings: the screenshot is still useful.
#[cfg(feature = "execute")]
async fn accessibility_candidates(
    window: Option<&super::window::WindowInfo>,
    frame: &ScreenFrame,
    interactive_only: bool,
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    use super::accessibility::{flatten, load_window_tree, resolve_window};
    let window = match window {
        Some(window) => Ok(Some(window.clone())),
        None => resolve_window("").await,
    };
    let tree = match window {
        Ok(Some(window)) => load_window_tree(&window.id, 32).await,
        Ok(None) => Err(flow_like_types::anyhow!(
            "no application window outside Flow-Like is open"
        )),
        Err(error) => Err(error),
    };
    match tree {
        Ok(tree) => dedup_ax(ax_candidates(
            flatten(&tree),
            &super::zoom::frame_rect(frame),
            interactive_only,
        )),
        Err(error) => {
            warnings.push(format!("Accessibility elements unavailable: {error}"));
            Vec::new()
        }
    }
}

#[cfg(feature = "execute")]
async fn text_candidates(
    picture: &image::RgbaImage,
    frame: &ScreenFrame,
    languages: Vec<String>,
    ax: &[Candidate],
    warnings: &mut Vec<String>,
) -> Vec<Candidate> {
    let options = super::ocr::OcrOptions {
        languages,
        ..Default::default()
    };
    match super::ocr::ocr_rgba(picture.clone(), options).await {
        Ok(mut lines) => {
            super::ocr::locate_lines(&mut lines, frame);
            ocr_candidates(&lines, ax)
        }
        Err(error) => {
            warnings.push(format!("Text recognition failed: {error}"));
            Vec::new()
        }
    }
}

/// The element `id` of the session's latest screen state, rejecting ids of older captures.
#[cfg(feature = "execute")]
async fn latest_element(
    context: &ExecutionContext,
    session_ref: &str,
    id: i64,
    generation: i64,
) -> flow_like_types::Result<Candidate> {
    let state = context
        .get_cache(&state_key(session_ref))
        .await
        .and_then(|value| value.as_any().downcast_ref::<ScreenState>().cloned())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "No screen state for this session; run Capture Screen State first"
            )
        })?;
    let stale = || {
        flow_like_types::anyhow!(
            "Stale screen element {} — capture the screen state again",
            id
        )
    };
    if generation != 0 && generation as u64 != state.generation {
        return Err(stale());
    }
    state
        .elements
        .into_iter()
        .find(|c| id > 0 && c.element.id as i64 == id)
        .ok_or_else(stale)
}

/// Presses an accessibility element natively; false when it cannot, so the caller clicks.
#[cfg(feature = "execute")]
async fn press_accessibly(context: &mut ExecutionContext, candidate: &Candidate) -> bool {
    let Some(native_id) = candidate.native_id.clone().filter(|_| candidate.invokable) else {
        return false;
    };
    let node = AccessibilityNode {
        native_id: Some(native_id),
        ..Default::default()
    };
    match super::native::accessibility_action(&node, "invoke", "", context.get_cancellation_token())
        .await
    {
        Ok(()) => true,
        Err(error) => {
            context.log_message(
                &format!(
                    "Accessibility press of element {} failed ({}); clicking its center",
                    candidate.element.id, error
                ),
                flow_like::flow::execution::LogLevel::Warn,
            );
            false
        }
    }
}

fn describe_target(window: Option<&super::window::WindowInfo>, display_index: i64) -> String {
    match window {
        Some(window) => format!(
            "window \"{}\"{}",
            quote(&window.title),
            window
                .app_name
                .as_deref()
                .map(|app| format!(" ({})", quote(app)))
                .unwrap_or_default()
        ),
        None => format!("display {}", display_index),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerCaptureStateNode;

impl ComputerCaptureStateNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerCaptureStateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_capture_state",
            "Capture Screen State",
            "Observes a window or display for an agent: a screenshot, the same screenshot with numbered boxes on every actionable element, and a compact element list (accessibility tree plus optional OCR text) whose ids Click Screen Element accepts",
            "Automation/Computer/Capture",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "captureState");
        node.add_icon("/flow/icons/computer.svg");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(5)
                .set_performance(5)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_input_pin(
            "target",
            "Target",
            "focused_window: the focused window outside Flow-Like; window: the window titled below; display: a whole display",
            VariableType::String,
        )
        .set_default_value(Some(json!("focused_window")))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    ["focused_window", "window", "display"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
                .build(),
        );
        node.add_input_pin(
            "window_title",
            "Window Title",
            "Title (or part of it) of the window for the window target",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "display_index",
            "Display Index",
            "Display for the display target",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "include_ax",
            "Accessibility Elements",
            "List elements from the accessibility tree",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "include_ocr",
            "OCR Text",
            "Add recognized text that no accessibility element already covers",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "interactive_only",
            "Interactive Only",
            "Only list controls that can be clicked, typed into or toggled",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "max_elements",
            "Max Elements",
            "Upper bound on listed elements (1–500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(150)))
        .set_options(
            PinOptions::new()
                .set_range((1.0, 500.0))
                .set_step(1.0)
                .build(),
        );
        node.add_input_pin(
            "languages",
            "OCR Languages",
            "Comma-separated OCR languages such as en-US, de; empty lets the OS engine choose",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
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
            "Screenshot of the target",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "annotated",
            "Annotated Image",
            "Screenshot with a numbered box on every listed element",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "frame",
            "Frame",
            "Desktop rectangle and pixel size of both images",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();
        node.add_output_pin(
            "elements",
            "Elements",
            "Listed elements with role, name, value, states and desktop bounds",
            VariableType::Struct,
        )
        .set_schema::<ScreenElement>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "summary",
            "Summary",
            "One line per element for a language model, e.g. [3] button \"Save\" @ (812,433)",
            VariableType::String,
        );
        node.add_output_pin(
            "generation",
            "Generation",
            "Identifies this capture; Click Screen Element rejects ids from older captures",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let target: String = context.evaluate_pin("target").await?;
        let window_title: String = context.evaluate_pin("window_title").await?;
        let display_index: i64 = context.evaluate_pin("display_index").await?;
        let include_ax: bool = context.evaluate_pin("include_ax").await?;
        let include_ocr: bool = context.evaluate_pin("include_ocr").await?;
        let interactive_only: bool = context.evaluate_pin("interactive_only").await?;
        let max_elements: i64 = context.evaluate_pin("max_elements").await?;
        let languages =
            super::ocr::parse_languages(&context.evaluate_pin::<String>("languages").await?);
        if !(1..=500).contains(&max_elements) {
            return Err(flow_like_types::anyhow!(
                "Max elements must be between 1 and 500, got {}",
                max_elements
            ));
        }

        let window = resolve_target(&target, &window_title).await?;
        let (picture, frame) = capture_target(window.as_ref(), display_index).await?;
        let mut warnings = Vec::new();
        let ax = if include_ax {
            accessibility_candidates(window.as_ref(), &frame, interactive_only, &mut warnings).await
        } else {
            Vec::new()
        };
        let ocr = if include_ocr {
            text_candidates(&picture, &frame, languages, &ax, &mut warnings).await
        } else {
            Vec::new()
        };
        let candidates = assemble(ax, ocr, max_elements as usize);
        let marks: Vec<(u32, PixelBox)> = candidates
            .iter()
            .filter_map(|c| {
                Some((
                    c.element.id,
                    super::zoom::pixel_crop(&frame, &c.element.bbox)?,
                ))
            })
            .collect();
        let annotated = {
            let picture = picture.clone();
            tokio::task::spawn_blocking(move || annotate(picture, &marks)).await?
        };

        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let elements: Vec<ScreenElement> = candidates.iter().map(|c| c.element.clone()).collect();
        let header = format!(
            "Screen state {generation}: {} at ({},{}) {}x{}, {} elements. Act on an element by its [id].",
            describe_target(window.as_ref(), display_index),
            frame.x,
            frame.y,
            frame.width,
            frame.height,
            elements.len()
        );
        let summary = render_summary(&header, &warnings, &elements);
        context
            .set_cache(
                &state_key(&session.session_ref),
                std::sync::Arc::new(ScreenState {
                    generation,
                    elements: candidates,
                }),
            )
            .await;

        let image = NodeImage::new(context, image::DynamicImage::ImageRgba8(picture)).await;
        let annotated = NodeImage::new(context, image::DynamicImage::ImageRgba8(annotated)).await;
        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("image", json!(image)).await?;
        context.set_pin_value("annotated", json!(annotated)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        context.set_pin_value("elements", json!(elements)).await?;
        context.set_pin_value("summary", json!(summary)).await?;
        context
            .set_pin_value("generation", json!(generation as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;
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
pub struct ComputerClickElementNode;

impl ComputerClickElementNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerClickElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_click_element",
            "Click Screen Element",
            "Clicks an element by its id from the latest Capture Screen State of this session, through the accessibility action when possible, otherwise with the mouse at its center",
            "Automation/Computer/Mouse",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "clickElement");
        node.add_icon("/flow/icons/computer.svg");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_input_pin(
            "element_id",
            "Element ID",
            "The [id] from the screen state",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "generation",
            "Generation",
            "Generation of the screen state the id comes from; 0 accepts the latest capture",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin("button", "Button", "Mouse button", VariableType::String)
            .set_default_value(Some(json!("left")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(
                        ["left", "right", "middle"]
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                    )
                    .build(),
            );
        node.add_input_pin(
            "double",
            "Double Click",
            "Click twice",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "prefer_accessibility_action",
            "Prefer Accessibility Action",
            "Press accessibility elements through the accessibility API (works when covered or off-focus) for single left clicks; falls back to the mouse",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "▶", "Clicked", VariableType::Execution);
        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_output_pin(
            "element",
            "Element",
            "The clicked element",
            VariableType::Struct,
        )
        .set_schema::<ScreenElement>();
        node.add_output_pin(
            "method",
            "Method",
            "accessibility or mouse",
            VariableType::String,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let id: i64 = context.evaluate_pin("element_id").await?;
        let generation: i64 = context.evaluate_pin("generation").await?;
        let button: String = context.evaluate_pin("button").await?;
        let double: bool = context.evaluate_pin("double").await?;
        let prefer_accessibility: bool =
            context.evaluate_pin("prefer_accessibility_action").await?;

        let candidate = latest_element(context, &session.session_ref, id, generation).await?;
        let pressed = prefer_accessibility
            && !double
            && button.eq_ignore_ascii_case("left")
            && press_accessibly(context, &candidate).await;
        let method = if pressed { "accessibility" } else { "mouse" };
        if method == "mouse" {
            click_point(context, &session, candidate.element.center, &button, double).await?;
        }
        session.apply_delay(context).await?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("element", json!(candidate.element))
            .await?;
        context.set_pin_value("method", json!(method)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::accessibility::{AccessibilityBounds, normalize_tree};

    fn rect(x: i32, y: i32, width: u32, height: u32) -> InputRect {
        InputRect {
            x,
            y,
            width,
            height,
        }
    }

    fn ax_node(role: &str, name: Option<&str>, bounds: (i32, i32, i32, i32)) -> AccessibilityNode {
        let mut node = AccessibilityNode {
            role: role.into(),
            name: name.map(str::to_string),
            bounds: Some(AccessibilityBounds {
                x: bounds.0,
                y: bounds.1,
                width: bounds.2,
                height: bounds.3,
            }),
            actions: vec!["AXPress".into()],
            native_id: Some("{}".into()),
            ..Default::default()
        };
        normalize_tree(&mut node);
        node
    }

    fn ocr_line(text: &str, bbox: InputRect) -> OcrLine {
        OcrLine {
            text: text.into(),
            confidence: Some(0.9),
            bbox_px: PixelBox::default(),
            bbox: Some(bbox),
            center: Some(bbox.center()),
            words: vec![],
        }
    }

    #[test]
    fn ax_elements_are_clipped_filtered_and_deduplicated() {
        let visible = rect(0, 0, 1000, 800);
        let nodes = vec![
            ax_node("AXWindow", Some("Doc"), (0, 0, 1000, 800)),
            ax_node("AXGroup", None, (100, 100, 80, 24)),
            ax_node("AXButton", Some("Save"), (100, 100, 80, 24)),
            ax_node("AXButton", Some("Off"), (2000, 0, 50, 20)),
            ax_node("AXButton", Some("Edge"), (980, 790, 40, 40)),
        ];
        let candidates = dedup_ax(ax_candidates(nodes, &visible, true));
        let names: Vec<_> = candidates
            .iter()
            .map(|c| c.element.name.as_deref().unwrap_or("-"))
            .collect();
        assert_eq!(names, ["Save", "Edge"]);
        assert_eq!(candidates[1].element.bbox, rect(980, 790, 20, 10));
        assert_eq!(candidates[1].element.center, InputPoint { x: 990, y: 795 });
        assert!(candidates[0].invokable);
    }

    #[test]
    fn ocr_text_inside_a_matching_control_is_not_repeated() {
        let ax = vec![
            (rect(100, 100, 80, 24), loose("Save")),
            (rect(0, 300, 600, 30), loose("Search")),
        ];
        assert!(ocr_covered(&rect(120, 106, 40, 12), "Save", &ax));
        assert!(!ocr_covered(&rect(10, 308, 120, 14), "hello world", &ax));
        assert!(ocr_covered(&rect(10, 308, 60, 14), "Search", &ax));
        assert!(!ocr_covered(&rect(400, 400, 60, 14), "Save", &ax));
    }

    #[test]
    fn assemble_prefers_ax_and_numbers_in_reading_order() {
        let visible = rect(0, 0, 1000, 800);
        let ax = ax_candidates(
            vec![
                ax_node("AXButton", Some("Later"), (500, 400, 80, 24)),
                ax_node("AXButton", Some("First"), (10, 10, 80, 24)),
            ],
            &visible,
            true,
        );
        let lines = [
            ocr_line("Heading", rect(200, 12, 100, 20)),
            ocr_line("Later", rect(510, 406, 60, 12)),
        ];
        let ocr = ocr_candidates(&lines, &ax);
        assert_eq!(ocr.len(), 1);
        let all = assemble(ax.clone(), ocr.clone(), 10);
        let ids: Vec<_> = all
            .iter()
            .map(|c| (c.element.id, c.element.name.clone().unwrap_or_default()))
            .collect();
        assert_eq!(
            ids,
            [
                (1, "First".to_string()),
                (2, "Heading".to_string()),
                (3, "Later".to_string())
            ]
        );
        assert_eq!(assemble(ax, ocr, 2).len(), 2);
    }

    #[test]
    fn summary_lines_are_compact() {
        let element = ScreenElement {
            id: 3,
            source: "ax".into(),
            role: "text_field".into(),
            name: Some("Search \"docs\"".into()),
            value: Some("flow".into()),
            states: vec!["enabled".into(), "focused".into()],
            bbox: rect(800, 420, 24, 26),
            center: InputPoint { x: 812, y: 433 },
        };
        assert_eq!(
            summary_line(&element),
            r#"[3] text_field "Search \"docs\"" value="flow" [focused] @ (812,433)"#
        );
        let button = ScreenElement {
            role: "button".into(),
            name: Some("Save".into()),
            value: None,
            states: vec![],
            ..element
        };
        let summary = render_summary("Screen state 1", &["OCR off".into()], &[button]);
        assert_eq!(
            summary,
            "Screen state 1\n! OCR off\n[3] button \"Save\" @ (812,433)"
        );
    }

    #[test]
    fn labels_stay_inside_and_do_not_overlap() {
        let size = label_size(12, 2);
        assert_eq!(size, (26, 18));
        let top = PixelBox {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let first = place_label(&top, size, (400, 300), &[]);
        assert_eq!((first.x, first.y), (0, 0));
        let second = place_label(&top, size, (400, 300), &[first]);
        assert!(!second.intersects(&first));
        assert!(second.x + second.width <= 400 && second.y + second.height <= 300);
        let middle = PixelBox {
            x: 50,
            y: 100,
            width: 40,
            height: 20,
        };
        let above = place_label(&middle, size, (400, 300), &[]);
        assert_eq!((above.x, above.y), (50, 82));
    }

    #[test]
    fn annotation_draws_outline_and_number() {
        let image = image::RgbaImage::from_pixel(200, 120, image::Rgba([255, 255, 255, 255]));
        let area = PixelBox {
            x: 40,
            y: 50,
            width: 60,
            height: 30,
        };
        let marked = annotate(image, &[(7, area)]);
        let red = image::Rgba([230, 25, 75, 255]);
        assert_eq!(*marked.get_pixel(40, 65), red);
        assert_eq!(*marked.get_pixel(99, 79), red);
        assert_eq!(*marked.get_pixel(70, 65), image::Rgba([255, 255, 255, 255]));
        let label = place_label(&area, label_size(7, 2), (200, 120), &[]);
        let ink = (label.y..label.y + label.height)
            .flat_map(|y| (label.x..label.x + label.width).map(move |x| (x, y)))
            .filter(|(x, y)| *marked.get_pixel(*x, *y) == image::Rgba([255, 255, 255, 255]))
            .count();
        assert!(ink > 10);
    }
}

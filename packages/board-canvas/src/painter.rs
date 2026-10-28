use crate::aura::{AURA_MARGIN, Aura, HeaderStatus, MAX_FLOWS};
use crate::geometry::{Curve, CurveKey, Detail, dom_baseline, header_outline};
use crate::model::{
    ConnectionKind, Direction, Edge, Endpoint, Header, Latch, Node, Overlay, PaintLayer, Palette,
    Pin, PinEditor, PositionUpdate, Rgba, Scene, SceneIndex, Trace,
};
use crate::status::{NodeStatus, RunState, StatusUpdate, Statuses};
use crate::text_cache::{TextCache, ZoomQuality};
use crate::trace_paint::{Tracing, paint_traced_wires};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc, time::Duration};
use wasm_bindgen::prelude::*;

const PULSE_DURATION_MS: f64 = 1_200.0;
const PULSE_FRAME_INTERVAL: Duration = Duration::from_millis(42);
const MAX_PULSE_EDGES: usize = 32;
/// About 30 frames a second while a visible node runs or finishes.
const STATUS_FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// Tailwind's violet-500 in sRGB, for hosts that send no header style.
const FUNCTION_HEADER: Rgba = [141, 84, 255, 255];
const WORKFLOW_GLYPH: &str = "glyph:workflow";
const VARIABLE_GLYPH: &str = "glyph:variable";
const CHECK_GLYPH: &str = "glyph:check";
const CHEVRON_DOWN_GLYPH: &str = "glyph:chevron-down";
const CHEVRON_UP_GLYPH: &str = "glyph:chevron-up";
/// The title's <small> keeps xyflow's 12px node text at 80%; shrunk titles sit on its baseline.
const TITLE_STRUT: f32 = 9.6;

/// The top-left position that puts `galley`'s first baseline at `baseline`, in graph units.
fn baseline_top(galley: &egui::Galley, baseline: f32, zoom: f32) -> f32 {
    let row = galley.rows.first();
    let offset = row
        .and_then(|row| Some(row.pos.y + row.row.glyphs.first()?.pos.y))
        .unwrap_or(galley.size().y * 0.5);
    baseline - offset / zoom
}

pub(crate) fn color(value: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(value[0], value[1], value[2], value[3])
}

pub(crate) fn alpha(value: Rgba, opacity: f32) -> Color32 {
    color([
        value[0],
        value[1],
        value[2],
        (value[3] as f32 * opacity) as u8,
    ])
}

pub(crate) fn mix(a: Rgba, b: Rgba, amount: f32) -> Rgba {
    std::array::from_fn(|i| (a[i] as f32 * (1.0 - amount) + b[i] as f32 * amount) as u8)
}

fn medium_family(medium_font: bool) -> egui::FontFamily {
    if medium_font {
        egui::FontFamily::Name("Inter Medium".into())
    } else {
        egui::FontFamily::Proportional
    }
}

fn error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}

pub(crate) type NodeText = TextCache<Arc<egui::Galley>>;

pub(crate) struct NodeIcon {
    texture: egui::TextureHandle,
    /// Monochrome icons arrive as white masks and take the node's text color.
    tint: bool,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    frames: u64,
    last_paint_cpu_ms: f64,
    paint_cpu_total_ms: f64,
    rendered_nodes: usize,
    rendered_edges: usize,
    node_count: usize,
    edge_count: usize,
    unresolved_edges: usize,
    geometry_rebuilds: u64,
    viewport: [f32; 3],
    lod_enabled: bool,
    lod_level: &'static str,
    labels_drawn: usize,
    text_layout_count: u64,
    pins_drawn: usize,
    edge_segments_drawn: usize,
    paint_layer: &'static str,
    activation_pulse: bool,
    pulse_edges_drawn: usize,
    pulse_count: u64,
    status_nodes: usize,
    comets_drawn: usize,
    flows_drawn: usize,
    status_animating: bool,
    trace_wires_drawn: usize,
    trace_previews_drawn: usize,
    trace_focused: bool,
}

struct Board {
    canvas: web_sys::HtmlCanvasElement,
    scene: Scene,
    index: SceneIndex,
    palette: Palette,
    curves: Vec<Option<Curve>>,
    text: Vec<NodeText>,
    icons: HashMap<String, NodeIcon>,
    text_zoom: ZoomQuality,
    overlay: Overlay,
    overlay_curve: Option<Curve>,
    viewport: [f32; 3],
    lod: bool,
    layer: PaintLayer,
    medium_font: bool,
    destroyed: bool,
    pulse_started: Option<f64>,
    status: Statuses,
    tracing: Tracing,
    ctx: Option<egui::Context>,
    stats: Stats,
}

impl Board {
    fn wake(&self) {
        if !self.destroyed
            && let Some(ctx) = &self.ctx
        {
            ctx.request_repaint();
        }
    }

    fn ensure_alive(&self) -> Result<(), JsValue> {
        if self.destroyed {
            Err(error("Board canvas has been destroyed"))
        } else {
            Ok(())
        }
    }

    fn clear_text(&mut self) {
        self.text = (0..self.scene.nodes.len())
            .map(|_| NodeText::default())
            .collect();
        self.tracing.clear_text();
    }

    fn replace_scene(&mut self, scene: Scene, index: SceneIndex) {
        let mut previous: HashMap<_, _> = self
            .scene
            .edges
            .iter()
            .map(|edge| edge.id.clone())
            .zip(std::mem::take(&mut self.curves))
            .collect();
        self.curves = scene
            .edges
            .iter()
            .map(|edge| {
                let key = CurveKey::for_edge(edge, &scene, &index)?;
                if let Some(Some(curve)) = previous.remove(&edge.id)
                    && curve.key == key
                {
                    return Some(curve);
                }
                self.stats.geometry_rebuilds += 1;
                Some(Curve::new(key))
            })
            .collect();
        self.stats.node_count = scene.nodes.len();
        self.stats.edge_count = scene.edges.len();
        self.stats.unresolved_edges = self.curves.iter().filter(|curve| curve.is_none()).count();
        self.scene = scene;
        self.index = index;
        self.tracing.state.resolve(&self.scene, &self.index);
        self.clear_text();
        self.wake();
    }

    /// Wires and their decorative activation sweep. Traced wires are drawn later, on top.
    fn paint_edges(
        &self,
        painter: &egui::Painter,
        view: Rect,
        transform: impl Fn([f32; 2]) -> Pos2,
        detail: Detail,
        pulse: Option<f32>,
    ) -> EdgeCounts {
        if self.layer == PaintLayer::Nodes {
            return EdgeCounts::default();
        }
        let zoom = self.viewport[2];
        let mut pulse_edges_drawn = 0;
        let pulse_stride = self.stats.rendered_edges.div_ceil(MAX_PULSE_EDGES).max(1);
        let mut rendered_edges = 0;
        let mut edge_segments_drawn = 0;
        let fade = self.tracing.state.unlit_opacity();
        for (position, (edge, curve)) in self.scene.edges.iter().zip(&self.curves).enumerate() {
            let Some(curve) = curve.as_ref().filter(|curve| {
                !edge.dom && !self.tracing.state.lit(position) && curve_rect(curve).intersects(view)
            }) else {
                continue;
            };
            rendered_edges += 1;
            let points: Vec<_> = curve
                .samples(detail.simplify_edges() && !edge.selected, zoom)
                .iter()
                .map(|point| transform(*point))
                .collect();
            edge_segments_drawn += points.len().saturating_sub(1);
            paint_wire(painter, points, edge, detail, zoom, fade);
            if let Some(progress) =
                pulse.filter(|_| self.sweeps(rendered_edges, pulse_edges_drawn, pulse_stride))
            {
                let tint = mix(edge.color, self.palette.primary, 0.6);
                pulse_edges_drawn += usize::from(paint_sweep(
                    painter, curve, &transform, progress, tint, zoom,
                ));
            }
        }
        EdgeCounts {
            rendered: rendered_edges,
            segments: edge_segments_drawn,
            sweeps: pulse_edges_drawn,
        }
    }

    /// Whether the activation sweep runs along the `rendered`th wire: every `stride`th, until
    /// `drawn` reaches the budget.
    fn sweeps(&self, rendered: usize, drawn: usize, stride: usize) -> bool {
        drawn < MAX_PULSE_EDGES
            && (rendered + self.stats.pulse_count as usize).is_multiple_of(stride)
    }

    /// The run's flows and the traced wires, over the other wires on the edge plane. Returns
    /// how many of each were drawn and whether they animate.
    fn paint_lit_wires(
        &self,
        aura: &Aura,
        view: Rect,
        transform: impl Fn([f32; 2]) -> Pos2 + Copy,
    ) -> (usize, usize, bool) {
        if self.layer == PaintLayer::Nodes {
            return (0, 0, false);
        }
        let (scene, curves) = (&self.scene, &self.curves);
        let (flows, flowing) = paint_flows(aura, scene, curves, &self.status, view, transform);
        let (traced, tracing) =
            paint_traced_wires(aura, scene, curves, &self.tracing.state, view, transform);
        (flows, traced, flowing | tracing)
    }

    /// The box selection, the wire being dragged and the hovered pin's ring.
    fn paint_overlay(&self, painter: &egui::Painter, transform: impl Fn([f32; 2]) -> Pos2) {
        let zoom = self.viewport[2];
        if let Some(rect) = self.overlay.selection_rect {
            let a = transform([rect[0], rect[1]]);
            let b = transform([rect[0] + rect[2], rect[1] + rect[3]]);
            painter.rect(
                Rect::from_two_pos(a, b),
                0.0,
                alpha(self.palette.primary, 0.1),
                Stroke::new(1.0, alpha(self.palette.primary, 0.8)),
                egui::StrokeKind::Inside,
            );
        }
        if let (Some(connection), Some(curve)) = (&self.overlay.connection, &self.overlay_curve) {
            let tint = if connection.valid == Some(false) {
                [239, 68, 68, 255]
            } else {
                connection.color
            };
            painter.add(Shape::line(
                curve.points.iter().map(|point| transform(*point)).collect(),
                Stroke::new((2.0 * zoom).max(1.0), color(tint)),
            ));
        }
        if let Some(hovered) = &self.overlay.hovered_pin {
            let endpoint = Endpoint {
                node_id: hovered.node_id.clone(),
                pin_id: hovered.pin_id.clone(),
            };
            if let Some((node, pin)) = self.index.endpoint(&self.scene, &endpoint) {
                let center = transform([node.x + pin.x, node.y + pin.y]);
                let tint = if hovered.valid {
                    self.palette.primary
                } else {
                    [239, 68, 68, 255]
                };
                painter.circle_stroke(center, (7.0 * zoom).max(5.0), Stroke::new(1.5, color(tint)));
            }
        }
    }

    fn paint(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let painter = ui.painter().with_clip_rect(area);
        let [offset_x, offset_y, zoom] = self.viewport;
        let origin = area.min + vec2(offset_x, offset_y);
        let transform = |point: [f32; 2]| origin + vec2(point[0], point[1]) * zoom;
        let view = Rect::from_min_max(
            pos2(-offset_x / zoom, -offset_y / zoom),
            pos2(
                (area.width() - offset_x) / zoom,
                (area.height() - offset_y) / zoom,
            ),
        );
        let detail = Detail::for_zoom(self.lod, zoom);
        let text_settle_delay = self.text_zoom.settle_delay_ms(now());
        painter.rect_filled(area, 0.0, color(self.palette.background));

        let pulse = self.pulse_progress();
        let edges = self.paint_edges(&painter, view, transform, detail, pulse);
        self.finish_pulse(ui, pulse, &edges);
        let now_ms = now();
        self.status.prune(now_ms);
        let aura = Aura {
            painter: &painter,
            palette: &self.palette,
            icons: &self.icons,
            zoom,
            now: now_ms,
            motion: self.status.motion(),
            medium: medium_family(self.medium_font),
        };
        // Over the wires on the edge plane, so comments and nodes stack above the run's path.
        let (flows, traced, wiring) = self.paint_lit_wires(&aura, view, transform);
        let nodes = if self.layer == PaintLayer::Edges {
            NodeCounts::default()
        } else {
            paint_nodes(
                &aura,
                &self.scene.nodes,
                &mut self.text,
                &self.status,
                view,
                transform,
                detail,
                text_settle_delay.is_none(),
            )
        };
        let marking = self.layer != PaintLayer::Edges
            && self.tracing.paint_marks(
                &aura,
                &self.scene,
                &self.index,
                &self.curves,
                area,
                transform,
            );
        let animating = wiring | nodes.animating | marking;
        if animating {
            repaint_after(ui, STATUS_FRAME_INTERVAL);
        }
        if nodes.labels > 0
            && let Some(delay_ms) = text_settle_delay
        {
            // Restore exact font hinting once zoom stops, without keeping idle frames alive.
            repaint_after(ui, Duration::from_secs_f64(delay_ms / 1000.0));
        }
        if self.layer != PaintLayer::Edges {
            self.paint_overlay(&painter, transform);
        }
        self.record(detail, &edges, flows, &nodes, animating);
        self.stats.trace_wires_drawn = traced;
        self.stats.trace_previews_drawn = self.tracing.previews_drawn();
        self.stats.trace_focused = self.tracing.state.focused();
    }

    /// Each decorative sweep is finite and uses cached curves with a fixed edge budget.
    fn pulse_progress(&mut self) -> Option<f32> {
        let pulse = self.pulse_started.and_then(|started| {
            let progress = ((now() - started) / PULSE_DURATION_MS) as f32;
            (progress < 1.0).then_some(progress.clamp(0.0, 1.0))
        });
        if pulse.is_none() {
            self.pulse_started = None;
        }
        pulse
    }

    fn finish_pulse(&mut self, ui: &egui::Ui, pulse: Option<f32>, edges: &EdgeCounts) {
        self.stats.activation_pulse = pulse.is_some() && edges.rendered > 0;
        self.stats.pulse_edges_drawn = edges.sweeps;
        if self.stats.activation_pulse {
            repaint_after(ui, PULSE_FRAME_INTERVAL);
        } else if edges.rendered == 0 {
            self.pulse_started = None;
        }
    }

    fn record(
        &mut self,
        detail: Detail,
        edges: &EdgeCounts,
        flows: usize,
        nodes: &NodeCounts,
        animating: bool,
    ) {
        let stats = &mut self.stats;
        stats.rendered_nodes = nodes.rendered;
        stats.rendered_edges = edges.rendered;
        stats.viewport = self.viewport;
        stats.lod_enabled = self.lod;
        stats.lod_level = detail.name();
        stats.labels_drawn = nodes.labels;
        stats.text_layout_count += nodes.layouts;
        stats.pins_drawn = nodes.pins;
        stats.status_nodes = nodes.statuses;
        stats.comets_drawn = nodes.comets;
        stats.flows_drawn = flows;
        stats.status_animating = animating;
        stats.edge_segments_drawn = edges.segments;
        stats.paint_layer = self.layer.name();
    }
}

/// egui subtracts its frame prediction from a delayed repaint; adding it back keeps the delay.
fn repaint_after(ui: &egui::Ui, delay: Duration) {
    let prediction =
        Duration::try_from_secs_f32(ui.input(|input| input.predicted_dt)).unwrap_or_default();
    ui.ctx().request_repaint_after(delay + prediction);
}

/// What one frame's edge pass drew.
#[derive(Default)]
struct EdgeCounts {
    rendered: usize,
    segments: usize,
    sweeps: usize,
}

/// The selected or plain variant of a stroke value.
fn pick(selected: bool, on: f32, off: f32) -> f32 {
    if selected { on } else { off }
}

/// One wire in its kind's stroke: execution solid, references glowing, data dotted. `fade`
/// scales its opacity.
pub(crate) fn paint_wire(
    painter: &egui::Painter,
    points: Vec<Pos2>,
    edge: &Edge,
    detail: Detail,
    zoom: f32,
    fade: f32,
) {
    match edge.kind {
        ConnectionKind::Execution => {
            let selected = edge.selected;
            painter.add(Shape::line(
                points,
                Stroke::new(
                    pick(selected, 3.0, 2.0) * zoom,
                    alpha(edge.color, pick(selected, 0.9, 0.7) * fade),
                ),
            ));
        }
        ConnectionKind::Reference => {
            paint_reference_wire(painter, points, edge, detail, zoom, fade)
        }
        ConnectionKind::Data => paint_data_wire(painter, points, edge, detail, zoom, fade),
    }
}

fn paint_reference_wire(
    painter: &egui::Painter,
    points: Vec<Pos2>,
    edge: &Edge,
    detail: Detail,
    zoom: f32,
    fade: f32,
) {
    let (selected, base) = (edge.selected, edge.color);
    if !detail.simplify_edges() {
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(
                pick(selected, 8.0, 5.0) * zoom,
                alpha(base, pick(selected, 0.08, 0.04) * fade),
            ),
        ));
    }
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(
            pick(selected, 2.5, 1.5) * zoom,
            alpha(base, pick(selected, 0.5, 0.3) * fade),
        ),
    ));
    if !detail.simplify_edges() {
        painter.add(Shape::line(
            points,
            Stroke::new(
                pick(selected, 1.2, 0.8) * zoom,
                alpha(mix(base, [255; 4], 0.2), pick(selected, 0.7, 0.4) * fade),
            ),
        ));
    }
}

fn paint_data_wire(
    painter: &egui::Painter,
    points: Vec<Pos2>,
    edge: &Edge,
    detail: Detail,
    zoom: f32,
    fade: f32,
) {
    let (selected, base) = (edge.selected, edge.color);
    if detail.simplify_edges() && !selected {
        painter.add(Shape::line(
            points,
            Stroke::new(1.5 * zoom, alpha(base, 0.35 * fade)),
        ));
        return;
    }
    // Dotted at rest; solid once selected, so it stands apart from the wires around it.
    let stroke = |points: &[Pos2], stroke: Stroke| {
        if selected {
            painter.add(Shape::line(points.to_vec(), stroke));
        } else {
            painter.extend(Shape::dashed_line(points, stroke, 1.5 * zoom, 4.0 * zoom));
        }
    };
    stroke(
        &points,
        Stroke::new(
            pick(selected, 2.5, 1.5) * zoom,
            alpha(base, pick(selected, 0.5, 0.35) * fade),
        ),
    );
    if !edge.lite {
        stroke(
            &points,
            Stroke::new(
                pick(selected, 1.5, 1.0) * zoom,
                alpha(mix(base, [255; 4], 0.3), pick(selected, 0.8, 0.5) * fade),
            ),
        );
    }
}

/// The activation sweep's head and short trail at `progress` along the wire. Returns whether
/// it was on screen.
fn paint_sweep(
    painter: &egui::Painter,
    curve: &Curve,
    transform: impl Fn([f32; 2]) -> Pos2,
    progress: f32,
    tint: Rgba,
    zoom: f32,
) -> bool {
    let opacity = (std::f32::consts::PI * progress).sin();
    let position = progress * (curve.points.len() - 1) as f32;
    let first = (position.floor() as usize).min(curve.points.len() - 2);
    let fraction = position - first as f32;
    let a = transform(curve.points[first]);
    let b = transform(curve.points[first + 1]);
    let head = a.lerp(b, fraction);
    // Clip each trail to the visible curve; off-screen edges add no shapes.
    if !painter.clip_rect().expand(12.0).contains(head) {
        return false;
    }
    let mut trail: Vec<_> = curve.points[first.saturating_sub(2)..=first]
        .iter()
        .map(|point| transform(*point))
        .collect();
    trail.push(head);
    let width = (2.0 * zoom).clamp(1.0, 2.5);
    painter.add(Shape::line(
        trail.clone(),
        Stroke::new(width + 5.0, alpha(tint, opacity * 0.10)),
    ));
    painter.add(Shape::line(
        trail,
        Stroke::new(width, alpha(tint, opacity * 0.65)),
    ));
    painter.circle_filled(head, width * 1.1, alpha(tint, opacity * 0.85));
    true
}

/// What one frame's node pass drew.
#[derive(Default)]
struct NodeCounts {
    rendered: usize,
    labels: usize,
    layouts: u64,
    pins: usize,
    statuses: usize,
    comets: usize,
    animating: bool,
}

/// Node bodies with their run marks. A node the DOM renders keeps its marks on the canvas.
#[allow(clippy::too_many_arguments)]
fn paint_nodes(
    aura: &Aura,
    nodes: &[Node],
    texts: &mut [NodeText],
    status: &Statuses,
    view: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
    detail: Detail,
    text_settled: bool,
) -> NodeCounts {
    let mut counts = NodeCounts::default();
    for (node, text) in nodes.iter().zip(texts) {
        let node_status = status.get(&node.id);
        let ripple = status.ripple(&node.id, aura.now);
        let decorated = node_status.is_some() || ripple.is_some() || node.long_running;
        if node.dom && !decorated {
            continue;
        }
        let reach = if decorated { AURA_MARGIN } else { 6.0 };
        let Some(rect) = screen_rect(node, reach, view, &transform) else {
            continue;
        };
        // Keep selected content legible when the host is not rendering a DOM editor.
        let node_detail = if node.selected { Detail::Full } else { detail };
        if let Some(node_status) = node_status {
            counts.statuses += 1;
            aura.halos(node, rect, node_status);
        }
        if !node.dom {
            counts.rendered += 1;
            let (header, done) = body_status(aura, node_detail, node_status, status);
            let (labels, pins, layouts) = paint_node(
                aura,
                node,
                rect,
                text_settled,
                node_detail,
                text,
                header.as_ref(),
                done,
            );
            counts.labels += labels;
            counts.pins += pins;
            counts.layouts += layouts;
        }
        if decorated {
            counts.animating |= aura.marks(
                node,
                rect,
                node_detail,
                node_status,
                ripple,
                &mut counts.comets,
            );
        }
    }
    counts
}

/// The node's screen rectangle when it, or `reach` around it, is in view.
fn screen_rect(
    node: &Node,
    reach: f32,
    view: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> Option<Rect> {
    let bounds = Rect::from_min_size(pos2(node.x, node.y), vec2(node.width, node.height));
    bounds.expand(reach).intersects(view).then(|| {
        Rect::from_min_max(
            transform([node.x, node.y]),
            transform([node.x + node.width, node.y + node.height]),
        )
    })
}

/// The header chip and the fade that a node's run state gives its body.
fn body_status(
    aura: &Aura,
    detail: Detail,
    node_status: Option<&NodeStatus>,
    status: &Statuses,
) -> (Option<HeaderStatus>, bool) {
    let header = node_status
        .filter(|_| detail == Detail::Full)
        .and_then(|node_status| {
            let activity = node_status
                .run_id
                .as_deref()
                .and_then(|run| status.activity(run));
            HeaderStatus::new(aura, node_status, activity)
        });
    let done = node_status.is_some_and(|node_status| node_status.run == Some(RunState::Done));
    (header, done)
}

/// Lights the run's path on execution wires. Returns how many it lit and whether they move.
fn paint_flows(
    aura: &Aura,
    scene: &Scene,
    curves: &[Option<Curve>],
    status: &Statuses,
    view: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> (usize, bool) {
    let mut lit = 0;
    let mut moving = false;
    for (edge, curve) in scene.edges.iter().zip(curves) {
        if lit == MAX_FLOWS {
            break;
        }
        if edge.kind != ConnectionKind::Execution {
            continue;
        }
        let (Some(curve), Some(flow)) = (
            curve,
            status.flow(&edge.source.node_id, &edge.target.node_id),
        ) else {
            continue;
        };
        if !curve_rect(curve).expand(8.0).intersects(view) {
            continue;
        }
        let points: Vec<Pos2> = curve.points.iter().map(|point| transform(*point)).collect();
        moving |= aura.flow(&points, flow);
        lit += 1;
    }
    (lit, moving)
}

pub(crate) fn curve_rect(curve: &Curve) -> Rect {
    Rect::from_min_max(
        pos2(curve.bounds[0], curve.bounds[1]),
        pos2(curve.bounds[2], curve.bounds[3]),
    )
}

/// The fade and text colors of one node this frame.
struct NodeLook {
    opacity: f32,
    text_color: Rgba,
    foreground: Rgba,
}

impl NodeLook {
    fn new(node: &Node, palette: &Palette, done: bool) -> Self {
        let (text_color, foreground) = if node.dimmed {
            (palette.muted, palette.muted)
        } else {
            (palette.node_text(), palette.foreground)
        };
        Self {
            // A finished node fades like the DOM node's `opacity-60`.
            opacity: (if node.dimmed { 0.45 } else { 1.0 }) * (if done { 0.6 } else { 1.0 }),
            text_color,
            foreground,
        }
    }
}

/// The host's header tint, or the node kind's when it sent none.
fn header_style(node: &Node, palette: &Palette) -> Header {
    node.header.unwrap_or(Header {
        color: match node.kind.as_str() {
            "event" => palette.primary,
            "function" => FUNCTION_HEADER,
            "pure" => palette.tertiary,
            _ => palette.card,
        },
        reversed: false,
    })
}

/// Text is rasterized at `raster` zoom and drawn scaled by `display`.
#[derive(Clone, Copy)]
struct TextScale {
    raster: f32,
    display: f32,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_node(
    aura: &Aura,
    node: &Node,
    rect: Rect,
    text_settled: bool,
    detail: Detail,
    text: &mut NodeText,
    header_status: Option<&HeaderStatus>,
    done: bool,
) -> (usize, usize, u64) {
    let palette = aura.palette;
    let look = NodeLook::new(node, palette, done);
    if node.kind == "reroute" {
        return (0, paint_reroute(aura, node, rect, detail, look.opacity), 0);
    }
    let header = header_style(node, palette);
    if detail == Detail::Tiny {
        aura.painter.rect_filled(
            rect,
            0.0,
            alpha(mix(palette.card, header.color, 0.65), look.opacity),
        );
        return (0, 0, 0);
    }
    paint_frame(aura, node, rect, header, look.opacity);
    let zoom = aura.zoom;
    let raster = if detail.labels() {
        text.prepare(zoom, aura.painter.ctx().pixels_per_point(), text_settled)
    } else {
        zoom
    };
    let scale = TextScale {
        raster,
        display: zoom / raster,
    };
    // The header's content box is centered between its 4px padding and bottom border.
    let header_center = (node.header_height.min(node.height) + 1.0) * 0.5;
    let reserved = header_status.map_or(0.0, |header| header.reserved);
    let (mut labels, mut pins, mut layouts) = (0, 0, 0);
    if detail.labels() {
        layouts += paint_title(aura, node, rect, text, scale, &look, reserved);
        labels += 1;
    }
    if detail == Detail::Full {
        paint_node_icon(aura, node, rect, header_center, &look);
        if let Some(header) = header_status {
            header.paint(aura, rect.min, node, header_center, look.opacity);
        }
        layouts += layout_pin_text(aura, node, text, scale, &look);
    }
    if matches!(detail, Detail::Full | Detail::Compact) {
        let (pin_labels, painted) = paint_pins(aura, node, rect, detail, text, &look);
        labels += pin_labels;
        pins += painted;
    }
    if detail == Detail::Full
        && let Some(latch) = node.latch
    {
        paint_latch(
            aura,
            rect.min,
            node,
            latch,
            text.scaled_latch.as_ref(),
            look.opacity,
        );
    }
    (labels, pins, layouts)
}

/// Returns the handles drawn.
fn paint_reroute(aura: &Aura, node: &Node, rect: Rect, detail: Detail, opacity: f32) -> usize {
    let (palette, zoom) = (aura.palette, aura.zoom);
    let tint = node.pins.first().map_or(palette.border, |pin| pin.color);
    aura.painter.rect(
        rect,
        node.corner_radius() * zoom,
        alpha(if node.selected { tint } else { palette.card }, opacity),
        Stroke::new(zoom, alpha(tint, opacity)),
        egui::StrokeKind::Inside,
    );
    if !matches!(detail, Detail::Full | Detail::Compact) {
        return 0;
    }
    // Reroutes hide execution diamonds and retain colored data handles.
    let mut pins = 0;
    for pin in node
        .pins
        .iter()
        .filter(|pin| pin.kind != ConnectionKind::Execution)
    {
        paint_pin(
            aura,
            rect.min + vec2(pin.x, pin.y) * zoom,
            pin,
            opacity,
            PinStyle::for_detail(detail, node.lite),
        );
        pins += 1;
    }
    pins
}

/// The card, its header gradient and the rule under the header.
fn paint_frame(aura: &Aura, node: &Node, rect: Rect, header: Header, opacity: f32) {
    let (painter, palette, zoom) = (aura.painter, aura.palette, aura.zoom);
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let border = if node.selected {
        palette.primary
    } else {
        palette.border
    };
    painter.rect(
        rect,
        node.corner_radius() * zoom,
        alpha(palette.card, opacity),
        Stroke::new(zoom, alpha(border, opacity)),
        egui::StrokeKind::Inside,
    );
    let header_height = node.header_height.min(node.height);
    let mut mesh = egui::Mesh::default();
    let header_width = (node.width - 2.0).max(0.0);
    for [x, y] in header_outline(
        header_width,
        (header_height - 1.0).max(0.0),
        (node.corner_radius() - 1.0).max(0.0),
    ) {
        let ramp = x / header_width.max(1.0);
        let amount = if header.reversed { 1.0 - ramp } else { ramp };
        mesh.colored_vertex(
            point(x + 1.0, y + 1.0),
            alpha(mix(palette.card, header.color, amount), opacity),
        );
    }
    for index in 2..mesh.vertices.len() as u32 {
        mesh.add_triangle(0, index - 1, index);
    }
    painter.add(mesh);
    // The DOM header is a 16px box inside the card border, with its bottom border inside it.
    painter.rect_filled(
        Rect::from_min_max(
            point(1.0, header_height),
            point(node.width - 1.0, header_height + 1.0),
        ),
        0.0,
        alpha(palette.border, opacity),
    );
}

/// The title in the room the header's status chip leaves. Returns the layouts it made.
fn paint_title(
    aura: &Aura,
    node: &Node,
    rect: Rect,
    text: &mut NodeText,
    scale: TextScale,
    look: &NodeLook,
    reserved: f32,
) -> u64 {
    let (painter, zoom) = (aura.painter, aura.zoom);
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let header_height = node.header_height.min(node.height);
    let header_center = (header_height + 1.0) * 0.5;
    let room = (node.width - 26.0 - reserved).max(0.0);
    let mut layouts = 0;
    text.fit_title(room);
    let title = text.title.get_or_insert_with(|| {
        layouts += 1;
        layout_label(
            painter,
            node.label.clone(),
            FontId::new(node.label_font_size * scale.raster, aura.medium.clone()),
            alpha(look.text_color, look.opacity),
            room * scale.raster,
        )
    });
    let title = text
        .scaled_title
        .get_or_insert_with(|| scale_galley(title, scale.display));
    let baseline = dom_baseline(
        header_center - TITLE_STRUT * 0.625,
        TITLE_STRUT,
        TITLE_STRUT * 1.25,
        node.lite,
    );
    painter
        .with_clip_rect(painter.clip_rect().intersect(Rect::from_min_max(
            point(17.0, 0.0),
            point((node.width - 9.0 - reserved).max(17.0), header_height),
        )))
        .galley(
            point(17.0, baseline_top(title, baseline, zoom)),
            title.clone(),
            alpha(look.text_color, look.opacity),
        );
    layouts
}

/// The catalog icon in the header, the workflow glyph without one, or a vector fallback.
fn paint_node_icon(aura: &Aura, node: &Node, rect: Rect, header_center: f32, look: &NodeLook) {
    let (painter, zoom) = (aura.painter, aura.zoom);
    let slot = Rect::from_min_size(
        rect.min + vec2(5.0, header_center - 4.0) * zoom,
        vec2(8.0, 8.0) * zoom,
    );
    let painted = node.icon.as_deref().is_some_and(|key| {
        paint_glyph(
            painter,
            aura.icons,
            key,
            slot,
            alpha(look.foreground, look.opacity),
            false,
        )
    }) || paint_glyph(
        painter,
        aura.icons,
        WORKFLOW_GLYPH,
        slot,
        alpha(look.text_color, look.opacity),
        false,
    );
    if painted {
        return;
    }
    let stroke = Stroke::new(0.5 * zoom, alpha(look.text_color, look.opacity));
    let glyph = |x: f32, y: f32| slot.min + vec2(x, y) * zoom;
    painter.add(Shape::line(
        vec![
            glyph(2.33, 3.67),
            glyph(2.33, 5.0),
            glyph(3.0, 5.67),
            glyph(4.33, 5.67),
        ],
        stroke,
    ));
    for (x, y) in [(1.0, 1.0), (4.33, 4.33)] {
        painter.rect_stroke(
            Rect::from_min_size(glyph(x, y), vec2(2.67, 2.67) * zoom),
            0.67 * zoom,
            stroke,
            egui::StrokeKind::Middle,
        );
    }
}

/// Pin labels and a collapsed latch's count, laid out once per raster. Returns the layouts.
fn layout_pin_text(
    aura: &Aura,
    node: &Node,
    text: &mut NodeText,
    scale: TextScale,
    look: &NodeLook,
) -> u64 {
    let (painter, palette) = (aura.painter, aura.palette);
    let mut layouts = 0;
    if text.pins.len() != node.pins.len() {
        layouts += node.pins.len() as u64;
        text.pins = node
            .pins
            .iter()
            .map(|pin| {
                let color = match pin.editor {
                    Some(PinEditor::Select { placeholder: true }) => palette.muted,
                    _ => look.text_color,
                };
                layout_label(
                    painter,
                    pin.label.clone(),
                    FontId::proportional(pin.font_size * scale.raster),
                    alpha(color, look.opacity),
                    pin.label_max_width
                        .unwrap_or((node.width * 0.5 - 12.0).max(0.0))
                        * scale.raster,
                )
            })
            .collect();
    }
    layouts += layout_latch_count(aura, node, text, scale, look.opacity);
    if text.scaled_pins.len() != text.pins.len() {
        text.scaled_pins = text
            .pins
            .iter()
            .map(|galley| scale_galley(galley, scale.display))
            .collect();
    }
    if text.scaled_latch.is_none() {
        text.scaled_latch = text
            .latch
            .as_ref()
            .map(|galley| scale_galley(galley, scale.display));
    }
    layouts
}

/// The hidden-pin count on a collapsed latch. Returns the layouts it made.
fn layout_latch_count(
    aura: &Aura,
    node: &Node,
    text: &mut NodeText,
    scale: TextScale,
    opacity: f32,
) -> u64 {
    let Some(latch) = node
        .latch
        .filter(|latch| latch.collapsed && text.latch.is_none())
    else {
        return 0;
    };
    text.latch = Some(layout_label(
        aura.painter,
        latch.hidden.to_string(),
        FontId::new(7.0 * scale.raster, aura.medium.clone()),
        alpha(aura.palette.primary, opacity),
        f32::INFINITY,
    ));
    1
}

/// Handles, and at full detail their labels and inline editors. Returns labels and pins drawn.
fn paint_pins(
    aura: &Aura,
    node: &Node,
    rect: Rect,
    detail: Detail,
    text: &NodeText,
    look: &NodeLook,
) -> (usize, usize) {
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * aura.zoom;
    let mut labels = 0;
    for (index, pin) in node.pins.iter().enumerate() {
        paint_pin(
            aura,
            point(pin.x, pin.y),
            pin,
            look.opacity,
            PinStyle::for_detail(detail, node.lite),
        );
        if detail != Detail::Full {
            continue;
        }
        let label_end = paint_pin_label(aura, node, rect, pin, &text.scaled_pins[index], look);
        labels += 1;
        if pin.direction == Direction::Input
            && let Some(editor) = &pin.editor
        {
            paint_editor(aura, editor, point(label_end, pin.y), look.opacity);
        }
    }
    (labels, node.pins.len())
}

/// A pin's label beside its handle. Returns the label's right edge in graph units.
fn paint_pin_label(
    aura: &Aura,
    node: &Node,
    rect: Rect,
    pin: &Pin,
    galley: &Arc<egui::Galley>,
    look: &NodeLook,
) -> f32 {
    let (painter, zoom) = (aura.painter, aura.zoom);
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let width = galley.size().x / zoom;
    // Label text sits 8px right of an input's handle center and ends 6.8px left of an output's.
    let label_x = match pin.direction {
        Direction::Input => pin.x + 8.0,
        Direction::Output => pin.x - 6.8 - width,
    };
    let baseline = dom_baseline(
        pin.y - pin.font_size * 0.625,
        pin.font_size,
        pin.font_size * 1.25,
        node.lite,
    );
    let clip = Rect::from_min_max(point(0.0, pin.y - 8.0), point(node.width, pin.y + 8.0));
    painter
        .with_clip_rect(painter.clip_rect().intersect(clip))
        .galley(
            point(label_x, baseline_top(galley, baseline, zoom)),
            galley.clone(),
            alpha(look.text_color, look.opacity),
        );
    label_x + width
}

pub(crate) fn paint_glyph(
    painter: &egui::Painter,
    icons: &HashMap<String, NodeIcon>,
    key: &str,
    rect: Rect,
    color: Color32,
    flipped: bool,
) -> bool {
    let Some(icon) = icons.get(key) else {
        return false;
    };
    let uv = if flipped {
        Rect::from_min_max(pos2(1.0, 1.0), Pos2::ZERO)
    } else {
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
    };
    let tint = if icon.tint {
        color
    } else {
        Color32::WHITE.gamma_multiply(color.a() as f32 / 255.0)
    };
    painter.image(icon.texture.id(), rect, uv, tint);
    true
}

/// `end` is the label's right edge on the pin's center line, in screen space.
fn paint_editor(aura: &Aura, editor: &PinEditor, end: Pos2, opacity: f32) {
    let (palette, zoom) = (aura.palette, aura.zoom);
    match editor {
        PinEditor::Variable { set } => {
            let tint = if *set {
                palette.foreground
            } else {
                palette.primary
            };
            paint_glyph(
                aura.painter,
                aura.icons,
                VARIABLE_GLYPH,
                Rect::from_min_size(end + vec2(4.0, -3.6) * zoom, vec2(7.2, 7.2) * zoom),
                alpha(tint, opacity),
                false,
            );
        }
        PinEditor::Boolean { checked } => paint_checkbox(
            aura,
            Rect::from_min_size(end + vec2(4.0, -5.0) * zoom, vec2(10.0, 10.0) * zoom),
            *checked,
            opacity,
        ),
        PinEditor::Select { .. } => {
            paint_glyph(
                aura.painter,
                aura.icons,
                CHEVRON_DOWN_GLYPH,
                Rect::from_min_size(end + vec2(2.0, -4.0) * zoom, vec2(8.0, 8.0) * zoom),
                alpha(palette.foreground, opacity),
                false,
            );
        }
    }
}

fn paint_checkbox(aura: &Aura, rect: Rect, checked: bool, opacity: f32) {
    let (palette, zoom) = (aura.palette, aura.zoom);
    if checked {
        aura.painter
            .rect_filled(rect, 2.0 * zoom, alpha(palette.primary, opacity));
        paint_glyph(
            aura.painter,
            aura.icons,
            CHECK_GLYPH,
            rect.shrink(zoom),
            alpha(palette.primary_foreground(), opacity),
            false,
        );
    } else {
        aura.painter.rect(
            rect,
            2.0 * zoom,
            alpha(palette.checkbox(), opacity),
            Stroke::new(zoom, alpha(palette.input(), opacity)),
            egui::StrokeKind::Inside,
        );
    }
}

/// The strip's fill, its top rule and the chevron's ink.
fn latch_colors(palette: &Palette, collapsed: bool) -> (Rgba, Rgba, Rgba) {
    if collapsed {
        let fill = mix(palette.card, palette.primary, 0.1);
        (fill, mix(fill, palette.primary, 0.3), palette.primary)
    } else {
        let fill = mix(palette.card, palette.muted_background(), 0.9);
        (fill, mix(fill, palette.border, 0.7), palette.muted)
    }
}

fn paint_latch(
    aura: &Aura,
    origin: Pos2,
    node: &Node,
    latch: Latch,
    count: Option<&Arc<egui::Galley>>,
    opacity: f32,
) {
    let (painter, palette, zoom) = (aura.painter, aura.palette, aura.zoom);
    let point = |x: f32, y: f32| origin + vec2(x, y) * zoom;
    let top = node.height - 11.0;
    let (fill, rule, ink) = latch_colors(palette, latch.collapsed);
    let radius = ((node.corner_radius() - 1.0).max(0.0) * zoom).round() as u8;
    painter.rect_filled(
        Rect::from_min_max(point(1.0, top), point(node.width - 1.0, node.height - 1.0)),
        egui::CornerRadius {
            sw: radius,
            se: radius,
            ..Default::default()
        },
        alpha(fill, opacity),
    );
    painter.rect_filled(
        Rect::from_min_max(point(1.0, top), point(node.width - 1.0, top + 1.0)),
        0.0,
        alpha(rule, opacity),
    );
    let center = node.height - 5.5;
    let count = count.filter(|_| latch.collapsed);
    let width = 8.0 + count.map_or(0.0, |galley| 2.0 + galley.size().x / zoom);
    let left = (node.width - width) * 0.5;
    paint_glyph(
        painter,
        aura.icons,
        CHEVRON_UP_GLYPH,
        Rect::from_min_size(point(left, center - 4.0), vec2(8.0, 8.0) * zoom),
        alpha(ink, opacity),
        latch.collapsed,
    );
    if let Some(count) = count {
        painter.galley(
            point(
                left + 10.0,
                baseline_top(count, dom_baseline(center - 3.5, 7.0, 7.0, node.lite), zoom),
            ),
            count.clone(),
            alpha(ink, opacity),
        );
    }
}

fn scale_galley(galley: &Arc<egui::Galley>, scale: f32) -> Arc<egui::Galley> {
    if scale == 1.0 {
        return galley.clone();
    }
    let mut shape = egui::epaint::TextShape::new(Pos2::ZERO, galley.clone(), Color32::PLACEHOLDER);
    shape.transform(egui::emath::TSTransform::from_scaling(scale));
    shape.galley
}

fn layout_label(
    painter: &egui::Painter,
    label: String,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(label, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PinStyle {
    Glossy,
    Flat,
    /// WebKit's production pins: flat, with a background ring around data handles.
    Lite,
}

impl PinStyle {
    fn for_detail(detail: Detail, lite: bool) -> Self {
        match (detail, lite) {
            (Detail::Full, true) => Self::Lite,
            (Detail::Full, false) => Self::Glossy,
            _ => Self::Flat,
        }
    }
}

fn paint_pin(aura: &Aura, center: Pos2, pin: &Pin, opacity: f32, style: PinStyle) {
    let (painter, palette, zoom) = (aura.painter, aura.palette, aura.zoom);
    let base = pin.color;
    let tint = alpha(base, opacity);
    if let Some(container) = pin.container
        && aura.icons.contains_key(container.glyph())
    {
        // Container icons cover the handle with an 8px square of xyflow's node background.
        let slot = Rect::from_center_size(center + vec2(0.4, 0.0) * zoom, vec2(8.0, 8.0) * zoom);
        painter.rect_filled(slot, 0.0, alpha(palette.node_backdrop(), opacity));
        paint_glyph(painter, aura.icons, container.glyph(), slot, tint, false);
        return;
    }
    match pin.kind {
        ConnectionKind::Execution => paint_execution_pin(aura, center, base, opacity, style),
        ConnectionKind::Reference => {
            painter.rect_filled(
                Rect::from_center_size(center, vec2(12.0, 12.0) * zoom),
                2.0 * zoom,
                tint,
            );
            if style == PinStyle::Glossy {
                painter.rect_stroke(
                    Rect::from_center_size(center, vec2(10.0, 10.0) * zoom),
                    zoom,
                    Stroke::new(zoom, alpha(mix(base, [255; 4], 0.12), opacity)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ConnectionKind::Data => paint_data_pin(aura, center, base, opacity, style),
    }
}

fn paint_execution_pin(aura: &Aura, center: Pos2, base: Rgba, opacity: f32, style: PinStyle) {
    let (painter, zoom) = (aura.painter, aura.zoom);
    // An 8px square with a 1.5px corner radius, rotated 45°.
    let outline = diamond(center, 4.0, 1.5, zoom);
    if style == PinStyle::Glossy {
        paint_glow(
            painter,
            center,
            &outline,
            5.0 * zoom,
            alpha(base, opacity * 0.125),
        );
    }
    painter.add(Shape::convex_polygon(
        outline,
        alpha(base, opacity),
        Stroke::NONE,
    ));
    if style == PinStyle::Glossy {
        // Chromium antialiases the rotated border and background separately, which
        // leaves a one-pixel seam inside the edge.
        painter.add(Shape::closed_line(
            diamond(center, 3.0, 0.0, zoom),
            Stroke::new(
                1.0 / painter.ctx().pixels_per_point(),
                alpha(mix(base, aura.palette.card, 0.12), opacity),
            ),
        ));
    }
}

fn paint_data_pin(aura: &Aura, center: Pos2, base: Rgba, opacity: f32, style: PinStyle) {
    let (painter, zoom) = (aura.painter, aura.zoom);
    let radius = 3.5;
    let tint = alpha(base, opacity);
    match style {
        PinStyle::Flat => {
            painter.circle_filled(center, radius * zoom, tint);
        }
        PinStyle::Lite => {
            painter.circle_filled(
                center,
                (radius + 1.0) * zoom,
                alpha(aura.palette.background, opacity),
            );
            painter.circle_filled(center, radius * zoom, tint);
        }
        PinStyle::Glossy => {
            let outline: Vec<Pos2> = (0..24)
                .map(|index| {
                    let angle = index as f32 * std::f32::consts::TAU / 24.0;
                    center + Vec2::angled(angle) * radius * zoom
                })
                .collect();
            paint_glow(
                painter,
                center,
                &outline,
                4.0 * zoom,
                alpha(base, opacity * 0.125),
            );
            // The radial gradient starts at 35% of the 7px box and reaches the base
            // color at 70% of its farthest-corner radius.
            let origin = center + vec2(-1.05, -1.05) * zoom;
            let highlight = mix(base, [255; 4], 0.2);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(origin, alpha(highlight, opacity));
            for edge in &outline {
                let ramp = ((*edge - origin).length() / (4.5 * zoom)).min(1.0);
                mesh.colored_vertex(*edge, alpha(mix(highlight, base, ramp), opacity));
            }
            let count = outline.len() as u32;
            for index in 0..count {
                mesh.add_triangle(0, index + 1, (index + 1) % count + 1);
            }
            painter.add(mesh);
            // epaint strokes circles outside their radius; the DOM border sits inside.
            painter.circle_stroke(center, (radius - 1.0) * zoom, Stroke::new(zoom, tint));
        }
    }
}

/// A square of half-size `half` with rounded corners, rotated 45° about `center`.
fn diamond(center: Pos2, half: f32, radius: f32, zoom: f32) -> Vec<Pos2> {
    let corners = [
        vec2(1.0, -1.0),
        vec2(1.0, 1.0),
        vec2(-1.0, 1.0),
        vec2(-1.0, -1.0),
    ];
    let steps = if radius > 0.0 { 4 } else { 0 };
    let mut points = Vec::with_capacity(corners.len() * (steps + 1));
    for (index, corner) in corners.into_iter().enumerate() {
        let start = index as f32 * std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_2;
        for step in 0..=steps {
            let angle = start + step as f32 * std::f32::consts::FRAC_PI_2 / 4.0;
            let local = corner * (half - radius) + Vec2::angled(angle) * radius;
            let rotated =
                vec2(local.x - local.y, local.x + local.y) * std::f32::consts::FRAC_1_SQRT_2;
            points.push(center + rotated * zoom);
        }
    }
    points
}

/// Approximates a CSS box-shadow blur with a ring that fades out from a convex outline.
fn paint_glow(painter: &egui::Painter, center: Pos2, outline: &[Pos2], blur: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    for point in outline {
        let offset = *point - center;
        mesh.colored_vertex(*point, color);
        mesh.colored_vertex(*point + offset.normalized() * blur, Color32::TRANSPARENT);
    }
    let count = outline.len() as u32;
    for index in 0..count {
        let next = (index + 1) % count;
        mesh.add_triangle(2 * index, 2 * index + 1, 2 * next);
        mesh.add_triangle(2 * index + 1, 2 * next + 1, 2 * next);
    }
    painter.add(mesh);
}

struct App(Rc<RefCell<Board>>);

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        // React owns gestures, focus navigation and keyboard shortcuts.
        input.events.clear();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = now();
        let mut board = self.0.borrow_mut();
        let bounds = board.canvas.get_bounding_client_rect();
        let dpr = web_sys::window()
            .map(|window| window.device_pixel_ratio())
            .unwrap_or(1.0);
        let width = (bounds.width() * dpr).round().max(1.0) as u32;
        let height = (bounds.height() * dpr).round().max(1.0) as u32;
        // Keep eframe's points and canvas backing pixels aligned after DPR changes.
        if board.canvas.width() != width || board.canvas.height() != height {
            board.canvas.set_width(width);
            board.canvas.set_height(height);
            ui.ctx().request_repaint();
            return;
        }
        board.paint(ui);
        let elapsed = now() - started;
        board.stats.frames += 1;
        board.stats.last_paint_cpu_ms = elapsed;
        board.stats.paint_cpu_total_ms += elapsed;
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // WebGL composites premultiplied pixels. The background shape applies
        // palette alpha once; a colored transparent clear would leak RGB.
        [0.0; 4]
    }
}

fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or(0.0)
}

#[wasm_bindgen]
pub struct Handle {
    runner: eframe::WebRunner,
    board: Rc<RefCell<Board>>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[wasm_bindgen]
impl Handle {
    pub fn set_scene(&self, scene_json: &str) -> Result<(), JsValue> {
        let scene: Scene = serde_json::from_str(scene_json).map_err(error)?;
        let index = SceneIndex::new(&scene).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.replace_scene(scene, index);
        Ok(())
    }

    pub fn set_positions(&self, positions_json: &str) -> Result<(), JsValue> {
        let positions: Vec<PositionUpdate> = serde_json::from_str(positions_json).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        let Board { scene, index, .. } = &mut *board;
        let affected = index.apply_positions(scene, &positions).map_err(error)?;
        for index in affected {
            board.curves[index] =
                CurveKey::for_edge(&board.scene.edges[index], &board.scene, &board.index)
                    .map(Curve::new);
            board.stats.geometry_rebuilds += 1;
        }
        board.wake();
        Ok(())
    }

    pub fn set_viewport(&self, x: f32, y: f32, zoom: f32) -> Result<(), JsValue> {
        if !x.is_finite() || !y.is_finite() || !zoom.is_finite() || zoom <= 0.0 {
            return Err(error(
                "Viewport coordinates must be finite and zoom must be positive",
            ));
        }
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.text_zoom.update(zoom, now());
        if board.viewport != [x, y, zoom] {
            board.viewport = [x, y, zoom];
        }
        // The host also resubmits the viewport when a hidden canvas becomes drawable.
        board.wake();
        Ok(())
    }

    pub fn set_overlay(&self, overlay_json: &str) -> Result<(), JsValue> {
        let overlay: Overlay = serde_json::from_str(overlay_json).map_err(error)?;
        overlay.validate().map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.overlay_curve = overlay.connection.as_ref().map(|connection| {
            Curve::new(CurveKey {
                from: connection.from,
                to: connection.to,
                from_direction: connection.from_direction,
                to_direction: match connection.from_direction {
                    Direction::Input => Direction::Output,
                    Direction::Output => Direction::Input,
                },
                from_vertical: connection.kind == Some(ConnectionKind::Reference),
                to_vertical: connection.kind == Some(ConnectionKind::Reference),
            })
        });
        board.overlay = overlay;
        board.wake();
        Ok(())
    }

    /// Run state, log counts and lint marks per node id; they never rebuild the scene.
    pub fn set_status(&self, status_json: &str) -> Result<(), JsValue> {
        let update: StatusUpdate = serde_json::from_str(status_json).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.status.update(update, now());
        board.wake();
        Ok(())
    }

    /// The wires traced from the hovered and the clicked pin, see `Trace`.
    pub fn set_trace(&self, trace_json: &str) -> Result<(), JsValue> {
        let trace: Trace = serde_json::from_str(trace_json).map_err(error)?;
        trace.validate().map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        let Board {
            tracing,
            scene,
            index,
            ..
        } = &mut *board;
        tracing.set(trace, scene, index, now());
        board.wake();
        Ok(())
    }

    /// The preview covering canvas point `[x, y]`, as `{"nodeId", "pinId", "rect"}` JSON with
    /// the card's `[x, y, width, height]` in canvas points.
    pub fn preview_at(&self, x: f32, y: f32) -> Option<String> {
        let board = self.board.borrow();
        let hit = board.tracing.preview_at(pos2(x, y))?;
        let rect = hit.rect;
        serde_json::to_string(&serde_json::json!({
            "nodeId": hit.endpoint.node_id,
            "pinId": hit.endpoint.pin_id,
            "rect": [rect.min.x, rect.min.y, rect.width(), rect.height()],
        }))
        .ok()
    }

    /// The preview under the pointer, as `{"nodeId", "pinId"}` JSON; an empty string clears it.
    pub fn set_preview_hover(&self, endpoint_json: &str) -> Result<(), JsValue> {
        let endpoint = if endpoint_json.is_empty() {
            None
        } else {
            Some(serde_json::from_str::<Endpoint>(endpoint_json).map_err(error)?)
        };
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        if board.tracing.hover(endpoint, now()) {
            board.wake();
        }
        Ok(())
    }

    /// The "last activity" text of each running run, keyed by run id.
    pub fn set_activity(&self, activity_json: &str) -> Result<(), JsValue> {
        let activity = serde_json::from_str(activity_json).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.status.set_activity(activity);
        board.wake();
        Ok(())
    }

    pub fn set_palette(&self, palette_json: &str) -> Result<(), JsValue> {
        let palette = serde_json::from_str(palette_json).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        board.palette = palette;
        board.clear_text();
        board.wake();
        Ok(())
    }

    pub fn set_lod(&self, enabled: bool) -> Result<(), JsValue> {
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        if board.lod != enabled {
            board.lod = enabled;
            board.wake();
        }
        Ok(())
    }

    pub fn pulse(&self) -> Result<(), JsValue> {
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        if board.layer != PaintLayer::Nodes && !board.scene.edges.is_empty() {
            board.pulse_started = Some(now());
            board.stats.pulse_count += 1;
            board.wake();
        }
        Ok(())
    }

    pub fn stop_pulse(&self) -> Result<(), JsValue> {
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        if board.pulse_started.take().is_some() {
            board.stats.activation_pulse = false;
            board.stats.pulse_edges_drawn = 0;
            board.wake();
        }
        Ok(())
    }

    /// Separate transparent canvases preserve DOM comments between edges and nodes.
    pub fn set_layer(&self, layer: &str) -> Result<(), JsValue> {
        let layer = PaintLayer::parse(layer).map_err(error)?;
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        if board.layer != layer {
            board.layer = layer;
            board.wake();
        }
        Ok(())
    }

    pub fn set_fonts(&self, regular: &[u8], medium: &[u8]) -> Result<(), JsValue> {
        if regular.is_empty() {
            return Err(error("Regular font data must not be empty"));
        }
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Inter".into(),
            egui::FontData::from_owned(regular.to_vec()).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Inter".into());
        if !medium.is_empty() {
            fonts.font_data.insert(
                "Inter Medium".into(),
                egui::FontData::from_owned(medium.to_vec()).into(),
            );
            fonts.families.insert(
                egui::FontFamily::Name("Inter Medium".into()),
                vec!["Inter Medium".into(), "Inter".into()],
            );
        }
        if let Some(ctx) = &board.ctx {
            ctx.set_fonts(fonts);
        }
        board.medium_font = !medium.is_empty();
        board.clear_text();
        board.wake();
        Ok(())
    }

    /// Replaces the raster for one node icon URL; the host re-sends it when the display changes.
    pub fn set_icon(
        &self,
        key: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
        tint: bool,
    ) -> Result<(), JsValue> {
        let size = [width as usize, height as usize];
        let expected = size[0] * size[1] * 4;
        if expected == 0 || rgba.len() != expected {
            return Err(error(format!(
                "Icon {key} needs {width}x{height} RGBA ({expected} bytes), got {} bytes",
                rgba.len()
            )));
        }
        let mut board = self.board.borrow_mut();
        board.ensure_alive()?;
        let ctx = board
            .ctx
            .clone()
            .ok_or_else(|| error("Board canvas has no rendering context for icons"))?;
        let texture = ctx.load_texture(
            format!("icon:{key}"),
            egui::ColorImage::from_rgba_unmultiplied(size, rgba),
            egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            },
        );
        board
            .icons
            .insert(key.to_owned(), NodeIcon { texture, tint });
        board.wake();
        Ok(())
    }

    pub fn stats(&self) -> String {
        serde_json::to_string(&self.board.borrow().stats).expect("Finite board statistics")
    }

    pub fn destroy(&self) {
        let mut board = self.board.borrow_mut();
        if !board.destroyed {
            board.destroyed = true;
            board.ctx = None;
            board.curves = Vec::new();
            board.text = Vec::new();
            board.icons = HashMap::new();
            board.overlay = Overlay::default();
            board.overlay_curve = None;
            board.status = Statuses::default();
            board.tracing = Tracing::default();
            board.scene = Scene::default();
            board.index.nodes = HashMap::new();
            board.index.pins = Vec::new();
            board.index.incident = Vec::new();
            drop(board);
            self.runner.destroy();
        }
    }
}

#[wasm_bindgen]
/// The host owns input on a separate focusable surface. Give this canvas
/// `pointer-events: none` and `tabIndex = -1` to bypass eframe's DOM input handlers.
pub async fn start(
    canvas: web_sys::HtmlCanvasElement,
    scene_json: &str,
    palette_json: &str,
) -> Result<Handle, JsValue> {
    let tab_index = canvas.tab_index();
    let previous_sibling = canvas.next_element_sibling();
    let document = canvas.owner_document();
    let previous_focus = document
        .as_ref()
        .and_then(|document| document.active_element());
    let scene: Scene = serde_json::from_str(scene_json).map_err(error)?;
    let index = SceneIndex::new(&scene).map_err(error)?;
    let palette = serde_json::from_str(palette_json).map_err(error)?;
    let curves: Vec<_> = scene
        .edges
        .iter()
        .map(|edge| CurveKey::for_edge(edge, &scene, &index).map(Curve::new))
        .collect();
    let stats = Stats {
        node_count: scene.nodes.len(),
        edge_count: scene.edges.len(),
        unresolved_edges: curves.iter().filter(|curve| curve.is_none()).count(),
        geometry_rebuilds: curves.iter().filter(|curve| curve.is_some()).count() as u64,
        viewport: [0.0, 0.0, 1.0],
        lod_level: "full",
        paint_layer: "all",
        ..Stats::default()
    };
    let text = (0..scene.nodes.len())
        .map(|_| NodeText::default())
        .collect();
    let board = Rc::new(RefCell::new(Board {
        canvas: canvas.clone(),
        scene,
        index,
        palette,
        curves,
        text,
        icons: HashMap::new(),
        text_zoom: ZoomQuality::new(1.0),
        overlay: Overlay::default(),
        overlay_curve: None,
        viewport: [0.0, 0.0, 1.0],
        lod: false,
        layer: PaintLayer::All,
        medium_font: false,
        destroyed: false,
        pulse_started: None,
        status: Statuses::default(),
        tracing: Tracing::default(),
        ctx: None,
        stats,
    }));
    let app_board = board.clone();
    let runner = eframe::WebRunner::new();
    let result = runner
        .start(
            canvas.clone(),
            eframe::WebOptions {
                renderer: eframe::Renderer::Glow,
                webgl_context_option: eframe::WebGlContextOption::WebGl2,
                dithering: false,
                should_stop_propagation: Box::new(|_| false),
                should_prevent_default: Box::new(|_| false),
                ..Default::default()
            },
            Box::new(move |cc| {
                app_board.borrow_mut().ctx = Some(cc.egui_ctx.clone());
                Ok(Box::new(App(app_board)))
            }),
        )
        .await;
    canvas.set_tab_index(tab_index);
    // eframe 0.36 inserts and focuses a hidden input immediately after the canvas.
    // This painter has no text editing; keep the host's DOM editor and tab order intact.
    if let Some(agent) = canvas
        .next_element_sibling()
        .and_then(|element| element.dyn_into::<web_sys::HtmlInputElement>().ok())
        && !previous_sibling
            .as_ref()
            .is_some_and(|previous| previous.is_same_node(Some(&agent)))
        && agent.get_attribute("autocapitalize").as_deref() == Some("off")
        && agent.style().get_property_value("width").ok().as_deref() == Some("1px")
    {
        let restore_focus = document
            .as_ref()
            .and_then(|document| document.active_element())
            .is_some_and(|active| {
                active.is_same_node(Some(&agent)) || active.is_same_node(Some(&canvas))
            });
        agent.remove();
        if restore_focus
            && let Some(previous) =
                previous_focus.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            let _ = previous.focus_with_options(&options);
        }
    }
    if let Err(cause) = result {
        runner.destroy();
        return Err(cause);
    }
    Ok(Handle { runner, board })
}

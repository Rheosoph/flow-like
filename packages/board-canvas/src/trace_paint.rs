//! A traced pin's wires over the others, marks on the pins they reach, and preview windows
//! onto the ends that are off screen, like waypoints at the edge of a game's view.

use crate::aura::Aura;
use crate::geometry::{Curve, Detail};
use crate::model::{
    ConnectionKind, Edge, Endpoint, Palette, Rgba, Scene, SceneIndex, Trace, WireTrace,
};
use crate::painter::{NodeText, alpha, color, curve_rect, mix, paint_node, paint_wire};
use crate::status::{Perimeter, cycle, ease_out};
use crate::trace::{
    Emphasis, FOCUS_FADE, PREVIEW_MARGIN, PreviewHover, TraceState, border_point, entry, outward,
    place_previews, preview_camera,
};
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use std::collections::HashMap;

/// Previews drawn at once; further off-screen ends get an arrow on the border.
const MAX_PREVIEWS: usize = 4;
const PREVIEW_SIZE: [f32; 2] = [232.0, 136.0];
const PREVIEW_RADIUS: f32 = 8.0;
const PREVIEW_INSET: f32 = 4.0;
const PING_MS: f64 = 1_600.0;
/// Graph units a light travels per millisecond, and the closest two lights sit.
const LIGHT_SPEED: f64 = 0.16;
const LIGHT_SPACING: f32 = 96.0;
/// Lights closer than this on screen thin out by halves, so the survivors stay put.
const MIN_LIGHT_GAP: f32 = 28.0;
const MAX_LIGHTS: usize = 64;

/// A preview the host can click, in canvas points.
pub struct PreviewHit {
    pub rect: Rect,
    pub endpoint: Endpoint,
}

/// A board's trace and what painting it leaves behind: the previews' click targets and node
/// text, and the preview under the pointer.
#[derive(Default)]
pub struct Tracing {
    pub state: TraceState,
    /// Text of nodes drawn inside previews, at the previews' own zoom.
    text: HashMap<usize, NodeText>,
    previews: Vec<PreviewHit>,
    hover: PreviewHover,
}

impl Tracing {
    pub fn set(&mut self, trace: Trace, scene: &Scene, index: &SceneIndex, now: f64) {
        self.state.set(trace, scene, index, now);
        self.text.clear();
    }

    pub fn clear_text(&mut self) {
        self.text.clear();
    }

    /// Returns whether the hovered preview changed.
    pub fn hover(&mut self, endpoint: Option<Endpoint>, now: f64) -> bool {
        self.hover.set(endpoint, now)
    }

    /// The topmost preview covering a canvas point.
    pub fn preview_at(&self, point: Pos2) -> Option<&PreviewHit> {
        self.previews
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(point))
    }

    pub fn previews_drawn(&self) -> usize {
        self.previews.len()
    }

    /// Marks where the trace runs and previews of its off-screen pins, over the nodes.
    /// Returns whether they animate.
    pub fn paint_marks(
        &mut self,
        aura: &Aura,
        scene: &Scene,
        index: &SceneIndex,
        curves: &[Option<Curve>],
        area: Rect,
        transform: impl Fn([f32; 2]) -> Pos2 + Copy,
    ) -> bool {
        paint_trace_marks(aura, scene, index, &self.state, area, transform)
            | self.paint_previews(aura, scene, index, curves, area, transform)
    }
}

/// Execution wires are drawn in the foreground color, so their trace takes the primary.
fn trace_tint(palette: &Palette, kind: ConnectionKind, base: Rgba) -> Rgba {
    if kind == ConnectionKind::Execution {
        palette.primary
    } else {
        base
    }
}

/// A traced wire's core width, kept legible when zoomed out.
fn core_width(emphasis: Emphasis, zoom: f32) -> f32 {
    match emphasis {
        Emphasis::Selected => (4.6 * zoom).max(4.0),
        Emphasis::Focus => (3.4 * zoom).max(3.0),
        _ => (2.4 * zoom).max(1.75),
    }
}

/// The opacity of a traced wire's wide and narrow glow.
fn glow_alpha(emphasis: Emphasis) -> (f32, f32) {
    match emphasis {
        Emphasis::Selected => (0.16, 0.5),
        Emphasis::Focus => (0.12, 0.36),
        _ => (0.07, 0.18),
    }
}

/// Traced wires above the others. Returns how many were drawn and whether they animate.
pub fn paint_traced_wires(
    aura: &Aura,
    scene: &Scene,
    curves: &[Option<Curve>],
    trace: &TraceState,
    view: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> (usize, bool) {
    let (mut drawn, mut moving) = (0, false);
    for ((edge, curve), emphasis) in scene.edges.iter().zip(curves).zip(&trace.edges) {
        if *emphasis == Emphasis::None || edge.dom {
            continue;
        }
        let Some(curve) = curve
            .as_ref()
            .filter(|curve| curve_rect(curve).expand(8.0).intersects(view))
        else {
            continue;
        };
        let points = curve.points.iter().map(|point| transform(*point)).collect();
        moving |= paint_traced_wire(aura, points, edge, *emphasis);
        drawn += 1;
    }
    (drawn, moving)
}

/// A glow in the trace's tint under a solid core in the wire's own stroke; a focused wire
/// carries lights from source to target, and the clicked one a bright stripe down its middle.
/// Returns whether it animates.
fn paint_traced_wire(aura: &Aura, points: Vec<Pos2>, edge: &Edge, emphasis: Emphasis) -> bool {
    let painter = aura.painter;
    let tint = trace_tint(aura.palette, edge.kind, edge.color);
    let width = core_width(emphasis, aura.zoom);
    let (wide, narrow) = glow_alpha(emphasis);
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(width * 5.5, alpha(tint, wide)),
    ));
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(width * 2.5, alpha(tint, narrow)),
    ));
    // Toward the foreground: brighter on dark themes, deeper on light ones.
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(
            width,
            alpha(mix(edge.color, aura.palette.foreground, 0.2), 1.0),
        ),
    ));
    if emphasis == Emphasis::Selected {
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(
                width * 0.36,
                alpha(mix(edge.color, aura.palette.foreground, 0.75), 1.0),
            ),
        ));
    }
    if emphasis < Emphasis::Focus || !aura.motion {
        return false;
    }
    paint_lights(aura, &points, tint, width);
    true
}

/// Lights at fixed graph spacing, so zooming neither speeds them up nor reshuffles them.
fn paint_lights(aura: &Aura, points: &[Pos2], tint: Rgba, width: f32) {
    let path = Perimeter::open(points.iter().map(|point| [point.x, point.y]).collect());
    let length = path.length();
    let zoom = aura.zoom.max(f32::EPSILON);
    let mut spacing = LIGHT_SPACING;
    while spacing * zoom < MIN_LIGHT_GAP {
        spacing *= 2.0;
    }
    let offset = (aura.now * LIGHT_SPEED).rem_euclid(spacing as f64) as f32;
    let bright = mix(tint, aura.palette.foreground, 0.5);
    for light in 0..MAX_LIGHTS {
        let distance = (offset + light as f32 * spacing) * zoom;
        if distance >= length {
            break;
        }
        let fade = (distance / 24.0).min(1.0) * ((length - distance) / 24.0).min(1.0);
        let [x, y] = path.at(distance);
        let center = pos2(x, y);
        aura.painter
            .circle_filled(center, width * 2.2, alpha(tint, 0.3 * fade));
        aura.painter
            .circle_filled(center, width * 0.85, alpha(bright, fade));
    }
}

/// A ring around a reached pin; a focused one sends out pings. Returns whether it animates.
fn paint_ring(aura: &Aura, center: Pos2, tint: Rgba, emphasis: Emphasis) -> bool {
    let radius = (6.5 * aura.zoom).max(6.0);
    let focus = emphasis >= Emphasis::Focus;
    aura.painter.circle_stroke(
        center,
        radius,
        Stroke::new(if focus { 2.0 } else { 1.5 }, alpha(tint, 0.95)),
    );
    if !focus || !aura.motion {
        return false;
    }
    let progress = cycle(aura.now, PING_MS);
    aura.painter.circle_stroke(
        center,
        radius * (1.0 + 1.6 * ease_out(progress)),
        Stroke::new(1.5, alpha(tint, 0.7 * (1.0 - progress))),
    );
    true
}

fn node_rect(scene: &Scene, node: usize, transform: impl Fn([f32; 2]) -> Pos2) -> Rect {
    let node = &scene.nodes[node];
    Rect::from_min_max(
        transform([node.x, node.y]),
        transform([node.x + node.width, node.y + node.height]),
    )
}

/// Rings on the pins a trace reaches, glows around its reroutes and, when focused, around
/// the nodes it reaches. Returns whether they animate.
fn paint_trace_marks(
    aura: &Aura,
    scene: &Scene,
    index: &SceneIndex,
    trace: &TraceState,
    area: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> bool {
    let visible = area.expand(24.0);
    let mut animating = false;
    for (wires, emphasis) in [
        (&trace.trace.hover, Emphasis::Hover),
        (&trace.trace.focus, Emphasis::Focus),
    ] {
        let Some(wires) = wires else { continue };
        paint_reroute_glows(aura, scene, index, wires, emphasis, visible, &transform);
        animating |= paint_end_marks(aura, scene, index, wires, emphasis, visible, &transform);
        if emphasis == Emphasis::Focus
            && let Some((node, pin)) = wires
                .origin
                .as_ref()
                .and_then(|origin| index.endpoint(scene, origin))
        {
            let center = transform([node.x + pin.x, node.y + pin.y]);
            if visible.contains(center) {
                paint_ring(aura, center, aura.palette.primary, Emphasis::Hover);
            }
        }
    }
    animating
}

fn paint_reroute_glows(
    aura: &Aura,
    scene: &Scene,
    index: &SceneIndex,
    wires: &WireTrace,
    emphasis: Emphasis,
    visible: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) {
    for &position in wires.reroutes.iter().filter_map(|id| index.nodes.get(id)) {
        let rect = node_rect(scene, position, &transform);
        if !visible.intersects(rect) {
            continue;
        }
        let node = &scene.nodes[position];
        let tint = node.pins.first().map_or(aura.palette.primary, |pin| {
            trace_tint(aura.palette, pin.kind, pin.color)
        });
        aura.glow(
            rect,
            node.corner_radius() * aura.zoom,
            (8.0 * aura.zoom).clamp(4.0, 12.0),
            tint,
            if emphasis == Emphasis::Focus {
                0.6
            } else {
                0.35
            },
        );
    }
}

/// Returns whether the marks animate.
fn paint_end_marks(
    aura: &Aura,
    scene: &Scene,
    index: &SceneIndex,
    wires: &WireTrace,
    emphasis: Emphasis,
    visible: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> bool {
    let mut animating = false;
    for end in &wires.ends {
        let (Some((node, pin)), Some(&position)) =
            (index.endpoint(scene, end), index.nodes.get(&end.node_id))
        else {
            continue;
        };
        let center = transform([node.x + pin.x, node.y + pin.y]);
        if !visible.contains(center) {
            continue;
        }
        let tint = trace_tint(aura.palette, pin.kind, pin.color);
        if emphasis == Emphasis::Focus {
            aura.glow(
                node_rect(scene, position, &transform),
                node.corner_radius() * aura.zoom,
                (10.0 * aura.zoom).clamp(4.0, 16.0),
                tint,
                0.3,
            );
        }
        animating |= paint_ring(aura, center, tint, emphasis);
    }
    animating
}

/// An off-screen pin of the previewed trace.
struct Target<'a> {
    endpoint: &'a Endpoint,
    node: usize,
    pin: usize,
    screen: Pos2,
    /// Where its preview should sit: where its wire enters the view.
    anchor: Pos2,
    tint: Rgba,
}

fn off_screen<'a>(
    palette: &Palette,
    scene: &Scene,
    index: &SceneIndex,
    endpoint: &'a Endpoint,
    inside: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> Option<Target<'a>> {
    let &node = index.nodes.get(&endpoint.node_id)?;
    let &pin = index.pins[node].get(&endpoint.pin_id)?;
    let (body, handle) = (&scene.nodes[node], &scene.nodes[node].pins[pin]);
    let screen = transform([body.x + handle.x, body.y + handle.y]);
    (!inside.contains(screen)).then(|| Target {
        endpoint,
        node,
        pin,
        screen,
        anchor: screen,
        tint: trace_tint(palette, handle.kind, handle.color),
    })
}

/// `aura` painting through `painter` at `zoom`.
fn through<'a>(aura: &'a Aura, painter: &'a egui::Painter, zoom: f32) -> Aura<'a> {
    Aura {
        painter,
        palette: aura.palette,
        icons: aura.icons,
        zoom,
        now: aura.now,
        motion: aura.motion,
        medium: aura.medium.clone(),
    }
}

/// The previewed trace's off-screen pins, nearest first and the traced pin before them, each
/// anchored where its wire enters the view, with the trace's emphasis. `None` when every pin
/// is on screen.
fn off_screen_targets<'a>(
    palette: &Palette,
    scene: &Scene,
    index: &SceneIndex,
    curves: &[Option<Curve>],
    trace: &'a TraceState,
    area: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> Option<(Vec<Target<'a>>, Emphasis)> {
    let (wires, level) = trace.previewed()?;
    let inside = area.shrink(2.0);
    let center = area.center();
    let mut ends: Vec<_> = wires
        .ends
        .iter()
        .filter_map(|end| off_screen(palette, scene, index, end, inside, &transform))
        .collect();
    ends.sort_by(|a, b| {
        a.screen
            .distance_sq(center)
            .total_cmp(&b.screen.distance_sq(center))
    });
    let origin = wires
        .origin
        .as_ref()
        .and_then(|origin| off_screen(palette, scene, index, origin, inside, &transform))
        .map(|origin| Target {
            tint: palette.primary,
            ..origin
        });
    let targets: Vec<_> = origin
        .into_iter()
        .chain(ends)
        .map(|target| Target {
            anchor: anchor(scene, curves, trace, level, &target, inside, &transform),
            ..target
        })
        .collect();
    (!targets.is_empty()).then_some((targets, level))
}

/// A preview card's size: smaller on small canvases rather than none.
fn preview_size(area: Rect) -> [f32; 2] {
    let scale = (area.width() / 900.0)
        .min(area.height() / 560.0)
        .clamp(0.6, 1.0);
    [PREVIEW_SIZE[0] * scale, PREVIEW_SIZE[1] * scale]
}

impl Tracing {
    /// Previews of a trace's off-screen pins on the canvas border: the focused trace's, else
    /// the hovered pin's, which fade in after a moment. Returns whether they animate.
    fn paint_previews(
        &mut self,
        aura: &Aura,
        scene: &Scene,
        index: &SceneIndex,
        curves: &[Option<Curve>],
        area: Rect,
        transform: impl Fn([f32; 2]) -> Pos2,
    ) -> bool {
        self.previews.clear();
        let Some((targets, level)) = off_screen_targets(
            aura.palette,
            scene,
            index,
            curves,
            &self.state,
            area,
            transform,
        ) else {
            return false;
        };
        let peek = if level == Emphasis::Hover {
            self.state.peek(aura.now)
        } else {
            1.0
        };
        if peek <= 0.0 {
            return true;
        }
        let mut faded = aura.painter.clone();
        faded.multiply_opacity(peek);
        let aura = &through(aura, &faded, aura.zoom);
        let size = preview_size(area);
        let avoid: Vec<_> = (self.state.trace.avoid.iter())
            .map(|rect| [rect[0] + area.min.x, rect[1] + area.min.y, rect[2], rect[3]])
            .collect();
        let anchors: Vec<_> = (targets.iter().take(MAX_PREVIEWS))
            .map(|target| [target.anchor.x, target.anchor.y])
            .collect();
        let bounds = [area.min.x, area.min.y, area.width(), area.height()];
        let slots = place_previews(bounds, size, &anchors, &avoid);
        let mut animating = peek < 1.0 || self.hover.settling(aura.now);
        for (position, target) in targets.iter().enumerate() {
            let Some([x, y]) = slots.get(position).copied().flatten() else {
                paint_border_arrow(aura, area, target);
                continue;
            };
            let card = Rect::from_center_size(pos2(x, y), Vec2::from(size));
            let lift = self.hover.strength(target.endpoint, aura.now);
            animating |= paint_preview(
                aura,
                scene,
                curves,
                &self.state,
                &mut self.text,
                card,
                target,
                lift,
            );
            self.previews.push(PreviewHit {
                rect: card.translate(-area.min.to_vec2()),
                endpoint: target.endpoint.clone(),
            });
        }
        animating
    }
}

/// Where the traced wire into the target enters the view, so its card sits on the wire.
/// Without a visible wire, where the line from the view's centre toward the target leaves it.
fn anchor(
    scene: &Scene,
    curves: &[Option<Curve>],
    trace: &TraceState,
    level: Emphasis,
    target: &Target,
    inside: Rect,
    transform: impl Fn([f32; 2]) -> Pos2,
) -> Pos2 {
    let (min, max) = ([inside.min.x, inside.min.y], [inside.max.x, inside.max.y]);
    let screen = |point: &[f32; 2]| {
        let point = transform(*point);
        [point.x, point.y]
    };
    let crossing =
        scene
            .edges
            .iter()
            .zip(curves)
            .zip(&trace.edges)
            .find_map(|((edge, curve), emphasis)| {
                let points = curve
                    .as_ref()
                    .filter(|_| *emphasis >= level)?
                    .points
                    .iter()
                    .map(screen);
                if edge.target == *target.endpoint {
                    entry(points.rev(), min, max)
                } else if edge.source == *target.endpoint {
                    entry(points, min, max)
                } else {
                    None
                }
            });
    let [x, y] = crossing.unwrap_or_else(|| {
        let center = inside.center();
        border_point(
            [min[0], min[1], inside.width(), inside.height()],
            0.0,
            [center.x, center.y],
            [target.screen.x, target.screen.y],
        )
    });
    pos2(x, y)
}

/// A card with a window onto the board around the target pin, and a tab pointing at it.
/// `lift` (0 to 1) is how far its hover state has eased in: a glow, a deeper shadow, a
/// stronger border and a longer tab.
#[allow(clippy::too_many_arguments)]
fn paint_preview(
    aura: &Aura,
    scene: &Scene,
    curves: &[Option<Curve>],
    trace: &TraceState,
    texts: &mut HashMap<usize, NodeText>,
    card: Rect,
    target: &Target,
    lift: f32,
) -> bool {
    let (painter, palette) = (aura.painter, aura.palette);
    painter.add(
        egui::Shadow {
            offset: [0, 6 + (3.0 * lift) as i8],
            blur: 22 + (10.0 * lift) as u8,
            spread: 0,
            color: Color32::from_black_alpha(90 + (50.0 * lift) as u8),
        }
        .as_shape(card, PREVIEW_RADIUS),
    );
    if lift > 0.0 {
        aura.glow(card, PREVIEW_RADIUS, 16.0 * lift, target.tint, 0.45 * lift);
    }
    paint_pointer(
        painter,
        card,
        target.screen,
        alpha(target.tint, 0.95),
        1.0 + 0.35 * lift,
    );
    painter.rect(
        card,
        PREVIEW_RADIUS,
        color(palette.card),
        Stroke::new(1.5 + lift, alpha(target.tint, 0.8 + 0.2 * lift)),
        egui::StrokeKind::Inside,
    );
    let window = card.shrink(PREVIEW_INSET);
    let clipped = painter.with_clip_rect(window.intersect(painter.clip_rect()));
    clipped.rect_filled(
        window,
        PREVIEW_RADIUS - PREVIEW_INSET,
        color(mix(palette.background, target.tint, 0.06 * lift)),
    );
    let node = &scene.nodes[target.node];
    let pin = &node.pins[target.pin];
    let pin_at = [node.x + pin.x, node.y + pin.y];
    let camera = preview_camera(
        [node.x, node.y, node.width, node.height],
        pin_at,
        outward(pin.direction, pin.kind),
        [window.width(), window.height()],
    );
    let place = |point: [f32; 2]| {
        window.center()
            + vec2(point[0] - camera.center[0], point[1] - camera.center[1]) * camera.zoom
    };
    let view = Rect::from_center_size(Pos2::from(camera.center), window.size() / camera.zoom);
    let lens = through(aura, &clipped, camera.zoom);
    let mut animating = false;
    for (position, (edge, curve)) in scene.edges.iter().zip(curves).enumerate() {
        let Some(curve) = curve
            .as_ref()
            .filter(|curve| curve_rect(curve).intersects(view))
        else {
            continue;
        };
        let points = curve.points.iter().map(|point| place(*point)).collect();
        match trace.edges.get(position).copied().unwrap_or_default() {
            Emphasis::None => {
                paint_wire(&clipped, points, edge, Detail::Full, lens.zoom, FOCUS_FADE)
            }
            emphasis => animating |= paint_traced_wire(&lens, points, edge, emphasis),
        }
    }
    for (position, body) in scene.nodes.iter().enumerate() {
        let bounds = Rect::from_min_size(pos2(body.x, body.y), vec2(body.width, body.height));
        if !bounds.intersects(view) {
            continue;
        }
        let rect = Rect::from_min_max(
            place([body.x, body.y]),
            place([body.x + body.width, body.y + body.height]),
        );
        let text = texts.entry(position).or_default();
        paint_node(&lens, body, rect, true, Detail::Full, text, None, false);
    }
    animating | paint_ring(&lens, place(pin_at), target.tint, Emphasis::Focus)
}

/// A tab on the card's border that points at the off-screen pin, `scale` times its rest size.
fn paint_pointer(painter: &egui::Painter, card: Rect, target: Pos2, fill: Color32, scale: f32) {
    let offset = target - card.center();
    if offset.length() < 1.0 {
        return;
    }
    let direction = offset.normalized();
    let reach =
        (card.width() * 0.5 / direction.x.abs()).min(card.height() * 0.5 / direction.y.abs());
    let base = card.center() + direction * (reach - 1.0);
    let side = direction.rot90() * 7.0 * scale;
    painter.add(Shape::convex_polygon(
        vec![base - side, base + direction * 10.0 * scale, base + side],
        fill,
        Stroke::NONE,
    ));
}

/// An arrowhead on the canvas border for an off-screen pin that has no preview.
fn paint_border_arrow(aura: &Aura, area: Rect, target: &Target) {
    let tip = area.shrink(PREVIEW_MARGIN + 6.0).clamp(target.anchor);
    let offset = target.screen - tip;
    if offset.length() < 1.0 {
        return;
    }
    let direction = offset.normalized();
    let back = tip - direction * 11.0;
    let side = direction.rot90() * 6.5;
    aura.painter
        .circle_filled(back + direction * 4.0, 10.0, alpha(aura.palette.card, 0.9));
    aura.painter.add(Shape::convex_polygon(
        vec![back - side, tip, back + side],
        alpha(target.tint, 0.95),
        Stroke::NONE,
    ));
}

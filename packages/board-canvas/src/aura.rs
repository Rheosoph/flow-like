//! Run state, logs and lint marks: glows and badges around a node, a status chip in its header.
//! Marks are also painted around nodes the DOM renders, so hovering one keeps its run state.

use crate::geometry::Detail;
use crate::model::{Node, Palette, Rgba};
use crate::painter::{NodeIcon, alpha, color, mix, paint_glyph};
use crate::status::{
    Activity, Flow, Heat, NodeStatus, Perimeter, QualityMark, RunState, Severity, Tone, breath,
    cycle, ease_out, rounded_outline,
};
use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, Shape, Stroke, pos2, vec2};
use std::{collections::HashMap, sync::Arc};

/// How far glows and badges reach past a node, in graph units.
pub const AURA_MARGIN: f32 = 28.0;
/// Comets drawn per frame; further running nodes keep their glow.
const MAX_COMETS: usize = 48;
/// Execution wires lit per frame.
pub const MAX_FLOWS: usize = 128;
const ORBIT_MS: f64 = 1_700.0;
const BREATH_MS: f64 = 1_400.0;
const SCROLL_GLYPH: &str = "glyph:scroll";
const CHECK_GLYPH: &str = "glyph:check";
const CLOCK_GLYPH: &str = "glyph:clock";
/// Horizontal padding and gap inside the header chip, in graph units.
const CHIP_PAD: f32 = 4.0;
const CHIP_GAP: f32 = 4.0;
/// A corner badge's radius; the header chip keeps clear of it.
const CORNER_RADIUS: f32 = 6.5;

fn severity_color(palette: &Palette, severity: Severity) -> Rgba {
    match severity {
        Severity::Info => palette.info(),
        Severity::Warning => palette.caution(),
        Severity::Error => palette.destructive(),
        Severity::Fatal => palette.fatal(),
    }
}

fn quality_color(palette: &Palette, severity: Severity) -> Rgba {
    match severity {
        Severity::Info => palette.info(),
        Severity::Warning => palette.warning(),
        Severity::Error | Severity::Fatal => palette.destructive(),
    }
}

fn tone_color(palette: &Palette, tone: Tone) -> Rgba {
    match tone {
        Tone::Active => palette.success(),
        Tone::Stale => palette.caution(),
        Tone::Warning => palette.destructive(),
        Tone::Inactive => palette.muted,
    }
}

/// Dark ink on light fills and white ink on dark ones.
fn ink_on(fill: Rgba, dark: Rgba) -> Color32 {
    let [r, g, b, _] = fill.map(|channel| channel as f32 / 255.0);
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.55 {
        color(dark)
    } else {
        Color32::WHITE
    }
}

/// The run severity that earns a corner badge.
fn alarm(status: &NodeStatus) -> Option<Severity> {
    status.severity.filter(|level| *level >= Severity::Warning)
}

fn points(min: Pos2, max: Pos2, radius: f32) -> impl Iterator<Item = ([f32; 2], [f32; 2])> {
    rounded_outline([min.x, min.y], [max.x, max.y], radius, 6).into_iter()
}

/// The galley's first baseline below its top, in screen points.
fn baseline(galley: &egui::Galley) -> f32 {
    galley
        .rows
        .first()
        .and_then(|row| Some(row.pos.y + row.row.glyphs.first()?.pos.y))
        .unwrap_or(galley.size().y * 0.8)
}

/// A tapered ribbon through `path` with a width and color per point, feathered by one pixel.
fn ribbon(painter: &egui::Painter, path: &[(Pos2, f32, Color32)]) {
    if path.len() < 2 {
        return;
    }
    let feather = 1.0 / painter.ctx().pixels_per_point();
    let mut mesh = egui::Mesh::default();
    for (index, (point, width, tint)) in path.iter().enumerate() {
        let previous = path[index.saturating_sub(1)].0;
        let next = path[(index + 1).min(path.len() - 1)].0;
        let direction = (next - previous).normalized();
        let normal = vec2(-direction.y, direction.x);
        let half = width * 0.5;
        mesh.colored_vertex(*point + normal * (half + feather), Color32::TRANSPARENT);
        mesh.colored_vertex(*point + normal * half, *tint);
        mesh.colored_vertex(*point - normal * half, *tint);
        mesh.colored_vertex(*point - normal * (half + feather), Color32::TRANSPARENT);
    }
    for index in 0..path.len() as u32 - 1 {
        let (a, b) = (index * 4, (index + 1) * 4);
        for lane in 0..3 {
            mesh.add_triangle(a + lane, a + lane + 1, b + lane);
            mesh.add_triangle(a + lane + 1, b + lane + 1, b + lane);
        }
    }
    painter.add(mesh);
}

pub struct Aura<'a> {
    pub painter: &'a egui::Painter,
    pub palette: &'a Palette,
    pub icons: &'a HashMap<String, NodeIcon>,
    pub zoom: f32,
    pub now: f64,
    pub motion: bool,
    pub medium: FontFamily,
}

impl Aura<'_> {
    /// Uncolored text; the painter tints it, so a fading node fades its text too.
    fn text(&self, text: &str, size: f32, family: FontFamily) -> Arc<egui::Galley> {
        self.painter.layout_no_wrap(
            text.to_owned(),
            FontId::new(size * self.zoom, family),
            Color32::PLACEHOLDER,
        )
    }

    /// Text starting at `start`, vertically centered on it, in screen points.
    fn place(&self, galley: &Arc<egui::Galley>, start: Pos2, tint: Color32) {
        self.painter.galley(
            pos2(start.x, start.y + 2.9 * self.zoom - baseline(galley)),
            galley.clone(),
            tint,
        );
    }

    fn glyph(&self, key: &str, center: Pos2, size: f32, tint: Color32) {
        paint_glyph(
            self.painter,
            self.icons,
            key,
            Rect::from_center_size(center, vec2(size, size) * self.zoom),
            tint,
            false,
        );
    }

    /// A glow fading out from the node's outline, like a blurred box-shadow ring.
    pub(crate) fn glow(&self, rect: Rect, radius: f32, width: f32, tint: Rgba, strength: f32) {
        if width <= 0.0 || strength <= 0.0 {
            return;
        }
        // Opacity at fractions of the width; the falloff approximates a gaussian blur.
        const BANDS: [(f32, f32); 4] = [(0.0, 1.0), (0.25, 0.5), (0.6, 0.16), (1.0, 0.0)];
        let mut mesh = egui::Mesh::default();
        let mut count = 0;
        for (point, normal) in points(rect.min, rect.max, radius) {
            for (offset, fade) in BANDS {
                mesh.colored_vertex(
                    pos2(
                        point[0] + normal[0] * width * offset,
                        point[1] + normal[1] * width * offset,
                    ),
                    alpha(tint, strength * fade),
                );
            }
            count += 1;
        }
        let bands = BANDS.len() as u32;
        for index in 0..count {
            let next = (index + 1) % count;
            for band in 0..bands - 1 {
                let (a, b) = (index * bands + band, next * bands + band);
                mesh.add_triangle(a, a + 1, b);
                mesh.add_triangle(a + 1, b + 1, b);
            }
        }
        self.painter.add(mesh);
    }

    /// A bright head with a fading tail that circles the node's border.
    fn comet(&self, rect: Rect, radius: f32, tint: Rgba) {
        let perimeter = Perimeter::new(
            points(rect.min, rect.max, radius)
                .map(|(point, _)| point)
                .collect(),
        );
        let length = perimeter.length();
        if length <= 0.0 {
            return;
        }
        let head = cycle(self.now, ORBIT_MS) * length;
        let tail = length * 0.3;
        let width = (2.0 * self.zoom).clamp(1.25, 3.5);
        let bright = mix(tint, [255; 4], 0.45);
        const STEPS: usize = 24;
        let sample = |step: usize| {
            let t = step as f32 / (STEPS - 1) as f32;
            let [x, y] = perimeter.at(head - tail * (1.0 - t));
            (pos2(x, y), t, mix(tint, bright, t))
        };
        let glow: Vec<_> = (0..STEPS)
            .map(sample)
            .map(|(point, t, shade)| (point, width * (1.0 + 2.6 * t), alpha(shade, 0.16 * t)))
            .collect();
        ribbon(self.painter, &glow);
        let core: Vec<_> = (0..STEPS)
            .map(sample)
            .map(|(point, t, shade)| {
                (
                    point,
                    width * (0.25 + 0.75 * t),
                    alpha(shade, 0.95 * t.powf(1.5)),
                )
            })
            .collect();
        ribbon(self.painter, &core);
        let [x, y] = perimeter.at(head);
        self.painter
            .circle_filled(pos2(x, y), width * 3.0, alpha(bright, 0.16));
        self.painter
            .circle_filled(pos2(x, y), width * 1.1, alpha(bright, 0.95));
    }

    /// A sonar ping in the success color when a node finishes.
    fn ripple(&self, rect: Rect, radius: f32, progress: f32) {
        let tint = self.palette.success();
        let spread = ease_out(progress) * (16.0 * self.zoom).clamp(8.0, 26.0);
        let fade = (1.0 - progress).powf(1.6);
        self.glow(rect, radius, spread + 3.0 * self.zoom, tint, 0.35 * fade);
        let outline: Vec<Pos2> = points(
            rect.min - vec2(spread, spread),
            rect.max + vec2(spread, spread),
            radius + spread,
        )
        .map(|(point, _)| pos2(point[0], point[1]))
        .collect();
        self.painter.add(Shape::closed_line(
            outline,
            Stroke::new((1.4 * self.zoom).clamp(1.0, 2.0), alpha(tint, 0.8 * fade)),
        ));
    }

    /// A glyph on a glowing disc centered on a corner of the node. A filled disc takes `tint`;
    /// otherwise the disc is the background and `tint` colors the glyph.
    fn badge(&self, center: Pos2, radius: f32, glyph: (&str, f32), tint: Rgba, filled: bool) {
        let radius = radius * self.zoom;
        self.painter
            .circle_filled(center, radius + 3.0 * self.zoom, alpha(tint, 0.16));
        if filled {
            self.painter.circle_filled(center, radius, color(tint));
            self.glyph(glyph.0, center, glyph.1, ink_on(tint, self.palette.card));
            return;
        }
        self.painter
            .circle_filled(center, radius, color(self.palette.background));
        self.glyph(glyph.0, center, glyph.1, color(tint));
    }

    /// Glyph and text in a rounded pill whose left edge is `left`, centered on `center_y`.
    /// Returns its width in graph units.
    #[allow(clippy::too_many_arguments)]
    fn pill(
        &self,
        left: f32,
        center_y: f32,
        glyph: Option<&str>,
        text: Option<&Arc<egui::Galley>>,
        fill: Color32,
        ring: Option<Color32>,
        ink: Color32,
    ) -> f32 {
        let zoom = self.zoom;
        let glyph_width = glyph.map_or(0.0, |_| 10.0);
        let text_width = text.map_or(0.0, |galley| galley.size().x / zoom);
        let gap = if glyph.is_some() && text.is_some() {
            2.0
        } else {
            0.0
        };
        let width = 8.0 + glyph_width + gap + text_width;
        self.painter.rect(
            Rect::from_min_size(pos2(left, center_y - 7.0 * zoom), vec2(width, 14.0) * zoom),
            7.0 * zoom,
            fill,
            ring.map_or(Stroke::NONE, |ring| Stroke::new(zoom, ring)),
            egui::StrokeKind::Inside,
        );
        let mut x = left + 4.0 * zoom;
        if let Some(glyph) = glyph {
            self.glyph(glyph, pos2(x + 5.0 * zoom, center_y), 10.0, ink);
            x += (glyph_width + gap) * zoom;
        }
        if let Some(galley) = text {
            self.place(galley, pos2(x, center_y), ink);
        }
        width
    }

    /// The run's path on an execution wire: a trail, and lights flowing into a running node.
    /// Returns whether the lights move.
    pub fn flow(&self, points: &[Pos2], flow: Flow) -> bool {
        if points.len() < 2 {
            return false;
        }
        let tint = self.palette.primary;
        let width = (2.0 * self.zoom).clamp(1.0, 3.0);
        let strength = if flow == Flow::Active { 1.0 } else { 0.6 };
        self.painter.add(Shape::line(
            points.to_vec(),
            Stroke::new(width * 3.5, alpha(tint, 0.12 * strength)),
        ));
        self.painter.add(Shape::line(
            points.to_vec(),
            Stroke::new(width, alpha(tint, 0.5 * strength)),
        ));
        if flow != Flow::Active || !self.motion {
            return false;
        }
        let path = Perimeter::open(points.iter().map(|point| [point.x, point.y]).collect());
        let length = path.length();
        // About 180 screen points a second, whatever the wire's length.
        let period = (length as f64 / 0.18).clamp(500.0, 2_400.0);
        let bright = mix(tint, [255; 4], 0.45);
        const LIGHTS: usize = 3;
        for light in 0..LIGHTS {
            let t = (cycle(self.now, period) + light as f32 / LIGHTS as f32).fract();
            // Lights fade in at the source and out at the target instead of popping.
            let fade = (t / 0.1).min(1.0) * ((1.0 - t) / 0.1).min(1.0);
            let [x, y] = path.at(t * length);
            self.painter
                .circle_filled(pos2(x, y), width * 3.2, alpha(bright, 0.22 * fade));
            self.painter
                .circle_filled(pos2(x, y), width * 1.2, alpha(bright, 0.95 * fade));
        }
        true
    }

    /// Glows behind the node body.
    pub fn halos(&self, node: &Node, rect: Rect, status: &NodeStatus) {
        let radius = node.corner_radius() * self.zoom;
        if status.moving() {
            let tint = self.run_tint(status);
            let strength = if self.motion {
                0.18 + 0.22 * breath(self.now, BREATH_MS)
            } else {
                0.3
            };
            self.glow(
                rect,
                radius,
                (10.0 * self.zoom).clamp(4.0, 18.0),
                tint,
                strength,
            );
        } else if let Some(severity) = alarm(status) {
            self.glow(
                rect,
                radius,
                (8.0 * self.zoom).clamp(3.0, 14.0),
                severity_color(self.palette, severity),
                0.3,
            );
        }
        if let Some(heat) = &status.heat {
            let intensity = heat.intensity.clamp(0.0, 1.0);
            self.glow(
                rect,
                radius,
                ((5.0 + 8.0 * intensity) * self.zoom).clamp(2.0, 20.0),
                self.palette.primary,
                0.1 + 0.35 * intensity,
            );
        }
    }

    /// Comet, ping, badges and pills over the node body. Returns whether the marks move.
    pub fn marks(
        &self,
        node: &Node,
        rect: Rect,
        detail: Detail,
        status: Option<&NodeStatus>,
        ripple: Option<f32>,
        comets: &mut usize,
    ) -> bool {
        let moving = self.motion_marks(node, rect, detail, status, ripple, comets);
        if !detail.labels() {
            return moving;
        }
        if node.long_running {
            // A bare disc, like the DOM node's.
            self.painter
                .circle_filled(rect.min, 4.5 * self.zoom, color(self.palette.background));
            self.glyph(CLOCK_GLYPH, rect.min, 8.0, color(self.palette.foreground));
        }
        if let Some(status) = status {
            self.corner_badge(rect, status);
            if let Some(mark) = &status.quality {
                self.quality_pill(rect, mark);
            }
            if let Some(heat) = &status.heat {
                self.heat_pills(rect, heat);
            }
        }
        moving
    }

    /// The comet around a running node and the ping of one that just finished.
    fn motion_marks(
        &self,
        node: &Node,
        rect: Rect,
        detail: Detail,
        status: Option<&NodeStatus>,
        ripple: Option<f32>,
        comets: &mut usize,
    ) -> bool {
        let radius = node.corner_radius() * self.zoom;
        let running = status.filter(|status| status.moving() && self.motion);
        if let Some(status) = running
            && detail != Detail::Tiny
            && *comets < MAX_COMETS
        {
            *comets += 1;
            self.comet(rect, radius, self.run_tint(status));
        }
        if let Some(progress) = ripple {
            self.ripple(rect, radius, progress);
        }
        running.is_some() || ripple.is_some()
    }

    fn run_tint(&self, status: &NodeStatus) -> Rgba {
        if status.run == Some(RunState::Running) {
            self.palette.primary
        } else {
            self.palette.remote()
        }
    }

    /// The run's worst log level on the top-right corner, or its completion.
    fn corner_badge(&self, rect: Rect, status: &NodeStatus) {
        let corner = pos2(rect.max.x, rect.min.y);
        if let Some(severity) = alarm(status) {
            self.badge(
                corner,
                CORNER_RADIUS,
                (severity.glyph(), 12.0),
                severity_color(self.palette, severity),
                false,
            );
        } else if status.run == Some(RunState::Done) {
            self.badge(
                corner,
                CORNER_RADIUS,
                (CHECK_GLYPH, 8.0),
                self.palette.success(),
                true,
            );
        }
    }

    /// The lint mark, centered on the node's top edge.
    fn quality_pill(&self, rect: Rect, mark: &QualityMark) {
        let tint = quality_color(self.palette, mark.severity);
        let count = mark
            .count
            .as_deref()
            .map(|count| self.text(count, 8.0, self.medium.clone()));
        let width = 18.0
            + count
                .as_ref()
                .map_or(0.0, |galley| 2.0 + galley.size().x / self.zoom);
        self.pill(
            rect.center().x - width * 0.5 * self.zoom,
            rect.min.y,
            Some(mark.severity.glyph()),
            count.as_ref(),
            color(mix(self.palette.background, tint, 0.15)),
            Some(alpha(tint, 0.5)),
            color(tint),
        );
    }

    /// Visit and error counts on the bottom edge, overhanging the left corner.
    fn heat_pills(&self, rect: Rect, heat: &Heat) {
        let zoom = self.zoom;
        let visits = self.text(&heat.visits, 8.0, self.medium.clone());
        let errors = heat
            .errors
            .as_deref()
            .map(|errors| self.text(errors, 8.0, self.medium.clone()));
        let width = 8.0
            + visits.size().x / zoom
            + errors
                .as_ref()
                .map_or(0.0, |galley| 4.0 + 8.0 + galley.size().x / zoom);
        let mut left = rect.min.x - width * 0.3 * zoom;
        left += (self.pill(
            left,
            rect.max.y,
            None,
            Some(&visits),
            color(self.palette.primary),
            None,
            color(self.palette.primary_foreground()),
        ) + 4.0)
            * zoom;
        if let Some(errors) = &errors {
            let tint = self.palette.destructive();
            self.pill(
                left,
                rect.max.y,
                None,
                Some(errors),
                color(tint),
                None,
                ink_on(tint, self.palette.card),
            );
        }
    }
}

struct LogItem {
    count: Option<Arc<egui::Galley>>,
    problems: bool,
    warnings: bool,
}

impl LogItem {
    fn width(&self, zoom: f32) -> f32 {
        8.0 + self
            .count
            .as_ref()
            .map_or(0.0, |galley| 2.0 + galley.size().x / zoom)
            + if self.warnings { 6.0 } else { 0.0 }
    }

    /// Paints from `x` in graph units on the header's center line; returns where it ends.
    fn paint(&self, aura: &Aura, origin: Pos2, x: f32, center: f32, opacity: f32) -> f32 {
        let palette = aura.palette;
        let point = |x: f32, y: f32| origin + vec2(x, y) * aura.zoom;
        let ink = if self.problems {
            palette.destructive()
        } else if self.count.is_some() {
            palette.muted
        } else {
            palette.node_text()
        };
        aura.glyph(
            SCROLL_GLYPH,
            point(x + 4.0, center),
            8.0,
            alpha(ink, opacity),
        );
        let mut x = x + 8.0;
        if let Some(count) = &self.count {
            x += 2.0;
            aura.place(count, point(x, center), alpha(ink, opacity));
            x += count.size().x / aura.zoom;
        }
        if self.warnings {
            aura.painter.circle_filled(
                point(x + 4.0, center),
                2.0 * aura.zoom,
                alpha(palette.warning(), opacity),
            );
            x += 6.0;
        }
        x
    }
}

/// A chip at the header's right end with the log count and, while running, the run's last
/// activity. It sits on its own backdrop, legible over any header gradient.
pub struct HeaderStatus {
    /// Graph units taken from the title, including the gap before the chip.
    pub reserved: f32,
    width: f32,
    /// Keeps clear of a badge on the node's top-right corner.
    inset: f32,
    logs: Option<LogItem>,
    activity: Option<(Arc<egui::Galley>, Tone)>,
}

impl HeaderStatus {
    pub fn new(aura: &Aura, status: &NodeStatus, activity: Option<&Activity>) -> Option<Self> {
        let zoom = aura.zoom;
        let activity = activity
            .filter(|_| status.run == Some(RunState::Running))
            .map(|activity| {
                (
                    aura.text(&activity.text, 8.0, FontFamily::Proportional),
                    activity.tone,
                )
            });
        let logs = status.logs.as_ref().map(|logs| LogItem {
            count: logs
                .count
                .as_deref()
                .map(|count| aura.text(count, 8.0, aura.medium.clone())),
            problems: logs.problems,
            warnings: logs.warnings,
        });
        if logs.is_none() && activity.is_none() {
            return None;
        }
        let mut width = 2.0 * CHIP_PAD;
        if let Some(item) = &logs {
            width += item.width(zoom);
        }
        if let Some((galley, _)) = &activity {
            if logs.is_some() {
                width += CHIP_GAP;
            }
            width += 7.0 + galley.size().x / zoom;
        }
        let corner = alarm(status).is_some() || status.run == Some(RunState::Done);
        let inset = if corner { CORNER_RADIUS } else { 0.0 };
        Some(Self {
            reserved: width + 4.0 + inset,
            width,
            inset,
            logs,
            activity,
        })
    }

    /// `origin` is the node's top-left corner; the chip ends at the header's right padding.
    pub fn paint(&self, aura: &Aura, origin: Pos2, node: &Node, center: f32, opacity: f32) {
        let zoom = aura.zoom;
        let palette = aura.palette;
        let point = |x: f32, y: f32| origin + vec2(x, y) * zoom;
        let left = node.width - 5.0 - self.inset - self.width;
        let ring = if self.logs.as_ref().is_some_and(|item| item.problems) {
            alpha(palette.destructive(), 0.5 * opacity)
        } else {
            alpha(palette.border, opacity)
        };
        aura.painter.rect(
            Rect::from_min_size(point(left, center - 6.0), vec2(self.width, 12.0) * zoom),
            6.0 * zoom,
            alpha(palette.card, 0.9 * opacity),
            Stroke::new(zoom, ring),
            egui::StrokeKind::Inside,
        );
        let mut x = left + CHIP_PAD;
        if let Some(item) = &self.logs {
            x = item.paint(aura, origin, x, center, opacity) + CHIP_GAP;
        }
        if let Some((galley, tone)) = &self.activity {
            Self::paint_activity(aura, point(x, center), galley, *tone, opacity);
        }
    }

    /// A live dot that breathes with the run's glow, then the last activity.
    fn paint_activity(
        aura: &Aura,
        start: Pos2,
        galley: &Arc<egui::Galley>,
        tone: Tone,
        opacity: f32,
    ) {
        let zoom = aura.zoom;
        let tint = tone_color(aura.palette, tone);
        let pulse = if aura.motion {
            0.45 + 0.55 * breath(aura.now, BREATH_MS)
        } else {
            1.0
        };
        let dot = start + vec2(2.5 * zoom, 0.0);
        aura.painter
            .circle_filled(dot, 4.5 * zoom, alpha(tint, 0.22 * pulse * opacity));
        aura.painter
            .circle_filled(dot, 2.5 * zoom, alpha(tint, pulse * opacity));
        aura.place(galley, start + vec2(7.0 * zoom, 0.0), alpha(tint, opacity));
    }
}

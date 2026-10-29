//! Wires traced from a pin: which edges and nodes it lights, and where the previews of its
//! off-screen ends sit on the canvas border.

use crate::geometry::Point;
use crate::model::{ConnectionKind, Direction, Endpoint, Scene, SceneIndex, Trace, WireTrace};
use crate::status::ease_out;
use std::collections::HashMap;

/// Opacity of the other wires while a path is focused.
pub const FOCUS_FADE: f32 = 0.25;
/// Screen gap between previews and the canvas edge, and between two previews.
pub const PREVIEW_MARGIN: f32 = 14.0;
const PREVIEW_GAP: f32 = 8.0;
/// How far a preview slides along the border per try when something else is in the way.
const SLIDE_STEP: f32 = 6.0;
/// Graph units of wire shown beside the previewed pin, and of board around its node.
const WIRE_ROOM: f32 = 56.0;
const NODE_ROOM: f32 = 10.0;
const MIN_PREVIEW_ZOOM: f32 = 0.55;
const MAX_PREVIEW_ZOOM: f32 = 1.0;
const HOVER_MS: f64 = 160.0;
/// A hovered pin's previews wait this long, so a pointer passing over pins does not flash
/// them, then fade in over `PEEK_MS`.
const PEEK_DELAY_MS: f64 = 180.0;
const PEEK_MS: f64 = 140.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Emphasis {
    #[default]
    None,
    Hover,
    Focus,
    /// The clicked wire of a focused path.
    Selected,
}

/// The host's trace resolved to scene indices. Rebuilt when the trace or the scene changes.
#[derive(Default)]
pub struct TraceState {
    pub trace: Trace,
    pub edges: Vec<Emphasis>,
    /// Reroutes on a traced path and the nodes at its ends.
    pub nodes: Vec<Emphasis>,
    /// When the pointer reached the hovered pin.
    hover_since: f64,
}

fn hovered(trace: &Trace) -> Option<&Endpoint> {
    trace.hover.as_ref()?.origin.as_ref()
}

impl TraceState {
    pub fn set(&mut self, trace: Trace, scene: &Scene, index: &SceneIndex, now: f64) {
        if hovered(&self.trace) != hovered(&trace) {
            self.hover_since = now;
        }
        self.trace = trace;
        self.resolve(scene, index);
    }

    pub fn resolve(&mut self, scene: &Scene, index: &SceneIndex) {
        self.edges = vec![Emphasis::None; scene.edges.len()];
        self.nodes = vec![Emphasis::None; scene.nodes.len()];
        if self.is_empty() {
            return;
        }
        let edges: HashMap<&str, usize> = scene
            .edges
            .iter()
            .enumerate()
            .map(|(position, edge)| (edge.id.as_str(), position))
            .collect();
        for (wires, level) in [
            (&self.trace.hover, Emphasis::Hover),
            (&self.trace.focus, Emphasis::Focus),
        ] {
            let Some(wires) = wires else { continue };
            for id in &wires.edges {
                if let Some(&position) = edges.get(id.as_str()) {
                    self.edges[position] = self.edges[position].max(level);
                }
            }
            for id in wires
                .reroutes
                .iter()
                .chain(wires.ends.iter().map(|end| &end.node_id))
            {
                if let Some(&position) = index.nodes.get(id) {
                    self.nodes[position] = self.nodes[position].max(level);
                }
            }
        }
        if let Some(&position) = self
            .trace
            .focus
            .as_ref()
            .and_then(|focus| focus.segment.as_ref())
            .and_then(|segment| edges.get(segment.as_str()))
        {
            self.edges[position] = Emphasis::Selected;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.trace.hover.is_none() && self.trace.focus.is_none()
    }

    pub fn focused(&self) -> bool {
        self.trace.focus.is_some()
    }

    /// Whether the trace lights the edge at `position`, so it is drawn with the trace.
    pub fn lit(&self, position: usize) -> bool {
        self.edges
            .get(position)
            .is_some_and(|emphasis| *emphasis != Emphasis::None)
    }

    /// Opacity of the wires the trace does not light: faded while a path is focused.
    pub fn unlit_opacity(&self) -> f32 {
        if self.focused() { FOCUS_FADE } else { 1.0 }
    }

    /// The trace whose off-screen ends get previews, and its emphasis: the focused one, else
    /// the hovered one.
    pub fn previewed(&self) -> Option<(&WireTrace, Emphasis)> {
        match (&self.trace.focus, &self.trace.hover) {
            (Some(focus), _) => Some((focus, Emphasis::Focus)),
            (None, Some(hover)) => Some((hover, Emphasis::Hover)),
            (None, None) => None,
        }
    }

    /// How far the hovered pin's previews have faded in, from 0 to 1.
    pub fn peek(&self, now: f64) -> f32 {
        ease_out(((now - self.hover_since - PEEK_DELAY_MS) / PEEK_MS).clamp(0.0, 1.0) as f32)
    }
}

/// The preview under the pointer eases in while the one it left eases out.
#[derive(Default)]
pub struct PreviewHover {
    current: Option<Endpoint>,
    previous: Option<Endpoint>,
    since: f64,
}

impl PreviewHover {
    /// Returns whether the hovered preview changed.
    pub fn set(&mut self, endpoint: Option<Endpoint>, now: f64) -> bool {
        if self.current == endpoint {
            return false;
        }
        self.previous = self.current.take();
        self.current = endpoint;
        self.since = now;
        true
    }

    fn progress(&self, now: f64) -> f32 {
        ((now - self.since) / HOVER_MS).clamp(0.0, 1.0) as f32
    }

    /// From 0 when the pointer is elsewhere to 1 when it rests on the preview.
    pub fn strength(&self, endpoint: &Endpoint, now: f64) -> f32 {
        let eased = ease_out(self.progress(now));
        if self.current.as_ref() == Some(endpoint) {
            eased
        } else if self.previous.as_ref() == Some(endpoint) {
            1.0 - eased
        } else {
            0.0
        }
    }

    pub fn settling(&self, now: f64) -> bool {
        (self.current.is_some() || self.previous.is_some()) && self.progress(now) < 1.0
    }
}

/// The side of a node a pin's wire leaves from.
pub fn outward(direction: Direction, kind: ConnectionKind) -> Point {
    match (kind, direction) {
        (ConnectionKind::Reference, Direction::Input) => [0.0, -1.0],
        (ConnectionKind::Reference, Direction::Output) => [0.0, 1.0],
        (_, Direction::Input) => [-1.0, 0.0],
        (_, Direction::Output) => [1.0, 0.0],
    }
}

/// A preview window's camera, in graph units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewCamera {
    pub zoom: f32,
    pub center: Point,
}

/// Frames a node (`[x, y, width, height]`) in a `size` window with room for the wire that
/// arrives at `pin`. A node too large for the window is framed around the pin.
pub fn preview_camera(
    bounds: [f32; 4],
    pin: Point,
    outward: Point,
    size: [f32; 2],
) -> PreviewCamera {
    let mut min = [bounds[0] - NODE_ROOM, bounds[1] - NODE_ROOM];
    let mut max = [
        bounds[0] + bounds[2] + NODE_ROOM,
        bounds[1] + bounds[3] + NODE_ROOM,
    ];
    for axis in 0..2 {
        if outward[axis] < 0.0 {
            min[axis] -= WIRE_ROOM;
        } else if outward[axis] > 0.0 {
            max[axis] += WIRE_ROOM;
        }
    }
    let zoom = (size[0] / (max[0] - min[0]))
        .min(size[1] / (max[1] - min[1]))
        .clamp(MIN_PREVIEW_ZOOM, MAX_PREVIEW_ZOOM);
    let center = std::array::from_fn(|axis| {
        let half = size[axis] / zoom * 0.5;
        if max[axis] - min[axis] <= half * 2.0 {
            (min[axis] + max[axis]) * 0.5
        } else {
            pin[axis].clamp(min[axis] + half, max[axis] - half)
        }
    });
    PreviewCamera { zoom, center }
}

/// The rectangle that preview centres may take: `area` inset by the margin and half a card.
fn centre_bounds(area: [f32; 4], size: [f32; 2]) -> Option<(Point, Point)> {
    let min = [
        area[0] + PREVIEW_MARGIN + size[0] * 0.5,
        area[1] + PREVIEW_MARGIN + size[1] * 0.5,
    ];
    let max = [
        area[0] + area[2] - PREVIEW_MARGIN - size[0] * 0.5,
        area[1] + area[3] - PREVIEW_MARGIN - size[1] * 0.5,
    ];
    (min[0] < max[0] && min[1] < max[1]).then_some((min, max))
}

/// Where the ray from `origin` (inside the bounds) toward `target` leaves them.
fn exit(origin: Point, target: Point, min: Point, max: Point) -> Point {
    let delta = [target[0] - origin[0], target[1] - origin[1]];
    let mut reach = f32::INFINITY;
    for axis in 0..2 {
        if delta[axis] > 0.0 {
            reach = reach.min((max[axis] - origin[axis]) / delta[axis]);
        } else if delta[axis] < 0.0 {
            reach = reach.min((min[axis] - origin[axis]) / delta[axis]);
        }
    }
    if !reach.is_finite() {
        return origin;
    }
    [origin[0] + delta[0] * reach, origin[1] + delta[1] * reach]
}

/// Where the ray from `from` toward `target` meets the border of `area` inset by `inset`.
pub fn border_point(area: [f32; 4], inset: f32, from: Point, target: Point) -> Point {
    let min = [area[0] + inset, area[1] + inset];
    let max = [area[0] + area[2] - inset, area[1] + area[3] - inset];
    exit(
        [from[0].clamp(min[0], max[0]), from[1].clamp(min[1], max[1])],
        target,
        min,
        max,
    )
}

/// Distance along the border, clockwise from the top-left corner, of a point on it.
fn along(point: Point, min: Point, max: Point) -> f32 {
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    let sides = [
        (point[1] - min[1]).abs(),
        (max[0] - point[0]).abs(),
        (max[1] - point[1]).abs(),
        (point[0] - min[0]).abs(),
    ];
    let nearest = (0..4)
        .min_by(|a, b| sides[*a].total_cmp(&sides[*b]))
        .unwrap_or(0);
    match nearest {
        0 => point[0] - min[0],
        1 => width + point[1] - min[1],
        2 => width + height + max[0] - point[0],
        _ => 2.0 * width + height + max[1] - point[1],
    }
}

/// The point `distance` clockwise along the border from its top-left corner.
fn at(distance: f32, min: Point, max: Point) -> Point {
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    let distance = distance.rem_euclid(2.0 * (width + height));
    if distance < width {
        [min[0] + distance, min[1]]
    } else if distance < width + height {
        [max[0], min[1] + distance - width]
    } else if distance < 2.0 * width + height {
        [max[0] - (distance - width - height), max[1]]
    } else {
        [min[0], max[1] - (distance - 2.0 * width - height)]
    }
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Where a wire running from an off-screen end first enters `[min, max]`, between the last
/// sample outside and the first inside. `None` when no part of it is inside.
pub fn entry(points: impl IntoIterator<Item = Point>, min: Point, max: Point) -> Option<Point> {
    let inside =
        |point: Point| (0..2).all(|axis| point[axis] >= min[axis] && point[axis] <= max[axis]);
    let mut outside = None;
    for point in points {
        if !inside(point) {
            outside = Some(point);
            continue;
        }
        let mut from = outside?;
        let mut to = point;
        for _ in 0..12 {
            let middle = [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5];
            if inside(middle) {
                to = middle;
            } else {
                from = middle;
            }
        }
        return Some(to);
    }
    None
}

/// Card centres on the border of `area` (`[x, y, width, height]`), each as close to its
/// anchor as the border allows, slid along it clear of `avoid` and of earlier cards. `None`
/// where no place is free or the canvas is too small for a card.
pub fn place_previews(
    area: [f32; 4],
    size: [f32; 2],
    anchors: &[Point],
    avoid: &[[f32; 4]],
) -> Vec<Option<Point>> {
    let Some((min, max)) = centre_bounds(area, size) else {
        return vec![None; anchors.len()];
    };
    let half = [size[0] * 0.5 + PREVIEW_GAP, size[1] * 0.5 + PREVIEW_GAP];
    let card = |centre: Point| {
        [
            centre[0] - half[0],
            centre[1] - half[1],
            centre[0] + half[0],
            centre[1] + half[1],
        ]
    };
    let mut taken: Vec<[f32; 4]> = avoid
        .iter()
        .map(|rect| [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]])
        .collect();
    let perimeter = 2.0 * (max[0] - min[0] + max[1] - min[1]);
    let tries = (perimeter / SLIDE_STEP).ceil() as usize;
    anchors
        .iter()
        .map(|anchor| {
            let nearest = [
                anchor[0].clamp(min[0], max[0]),
                anchor[1].clamp(min[1], max[1]),
            ];
            let start = along(nearest, min, max);
            let free = (0..=tries)
                .flat_map(|step| [step as f32, -(step as f32)])
                .map(|step| at(start + step * SLIDE_STEP, min, max))
                .find(|centre| !taken.iter().any(|rect| overlaps(card(*centre), *rect)))?;
            taken.push(card(free));
            Some(free)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Endpoint, WireTrace, fixture};

    const AREA: [f32; 4] = [0.0, 0.0, 1000.0, 600.0];
    const CARD: [f32; 2] = [180.0, 110.0];

    fn pin(node: &str, pin: &str) -> Endpoint {
        Endpoint {
            node_id: node.into(),
            pin_id: pin.into(),
        }
    }

    fn wires(edges: &[&str], ends: &[(&str, &str)]) -> WireTrace {
        WireTrace {
            origin: Some(pin("a", "out")),
            edges: edges.iter().map(|id| id.to_string()).collect(),
            reroutes: Vec::new(),
            ends: ends.iter().map(|(node, id)| pin(node, id)).collect(),
            segment: None,
        }
    }

    fn hovering(trace: WireTrace) -> Trace {
        Trace {
            hover: Some(trace),
            ..Trace::default()
        }
    }

    fn card_rect(centre: Point) -> [f32; 4] {
        [
            centre[0] - CARD[0] * 0.5,
            centre[1] - CARD[1] * 0.5,
            centre[0] + CARD[0] * 0.5,
            centre[1] + CARD[1] * 0.5,
        ]
    }

    #[test]
    fn focus_outranks_hover_and_unknown_ids_are_ignored() {
        let scene = fixture();
        let index = SceneIndex::new(&scene).unwrap();
        let mut state = TraceState::default();
        state.set(
            Trace {
                hover: Some(wires(&["ab", "gone"], &[("b", "in")])),
                focus: Some(wires(&["ab"], &[("c", "missing")])),
                avoid: Vec::new(),
            },
            &scene,
            &index,
            0.0,
        );
        assert_eq!(state.edges, vec![Emphasis::Focus]);
        assert_eq!(
            state.nodes,
            vec![Emphasis::None, Emphasis::Hover, Emphasis::Focus]
        );
        assert!(state.focused());
        assert_eq!(
            state.previewed().map(|(_, level)| level),
            Some(Emphasis::Focus)
        );
        state.set(Trace::default(), &scene, &index, 0.0);
        assert!(state.is_empty());
        assert_eq!(state.edges, vec![Emphasis::None]);
        assert!(state.previewed().is_none());
    }

    #[test]
    fn a_clicked_wire_outranks_the_path_it_lies_on() {
        let scene = fixture();
        let index = SceneIndex::new(&scene).unwrap();
        let mut state = TraceState::default();
        let path = WireTrace {
            origin: None,
            segment: Some("ab".into()),
            ..wires(&["ab"], &[("a", "out"), ("b", "in")])
        };
        state.set(
            Trace {
                focus: Some(path),
                ..Trace::default()
            },
            &scene,
            &index,
            0.0,
        );
        assert_eq!(state.edges, vec![Emphasis::Selected]);
        assert_eq!(state.nodes[0], Emphasis::Focus);
        assert!(state.lit(0) && !state.lit(1));
        assert_eq!(state.unlit_opacity(), FOCUS_FADE);
    }

    #[test]
    fn hovered_previews_wait_before_fading_in() {
        let scene = fixture();
        let index = SceneIndex::new(&scene).unwrap();
        let mut state = TraceState::default();
        state.set(hovering(wires(&["ab"], &[])), &scene, &index, 1000.0);
        assert_eq!(
            state.previewed().map(|(_, level)| level),
            Some(Emphasis::Hover)
        );
        assert_eq!(state.peek(1000.0 + PEEK_DELAY_MS), 0.0);
        assert!(state.peek(1000.0 + PEEK_DELAY_MS + PEEK_MS * 0.5) > 0.0);
        // The same pin, retraced for a new scene, keeps its previews.
        state.set(hovering(wires(&["ab"], &[])), &scene, &index, 5000.0);
        assert_eq!(state.peek(5000.0), 1.0);
        let other = WireTrace {
            origin: Some(pin("b", "in")),
            ..wires(&["ab"], &[])
        };
        state.set(hovering(other), &scene, &index, 6000.0);
        assert_eq!(state.peek(6000.0), 0.0);
    }

    #[test]
    fn previews_sit_on_the_border_nearest_their_anchor() {
        let from = [500.0, 300.0];
        let anchors: Vec<_> = [[5000.0, 300.0], [500.0, -4000.0], [-900.0, 300.0]]
            .map(|target| border_point(AREA, 0.0, from, target))
            .to_vec();
        let placed = place_previews(AREA, CARD, &anchors, &[]);
        let right = placed[0].unwrap();
        assert_eq!(right, [1000.0 - PREVIEW_MARGIN - 90.0, 300.0]);
        let top = placed[1].unwrap();
        assert_eq!(top, [500.0, PREVIEW_MARGIN + 55.0]);
        let left = placed[2].unwrap();
        assert_eq!(left, [PREVIEW_MARGIN + 90.0, 300.0]);
    }

    #[test]
    fn previews_in_one_direction_slide_apart_along_the_border() {
        let placed = place_previews(
            AREA,
            CARD,
            &[[1000.0, 300.0], [1000.0, 310.0], [1000.0, 290.0]],
            &[],
        );
        let cards: Vec<_> = placed
            .iter()
            .map(|centre| card_rect(centre.unwrap()))
            .collect();
        for (i, a) in cards.iter().enumerate() {
            for b in &cards[i + 1..] {
                assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
            }
        }
        assert!(
            cards
                .iter()
                .all(|card| card[2] <= AREA[2] - PREVIEW_MARGIN + 0.01)
        );
    }

    #[test]
    fn previews_keep_clear_of_host_chrome() {
        let minimap = [784.0, 434.0, 200.0, 150.0];
        let placed = place_previews(AREA, CARD, &[[880.0, 600.0]], &[minimap]);
        let card = card_rect(placed[0].unwrap());
        assert!(!overlaps(
            card,
            [
                minimap[0],
                minimap[1],
                minimap[0] + minimap[2],
                minimap[1] + minimap[3]
            ]
        ));
    }

    #[test]
    fn a_canvas_smaller_than_a_card_has_no_previews() {
        assert_eq!(
            place_previews([0.0, 0.0, 150.0, 90.0], CARD, &[[150.0, 45.0]], &[]),
            vec![None]
        );
    }

    #[test]
    fn a_wire_enters_the_view_where_it_crosses_the_border() {
        let (min, max) = ([0.0, 0.0], [1000.0, 600.0]);
        let wire = [
            [1500.0, 900.0],
            [1200.0, 700.0],
            [900.0, 500.0],
            [600.0, 300.0],
        ];
        let [x, y] = entry(wire, min, max).unwrap();
        // The segment from (1200, 700) to (900, 500) crosses x = 1000 at y ≈ 566.7.
        assert!(
            (x - 1000.0).abs() < 0.2 && (y - 566.7).abs() < 0.2,
            "{x}, {y}"
        );
        assert!(entry([[1500.0, 900.0], [1200.0, 700.0]], min, max).is_none());
        assert!(entry([[500.0, 300.0]], min, max).is_none());
    }

    #[test]
    fn hovering_eases_the_new_preview_in_and_the_old_one_out() {
        let at = |node: &str| Endpoint {
            node_id: node.into(),
            pin_id: "in".into(),
        };
        let mut hover = PreviewHover::default();
        assert!(!hover.settling(0.0));
        assert!(hover.set(Some(at("a")), 0.0));
        assert!(!hover.set(Some(at("a")), 10.0));
        assert_eq!(hover.strength(&at("a"), 0.0), 0.0);
        assert!(hover.settling(80.0));
        assert_eq!(hover.strength(&at("a"), HOVER_MS), 1.0);
        assert!(!hover.settling(HOVER_MS));
        assert!(hover.set(Some(at("b")), 1000.0));
        assert_eq!(hover.strength(&at("a"), 1000.0), 1.0);
        assert_eq!(hover.strength(&at("b"), 1000.0 + HOVER_MS), 1.0);
        assert_eq!(hover.strength(&at("a"), 1000.0 + HOVER_MS), 0.0);
        assert!(hover.set(None, 2000.0));
        assert_eq!(hover.strength(&at("b"), 2000.0 + HOVER_MS), 0.0);
        assert_eq!(hover.strength(&at("c"), 2000.0), 0.0);
    }

    #[test]
    fn border_positions_round_trip() {
        let (min, max) = ([10.0, 20.0], [110.0, 70.0]);
        for distance in [0.0, 40.0, 100.0, 130.0, 180.0, 250.0, 290.0] {
            let point = at(distance, min, max);
            assert!(
                (along(point, min, max) - distance).abs() < 0.01,
                "{distance}"
            );
        }
    }

    #[test]
    fn a_small_node_is_framed_whole_with_room_for_its_wire() {
        let camera = preview_camera([0.0, 0.0, 150.0, 58.0], [0.0, 28.0], [-1.0, 0.0], CARD);
        assert!(camera.zoom < 1.0 && camera.zoom >= MIN_PREVIEW_ZOOM);
        let half = CARD[0] / camera.zoom * 0.5;
        assert!(camera.center[0] - half <= -WIRE_ROOM - NODE_ROOM + 0.01);
        assert!(camera.center[0] + half >= 150.0 + NODE_ROOM - 0.01);
        assert_eq!(camera.center[1], 29.0);
    }

    #[test]
    fn a_tall_node_is_framed_around_the_pin() {
        let camera = preview_camera([0.0, 0.0, 150.0, 900.0], [150.0, 640.0], [1.0, 0.0], CARD);
        assert_eq!(camera.zoom, MIN_PREVIEW_ZOOM);
        assert_eq!(camera.center[1], 640.0);
        assert_eq!(
            outward(Direction::Output, ConnectionKind::Reference),
            [0.0, 1.0]
        );
    }
}

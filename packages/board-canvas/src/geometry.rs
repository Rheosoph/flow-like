use crate::model::{ConnectionKind, Direction, Edge, Scene, SceneIndex};

pub type Point = [f32; 2];

/// Inter's hhea ascender and descender, in em.
const INTER_ASCENT: f32 = 1984.0 / 2048.0;
const INTER_DESCENT: f32 = 494.0 / 2048.0;

/// Where the DOM paints the baseline of a line box starting at `top`, so native text stays still
/// when the DOM node replaces it. Both engines round Inter's ascent and descent to whole pixels.
/// Chrome floors the leading above the baseline and snaps the baseline to a whole pixel;
/// WebKit keeps the exact half-leading.
pub fn dom_baseline(top: f32, size: f32, line_height: f32, webkit: bool) -> f32 {
    let ascent = (size * INTER_ASCENT).round();
    let descent = (size * INTER_DESCENT).round();
    let leading = (line_height - ascent - descent) * 0.5;
    if webkit {
        top + ascent + leading
    } else {
        (top + ascent + leading.floor()).round()
    }
}

/** A convex header outline with rounded top corners and a square bottom edge. */
pub fn header_outline(width: f32, height: f32, radius: f32) -> Vec<Point> {
    let radius = radius.max(0.0).min(width * 0.5).min(height);
    if radius == 0.0 {
        return vec![[0.0, height], [width, height], [width, 0.0], [0.0, 0.0]];
    }
    let mut points = Vec::with_capacity(12);
    points.extend([[0.0, height], [width, height]]);
    const ARC: [Point; 5] = [
        [1.0, 0.0],
        [0.9238795, -0.3826834],
        [
            std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
        ],
        [0.3826834, -0.9238795],
        [0.0, -1.0],
    ];
    for left in [false, true] {
        let center_x = if left { radius } else { width - radius };
        for [x, y] in ARC {
            let [x, y] = if left { [y, -x] } else { [x, y] };
            points.push([
                (center_x + radius * x).clamp(0.0, width),
                (radius + radius * y).clamp(0.0, height),
            ]);
        }
    }
    points
}

#[derive(Clone, Copy, PartialEq)]
pub struct CurveKey {
    pub from: Point,
    pub to: Point,
    pub from_direction: Direction,
    pub to_direction: Direction,
    pub from_vertical: bool,
    pub to_vertical: bool,
}

impl CurveKey {
    pub fn for_edge(edge: &Edge, scene: &Scene, index: &SceneIndex) -> Option<Self> {
        let (source, source_pin) = index.endpoint(scene, &edge.source)?;
        let (target, target_pin) = index.endpoint(scene, &edge.target)?;
        Some(Self {
            from: [source.x + source_pin.x, source.y + source_pin.y],
            to: [target.x + target_pin.x, target.y + target_pin.y],
            from_direction: source_pin.direction,
            to_direction: target_pin.direction,
            from_vertical: source_pin.kind == ConnectionKind::Reference,
            to_vertical: target_pin.kind == ConnectionKind::Reference,
        })
    }
}

pub struct Curve {
    pub key: CurveKey,
    pub points: Vec<Point>,
    pub coarse: Vec<Point>,
    pub coarse_error: f32,
    chord: [Point; 2],
    chord_error: f32,
    pub bounds: [f32; 4],
}

fn handle_offset(distance: f32) -> f32 {
    if distance >= 0.0 {
        distance * 0.5
    } else {
        6.25 * (-distance).sqrt()
    }
}

fn distance_to_segment(point: Point, start: Point, end: Point) -> f32 {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let length_sq = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if length_sq > 0.0 {
        (((point[0] - start[0]) * delta[0] + (point[1] - start[1]) * delta[1]) / length_sq)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point[0] - start[0] - delta[0] * t).hypot(point[1] - start[1] - delta[1] * t)
}

impl Curve {
    pub fn new(key: CurveKey) -> Self {
        let a = key.from;
        let d = key.to;
        let from_axis = usize::from(key.from_vertical);
        let to_axis = usize::from(key.to_vertical);
        let mut b = a;
        let mut c = d;
        b[from_axis] += key.from_direction.sign()
            * handle_offset((d[from_axis] - a[from_axis]) * key.from_direction.sign());
        c[to_axis] += key.to_direction.sign()
            * handle_offset((a[to_axis] - d[to_axis]) * key.to_direction.sign());
        let points: Vec<_> = (0..=32)
            .map(|i| {
                let t = i as f32 / 32.0;
                let u = 1.0 - t;
                std::array::from_fn(|axis| {
                    a[axis] * u.powi(3)
                        + b[axis] * 3.0 * u * u * t
                        + c[axis] * 3.0 * u * t * t
                        + d[axis] * t.powi(3)
                })
            })
            .collect();
        // Clamp to the segment so backwards wires retain endpoint overshoot and retracing.
        let chord_error = points
            .iter()
            .map(|point| distance_to_segment(*point, a, d))
            .fold(0.0, f32::max);
        let coarse: Vec<_> = points.iter().step_by(4).copied().collect();
        let mut coarse_error: f32 = 0.0;
        for (i, point) in points.iter().enumerate().take(32) {
            let start = coarse[i / 4];
            let end = coarse[i / 4 + 1];
            let delta = [end[0] - start[0], end[1] - start[1]];
            let length_sq = delta[0] * delta[0] + delta[1] * delta[1];
            let t = (((point[0] - start[0]) * delta[0] + (point[1] - start[1]) * delta[1])
                / length_sq.max(0.001))
            .clamp(0.0, 1.0);
            coarse_error = coarse_error.max(
                (point[0] - start[0] - delta[0] * t).hypot(point[1] - start[1] - delta[1] * t),
            );
        }
        let hull = [a, b, c, d];
        let bounds = [
            hull.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - 4.0,
            hull.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min) - 4.0,
            hull.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max) + 4.0,
            hull.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max) + 4.0,
        ];
        Self {
            key,
            points,
            coarse,
            coarse_error,
            chord: [a, d],
            chord_error,
            bounds,
        }
    }

    pub fn samples(&self, simplify: bool, zoom: f32) -> &[Point] {
        if simplify && self.chord_error * zoom <= 0.75 {
            &self.chord
        } else if simplify && self.coarse_error * zoom <= 0.75 {
            &self.coarse
        } else {
            &self.points
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Full,
    Compact,
    Overview,
    Tiny,
}

impl Detail {
    pub fn for_zoom(lod: bool, zoom: f32) -> Self {
        if !lod || zoom >= 0.5 {
            Self::Full
        } else if zoom >= 0.2 {
            Self::Compact
        } else if zoom >= 0.1 {
            Self::Overview
        } else {
            Self::Tiny
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Overview => "overview",
            Self::Tiny => "tiny",
        }
    }
    pub fn labels(self) -> bool {
        matches!(self, Self::Full | Self::Compact)
    }
    pub fn simplify_edges(self) -> bool {
        matches!(self, Self::Overview | Self::Tiny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baselines_match_measured_chrome_and_webkit_layout() {
        // Line boxes and baselines measured on production nodes, in CSS pixels.
        // Chrome floors the half-leading: Inter at 9.6px on 12px lines sits 9px below the top.
        assert_eq!(dom_baseline(23.0, 9.6, 12.0, false), 32.0);
        // The title box starts at 2.515625 after flex centering; Chrome snaps 11.52 to 12.
        assert_eq!(dom_baseline(2.515625, 9.6, 12.0, false), 12.0);
        // WebKit keeps the half-leading.
        assert_eq!(dom_baseline(23.0, 9.6, 12.0, true), 32.5);
        assert_eq!(dom_baseline(2.5, 9.6, 12.0, true), 12.0);
        // Chrome cases from isolated line boxes: (size, line height, baseline from the top).
        for (size, line, baseline) in [
            (9.6, 9.6, 8.0),
            (9.6, 11.5, 9.0),
            (12.0, 15.0, 12.0),
            (7.0, 7.0, 6.0),
            (8.0, 8.0, 7.0),
            (5.76, 12.0, 8.0),
            (11.52, 14.4, 11.0),
        ] {
            assert_eq!(dom_baseline(100.0, size, line, false), 100.0 + baseline);
        }
    }

    #[test]
    fn header_gradient_avoids_corner_cutouts_but_keeps_square_separator() {
        let points = header_outline(148.0, 15.0, 3.0);
        assert_eq!(&points[..2], &[[0.0, 15.0], [148.0, 15.0]]);
        assert!(!points.contains(&[0.0, 0.0]));
        assert!(!points.contains(&[148.0, 0.0]));
        assert!(points.contains(&[3.0, 0.0]));
        assert!(points.contains(&[145.0, 0.0]));
        for point in &points {
            assert!((0.0..=148.0).contains(&point[0]));
            assert!((0.0..=15.0).contains(&point[1]));
            if point[1] < 3.0 {
                let center_x = if point[0] < 74.0 { 3.0 } else { 145.0 };
                assert!((point[0] - center_x).hypot(point[1] - 3.0) <= 3.0001);
            }
        }
        let narrow = header_outline(6.0, 3.0, 200.0);
        assert_eq!(narrow.last(), Some(&[0.0, 3.0]));
        assert!(narrow.contains(&[3.0, 0.0]));
        assert_eq!(header_outline(148.0, 15.0, 0.0).len(), 4);
    }

    #[test]
    fn curves_follow_real_pin_offsets_and_moved_nodes() {
        let mut scene = crate::model::fixture();
        let index = SceneIndex::new(&scene).unwrap();
        let key = CurveKey::for_edge(&scene.edges[0], &scene, &index).unwrap();
        assert_eq!(key.from, [100.0, 30.0]);
        assert_eq!(key.to, [200.0, 30.0]);
        scene.nodes[1].y = 90.0;
        let curve = Curve::new(CurveKey::for_edge(&scene.edges[0], &scene, &index).unwrap());
        assert_eq!(curve.points.first(), Some(&[100.0, 30.0]));
        assert_eq!(curve.points.last(), Some(&[200.0, 120.0]));
    }

    #[test]
    fn backwards_wire_hull_covers_every_sample() {
        let curve = Curve::new(CurveKey {
            from: [300.0, 0.0],
            to: [0.0, 100.0],
            from_direction: Direction::Output,
            to_direction: Direction::Input,
            from_vertical: false,
            to_vertical: false,
        });
        assert!(curve.bounds[0] < 0.0 && curve.bounds[2] > 300.0);
        for point in &curve.points {
            assert!(point[0] >= curve.bounds[0] && point[0] <= curve.bounds[2]);
            assert!(point[1] >= curve.bounds[1] && point[1] <= curve.bounds[3]);
        }
        assert_eq!(curve.samples(false, 0.01).len(), 33);
        assert_eq!(curve.samples(true, 0.1).len(), 9);
    }

    #[test]
    fn straight_overview_wires_use_one_segment_without_changing_detail() {
        for vertical in [false, true] {
            let curve = Curve::new(CurveKey {
                from: [20.0, 30.0],
                to: if vertical {
                    [20.0, 230.0]
                } else {
                    [220.0, 30.0]
                },
                from_direction: Direction::Output,
                to_direction: Direction::Input,
                from_vertical: vertical,
                to_vertical: vertical,
            });
            for zoom in [0.02, 0.1, 0.19] {
                assert_eq!(curve.samples(true, zoom), &[curve.key.from, curve.key.to]);
                assert_eq!(curve.samples(false, zoom).len(), 33);
            }
        }
    }

    #[test]
    fn bowed_wire_chord_respects_the_screen_error_budget() {
        let curve = Curve::new(CurveKey {
            from: [0.0, 0.0],
            to: [200.0, 160.0],
            from_direction: Direction::Output,
            to_direction: Direction::Input,
            from_vertical: false,
            to_vertical: false,
        });
        assert!(curve.chord_error > 1.0);
        let accepted_zoom = 0.74 / curve.chord_error;
        assert_eq!(curve.samples(true, accepted_zoom).len(), 2);
        for point in &curve.points {
            // This forward curve projects inside its endpoint segment.
            let distance = (point[0] * 160.0 - point[1] * 200.0).abs() / 200.0_f32.hypot(160.0);
            assert!(distance * accepted_zoom <= 0.75);
        }
        assert_eq!(curve.samples(true, 0.76 / curve.chord_error).len(), 9);
        assert_eq!(curve.samples(true, 0.76 / curve.coarse_error).len(), 33);
        assert_eq!(curve.samples(false, accepted_zoom).len(), 33);
    }

    #[test]
    fn collinear_backwards_overshoot_is_not_mistaken_for_a_straight_segment() {
        let curve = Curve::new(CurveKey {
            from: [300.0, 0.0],
            to: [0.0, 0.0],
            from_direction: Direction::Output,
            to_direction: Direction::Input,
            from_vertical: false,
            to_vertical: false,
        });
        let overshoot = curve
            .points
            .iter()
            .map(|point| (point[0] - point[0].clamp(0.0, 300.0)).abs())
            .fold(0.0, f32::max);
        assert!(overshoot > 1.0);
        assert!((curve.chord_error - overshoot).abs() < 0.0001);
        assert!(curve.samples(true, 0.76 / overshoot).len() > 2);
        assert_eq!(curve.samples(true, 0.74 / overshoot).len(), 2);
    }

    #[test]
    fn missing_endpoint_is_not_a_painter_panic() {
        let mut scene = crate::model::fixture();
        scene.edges[0].target.pin_id = "gone".into();
        let index = SceneIndex::new(&scene).unwrap();
        assert!(CurveKey::for_edge(&scene.edges[0], &scene, &index).is_none());
    }

    #[test]
    fn reference_wires_leave_bottom_and_enter_top() {
        let mut scene = crate::model::fixture();
        scene.nodes[0].pins[0].kind = ConnectionKind::Reference;
        scene.nodes[1].pins[0].kind = ConnectionKind::Reference;
        scene.nodes[1].y = 200.0;
        let index = SceneIndex::new(&scene).unwrap();
        let curve = Curve::new(CurveKey::for_edge(&scene.edges[0], &scene, &index).unwrap());
        let first = curve.points[1];
        assert!(first[1] - curve.key.from[1] > first[0] - curve.key.from[0]);
        let last = curve.points[31];
        assert!(curve.key.to[1] - last[1] > curve.key.to[0] - last[0]);
    }

    #[test]
    fn selected_detail_can_be_restored_at_every_zoom() {
        for zoom in [0.01, 0.1, 0.2, 0.49, 0.5, 3.0] {
            assert_eq!(Detail::for_zoom(false, zoom), Detail::Full);
        }
        assert_eq!(Detail::for_zoom(true, 0.1), Detail::Overview);
        assert_eq!(Detail::for_zoom(true, 0.2), Detail::Compact);
        assert_eq!(Detail::for_zoom(true, 0.5), Detail::Full);
    }
}

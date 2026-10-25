//! Run state, logs and lint marks drawn over the scene, updated without rebuilding it.

use crate::geometry::Point;
use serde::Deserialize;
use std::collections::HashMap;

pub const RIPPLE_MS: f64 = 700.0;
/// A burst of finishing nodes plays a bounded number of ripples.
const MAX_RIPPLES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Running,
    Done,
}

/// What an execution wire shows while its run is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Both ends finished: the path the run took.
    Trail,
    /// The target is running: execution flows into it.
    Active,
}

/// Log levels of a run (warning and up) and lint findings (info and up).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Info => "glyph:info",
            Self::Warning => "glyph:triangle-alert",
            Self::Error => "glyph:circle-x",
            Self::Fatal => "glyph:ban",
        }
    }
}

/// How recently the node's run reported progress.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Active,
    Stale,
    Warning,
    #[default]
    Inactive,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogBadge {
    /// Formatted by the host. Absent until the run's log summary arrives.
    pub count: Option<String>,
    #[serde(default)]
    pub problems: bool,
    #[serde(default)]
    pub warnings: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Heat {
    /// Visits relative to the busiest node, from 0 to 1.
    pub intensity: f32,
    pub visits: String,
    pub errors: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QualityMark {
    pub severity: Severity,
    pub count: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatus {
    pub run: Option<RunState>,
    pub run_id: Option<String>,
    pub logs: Option<LogBadge>,
    /// The worst level the node logged in the current run.
    pub severity: Option<Severity>,
    pub heat: Option<Heat>,
    pub quality: Option<QualityMark>,
    /// A collaborator's run is executing this node.
    #[serde(default)]
    pub remote: bool,
}

impl NodeStatus {
    pub fn moving(&self) -> bool {
        self.remote || self.run == Some(RunState::Running)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub text: String,
    #[serde(default)]
    pub tone: Tone,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate {
    #[serde(default)]
    pub nodes: HashMap<String, NodeStatus>,
    /// False under `prefers-reduced-motion`.
    #[serde(default = "enabled")]
    pub motion: bool,
}

pub struct Statuses {
    nodes: HashMap<String, NodeStatus>,
    /// The shared "last activity" clock of each run, keyed by run id.
    activity: HashMap<String, Activity>,
    motion: bool,
    ripples: HashMap<String, f64>,
}

impl Default for Statuses {
    fn default() -> Self {
        Self {
            nodes: HashMap::new(),
            activity: HashMap::new(),
            motion: true,
            ripples: HashMap::new(),
        }
    }
}

impl Statuses {
    pub fn get(&self, id: &str) -> Option<&NodeStatus> {
        self.nodes.get(id)
    }

    pub fn activity(&self, run_id: &str) -> Option<&Activity> {
        self.activity.get(run_id)
    }

    pub fn motion(&self) -> bool {
        self.motion
    }

    /// Replaces every status. A node seen running that has now finished starts its completion
    /// ripple; nodes that finish between two updates were never visibly running and stay quiet.
    pub fn update(&mut self, update: StatusUpdate, now: f64) {
        if update.motion {
            for (id, status) in &update.nodes {
                let finished = status.run == Some(RunState::Done)
                    && self.nodes.get(id).and_then(|previous| previous.run)
                        == Some(RunState::Running);
                if finished && self.ripples.len() < MAX_RIPPLES {
                    self.ripples.insert(id.clone(), now);
                }
            }
        } else {
            self.ripples.clear();
        }
        self.nodes = update.nodes;
        self.motion = update.motion;
    }

    pub fn set_activity(&mut self, activity: HashMap<String, Activity>) {
        self.activity = activity;
    }

    /// The flow on an execution wire from `source` to `target`.
    pub fn flow(&self, source: &str, target: &str) -> Option<Flow> {
        let source = self.nodes.get(source)?.run?;
        match self.nodes.get(target)?.run? {
            RunState::Running => Some(Flow::Active),
            RunState::Done => (source == RunState::Done).then_some(Flow::Trail),
        }
    }

    /// The completion ripple's progress from 0 to 1 while it plays.
    pub fn ripple(&self, id: &str, now: f64) -> Option<f32> {
        let progress = (now - *self.ripples.get(id)?) / RIPPLE_MS;
        (0.0..1.0).contains(&progress).then_some(progress as f32)
    }

    pub fn prune(&mut self, now: f64) {
        self.ripples.retain(|_, started| now - *started < RIPPLE_MS);
    }
}

/// A closed rounded-rectangle outline, clockwise on screen, with each vertex's outward normal.
pub fn rounded_outline(
    min: Point,
    max: Point,
    radius: f32,
    segments: usize,
) -> Vec<(Point, Point)> {
    let radius = radius
        .max(0.0)
        .min((max[0] - min[0]) * 0.5)
        .min((max[1] - min[1]) * 0.5);
    let corners = [
        ([max[0] - radius, min[1] + radius], -0.5),
        ([max[0] - radius, max[1] - radius], 0.0),
        ([min[0] + radius, max[1] - radius], 0.5),
        ([min[0] + radius, min[1] + radius], 1.0),
    ];
    let segments = segments.max(1);
    let mut outline = Vec::with_capacity(corners.len() * (segments + 1));
    for (center, start) in corners {
        for step in 0..=segments {
            let angle = std::f32::consts::PI * (start + 0.5 * step as f32 / segments as f32);
            let normal = [angle.cos(), angle.sin()];
            outline.push((
                [
                    center[0] + radius * normal[0],
                    center[1] + radius * normal[1],
                ],
                normal,
            ));
        }
    }
    outline
}

/// Arc-length positions along a polyline, closed around an outline or open along a wire.
pub struct Perimeter {
    points: Vec<Point>,
    lengths: Vec<f32>,
    closed: bool,
}

impl Perimeter {
    pub fn new(points: Vec<Point>) -> Self {
        Self::with_ends(points, true)
    }

    pub fn open(points: Vec<Point>) -> Self {
        Self::with_ends(points, false)
    }

    fn with_ends(points: Vec<Point>, closed: bool) -> Self {
        let segments = if closed {
            points.len()
        } else {
            points.len().saturating_sub(1)
        };
        let mut lengths = Vec::with_capacity(segments + 1);
        let mut total = 0.0;
        lengths.push(0.0);
        for index in 0..segments {
            let [point, next] = [points[index], points[(index + 1) % points.len()]];
            total += (next[0] - point[0]).hypot(next[1] - point[1]);
            lengths.push(total);
        }
        Self {
            points,
            lengths,
            closed,
        }
    }

    pub fn length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The point `distance` along the path. A closed path wraps; an open one stops at its ends.
    pub fn at(&self, distance: f32) -> Point {
        let total = self.length();
        if self.points.is_empty() || total <= 0.0 {
            return self.points.first().copied().unwrap_or([0.0, 0.0]);
        }
        let distance = if self.closed {
            distance.rem_euclid(total)
        } else {
            distance.clamp(0.0, total)
        };
        let segment = self
            .lengths
            .partition_point(|length| *length <= distance)
            .saturating_sub(1)
            .min(self.lengths.len() - 2);
        let start = self.points[segment];
        let end = self.points[(segment + 1) % self.points.len()];
        let span = self.lengths[segment + 1] - self.lengths[segment];
        let t = if span > 0.0 {
            (distance - self.lengths[segment]) / span
        } else {
            0.0
        };
        [
            start[0] + (end[0] - start[0]) * t,
            start[1] + (end[1] - start[1]) * t,
        ]
    }
}

/// Position in a repeating cycle, from 0 to 1.
pub fn cycle(now: f64, period_ms: f64) -> f32 {
    (now.rem_euclid(period_ms) / period_ms) as f32
}

/// A smooth 0 → 1 → 0 swell over each period.
pub fn breath(now: f64, period_ms: f64) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * cycle(now, period_ms)).cos()
}

pub fn ease_out(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(run: RunState) -> NodeStatus {
        NodeStatus {
            run: Some(run),
            ..NodeStatus::default()
        }
    }

    fn update(nodes: &[(&str, NodeStatus)], motion: bool) -> StatusUpdate {
        StatusUpdate {
            nodes: nodes
                .iter()
                .map(|(id, status)| (id.to_string(), status.clone()))
                .collect(),
            motion,
        }
    }

    #[test]
    fn status_json_is_sparse_and_camel_case() {
        let update: StatusUpdate = serde_json::from_str(
            r#"{"nodes":{"a":{"run":"running","runId":"r1","logs":{"count":"12","problems":true},
            "severity":"fatal","quality":{"severity":"warning"},"heat":{"intensity":0.5,"visits":"3×"},
            "remote":true},"b":{}}}"#,
        )
        .unwrap();
        assert!(update.motion);
        let a = &update.nodes["a"];
        assert_eq!(a.run, Some(RunState::Running));
        assert_eq!(a.run_id.as_deref(), Some("r1"));
        assert_eq!(a.logs.as_ref().unwrap().count.as_deref(), Some("12"));
        assert!(a.logs.as_ref().unwrap().problems && !a.logs.as_ref().unwrap().warnings);
        assert_eq!(a.severity, Some(Severity::Fatal));
        assert_eq!(a.quality.as_ref().unwrap().severity, Severity::Warning);
        assert!(a.remote && a.moving());
        assert_eq!(update.nodes["b"], NodeStatus::default());
        assert!(Severity::Fatal > Severity::Error && Severity::Warning > Severity::Info);
        let activity: HashMap<String, Activity> =
            serde_json::from_str(r#"{"r1":{"text":"5s ago","tone":"stale"},"r2":{"text":"--"}}"#)
                .unwrap();
        assert_eq!(activity["r1"].tone, Tone::Stale);
        assert_eq!(activity["r2"].tone, Tone::Inactive);
    }

    #[test]
    fn only_nodes_seen_running_ripple_when_they_finish() {
        let mut statuses = Statuses::default();
        statuses.update(update(&[("a", running(RunState::Running))], true), 0.0);
        statuses.update(
            update(
                &[
                    ("a", running(RunState::Done)),
                    ("fast", running(RunState::Done)),
                ],
                true,
            ),
            100.0,
        );
        assert_eq!(statuses.ripple("a", 100.0), Some(0.0));
        assert!(statuses.ripple("a", 450.0).unwrap() > 0.4);
        assert_eq!(statuses.ripple("fast", 100.0), None);
        assert_eq!(statuses.ripple("a", 100.0 + RIPPLE_MS), None);
        statuses.prune(100.0 + RIPPLE_MS);
        assert!(statuses.ripples.is_empty());

        statuses.update(update(&[("b", running(RunState::Running))], false), 200.0);
        statuses.update(update(&[("b", running(RunState::Done))], false), 300.0);
        assert_eq!(statuses.ripple("b", 300.0), None);
        assert!(!statuses.motion());
    }

    #[test]
    fn outline_is_closed_clockwise_with_outward_normals() {
        let outline = rounded_outline([10.0, 20.0], [160.0, 70.0], 4.0, 4);
        assert_eq!(outline.len(), 20);
        let center = [85.0, 45.0];
        let mut area = 0.0;
        for (index, (point, normal)) in outline.iter().enumerate() {
            assert!((normal[0].hypot(normal[1]) - 1.0).abs() < 1e-5);
            let outward = [point[0] - center[0], point[1] - center[1]];
            assert!(outward[0] * normal[0] + outward[1] * normal[1] > 0.0);
            assert!((10.0 - 1e-4..=160.0 + 1e-4).contains(&point[0]));
            assert!((20.0 - 1e-4..=70.0 + 1e-4).contains(&point[1]));
            let next = outline[(index + 1) % outline.len()].0;
            area += point[0] * next[1] - next[0] * point[1];
        }
        // Positive signed area with y pointing down runs clockwise on screen.
        assert!(area > 0.0);
        let square = rounded_outline([0.0, 0.0], [10.0, 10.0], 0.0, 2);
        assert!(
            square
                .iter()
                .all(|(point, _)| point[0] == 0.0 || point[0] == 10.0)
        );
    }

    #[test]
    fn perimeter_positions_wrap_and_follow_arc_length() {
        let perimeter = Perimeter::new(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        assert_eq!(perimeter.length(), 40.0);
        assert_eq!(perimeter.at(5.0), [5.0, 0.0]);
        assert_eq!(perimeter.at(15.0), [10.0, 5.0]);
        assert_eq!(perimeter.at(45.0), [5.0, 0.0]);
        assert_eq!(perimeter.at(-5.0), [0.0, 5.0]);
        assert_eq!(perimeter.at(40.0), [0.0, 0.0]);
        assert_eq!(Perimeter::new(Vec::new()).at(3.0), [0.0, 0.0]);

        let wire = Perimeter::open(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        assert_eq!(wire.length(), 20.0);
        assert_eq!(wire.at(15.0), [10.0, 5.0]);
        assert_eq!(wire.at(25.0), [10.0, 10.0]);
        assert_eq!(wire.at(-5.0), [0.0, 0.0]);
        assert_eq!(Perimeter::open(vec![[3.0, 4.0]]).at(1.0), [3.0, 4.0]);
    }

    #[test]
    fn execution_flows_into_running_nodes_and_trails_between_finished_ones() {
        let mut statuses = Statuses::default();
        statuses.update(
            update(
                &[
                    ("start", running(RunState::Done)),
                    ("busy", running(RunState::Running)),
                    ("next", running(RunState::Done)),
                    ("idle", NodeStatus::default()),
                ],
                true,
            ),
            0.0,
        );
        assert_eq!(statuses.flow("start", "busy"), Some(Flow::Active));
        assert_eq!(statuses.flow("busy", "busy"), Some(Flow::Active));
        assert_eq!(statuses.flow("start", "next"), Some(Flow::Trail));
        assert_eq!(statuses.flow("busy", "next"), None);
        assert_eq!(statuses.flow("idle", "busy"), None);
        assert_eq!(statuses.flow("start", "missing"), None);
    }

    #[test]
    fn timing_curves_stay_in_range() {
        for step in 0..200 {
            let now = step as f64 * 37.0;
            assert!((0.0..1.0).contains(&cycle(now, 1600.0)));
            assert!((0.0..=1.0).contains(&breath(now, 1400.0)));
        }
        assert_eq!(breath(0.0, 1000.0), 0.0);
        assert!((breath(500.0, 1000.0) - 1.0).abs() < 1e-6);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
    }
}

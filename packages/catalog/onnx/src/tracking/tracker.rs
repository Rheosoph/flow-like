//! ByteTrack multi-object tracker with BoT-SORT appearance fusion, driven by frame timestamps
//! instead of frame counts.

use super::{
    assignment::linear_assignment,
    embedding,
    kalman::{self, KalmanState, Measurement},
    types::{AppearanceObservation, TrackState, TrackedObject},
};
use flow_like_catalog_core::BoundingBox;
use flow_like_types::{Result, anyhow};

const PROXIMITY_GATE: f32 = 0.5;
const SECOND_MATCH_GATE: f32 = 0.5;
const UNCONFIRMED_MATCH_GATE: f32 = 0.7;
const DUPLICATE_IOU: f64 = 0.85;
const EMBEDDING_MOMENTUM: f32 = 0.9;

#[derive(Debug, Clone, PartialEq)]
pub struct TrackerConfig {
    /// Detections at or above this score take part in the first association
    pub high_threshold: f32,
    /// Detections in `[low_threshold, high_threshold)` can only keep tracked tracks alive
    pub low_threshold: f32,
    /// Minimum score for an unmatched detection to start a new track
    pub new_track_threshold: f32,
    /// Cost gate of the first association (IoU distance fused with the detection score)
    pub match_threshold: f32,
    /// How long a lost track can be re-acquired after it was last seen
    pub max_lost_ms: i64,
    /// Minimum cosine similarity for appearance to count as a match
    pub min_appearance_similarity: f32,
    /// Never match tracks and detections of different classes
    pub class_aware: bool,
    /// Also output lost tracks with their predicted boxes
    pub include_lost: bool,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            high_threshold: 0.5,
            low_threshold: 0.1,
            new_track_threshold: 0.6,
            match_threshold: 0.8,
            max_lost_ms: 2000,
            min_appearance_similarity: 0.5,
            class_aware: true,
            include_lost: false,
        }
    }
}

impl TrackerConfig {
    pub fn validate(&self) -> Result<()> {
        for (pin, value) in [
            ("high_threshold", self.high_threshold),
            ("low_threshold", self.low_threshold),
            ("new_track_threshold", self.new_track_threshold),
            ("match_threshold", self.match_threshold),
            ("min_appearance_similarity", self.min_appearance_similarity),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(anyhow!("{pin} must be between 0 and 1, got {value}"));
            }
        }
        if self.low_threshold > self.high_threshold {
            return Err(anyhow!(
                "low_threshold ({}) must not exceed high_threshold ({})",
                self.low_threshold,
                self.high_threshold
            ));
        }
        if self.max_lost_ms <= 0 {
            return Err(anyhow!(
                "max_lost_ms must be greater than 0, got {}",
                self.max_lost_ms
            ));
        }
        Ok(())
    }
}

/// One camera frame's detections.
pub struct Frame<'a> {
    pub detections: &'a [AppearanceObservation],
    /// Capture time in Unix milliseconds; `None` for the current time
    pub timestamp_ms: Option<i64>,
    /// Empty keeps the camera carried by the detections
    pub camera_id: &'a str,
    /// Empty keeps the session carried by the detections
    pub session_id: &'a str,
}

#[derive(Debug)]
pub enum FrameOutcome {
    Tracked {
        tracks: Vec<TrackedObject>,
        /// How far the frame lay behind the last processed one when that exceeded
        /// `max_lost_ms`: the clock was reset, so all tracks were dropped and the frame was
        /// handled like a first frame
        clock_jump_ms: Option<i64>,
    },
    /// The frame is at most `max_lost_ms` older than the last processed one; the tracker was not
    /// changed
    Stale {
        timestamp_ms: i64,
        last_timestamp_ms: i64,
    },
}

/// Camera and session a frame or track is attributed to.
#[derive(Debug, Clone, Default, PartialEq)]
struct Source {
    camera_id: String,
    session_id: String,
}

/// Tracker state of one camera session.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracker {
    tracker_id: String,
    tracks: Vec<Track>,
    issued_ids: u64,
    last_timestamp_ms: Option<i64>,
    /// Last non-empty camera and session, for frames that carry neither
    source: Source,
}

#[derive(Debug, Clone, PartialEq)]
struct Track {
    id: u64,
    kalman: KalmanState,
    state: TrackState,
    confirmed: bool,
    embedding: Vec<f32>,
    score: f32,
    class_idx: i32,
    class_name: Option<String>,
    hits: u32,
    first_seen_ms: i64,
    last_seen_ms: i64,
    detection_index: Option<u32>,
    source: Source,
}

struct Detection {
    index: Option<u32>,
    measurement: Measurement,
    xyxy: [f64; 4],
    score: f32,
    class_idx: i32,
    class_name: Option<String>,
    embedding: Option<Vec<f32>>,
}

impl Tracker {
    /// `tracker_id` names this instance; track ids are only unique within it.
    pub fn new(tracker_id: String) -> Self {
        Self {
            tracker_id,
            tracks: Vec::new(),
            issued_ids: 0,
            last_timestamp_ms: None,
            source: Source::default(),
        }
    }

    pub fn tracker_id(&self) -> &str {
        &self.tracker_id
    }

    /// Processes one frame. A frame without timestamp is taken at `wall_clock_ms`, but never
    /// before the last processed frame, so overlapping runs cannot make each other stale.
    pub fn update(
        &mut self,
        frame: Frame<'_>,
        wall_clock_ms: i64,
        config: &TrackerConfig,
    ) -> FrameOutcome {
        let now = frame.timestamp_ms.unwrap_or_else(|| {
            self.last_timestamp_ms
                .map_or(wall_clock_ms, |last| wall_clock_ms.max(last))
        });
        let mut clock_jump_ms = None;
        if let Some(last) = self.last_timestamp_ms
            && now < last
        {
            let behind = last.saturating_sub(now);
            if behind <= config.max_lost_ms {
                return FrameOutcome::Stale {
                    timestamp_ms: now,
                    last_timestamp_ms: last,
                };
            }
            self.tracks.clear();
            self.last_timestamp_ms = None;
            clock_jump_ms = Some(behind);
        }
        let first_frame = self.last_timestamp_ms.is_none();
        let dt = kalman::dt_frames(now.saturating_sub(self.last_timestamp_ms.unwrap_or(now)));
        self.last_timestamp_ms = Some(now);
        let source = self.resolve_source(&frame);

        self.expire_lost(now, config.max_lost_ms);
        for track in &mut self.tracks {
            track.detection_index = None;
            if track.state == TrackState::Lost {
                track.kalman.freeze_size();
            }
            track.kalman.predict(dt);
        }

        let (high, low) = split_detections(frame.detections, config);
        let pool: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| self.tracks[i].confirmed)
            .collect();
        let unconfirmed: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| !self.tracks[i].confirmed)
            .collect();

        let first = linear_assignment(pool.len(), high.len(), config.match_threshold, |r, c| {
            fused_cost(&self.tracks[pool[r]], &high[c], config)
        });
        for &(r, c) in &first.matches {
            self.tracks[pool[r]].absorb(&high[c], now, &source);
        }

        let remaining_tracked: Vec<usize> = first
            .unmatched_rows
            .iter()
            .map(|&r| pool[r])
            .filter(|&i| self.tracks[i].state == TrackState::Tracked)
            .collect();
        let second = linear_assignment(
            remaining_tracked.len(),
            low.len(),
            SECOND_MATCH_GATE,
            |r, c| iou_cost(&self.tracks[remaining_tracked[r]], &low[c], config),
        );
        for &(r, c) in &second.matches {
            self.tracks[remaining_tracked[r]].absorb(&low[c], now, &source);
        }
        for &r in &second.unmatched_rows {
            self.tracks[remaining_tracked[r]].state = TrackState::Lost;
        }

        let remaining_high: Vec<&Detection> =
            first.unmatched_cols.iter().map(|&c| &high[c]).collect();
        let third = linear_assignment(
            unconfirmed.len(),
            remaining_high.len(),
            UNCONFIRMED_MATCH_GATE,
            |r, c| fused_cost(&self.tracks[unconfirmed[r]], remaining_high[c], config),
        );
        for &(r, c) in &third.matches {
            self.tracks[unconfirmed[r]].absorb(remaining_high[c], now, &source);
        }
        let mut removed = vec![false; self.tracks.len()];
        for &r in &third.unmatched_rows {
            removed[unconfirmed[r]] = true;
        }
        drop_marked(&mut self.tracks, &removed);

        for &c in &third.unmatched_cols {
            let detection = remaining_high[c];
            if detection.score >= config.new_track_threshold {
                self.start_track(detection, now, first_frame, &source);
            }
        }

        self.expire_lost(now, config.max_lost_ms);
        self.remove_duplicates(config.class_aware);

        let mut tracks: Vec<TrackedObject> = self
            .tracks
            .iter()
            .filter(|track| match track.state {
                TrackState::Tracked => track.confirmed,
                TrackState::Lost => config.include_lost,
            })
            .map(|track| track.snapshot(&self.tracker_id, now))
            .collect();
        tracks.sort_by_key(|track| track.track_id);
        FrameOutcome::Tracked {
            tracks,
            clock_jump_ms,
        }
    }

    /// Pin values win, then the first value carried by a detection, then the last known one.
    fn resolve_source(&mut self, frame: &Frame<'_>) -> Source {
        let carried = || frame.detections.iter().filter(|o| is_detection(o));
        let source = Source {
            camera_id: resolve_label(
                frame.camera_id,
                carried().map(|o| o.camera_id.as_str()),
                &self.source.camera_id,
            ),
            session_id: resolve_label(
                frame.session_id,
                carried().map(|o| o.session_id.as_str()),
                &self.source.session_id,
            ),
        };
        self.source = source.clone();
        source
    }

    fn start_track(&mut self, detection: &Detection, now: i64, confirmed: bool, source: &Source) {
        let Some(kalman) = KalmanState::initiate(detection.measurement) else {
            return;
        };
        self.issued_ids += 1;
        self.tracks.push(Track {
            id: self.issued_ids,
            kalman,
            state: TrackState::Tracked,
            confirmed,
            embedding: detection.embedding.clone().unwrap_or_default(),
            score: detection.score,
            class_idx: detection.class_idx,
            class_name: detection.class_name.clone(),
            hits: 1,
            first_seen_ms: now,
            last_seen_ms: now,
            detection_index: detection.index,
            source: source.clone(),
        });
    }

    fn expire_lost(&mut self, now: i64, max_lost_ms: i64) {
        self.tracks.retain(|track| {
            track.state != TrackState::Lost || now.saturating_sub(track.last_seen_ms) <= max_lost_ms
        });
    }

    /// ByteTrack `remove_duplicate_stracks`: of a tracked and a lost track covering the same
    /// object, keep the one with the longer history.
    fn remove_duplicates(&mut self, class_aware: bool) {
        let mut removed = vec![false; self.tracks.len()];
        for (p, tracked) in self.tracks.iter().enumerate() {
            if tracked.state != TrackState::Tracked {
                continue;
            }
            for (q, lost) in self.tracks.iter().enumerate() {
                if lost.state != TrackState::Lost
                    || (class_aware && tracked.class_idx != lost.class_idx)
                    || iou(&tracked.kalman.xyxy(), &lost.kalman.xyxy()) <= DUPLICATE_IOU
                {
                    continue;
                }
                if tracked.history_ms() > lost.history_ms() {
                    removed[q] = true;
                } else {
                    removed[p] = true;
                }
            }
        }
        drop_marked(&mut self.tracks, &removed);
    }
}

fn drop_marked(tracks: &mut Vec<Track>, removed: &[bool]) {
    let mut flags = removed.iter();
    tracks.retain(|_| !flags.next().copied().unwrap_or(false));
}

/// Lost elements are predicted boxes of lost tracks fed back in, not detections.
fn is_detection(observation: &AppearanceObservation) -> bool {
    observation.state != TrackState::Lost
}

fn resolve_label<'a>(
    pin: &'a str,
    carried: impl IntoIterator<Item = &'a str>,
    last: &'a str,
) -> String {
    if !pin.is_empty() {
        return pin.to_owned();
    }
    carried
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or(last)
        .to_owned()
}

impl Track {
    fn absorb(&mut self, detection: &Detection, now: i64, source: &Source) {
        if self.kalman.update(detection.measurement).is_err()
            && let Some(kalman) = KalmanState::initiate(detection.measurement)
        {
            self.kalman = kalman;
        }
        if let Some(next) = &detection.embedding {
            self.embedding = if self.embedding.is_empty() {
                next.clone()
            } else {
                embedding::blend(&self.embedding, next, EMBEDDING_MOMENTUM)
            };
        }
        self.state = TrackState::Tracked;
        self.confirmed = true;
        self.score = detection.score;
        self.class_idx = detection.class_idx;
        self.class_name = detection.class_name.clone();
        self.hits = self.hits.saturating_add(1);
        self.last_seen_ms = now;
        self.detection_index = detection.index;
        self.source.clone_from(source);
    }

    fn history_ms(&self) -> i64 {
        self.last_seen_ms.saturating_sub(self.first_seen_ms)
    }

    fn snapshot(&self, tracker_id: &str, timestamp_ms: i64) -> TrackedObject {
        let [x1, y1, x2, y2] = self.kalman.xyxy();
        let (velocity_x, velocity_y) = self.kalman.velocity_per_second();
        TrackedObject {
            bbox: BoundingBox {
                x1: finite_f32(x1),
                y1: finite_f32(y1),
                x2: finite_f32(x2),
                y2: finite_f32(y2),
                score: self.score,
                class_idx: self.class_idx,
                class_name: self.class_name.clone(),
            },
            embedding: self.embedding.clone(),
            camera_id: self.source.camera_id.clone(),
            session_id: self.source.session_id.clone(),
            tracker_id: tracker_id.to_owned(),
            track_id: Some(self.id),
            timestamp_ms,
            detection_index: match self.state {
                TrackState::Tracked => self.detection_index,
                TrackState::Lost => None,
            },
            state: self.state,
            hits: self.hits,
            first_seen_ms: self.first_seen_ms,
            last_seen_ms: self.last_seen_ms,
            velocity_x: finite_f32(velocity_x),
            velocity_y: finite_f32(velocity_y),
        }
    }
}

/// High (`score ≥ high`) and low (`low ≤ score < high`) detections; unusable boxes are dropped.
fn split_detections(
    observations: &[AppearanceObservation],
    config: &TrackerConfig,
) -> (Vec<Detection>, Vec<Detection>) {
    let mut high = Vec::new();
    let mut low = Vec::new();
    for (index, observation) in observations.iter().enumerate() {
        if !is_detection(observation) {
            continue;
        }
        let Some(detection) = Detection::parse(index, observation) else {
            continue;
        };
        if detection.score >= config.high_threshold {
            high.push(detection);
        } else if detection.score >= config.low_threshold {
            low.push(detection);
        }
    }
    (high, low)
}

impl Detection {
    fn parse(index: usize, observation: &AppearanceObservation) -> Option<Self> {
        let bbox = &observation.bbox;
        let xyxy = [bbox.x1, bbox.y1, bbox.x2, bbox.y2].map(f64::from);
        let [x1, y1, x2, y2] = xyxy;
        let measurement = [(x1 + x2) / 2.0, (y1 + y2) / 2.0, x2 - x1, y2 - y1];
        if !bbox.score.is_finite() || !kalman::is_valid_measurement(&measurement) {
            return None;
        }
        Some(Self {
            index: u32::try_from(index).ok(),
            measurement,
            xyxy,
            score: bbox.score,
            class_idx: bbox.class_idx,
            class_name: bbox.class_name.clone(),
            embedding: embedding::normalized(&observation.embedding),
        })
    }
}

fn class_blocked(track: &Track, detection: &Detection, config: &TrackerConfig) -> bool {
    config.class_aware && track.class_idx != detection.class_idx
}

/// BoT-SORT cost: IoU distance fused with the detection score, replaced by the appearance
/// distance when that is lower, the boxes are close and the appearance is similar enough.
fn fused_cost(track: &Track, detection: &Detection, config: &TrackerConfig) -> f32 {
    if class_blocked(track, detection, config) {
        return f32::INFINITY;
    }
    let overlap = iou(&track.kalman.xyxy(), &detection.xyxy) as f32;
    let iou_cost = 1.0 - overlap * detection.score.clamp(0.0, 1.0);
    let Some(appearance) = detection
        .embedding
        .as_deref()
        .filter(|next| next.len() == track.embedding.len())
    else {
        return iou_cost;
    };
    let mut appearance_cost = (1.0 - embedding::cosine(&track.embedding, appearance)) / 2.0;
    if appearance_cost > (1.0 - config.min_appearance_similarity) / 2.0
        || 1.0 - overlap > PROXIMITY_GATE
    {
        appearance_cost = 1.0;
    }
    iou_cost.min(appearance_cost)
}

fn iou_cost(track: &Track, detection: &Detection, config: &TrackerConfig) -> f32 {
    if class_blocked(track, detection, config) {
        return f32::INFINITY;
    }
    1.0 - iou(&track.kalman.xyxy(), &detection.xyxy) as f32
}

fn iou(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    let area = |r: &[f64; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let intersection = w * h;
    let union = area(a) + area(b) - intersection;
    if union > 0.0 && union.is_finite() {
        (intersection / union).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn finite_f32(value: f64) -> f32 {
    if value.is_finite() {
        (value as f32).clamp(f32::MIN, f32::MAX)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_ms(frame: i64) -> i64 {
        frame * 1000 / 30
    }

    fn detection(x: f32, y: f32, w: f32, h: f32, score: f32) -> AppearanceObservation {
        AppearanceObservation {
            bbox: BoundingBox {
                x1: x,
                y1: y,
                x2: x + w,
                y2: y + h,
                score,
                class_idx: 0,
                class_name: Some("person".into()),
            },
            ..Default::default()
        }
    }

    fn with_class(mut observation: AppearanceObservation, class_idx: i32) -> AppearanceObservation {
        observation.bbox.class_idx = class_idx;
        observation
    }

    fn with_embedding(
        mut observation: AppearanceObservation,
        embedding: &[f32],
    ) -> AppearanceObservation {
        observation.embedding = embedding.to_vec();
        observation
    }

    const TRACKER_ID: &str = "tracker-a";
    const WALL_CLOCK_MS: i64 = 1_000_000;

    fn new_tracker() -> Tracker {
        Tracker::new(TRACKER_ID.into())
    }

    fn frame<'a>(
        detections: &'a [AppearanceObservation],
        timestamp_ms: Option<i64>,
        camera_id: &'a str,
        session_id: &'a str,
    ) -> Frame<'a> {
        Frame {
            detections,
            timestamp_ms,
            camera_id,
            session_id,
        }
    }

    /// Tracks of a frame that was neither stale nor a clock reset.
    fn tracked(tracker: &Tracker, outcome: FrameOutcome) -> Vec<TrackedObject> {
        match outcome {
            FrameOutcome::Tracked {
                tracks,
                clock_jump_ms: None,
            } => {
                for track in &tracks {
                    let values = [
                        track.bbox.x1,
                        track.bbox.y1,
                        track.bbox.x2,
                        track.bbox.y2,
                        track.velocity_x,
                        track.velocity_y,
                    ];
                    assert!(values.iter().all(|v| v.is_finite()), "{track:?}");
                    assert_eq!(track.tracker_id, tracker.tracker_id);
                }
                tracks
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    fn step(
        tracker: &mut Tracker,
        detections: &[AppearanceObservation],
        timestamp_ms: i64,
        config: &TrackerConfig,
    ) -> Vec<TrackedObject> {
        let outcome = tracker.update(
            frame(detections, Some(timestamp_ms), "cam", "s1"),
            WALL_CLOCK_MS,
            config,
        );
        tracked(tracker, outcome)
    }

    fn step_unlabeled(
        tracker: &mut Tracker,
        detections: &[AppearanceObservation],
        timestamp_ms: i64,
        config: &TrackerConfig,
    ) -> Vec<TrackedObject> {
        let outcome = tracker.update(
            frame(detections, Some(timestamp_ms), "", ""),
            WALL_CLOCK_MS,
            config,
        );
        tracked(tracker, outcome)
    }

    fn from_source(
        mut observation: AppearanceObservation,
        camera_id: &str,
        session_id: &str,
    ) -> AppearanceObservation {
        observation.camera_id = camera_id.into();
        observation.session_id = session_id.into();
        observation
    }

    fn ids(tracks: &[TrackedObject]) -> Vec<u64> {
        tracks.iter().filter_map(|track| track.track_id).collect()
    }

    fn track_for(tracks: &[TrackedObject], detection_index: u32) -> u64 {
        tracks
            .iter()
            .find(|track| track.detection_index == Some(detection_index))
            .and_then(|track| track.track_id)
            .unwrap_or_else(|| panic!("no track for detection {detection_index}: {tracks:?}"))
    }

    #[test]
    fn default_config_is_valid() {
        assert!(TrackerConfig::default().validate().is_ok());
    }

    #[test]
    fn invalid_config_names_the_pin() {
        let cases = [
            (
                TrackerConfig {
                    high_threshold: 1.5,
                    ..Default::default()
                },
                "high_threshold",
            ),
            (
                TrackerConfig {
                    match_threshold: f32::NAN,
                    ..Default::default()
                },
                "match_threshold",
            ),
            (
                TrackerConfig {
                    min_appearance_similarity: -0.1,
                    ..Default::default()
                },
                "min_appearance_similarity",
            ),
            (
                TrackerConfig {
                    low_threshold: 0.7,
                    ..Default::default()
                },
                "low_threshold",
            ),
            (
                TrackerConfig {
                    max_lost_ms: 0,
                    ..Default::default()
                },
                "max_lost_ms",
            ),
        ];
        for (config, pin) in cases {
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains(pin), "{error}");
        }
    }

    #[test]
    fn constant_velocity_box_keeps_one_id() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let mut last = Vec::new();
        for frame in 0..30 {
            let x = 10.0 + 5.0 * frame as f32;
            last = step(
                &mut tracker,
                &[detection(x, 40.0, 50.0, 100.0, 0.9)],
                frame_ms(frame),
                &config,
            );
            assert_eq!(ids(&last), vec![1], "frame {frame}");
            assert_eq!(last[0].detection_index, Some(0));
        }
        let track = &last[0];
        assert_eq!(track.hits, 30);
        assert_eq!(track.state, TrackState::Tracked);
        assert_eq!(track.first_seen_ms, 0);
        assert_eq!(track.camera_id, "cam");
        assert_eq!(track.session_id, "s1");
        assert!((track.velocity_x - 150.0).abs() < 10.0, "{track:?}");
        assert!(track.velocity_y.abs() < 5.0);
        assert!((track.bbox.x1 - 155.0).abs() < 2.0);
    }

    #[test]
    fn sparse_frames_use_elapsed_time() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let mut last = Vec::new();
        for frame in 0..30 {
            let x = 15.0 * frame as f32;
            last = step(
                &mut tracker,
                &[detection(x, 0.0, 50.0, 100.0, 0.9)],
                100 * frame,
                &config,
            );
            assert_eq!(ids(&last), vec![1], "frame {frame}");
        }
        assert!((last[0].velocity_x - 150.0).abs() < 10.0, "{:?}", last[0]);
    }

    #[test]
    fn crossing_boxes_keep_ids_with_embeddings() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let red = [1.0, 0.0, 0.0];
        let blue = [0.0, 1.0, 0.0];
        let mut red_id = None;
        let mut blue_id = None;
        for frame in 0..31 {
            let left = 10.0 * frame as f32;
            let right = 300.0 - 10.0 * frame as f32;
            let a = with_embedding(detection(left, 50.0, 50.0, 100.0, 0.9), &red);
            let b = with_embedding(detection(right, 50.0, 50.0, 100.0, 0.9), &blue);
            let (detections, red_index, blue_index) = if frame % 2 == 0 {
                (vec![a, b], 0, 1)
            } else {
                (vec![b, a], 1, 0)
            };
            let tracks = step(&mut tracker, &detections, frame_ms(frame), &config);
            assert_eq!(tracks.len(), 2, "frame {frame}: {tracks:?}");
            let red_track = track_for(&tracks, red_index);
            let blue_track = track_for(&tracks, blue_index);
            assert_eq!(*red_id.get_or_insert(red_track), red_track, "frame {frame}");
            assert_eq!(
                *blue_id.get_or_insert(blue_track),
                blue_track,
                "frame {frame}"
            );
        }
        assert_ne!(red_id, blue_id);
    }

    #[test]
    fn track_embedding_is_smoothed() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let first = step(
            &mut tracker,
            &[with_embedding(
                detection(0.0, 0.0, 50.0, 100.0, 0.9),
                &[3.0, 4.0],
            )],
            0,
            &config,
        );
        assert_eq!(first[0].embedding, vec![0.6, 0.8]);
        let second = step(
            &mut tracker,
            &[with_embedding(
                detection(0.0, 0.0, 50.0, 100.0, 0.9),
                &[1.0, 0.0],
            )],
            33,
            &config,
        );
        assert_eq!(
            second[0].embedding,
            embedding::blend(&[0.6, 0.8], &[1.0, 0.0], EMBEDDING_MOMENTUM)
        );
        let third = step(
            &mut tracker,
            &[detection(0.0, 0.0, 50.0, 100.0, 0.9)],
            66,
            &config,
        );
        assert_eq!(third[0].embedding, second[0].embedding);
    }

    #[test]
    fn appearance_gate_rejects_dissimilar_neighbours() {
        let config = TrackerConfig {
            match_threshold: 0.3,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        step(
            &mut tracker,
            &[with_embedding(
                detection(0.0, 0.0, 50.0, 100.0, 0.9),
                &[1.0, 0.0],
            )],
            0,
            &config,
        );
        let shifted = detection(10.0, 0.0, 50.0, 100.0, 0.9);
        let mut similar = tracker.clone();
        let tracks = step(
            &mut similar,
            &[with_embedding(shifted.clone(), &[1.0, 0.0])],
            33,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        let tracks = step(
            &mut tracker,
            &[with_embedding(shifted, &[0.0, 1.0])],
            33,
            &config,
        );
        assert!(tracks.is_empty(), "{tracks:?}");
    }

    #[test]
    fn low_score_detection_keeps_track_alive() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        for frame in 0..3 {
            step(
                &mut tracker,
                &[detection(100.0, 100.0, 40.0, 80.0, 0.9)],
                frame_ms(frame),
                &config,
            );
        }
        let mut without = tracker.clone();
        let tracks = step(
            &mut tracker,
            &[
                detection(500.0, 500.0, 40.0, 80.0, 0.05),
                detection(101.0, 100.0, 40.0, 80.0, 0.3),
            ],
            frame_ms(3),
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].detection_index, Some(1));
        assert_eq!(tracks[0].bbox.score, 0.3);
        assert_eq!(tracks[0].hits, 4);

        assert!(step(&mut without, &[], frame_ms(3), &config).is_empty());
    }

    #[test]
    fn low_score_detections_never_start_tracks() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        for frame in 0..5 {
            let tracks = step(
                &mut tracker,
                &[
                    detection(0.0, 0.0, 40.0, 80.0, 0.3),
                    detection(200.0, 0.0, 40.0, 80.0, 0.55),
                ],
                frame_ms(frame),
                &config,
            );
            assert!(tracks.is_empty(), "frame {frame}: {tracks:?}");
        }
        assert!(tracker.tracks.is_empty());
    }

    #[test]
    fn lost_track_is_reacquired_within_max_lost() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let object = [detection(100.0, 100.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &object, 0, &config);
        step(&mut tracker, &object, 33, &config);

        let lost = step(&mut tracker, &[], 66, &config);
        assert_eq!(ids(&lost), vec![1]);
        assert_eq!(lost[0].state, TrackState::Lost);
        assert_eq!(lost[0].detection_index, None);
        assert_eq!(lost[0].last_seen_ms, 33);

        let mut sparse = tracker.clone();
        for frame in 3..40 {
            step(&mut tracker, &[], frame_ms(frame), &config);
        }
        let back = step(&mut tracker, &object, 1500, &config);
        assert_eq!(ids(&back), vec![1]);
        assert_eq!(back[0].state, TrackState::Tracked);

        let back = step(&mut sparse, &object, 2033, &config);
        assert_eq!(ids(&back), vec![1]);
    }

    #[test]
    fn lost_track_expires_after_max_lost() {
        let config = TrackerConfig::default();
        let object = [detection(100.0, 100.0, 40.0, 80.0, 0.9)];
        let mut dense = new_tracker();
        step(&mut dense, &object, 0, &config);
        step(&mut dense, &object, 33, &config);
        step(&mut dense, &[], 66, &config);
        let mut sparse = dense.clone();

        for frame in 3..63 {
            step(&mut dense, &[], frame_ms(frame), &config);
        }
        assert!(dense.tracks.is_empty());

        for tracker in [&mut dense, &mut sparse] {
            assert!(step(tracker, &object, 2100, &config).is_empty());
            let tracks = step(tracker, &object, 2133, &config);
            assert_eq!(ids(&tracks), vec![2]);
        }
    }

    #[test]
    fn tracked_track_survives_a_gap_without_frames() {
        let config = TrackerConfig::default();
        let object = [detection(100.0, 100.0, 40.0, 80.0, 0.9)];
        let mut tracker = new_tracker();
        step(&mut tracker, &object, 0, &config);
        step(&mut tracker, &object, 33, &config);
        let mut unmatched = tracker.clone();

        let tracks = step(&mut tracker, &object, 10_000, &config);
        assert_eq!(ids(&tracks), vec![1]);

        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        assert!(step(&mut unmatched, &[], 10_000, &config).is_empty());
        assert!(unmatched.tracks.is_empty());
    }

    #[test]
    fn stale_frame_is_skipped_without_mutation() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        step(
            &mut tracker,
            &[detection(0.0, 0.0, 40.0, 80.0, 0.9)],
            1000,
            &config,
        );
        step(
            &mut tracker,
            &[detection(2.0, 0.0, 40.0, 80.0, 0.9)],
            1100,
            &config,
        );
        let before = tracker.clone();
        let late = [from_source(
            detection(300.0, 0.0, 40.0, 80.0, 0.9),
            "other",
            "s9",
        )];
        let outcome = tracker.update(frame(&late, Some(1050), "", ""), WALL_CLOCK_MS, &config);
        assert!(matches!(
            outcome,
            FrameOutcome::Stale {
                timestamp_ms: 1050,
                last_timestamp_ms: 1100
            }
        ));
        assert_eq!(tracker, before);
    }

    #[test]
    fn frame_up_to_max_lost_behind_is_stale() {
        let config = TrackerConfig {
            max_lost_ms: 500,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let object = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &object, 10_000, &config);
        let before = tracker.clone();
        let outcome = tracker.update(frame(&object, Some(9_500), "cam", "s1"), 0, &config);
        assert!(
            matches!(
                outcome,
                FrameOutcome::Stale {
                    timestamp_ms: 9_500,
                    last_timestamp_ms: 10_000
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(tracker, before);
    }

    #[test]
    fn clock_jump_beyond_max_lost_resets_tracks() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let object = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        let elsewhere = [detection(300.0, 0.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &object, 10_000, &config);
        step(&mut tracker, &elsewhere, 10_033, &config);
        assert_eq!(tracker.tracks.len(), 2);

        let outcome = tracker.update(frame(&object, Some(5_000), "cam", "s1"), 0, &config);
        let FrameOutcome::Tracked {
            tracks,
            clock_jump_ms: Some(5_033),
        } = outcome
        else {
            panic!("expected a clock reset, got {outcome:?}");
        };
        assert_eq!(ids(&tracks), vec![3], "new tracks are confirmed at once");
        assert_eq!(tracks[0].tracker_id, TRACKER_ID);
        assert_eq!(tracks[0].timestamp_ms, 5_000);
        assert_eq!(tracks[0].first_seen_ms, 5_000);
        assert_eq!(tracks[0].hits, 1);
        assert_eq!(tracker.tracks.len(), 1);
        assert_eq!(tracker.tracker_id(), TRACKER_ID);

        let tracks = step(&mut tracker, &object, 5_033, &config);
        assert_eq!(ids(&tracks), vec![3]);
        assert_eq!(tracks[0].hits, 2);
    }

    #[test]
    fn missing_timestamp_uses_wall_clock_but_never_goes_back() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let object = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        let outcome = tracker.update(frame(&object, None, "cam", "s1"), 5_000, &config);
        assert_eq!(tracked(&tracker, outcome)[0].timestamp_ms, 5_000);

        step(&mut tracker, &object, 6_000, &config);
        let outcome = tracker.update(frame(&object, None, "cam", "s1"), 5_500, &config);
        let tracks = tracked(&tracker, outcome);
        assert_eq!(tracks[0].timestamp_ms, 6_000);
        assert_eq!(tracks[0].hits, 3);

        let outcome = tracker.update(frame(&object, None, "cam", "s1"), 6_100, &config);
        assert_eq!(tracked(&tracker, outcome)[0].timestamp_ms, 6_100);
        assert_eq!(tracker.last_timestamp_ms, Some(6_100));
    }

    #[test]
    fn tracks_carry_the_id_of_their_tracker() {
        let config = TrackerConfig::default();
        let object = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        let mut first = new_tracker();
        let mut second = Tracker::new("tracker-b".into());
        let a = step(&mut first, &object, 0, &config);
        let b = step(&mut second, &object, 0, &config);
        assert_eq!(a[0].track_id, b[0].track_id);
        assert_eq!(a[0].tracker_id, TRACKER_ID);
        assert_eq!(b[0].tracker_id, "tracker-b");
    }

    #[test]
    fn pins_override_the_source_carried_by_detections() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let carried = [from_source(
            detection(0.0, 0.0, 40.0, 80.0, 0.9),
            "door",
            "s9",
        )];
        let tracks = step(&mut tracker, &carried, 0, &config);
        assert_eq!(tracks[0].camera_id, "cam");
        assert_eq!(tracks[0].session_id, "s1");

        let outcome = tracker.update(frame(&carried, Some(33), "", "s2"), 0, &config);
        let tracks = tracked(&tracker, outcome);
        assert_eq!(tracks[0].camera_id, "door");
        assert_eq!(tracks[0].session_id, "s2");
    }

    #[test]
    fn empty_pins_keep_the_source_carried_by_detections() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let plain = detection(0.0, 0.0, 40.0, 80.0, 0.9);
        assert_eq!(
            step_unlabeled(&mut tracker, std::slice::from_ref(&plain), 0, &config)[0].camera_id,
            ""
        );

        let frame_detections = [
            from_source(detection(200.0, 0.0, 40.0, 80.0, 0.9), "", ""),
            from_source(plain.clone(), "door", ""),
            from_source(detection(400.0, 0.0, 40.0, 80.0, 0.9), "yard", "s9"),
        ];
        let tracks = step_unlabeled(&mut tracker, &frame_detections, 33, &config);
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].camera_id, "door");
        assert_eq!(tracks[0].session_id, "s9");

        let tracks = step_unlabeled(&mut tracker, &[plain], 66, &config);
        assert_eq!(tracks[0].camera_id, "door");
        assert_eq!(tracks[0].session_id, "s9");
    }

    #[test]
    fn lost_tracks_keep_the_source_they_were_last_stamped_with() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let door = [from_source(
            detection(0.0, 0.0, 40.0, 80.0, 0.9),
            "door",
            "s1",
        )];
        let yard = [from_source(
            detection(400.0, 0.0, 40.0, 80.0, 0.9),
            "yard",
            "s2",
        )];
        step_unlabeled(&mut tracker, &door, 0, &config);
        step_unlabeled(&mut tracker, &yard, 33, &config);
        let tracks = step_unlabeled(&mut tracker, &yard, 66, &config);
        assert_eq!(ids(&tracks), vec![1, 2]);
        assert_eq!(tracks[0].state, TrackState::Lost);
        assert_eq!(
            (tracks[0].camera_id.as_str(), tracks[0].session_id.as_str()),
            ("door", "s1")
        );
        assert_eq!(tracks[1].state, TrackState::Tracked);
        assert_eq!(
            (tracks[1].camera_id.as_str(), tracks[1].session_id.as_str()),
            ("yard", "s2")
        );
    }

    #[test]
    fn lost_inputs_are_not_detections() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let mut fed_back = from_source(detection(0.0, 0.0, 40.0, 80.0, 0.9), "ghost", "g");
        fed_back.state = TrackState::Lost;
        let tracks = step_unlabeled(
            &mut tracker,
            &[fed_back.clone(), detection(200.0, 0.0, 40.0, 80.0, 0.9)],
            0,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].detection_index, Some(1));
        assert_eq!(tracks[0].camera_id, "");

        let mut lost = fed_back.clone();
        lost.bbox = detection(200.0, 0.0, 40.0, 80.0, 0.9).bbox;
        let tracks = step_unlabeled(&mut tracker, &[fed_back, lost], 33, &config);
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].state, TrackState::Lost);
        assert_eq!(tracker.tracks.len(), 1);
    }

    #[test]
    fn equal_timestamps_are_processed_without_motion() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let object = [detection(10.0, 20.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &object, 500, &config);
        let tracks = step(&mut tracker, &object, 500, &config);
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].hits, 2);
        assert_eq!(tracks[0].velocity_x, 0.0);
        assert!((tracks[0].bbox.x1 - 10.0).abs() < 1e-3);
        assert!((tracks[0].bbox.y2 - 100.0).abs() < 1e-3);
    }

    #[test]
    fn class_aware_prevents_cross_class_matches() {
        let object = [detection(100.0, 100.0, 40.0, 80.0, 0.9)];
        let other_class = [with_class(object[0].clone(), 2)];

        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        step(&mut tracker, &object, 0, &config);
        assert!(step(&mut tracker, &other_class, 33, &config).is_empty());
        let tracks = step(&mut tracker, &other_class, 66, &config);
        assert_eq!(ids(&tracks), vec![2]);
        assert_eq!(tracks[0].bbox.class_idx, 2);

        let config = TrackerConfig {
            class_aware: false,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        step(&mut tracker, &object, 0, &config);
        let tracks = step(&mut tracker, &other_class, 33, &config);
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].bbox.class_idx, 2);
    }

    #[test]
    fn single_frame_false_positive_is_never_confirmed() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let anchor = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &anchor, 0, &config);
        let tracks = step(
            &mut tracker,
            &[anchor[0].clone(), detection(400.0, 400.0, 40.0, 80.0, 0.95)],
            33,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        let tracks = step(&mut tracker, &anchor, 66, &config);
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracker.tracks.len(), 1);
    }

    #[test]
    fn first_frame_tracks_are_confirmed_immediately() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        assert!(step(&mut tracker, &[], 0, &config).is_empty());
        let tracks = step(
            &mut tracker,
            &[detection(0.0, 0.0, 40.0, 80.0, 0.9)],
            33,
            &config,
        );
        assert!(tracks.is_empty());

        let mut tracker = new_tracker();
        let tracks = step(
            &mut tracker,
            &[
                detection(0.0, 0.0, 40.0, 80.0, 0.9),
                detection(200.0, 0.0, 40.0, 80.0, 0.7),
            ],
            0,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1, 2]);
    }

    #[test]
    fn new_track_threshold_filters_new_tracks() {
        let config = TrackerConfig {
            new_track_threshold: 0.8,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let tracks = step(
            &mut tracker,
            &[
                detection(0.0, 0.0, 40.0, 80.0, 0.7),
                detection(200.0, 0.0, 40.0, 80.0, 0.85),
            ],
            0,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].detection_index, Some(1));
    }

    #[test]
    fn ids_are_never_reused() {
        let config = TrackerConfig {
            max_lost_ms: 100,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let mut seen = Vec::new();
        let mut timestamp = 0;
        for round in 0..5 {
            let x = 100.0 * round as f32;
            for _ in 0..3 {
                let tracks = step(
                    &mut tracker,
                    &[detection(x, 0.0, 40.0, 80.0, 0.9)],
                    timestamp,
                    &config,
                );
                seen.extend(ids(&tracks));
                timestamp += 33;
            }
            timestamp += 1000;
            step(&mut tracker, &[], timestamp, &config);
            assert!(tracker.tracks.is_empty(), "round {round}");
        }
        seen.dedup();
        assert_eq!(seen.len(), 5);
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{seen:?}");
    }

    #[test]
    fn unusable_detections_are_ignored() {
        let config = TrackerConfig::default();
        let mut tracker = new_tracker();
        let tracks = step(
            &mut tracker,
            &[
                detection(0.0, 0.0, 0.0, 80.0, 0.9),
                detection(10.0, 10.0, -5.0, 20.0, 0.9),
                detection(f32::NAN, 0.0, 40.0, 80.0, 0.9),
                detection(0.0, 0.0, f32::INFINITY, 80.0, 0.9),
                detection(0.0, 0.0, 40.0, 80.0, f32::NAN),
                detection(50.0, 50.0, 40.0, 80.0, 0.9),
            ],
            0,
            &config,
        );
        assert_eq!(ids(&tracks), vec![1]);
        assert_eq!(tracks[0].detection_index, Some(5));
    }

    #[test]
    fn empty_frames_are_harmless() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        for frame in 0..5 {
            assert!(step(&mut tracker, &[], frame_ms(frame), &config).is_empty());
        }
        assert!(tracker.tracks.is_empty());
        assert_eq!(tracker.issued_ids, 0);
    }

    #[test]
    fn duplicate_tracked_and_lost_tracks_keep_the_older_one() {
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let mut tracker = new_tracker();
        let object = [detection(0.0, 0.0, 40.0, 80.0, 0.9)];
        step(&mut tracker, &object, 0, &config);
        step(&mut tracker, &object, 33, &config);
        tracker.tracks[0].state = TrackState::Lost;
        let mut duplicate = tracker.tracks[0].clone();
        duplicate.id = 9;
        duplicate.state = TrackState::Tracked;
        duplicate.first_seen_ms = 30;

        let mut older_tracked = tracker.clone();
        older_tracked.tracks[0].first_seen_ms = 32;
        older_tracked.tracks.push(duplicate.clone());
        older_tracked.remove_duplicates(true);
        assert_eq!(older_tracked.tracks.len(), 1);
        assert_eq!(older_tracked.tracks[0].id, 9);

        tracker.tracks.push(duplicate);
        tracker.remove_duplicates(true);
        assert_eq!(tracker.tracks.len(), 1);
        assert_eq!(tracker.tracks[0].id, 1);
        assert_eq!(tracker.tracks[0].state, TrackState::Lost);

        tracker.tracks[0].class_idx = 1;
        let mut other_class = tracker.tracks[0].clone();
        other_class.id = 10;
        other_class.class_idx = 0;
        other_class.state = TrackState::Tracked;
        tracker.tracks.push(other_class);
        tracker.remove_duplicates(true);
        assert_eq!(tracker.tracks.len(), 2);
    }

    #[test]
    fn iou_handles_degenerate_boxes() {
        assert_eq!(iou(&[0.0, 0.0, 10.0, 10.0], &[0.0, 0.0, 10.0, 10.0]), 1.0);
        assert_eq!(iou(&[0.0, 0.0, 10.0, 10.0], &[20.0, 20.0, 30.0, 30.0]), 0.0);
        assert_eq!(iou(&[0.0, 0.0, 0.0, 0.0], &[0.0, 0.0, 0.0, 0.0]), 0.0);
        assert!((iou(&[0.0, 0.0, 10.0, 10.0], &[5.0, 0.0, 15.0, 10.0]) - 1.0 / 3.0).abs() < 1e-12);
    }
}

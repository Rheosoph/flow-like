use super::{
    registry::{Eviction, RegistryLimits, StateRegistry, now_ms, user_key},
    tracker::{Frame, FrameOutcome, Tracker, TrackerConfig},
    types::{AppearanceObservation, TrackedObject},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::BoundingBox;
use flow_like_types::{Result, anyhow, async_trait, json::json};
use std::{sync::LazyLock, time::Duration};

static TRACKERS: LazyLock<StateRegistry<Tracker>> = LazyLock::new(|| {
    StateRegistry::new(
        "Tracker",
        RegistryLimits {
            idle_ttl: Duration::from_secs(10 * 60),
            capacity: 1024,
            app_capacity: 256,
            eviction: Eviction::LeastRecentlyUsed,
        },
    )
});

#[crate::register_node]
#[derive(Default)]
pub struct TrackDetectionsNode {}

impl TrackDetectionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TrackDetectionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tracking_track_detections",
            "Track Detections",
            "Follows detected objects across the frames of one camera and gives each a stable track id (ByteTrack, with BoT-SORT appearance matching when detections carry embeddings). Run it once per frame: the tracker is kept in memory per user, board, node, Camera pin and Session pin between runs and uses frame timestamps for motion and expiry. Track ids are only unique per tracker instance, named by each track's tracker_id: a new Camera or Session pin value, 10 minutes without frames, eviction when too many trackers are open, or a process restart starts a new tracker with a new tracker_id (a session carried by the detections does not). Deployments that spread runs over several processes keep one independent tracker per process.",
            "AI/ML/Tracking",
        );
        node.set_flowscript_name("tracking", "trackDetections");
        node.set_receiver("detections");
        node.add_icon("/flow/icons/cctv.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "detections",
            "Detections",
            "Boxes detected in this frame. Embeddings from Extract Appearance are used for appearance matching when present. Elements in the lost state (predicted boxes of lost tracks fed back in) are ignored.",
            VariableType::Struct,
        )
        .set_schema::<BoundingBox>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "camera_id",
            "Camera",
            "Camera the frame comes from. Empty keeps the camera carried by the detections, or the last one seen. Every value of this pin gets its own tracker, so set it when one node tracks several cameras.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "session_id",
            "Session",
            "Stream session of the camera. Empty keeps the session carried by the detections, or the last one seen. A new value of this pin starts a fresh tracker with a new tracker_id.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "timestamp_ms",
            "Timestamp (ms)",
            "Frame capture time in Unix milliseconds; 0 uses the current time, never earlier than the last processed frame. A frame up to Max Lost older than the last processed one is skipped; one further back is taken as a clock reset that drops all tracks.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        add_fraction_pin(
            &mut node,
            "high_threshold",
            "High Threshold",
            "Detections scoring at least this take part in the first association",
            0.5,
        );
        add_fraction_pin(
            &mut node,
            "low_threshold",
            "Low Threshold",
            "Detections scoring at least this, but below the high threshold, can only keep existing tracks alive",
            0.1,
        );
        add_fraction_pin(
            &mut node,
            "new_track_threshold",
            "New Track Threshold",
            "Minimum score for an unmatched detection to start a new track",
            0.6,
        );
        add_fraction_pin(
            &mut node,
            "match_threshold",
            "Match Threshold",
            "Maximum matching cost (1 - IoU × score, or appearance distance) of the first association; higher matches more loosely",
            0.8,
        );

        node.add_input_pin(
            "max_lost_ms",
            "Max Lost (ms)",
            "How long a track that lost its object can be re-acquired with the same id",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2000)));

        add_fraction_pin(
            &mut node,
            "min_appearance_similarity",
            "Min Appearance Similarity",
            "Minimum cosine similarity of embeddings for appearance to count as a match",
            0.5,
        );

        node.add_input_pin(
            "class_aware",
            "Class Aware",
            "Never match a track with a detection of another class",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_lost",
            "Include Lost",
            "Also output tracks that lost their object. Their boxes are Kalman predictions, not detections: Extract Appearance and Associate Entities treat them as passive, and this node ignores them when they are fed back in.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "tracks",
            "Tracks",
            "Confirmed tracks matched in this frame (plus lost tracks if enabled), sorted by track id. Track ids are unique per tracker_id.",
            VariableType::Struct,
        )
        .set_schema::<TrackedObject>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "skipped",
            "Skipped",
            "True when the frame was older than the last processed one and was ignored",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let detections: Vec<AppearanceObservation> = context.evaluate_pin("detections").await?;
        let camera_id: String = context.evaluate_pin("camera_id").await?;
        let session_id: String = context.evaluate_pin("session_id").await?;
        let timestamp_ms = frame_timestamp(context.evaluate_pin("timestamp_ms").await?)?;
        let config = TrackerConfig {
            high_threshold: context.evaluate_pin("high_threshold").await?,
            low_threshold: context.evaluate_pin("low_threshold").await?,
            new_track_threshold: context.evaluate_pin("new_track_threshold").await?,
            match_threshold: context.evaluate_pin("match_threshold").await?,
            max_lost_ms: context.evaluate_pin("max_lost_ms").await?,
            min_appearance_similarity: context.evaluate_pin("min_appearance_similarity").await?,
            class_aware: context.evaluate_pin("class_aware").await?,
            include_lost: context.evaluate_pin("include_lost").await?,
        };
        config.validate()?;

        let board_id = context
            .execution_cache
            .as_ref()
            .map(|cache| cache.board_id.clone())
            .unwrap_or_default();
        let key = user_key(
            context,
            &[&board_id, context.node.node_id(), &camera_id, &session_id],
        );
        let frame = Frame {
            detections: &detections,
            timestamp_ms,
            camera_id: &camera_id,
            session_id: &session_id,
        };
        let outcome = TRACKERS.with_state(&key, new_tracker, |tracker| {
            tracker.update(frame, now_ms(), &config)
        })?;

        let (tracks, skipped, warning) = split_outcome(outcome, &camera_id);
        if let Some(warning) = warning {
            context.log_message(&warning, LogLevel::Warn);
        }

        context.set_pin_value("tracks", json!(tracks)).await?;
        context.set_pin_value("skipped", json!(skipped)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Every tracker instance gets a fresh id, so track ids of a replaced tracker never collide.
fn new_tracker() -> Tracker {
    Tracker::new(flow_like_types::create_id())
}

/// Output tracks, the `skipped` flag and a warning to log.
fn split_outcome(
    outcome: FrameOutcome,
    camera_id: &str,
) -> (Vec<TrackedObject>, bool, Option<String>) {
    let of_camera = if camera_id.is_empty() {
        String::new()
    } else {
        format!(" of camera '{camera_id}'")
    };
    match outcome {
        FrameOutcome::Tracked {
            tracks,
            clock_jump_ms,
        } => {
            let warning = clock_jump_ms.map(|jump_ms| {
                format!("Clock{of_camera} jumped back by {jump_ms} ms; tracks reset")
            });
            (tracks, false, warning)
        }
        FrameOutcome::Stale {
            timestamp_ms,
            last_timestamp_ms,
        } => (
            Vec::new(),
            true,
            Some(format!(
                "Skipped a stale frame{of_camera}: timestamp {timestamp_ms} ms is older than the last processed frame at {last_timestamp_ms} ms"
            )),
        ),
    }
}

fn add_fraction_pin(
    node: &mut Node,
    name: &str,
    friendly_name: &str,
    description: &str,
    default: f64,
) {
    node.add_input_pin(name, friendly_name, description, VariableType::Float)
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(default)));
}

/// Parses the `timestamp_ms` pin: 0 means now, which the tracker resolves under its lock;
/// negative values are rejected.
fn frame_timestamp(timestamp_ms: i64) -> Result<Option<i64>> {
    match timestamp_ms {
        0 => Ok(None),
        value if value < 0 => Err(anyhow!(
            "timestamp_ms must be a Unix time in milliseconds or 0 for now, got {value}"
        )),
        value => Ok(Some(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn timestamp_zero_means_now() {
        assert_eq!(frame_timestamp(0).unwrap(), None);
        assert_eq!(
            frame_timestamp(1_700_000_000_000).unwrap(),
            Some(1_700_000_000_000)
        );
        let error = frame_timestamp(-1).unwrap_err().to_string();
        assert!(error.contains("timestamp_ms"), "{error}");
    }

    #[test]
    fn every_new_tracker_gets_its_own_id() {
        let first = new_tracker();
        let second = new_tracker();
        assert!(!first.tracker_id().is_empty());
        assert_ne!(first.tracker_id(), second.tracker_id());
    }

    #[test]
    fn clock_reset_and_stale_frames_are_reported() {
        let (tracks, skipped, warning) = split_outcome(
            FrameOutcome::Tracked {
                tracks: vec![TrackedObject::default()],
                clock_jump_ms: Some(5_033),
            },
            "door",
        );
        assert_eq!(tracks.len(), 1);
        assert!(!skipped);
        assert_eq!(
            warning.as_deref(),
            Some("Clock of camera 'door' jumped back by 5033 ms; tracks reset")
        );

        let (tracks, skipped, warning) = split_outcome(
            FrameOutcome::Tracked {
                tracks: Vec::new(),
                clock_jump_ms: None,
            },
            "door",
        );
        assert!(tracks.is_empty() && !skipped && warning.is_none());

        let (tracks, skipped, warning) = split_outcome(
            FrameOutcome::Stale {
                timestamp_ms: 1_050,
                last_timestamp_ms: 1_100,
            },
            "",
        );
        assert!(tracks.is_empty());
        assert!(skipped);
        assert_eq!(
            warning.as_deref(),
            Some(
                "Skipped a stale frame: timestamp 1050 ms is older than the last processed frame at 1100 ms"
            )
        );
    }

    #[test]
    fn pins_match_the_tracker_defaults() {
        let node = TrackDetectionsNode::new().get_node();
        let input = |name: &str| {
            node.pins
                .values()
                .find(|pin| pin.name == name && pin.pin_type == PinType::Input)
                .unwrap_or_else(|| panic!("missing input pin {name}"))
        };
        let default = |name: &str| -> flow_like_types::Value {
            flow_like_types::json::from_slice(input(name).default_value.as_ref().unwrap()).unwrap()
        };
        let config = TrackerConfig::default();
        for (name, expected) in [
            ("high_threshold", config.high_threshold),
            ("low_threshold", config.low_threshold),
            ("new_track_threshold", config.new_track_threshold),
            ("match_threshold", config.match_threshold),
            (
                "min_appearance_similarity",
                config.min_appearance_similarity,
            ),
        ] {
            let value = default(name).as_f64().unwrap();
            assert!((value - f64::from(expected)).abs() < 1e-6, "{name}");
        }
        assert_eq!(default("max_lost_ms"), json!(config.max_lost_ms));
        assert_eq!(default("class_aware"), json!(config.class_aware));
        assert_eq!(default("include_lost"), json!(config.include_lost));
        assert_eq!(default("detections"), json!([]));
        assert_eq!(default("camera_id"), json!(""));
        assert_eq!(default("session_id"), json!(""));
        assert_eq!(default("timestamp_ms"), json!(0));
    }
}

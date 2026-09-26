use flow_like_catalog_core::BoundingBox;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A detection plus the identity context needed to follow it over time.
///
/// Every field of `BoundingBox` is flattened in, so plain detection boxes deserialize into it and
/// any pin expecting boxes accepts it. Output of Extract Appearance; input of Associate Entities.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AppearanceObservation {
    #[serde(flatten)]
    pub bbox: BoundingBox,
    /// L2-normalized appearance embedding; empty when none was extracted
    #[serde(default)]
    pub embedding: Vec<f32>,
    /// Camera the observation came from
    #[serde(default)]
    pub camera_id: String,
    /// Stream session of that camera
    #[serde(default)]
    pub session_id: String,
    /// Tracker instance that issued `track_id`; track ids are only unique per tracker instance
    #[serde(default)]
    pub tracker_id: String,
    /// Local track id from Track Detections, if the observation belongs to a track
    #[serde(default)]
    pub track_id: Option<u64>,
    /// State of the source track; plain detections are `tracked`
    #[serde(default)]
    pub state: TrackState,
    /// Frame capture time in Unix milliseconds
    #[serde(default)]
    pub timestamp_ms: i64,
    /// Index of the source element in the producing node's input array
    #[serde(default)]
    pub detection_index: Option<u32>,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrackState {
    /// Matched to a detection in the latest frame
    #[default]
    Tracked,
    /// Not matched recently; kept alive for re-identification until it expires
    Lost,
}

/// A local track from Track Detections. Covers `AppearanceObservation`, so tracks can be fed
/// straight into Extract Appearance or Associate Entities.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TrackedObject {
    #[serde(flatten)]
    pub bbox: BoundingBox,
    /// Smoothed appearance embedding of the track; empty when no embeddings were supplied
    #[serde(default)]
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub camera_id: String,
    #[serde(default)]
    pub session_id: String,
    /// Tracker instance that issued `track_id`. A new tracker (new Camera/Session pin value, 10
    /// minutes without frames, eviction or restart) gets a new one; a clock reset keeps it
    #[serde(default)]
    pub tracker_id: String,
    /// Track id, unique within one tracker instance
    #[serde(default)]
    pub track_id: Option<u64>,
    /// Timestamp of the frame this track state belongs to, in Unix milliseconds
    #[serde(default)]
    pub timestamp_ms: i64,
    /// Index of the matched detection in this frame's input, or null for lost tracks
    #[serde(default)]
    pub detection_index: Option<u32>,
    #[serde(default)]
    pub state: TrackState,
    /// Number of frames the track was matched in
    #[serde(default)]
    pub hits: u32,
    #[serde(default)]
    pub first_seen_ms: i64,
    #[serde(default)]
    pub last_seen_ms: i64,
    /// Box-center velocity in pixels per second
    #[serde(default)]
    pub velocity_x: f32,
    #[serde(default)]
    pub velocity_y: f32,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssociationStatus {
    /// The local track was already bound to this entity
    #[default]
    Tracked,
    /// The local track was newly matched to an existing entity
    Matched,
    /// No existing entity matched; a new one was created
    Created,
}

/// A global identity assigned to one observation.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EntityAssociation {
    #[serde(flatten)]
    pub bbox: BoundingBox,
    #[serde(default)]
    pub camera_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub tracker_id: String,
    #[serde(default)]
    pub track_id: Option<u64>,
    #[serde(default)]
    pub timestamp_ms: i64,
    #[serde(default)]
    pub detection_index: Option<u32>,
    /// Global entity id, unique within the association task
    pub entity_id: u64,
    /// Cosine similarity to the entity's prototype: of this observation when `matched` or
    /// `created`, of the track's last embedded observation when `tracked`
    pub similarity: f32,
    pub status: AssociationStatus,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    /// The local track has not been observed often enough to create an entity
    #[default]
    Pending,
    /// Two or more entities match within the ambiguity margin
    Ambiguous,
    /// The observation carries no usable embedding
    MissingEmbedding,
    /// No track id, so no evidence can accumulate and no entity is created
    Untracked,
    /// The track is lost: its box is a prediction, so it neither matches nor creates an entity
    Lost,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct EntityCandidate {
    pub entity_id: u64,
    pub similarity: f32,
}

/// An observation that could not be given a global identity yet.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct UnresolvedObservation {
    #[serde(flatten)]
    pub bbox: BoundingBox,
    #[serde(default)]
    pub camera_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub tracker_id: String,
    #[serde(default)]
    pub track_id: Option<u64>,
    #[serde(default)]
    pub timestamp_ms: i64,
    #[serde(default)]
    pub detection_index: Option<u32>,
    pub reason: UnresolvedReason,
    /// Observations accumulated for this local track so far
    pub observations: u32,
    /// Closest entities, most similar first
    pub candidates: Vec<EntityCandidate>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::{Pin, schema_covers};

    fn schema<T: JsonSchema>() -> String {
        Pin::schema_string_for::<T>().expect("schema serializes")
    }

    #[test]
    fn observation_types_cover_bounding_box_pins() {
        let bbox = schema::<BoundingBox>();
        assert!(schema_covers(&schema::<AppearanceObservation>(), &bbox));
        assert!(schema_covers(&schema::<TrackedObject>(), &bbox));
        assert!(schema_covers(&schema::<EntityAssociation>(), &bbox));
        assert!(schema_covers(&schema::<UnresolvedObservation>(), &bbox));
    }

    #[test]
    fn tracks_cover_appearance_observations() {
        assert!(schema_covers(
            &schema::<TrackedObject>(),
            &schema::<AppearanceObservation>()
        ));
        assert!(!schema_covers(
            &schema::<BoundingBox>(),
            &schema::<AppearanceObservation>()
        ));
    }

    #[test]
    fn bounding_box_json_deserializes_as_observation() {
        let bbox = BoundingBox {
            x1: 1.0,
            y1: 2.0,
            x2: 3.0,
            y2: 4.0,
            score: 0.9,
            class_idx: 0,
            class_name: Some("person".into()),
        };
        let value = flow_like_types::json::to_value(&bbox).unwrap();
        let observation: AppearanceObservation = flow_like_types::json::from_value(value).unwrap();
        assert_eq!(observation.bbox.x2, 3.0);
        assert_eq!(observation.bbox.class_name.as_deref(), Some("person"));
        assert!(observation.embedding.is_empty());
        assert_eq!(observation.track_id, None);
        assert_eq!(observation.state, TrackState::Tracked);
    }

    #[test]
    fn tracked_object_json_deserializes_as_observation() {
        let track = TrackedObject {
            embedding: vec![0.6, 0.8],
            camera_id: "cam".into(),
            session_id: "s".into(),
            tracker_id: "tracker".into(),
            track_id: Some(7),
            timestamp_ms: 42,
            detection_index: Some(1),
            state: TrackState::Lost,
            ..Default::default()
        };
        let value = flow_like_types::json::to_value(&track).unwrap();
        let observation: AppearanceObservation = flow_like_types::json::from_value(value).unwrap();
        assert_eq!(observation.track_id, Some(7));
        assert_eq!(observation.embedding, vec![0.6, 0.8]);
        assert_eq!(observation.camera_id, "cam");
        assert_eq!(observation.tracker_id, "tracker");
        assert_eq!(observation.state, TrackState::Lost);
        assert_eq!(observation.timestamp_ms, 42);
    }
}

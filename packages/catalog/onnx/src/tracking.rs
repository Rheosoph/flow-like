//! Multi-object tracking and cross-camera re-identification: per-camera tracks (ByteTrack with
//! BoT-SORT-style appearance fusion), appearance embeddings from ONNX re-identification models, and
//! association of tracks from several cameras into global entities.

pub mod appearance_models;
pub mod assignment;
pub mod associate_entities;
pub mod association;
pub mod embedding;
pub mod extract_appearance;
pub mod kalman;
pub mod registry;
pub mod track_detections;
pub mod tracker;
pub mod types;

/// Track Detections → Extract Appearance → Associate Entities over their pure cores, with a
/// simulated re-identification model.
#[cfg(test)]
mod pipeline_tests {
    use super::{
        association::{AssociationConfig, Associator},
        extract_appearance::{IdentityOverrides, assemble, plan_crops},
        tracker::{Frame, FrameOutcome, Tracker, TrackerConfig},
        types::{AppearanceObservation, AssociationStatus, TrackState, UnresolvedReason},
    };
    use flow_like_catalog_core::BoundingBox;
    use flow_like_types::json::{from_value, to_value};
    use std::collections::BTreeMap;

    const T0: i64 = 1_700_000_000_000;
    const FRAME_MS: i64 = 100;
    const FRAMES: i64 = 12;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Person {
        A,
        B,
        C,
    }

    impl Person {
        fn appearance(self) -> Vec<f32> {
            match self {
                Self::A => vec![1.0, 0.0, 0.0, 0.0],
                Self::B => vec![0.0, 1.0, 0.0, 0.0],
                Self::C => vec![0.0, 0.0, 1.0, 0.0],
            }
        }
    }

    type Scene = fn(i64) -> Vec<(Person, AppearanceObservation)>;
    type TrackKey = (String, u64);

    /// Both trackers get detections stamped with the same camera and session and have empty Camera
    /// and Session pins, so only the tracker id keeps their track ids apart.
    struct Camera {
        tracker: Tracker,
        /// Sends frame timestamps; otherwise the pin is 0 and the wall clock is used
        timestamped: bool,
        scene: Scene,
    }

    fn sighting(person: Person, x: f32, y: f32) -> (Person, AppearanceObservation) {
        let observation = AppearanceObservation {
            bbox: BoundingBox {
                x1: x,
                y1: y,
                x2: x + 80.0,
                y2: y + 200.0,
                score: 0.9,
                class_idx: 0,
                class_name: Some("person".into()),
            },
            camera_id: "lobby".into(),
            session_id: "s1".into(),
            ..Default::default()
        };
        (person, observation)
    }

    /// A walks right; B walks left and leaves after frame 7.
    fn first_scene(frame: i64) -> Vec<(Person, AppearanceObservation)> {
        let step = frame as f32;
        let mut scene = vec![sighting(Person::A, 100.0 + 10.0 * step, 100.0)];
        if frame <= 7 {
            scene.push(sighting(Person::B, 1500.0 - 10.0 * step, 600.0));
        }
        scene
    }

    /// C stands still; A enters at frame 6.
    fn second_scene(frame: i64) -> Vec<(Person, AppearanceObservation)> {
        let mut scene = vec![sighting(Person::C, 800.0, 300.0)];
        if frame >= 6 {
            let step = (frame - 6) as f32;
            scene.push(sighting(Person::A, 300.0 + 8.0 * step, 700.0));
        }
        scene
    }

    fn track_key(tracker_id: &str, track_id: Option<u64>) -> TrackKey {
        (
            tracker_id.to_owned(),
            track_id.expect("tracks carry a track id"),
        )
    }

    /// One frame through Track Detections and Extract Appearance. The model returns the true
    /// appearance of the person a track's matched detection shows.
    fn observe(
        camera: &mut Camera,
        frame: i64,
        persons: &mut BTreeMap<TrackKey, Person>,
    ) -> Vec<AppearanceObservation> {
        let wall_ms = T0 + frame * FRAME_MS;
        let (people, detections): (Vec<Person>, Vec<AppearanceObservation>) =
            (camera.scene)(frame).into_iter().unzip();
        let config = TrackerConfig {
            include_lost: true,
            ..Default::default()
        };
        let frame_input = Frame {
            detections: &detections,
            timestamp_ms: camera.timestamped.then_some(wall_ms),
            camera_id: "",
            session_id: "",
        };
        let tracks = match camera.tracker.update(frame_input, wall_ms, &config) {
            FrameOutcome::Tracked {
                tracks,
                clock_jump_ms: None,
            } => tracks,
            other => panic!("frame {frame} was not tracked normally: {other:?}"),
        };
        for track in &tracks {
            assert_eq!(track.tracker_id, camera.tracker.tracker_id());
            assert_eq!(
                (track.camera_id.as_str(), track.session_id.as_str()),
                ("lobby", "s1")
            );
            assert_eq!(track.timestamp_ms, wall_ms);
            if let Some(index) = track.detection_index {
                let person = people[index as usize];
                let key = track_key(&track.tracker_id, track.track_id);
                let known = *persons.entry(key.clone()).or_insert(person);
                assert_eq!(known, person, "track {key:?} switched person");
            }
        }

        let inputs: Vec<AppearanceObservation> = from_value(to_value(&tracks).unwrap()).unwrap();
        let plan = plan_crops(&inputs, 1920, 1080, 0.0, 4.0);
        assert!(plan.skipped.is_empty(), "{plan:?}");
        let embeddings = plan
            .crops
            .iter()
            .map(|&(index, _)| {
                let key = track_key(&tracks[index].tracker_id, tracks[index].track_id);
                (index, persons[&key].appearance())
            })
            .collect();
        let observations = assemble(inputs, embeddings, &IdentityOverrides::default(), wall_ms);
        assert_eq!(observations.len(), tracks.len());
        for (observation, track) in observations.iter().zip(&tracks) {
            assert_eq!(observation.tracker_id, track.tracker_id);
            assert_eq!(observation.track_id, track.track_id);
            assert_eq!(observation.state, track.state);
            assert_eq!(observation.camera_id, track.camera_id);
            assert_eq!(observation.timestamp_ms, track.timestamp_ms);
        }
        observations
    }

    #[test]
    fn trackers_sharing_labels_keep_stable_entities_and_never_share_bindings() {
        let mut cameras = [
            Camera {
                tracker: Tracker::new("t1".into()),
                timestamped: true,
                scene: first_scene,
            },
            Camera {
                tracker: Tracker::new("t2".into()),
                timestamped: false,
                scene: second_scene,
            },
        ];
        let config = AssociationConfig::new(0.6, 0.05, 3, 600_000).unwrap();
        let mut associator = Associator::new();
        let mut persons = BTreeMap::<TrackKey, Person>::new();
        let mut entities = BTreeMap::<Person, u64>::new();
        let mut owners = BTreeMap::<u64, Person>::new();
        let mut statuses = BTreeMap::<TrackKey, Vec<AssociationStatus>>::new();
        let mut lost_reports = 0;

        for frame in 0..FRAMES {
            let wall_ms = T0 + frame * FRAME_MS;
            for camera in &mut cameras {
                let observations = observe(camera, frame, &mut persons);
                let output = associator
                    .associate(&observations, &config, wall_ms)
                    .unwrap();
                assert_eq!(output.clamped_timestamps, 0);
                assert_eq!(
                    output.associations.len() + output.unresolved.len(),
                    observations.len()
                );
                for unresolved in &output.unresolved {
                    assert_eq!(
                        unresolved.reason,
                        UnresolvedReason::Pending,
                        "{unresolved:?}"
                    );
                }
                for association in &output.associations {
                    let key = track_key(&association.tracker_id, association.track_id);
                    let person = persons[&key];
                    let entity_id = *entities.entry(person).or_insert(association.entity_id);
                    assert_eq!(
                        entity_id, association.entity_id,
                        "{person:?} changed entity in frame {frame}: {association:?}"
                    );
                    let owner = *owners.entry(entity_id).or_insert(person);
                    assert_eq!(
                        owner, person,
                        "entity {entity_id} shared in frame {frame}: {association:?}"
                    );
                    let index = association.detection_index.expect("index into the batch");
                    if observations[index as usize].state == TrackState::Lost {
                        lost_reports += 1;
                        assert_eq!(association.status, AssociationStatus::Tracked);
                    }
                    statuses.entry(key).or_default().push(association.status);
                }
            }
        }

        assert_eq!(persons[&track_key("t1", Some(1))], Person::A);
        assert_eq!(persons[&track_key("t2", Some(1))], Person::C);
        assert_eq!(
            entities,
            BTreeMap::from([(Person::A, 1), (Person::B, 2), (Person::C, 3)])
        );
        assert_eq!(
            statuses[&track_key("t2", Some(2))].first(),
            Some(&AssociationStatus::Matched),
            "A is recognised on the second tracker"
        );
        assert_eq!(lost_reports, 4, "B stays on its entity while lost");
    }
}

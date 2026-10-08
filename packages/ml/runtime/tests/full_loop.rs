#![cfg(all(feature = "native", feature = "burn"))]
use flow_like_ml_core::{
    Annotation, ComputeBackend, ComputeConfig, LabelProvenance, Sample, TensorData,
};
use flow_like_ml_runtime::{engines::*, worker::TrainingWorker, *};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

static TRAINING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const LABELS: [&str; 2] = ["good", "defect"];

fn compute() -> ComputeConfig {
    ComputeConfig {
        backend: ComputeBackend::Cpu,
        ..ComputeConfig::default()
    }
}
fn split() -> SplitPolicy {
    SplitPolicy::Group {
        train_fraction: 0.6,
        validation_fraction: 0.2,
        seed: 73,
    }
}
fn record_teachers(
    repo: &TrainingRepository,
    stream: &StreamKey,
    image: bool,
    indices: std::ops::Range<usize>,
) {
    for i in indices {
        let class = i % 2;
        let offset = i as f32 * 0.0001;
        let input = if image {
            TensorData {
                shape: vec![1, 8, 8],
                values: (0..64)
                    .map(|pixel| {
                        let foreground = if class == 0 {
                            pixel % 8 >= 4
                        } else {
                            pixel / 8 >= 4
                        };
                        (if foreground { 1. } else { -1. }) + offset
                    })
                    .collect(),
            }
        } else {
            TensorData {
                shape: vec![4, 1],
                values: (0..4)
                    .map(|t| (if class == 0 { -1. } else { 1. }) * (1. + t as f32 * 0.1) + offset)
                    .collect(),
            }
        };
        let content_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&input).unwrap()));
        let sample = Sample {
            id: format!("sample-{i}"),
            group_id: format!("lot-{}", i / 2),
            stream_id: stream.stream_id.clone(),
            timestamp_ms: i as i64 * 10,
            window_start_ms: i as i64 * 10,
            window_end_ms: i as i64 * 10 + 4,
            input,
            annotation: Annotation::Class {
                class_id: class as u32,
            },
            provenance: LabelProvenance::Teacher {
                model: "fixture-teacher".into(),
                prompt_digest: "quality-spec-v1".into(),
                confidence: Some(0.99),
            },
            outcome: None,
        };
        let mut payload = serde_json::to_value(&sample).unwrap();
        payload["label"] = json!(LABELS[class]);
        repo.record_sample(
            stream,
            &TrainingSample {
                id: sample.id,
                annotation_revision: 1,
                group_id: sample.group_id,
                captured_at_ms: sample.timestamp_ms,
                label_available_at_ms: sample.window_end_ms,
                content_digest,
                source: LabelSource::Teacher,
                accepted: true,
                payload,
            },
        )
        .unwrap();
    }
}
fn request(image: bool) -> TrainingRequest {
    let recipe = if image {
        json!({"architecture":"res_net18","input_channels":1,"classes":2,"base_channels":4})
    } else {
        json!({"architecture":"lstm","input_features":1,"hidden":4,"outputs":2,"objective":"classification"})
    };
    TrainingRequest {
        engine: "burn".into(),
        recipe: json!({"config":{"recipe":recipe,"backend":{"backend":"cpu"},"epochs":if image {50} else {14},"batch_size":8,"learning_rate":if image {0.005} else {0.03},"seed":73,"gradient_clip":5.},"labels":LABELS,"minimum_examples":4,"minimum_examples_per_class":2}),
        compute: serde_json::to_value(compute()).unwrap(),
    }
}
fn audit(repo: &TrainingRepository, artifact: &ModelArtifact) -> EvaluationReport {
    let snapshot = repo.get_snapshot(&artifact.snapshot_id).unwrap();
    assert!(
        snapshot
            .train
            .iter()
            .any(|sample| sample.source == LabelSource::Teacher)
    );
    let mut input = TensorData {
        shape: vec![snapshot.test.len()],
        values: Vec::new(),
    };
    for (i, sample) in snapshot.test.iter().enumerate() {
        let core: Sample = serde_json::from_value(sample.payload.clone()).unwrap();
        if i == 0 {
            input.shape.extend(&core.input.shape);
        }
        input.values.extend(core.input.values);
    }
    let prediction = load_artifact_predictions(repo, &artifact.id, &input, &compute()).unwrap();
    let classes = prediction["classes"].as_array().unwrap();
    assert_eq!(classes.len(), snapshot.test.len());
    for (sample, predicted) in snapshot.test.into_iter().zip(classes) {
        let class = predicted.as_u64().unwrap() as usize;
        assert!(class < LABELS.len());
        let mut reviewed = sample.clone();
        if reviewed.source == LabelSource::Teacher {
            reviewed.annotation_revision += 1;
            reviewed.source = LabelSource::Reviewed;
            reviewed.label_available_at_ms = 5000;
            reviewed.payload["provenance"] =
                json!({"kind":"reviewed","reviewer":"independent-operator","reviewed_at_ms":5000});
            repo.record_sample(&artifact.stream, &reviewed).unwrap();
        }
        let actual = sample.payload["label"].as_str().unwrap().to_owned();
        repo.shadow_telemetry()
            .record_prediction(&PredictionRecord {
                sample_id: sample.id,
                artifact_id: artifact.id.clone(),
                predicted_label: Some(LABELS[class].into()),
                teacher_label: Some(actual.clone()),
                actual_label: Some(actual),
                actual_source: Some(LabelSource::Reviewed),
                latency_ms: 1.,
                failed: false,
                recorded_at_ms: 6000,
                details: json!({"class_id":class}),
            })
            .unwrap();
    }
    let report = repo.evaluate_classification(&artifact.id, 6001).unwrap();
    assert!(report.audited_samples >= 2);
    assert!(
        report.metrics["accuracy"] >= 0.75,
        "model did not learn the held-out fixture: {:?}",
        report.metrics
    );
    assert!(report.per_class_recall.contains_key("good"));
    assert!(report.per_class_recall.contains_key("defect"));
    report
}
fn policy(audits: usize) -> PromotionPolicy {
    PromotionPolicy {
        minimum_audited_samples: audits,
        minimum_accuracy: 0.75,
        minimum_class_recall: BTreeMap::new(),
        maximum_mean_latency_ms: 10.,
        maximum_failure_rate: 0.,
    }
}
fn full_loop(image: bool) {
    let _lock = TRAINING_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "factory".into(),
        stream_id: if image { "camera" } else { "vibration" }.into(),
        inspection_version: "v1".into(),
    };
    record_teachers(&repo, &stream, image, 0..15);
    assert!(
        repo.trigger_after_n(&stream, 16, request(image), split(), 10000)
            .unwrap()
            .is_none()
    );
    record_teachers(&repo, &stream, image, 15..16);
    let job = repo
        .trigger_after_n(&stream, 16, request(image), split(), 10000)
        .unwrap()
        .unwrap();
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    register_burn_engine(&mut worker).unwrap();
    let first = worker.run(&job.id).unwrap();
    assert_eq!(first.manifest["labels"], json!(LABELS));
    let report = audit(&repo, &first);
    repo.promote("production", 0, &report.id, policy(2), 11000)
        .unwrap();
    assert!(
        repo.promote(
            "production",
            1,
            &report.id,
            policy(report.audited_samples + 1),
            11001
        )
        .is_err()
    );
    assert_eq!(
        repo.get_deployment("production")
            .unwrap()
            .active_artifact_id,
        first.id
    );
    record_teachers(&repo, &stream, image, 16..19);
    assert!(
        repo.trigger_after_n(&stream, 4, request(image), split(), 12000)
            .unwrap()
            .is_none(),
        "review revisions must not count as new training examples"
    );
    record_teachers(&repo, &stream, image, 19..20);
    let next = repo
        .trigger_after_n(&stream, 4, request(image), split(), 12000)
        .unwrap()
        .unwrap();
    let second = worker.run(&next.id).unwrap();
    let report = audit(&repo, &second);
    let deployment = repo
        .promote("production", 1, &report.id, policy(2), 13000)
        .unwrap();
    assert!(!deployment.paused);
    assert_eq!(
        deployment.previous_artifact_id.as_deref(),
        Some(first.id.as_str())
    );
    drop(worker);
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let route = repo.get_deployment("production").unwrap();
    assert_eq!(route.active_artifact_id, second.id);
    let sample = repo
        .get_snapshot(&second.snapshot_id)
        .unwrap()
        .test
        .remove(0);
    let sample: Sample = serde_json::from_value(sample.payload).unwrap();
    let mut input = sample.input;
    input.shape.insert(0, 1);
    let output: Value =
        load_artifact_predictions(&repo, &route.active_artifact_id, &input, &compute()).unwrap();
    assert_eq!(output["classes"].as_array().unwrap().len(), 1);
    let suspended = repo
        .set_deployment_paused("production", route.generation, true, 13001)
        .unwrap();
    assert!(suspended.paused);
    let rolled = repo
        .rollback("production", suspended.generation, 13002)
        .unwrap();
    assert_eq!(rolled.active_artifact_id, first.id);
    assert!(!rolled.paused);
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Succeeded);
    assert_eq!(repo.get_job(&next.id).unwrap().status, JobStatus::Succeeded);
}
#[test]
fn image_teacher_student_loop_trains_audits_promotes_and_rolls_back() {
    full_loop(true);
}
#[test]
fn lstm_teacher_student_loop_retrains_after_failed_gate() {
    full_loop(false);
}

#![cfg(feature = "native")]
use flow_like_ml_core::{Annotation, BoundingBox, LabelProvenance, Sample, TensorData};
use flow_like_ml_runtime::*;
use serde_json::json;

fn fixture(annotation: Annotation) -> (tempfile::TempDir, TrainingRepository, ModelArtifact) {
    fixture_adjusted(annotation, |_| {})
}
fn fixture_adjusted(
    annotation: Annotation,
    customize: impl Fn(&mut Sample),
) -> (tempfile::TempDir, TrainingRepository, ModelArtifact) {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("ledger.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "factory".into(),
        stream_id: "camera".into(),
        inspection_version: "v1".into(),
    };
    for i in 0..9 {
        let mut sample = Sample {
            id: format!("image-{i}"),
            group_id: format!("lot-{i}"),
            stream_id: "camera".into(),
            timestamp_ms: i,
            window_start_ms: i,
            window_end_ms: i,
            input: TensorData {
                shape: vec![1],
                values: vec![i as f32],
            },
            annotation: annotation.clone(),
            provenance: LabelProvenance::Reviewed {
                reviewer: "operator".into(),
                reviewed_at_ms: i,
            },
            outcome: None,
        };
        customize(&mut sample);
        repo.record_sample(
            &stream,
            &TrainingSample {
                id: sample.id.clone(),
                annotation_revision: 1,
                group_id: sample.group_id.clone(),
                captured_at_ms: i,
                label_available_at_ms: sample.window_end_ms.max(
                    sample
                        .outcome
                        .as_ref()
                        .map_or(i, |outcome| outcome.available_at_ms),
                ),
                content_digest: format!("{i:064x}"),
                source: LabelSource::Reviewed,
                accepted: true,
                payload: serde_json::to_value(sample).unwrap(),
            },
        )
        .unwrap();
    }
    let job = repo
        .trigger_after_n(
            &stream,
            9,
            TrainingRequest {
                engine: "fixture".into(),
                recipe: json!({}),
                compute: json!({}),
            },
            SplitPolicy::Group {
                train_fraction: 0.5,
                validation_fraction: 0.2,
                seed: 42,
            },
            1000,
        )
        .unwrap()
        .unwrap();
    let lease = repo.claim_job(&job.id, "worker", 1000, 1000).unwrap();
    let artifact = repo
        .finish_training(&lease, b"model", json!({}), 1001)
        .unwrap();
    (dir, repo, artifact)
}

#[test]
fn event_evidence_deduplicates_alarms_events_and_observed_time() {
    let (_dir, repo, artifact) =
        fixture_adjusted(Annotation::Anomaly { is_anomaly: true }, |sample| {
            sample.window_start_ms = 0;
            sample.window_end_ms = 100;
            sample.provenance = LabelProvenance::Reviewed {
                reviewer: "operator".into(),
                reviewed_at_ms: 115,
            };
            sample.outcome = Some(flow_like_ml_core::Outcome {
                available_at_ms: 115,
                target_start_ms: 110,
                target_end_ms: 115,
            });
        });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Events {
            alarms_ms: vec![5, 90],
        },
    );
    let report = repo
        .evaluate_task(
            &artifact.id,
            EvaluationTask::Events { horizon_ms: 30 },
            2001,
        )
        .unwrap();
    assert_eq!(report.metrics["events"], 1.);
    assert_eq!(report.metrics["detected_events"], 1.);
    assert_eq!(report.metrics["event_recall"], 1.);
    assert_eq!(report.metrics["mean_lead_time_ms"], 20.);
    assert_eq!(report.metrics["observed_duration_ms"], 100.);
    assert_eq!(report.metrics["false_alarms_per_hour"], 36000.);
    assert!(
        repo.promote_metrics(
            "alarms",
            0,
            &report.id,
            policy("false_alarms_per_hour", None, Some(1.)),
            2002
        )
        .is_err()
    );
}
fn record(repo: &TrainingRepository, artifact: &ModelArtifact, evidence: EvidencePrediction) {
    for sample in repo.get_snapshot(&artifact.snapshot_id).unwrap().test {
        repo.record_prediction(&PredictionRecord {
            sample_id: sample.id,
            artifact_id: artifact.id.clone(),
            predicted_label: None,
            teacher_label: None,
            actual_label: None,
            actual_source: Some(LabelSource::Reviewed),
            latency_ms: 2.,
            failed: false,
            recorded_at_ms: 2000,
            details: serde_json::to_value(&evidence).unwrap(),
        })
        .unwrap();
    }
}
fn policy(name: &str, min: Option<f64>, max: Option<f64>) -> MetricPromotionPolicy {
    MetricPromotionPolicy {
        minimum_audited_samples: 1,
        bounds: vec![MetricBound {
            name: name.into(),
            minimum: min,
            maximum: max,
        }],
    }
}

#[test]
fn regression_proof_is_computed_and_metric_direction_is_explicit() {
    let (_dir, repo, artifact) = fixture(Annotation::Scalar { value: 4. });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Regression { values: vec![3.] },
    );
    let report = repo
        .evaluate_task(&artifact.id, EvaluationTask::Regression, 2001)
        .unwrap();
    assert_eq!(report.metrics["rmse"], 1.);
    assert!(
        repo.promote_metrics(
            "production",
            0,
            &report.id,
            policy("rmse", None, Some(0.5)),
            2002
        )
        .is_err()
    );
    let deployed = repo
        .promote_metrics(
            "production",
            0,
            &report.id,
            policy("rmse", None, Some(1.)),
            2002,
        )
        .unwrap();
    assert!(deployed.policy.is_none());
    assert!(deployed.metric_policy.is_some());
    assert!(
        repo.promote_metrics(
            "other",
            0,
            &report.id,
            policy("invented_accuracy", Some(0.), None),
            2003
        )
        .is_err()
    );
}

#[test]
fn detection_and_segmentation_use_audited_localization_truth() {
    let bounds = BoundingBox {
        class_id: 0,
        x_min: 0.1,
        y_min: 0.2,
        x_max: 0.8,
        y_max: 0.9,
    };
    let (_dir, repo, artifact) = fixture(Annotation::Boxes {
        boxes: vec![bounds.clone()],
    });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Detection {
            boxes: vec![DetectionEvidence {
                class_id: 0,
                x_min: bounds.x_min,
                y_min: bounds.y_min,
                x_max: bounds.x_max,
                y_max: bounds.y_max,
                confidence: 0.9,
            }],
        },
    );
    let report = repo
        .evaluate_task(
            &artifact.id,
            EvaluationTask::Detection {
                classes: 1,
                iou_threshold: 0.5,
            },
            2001,
        )
        .unwrap();
    assert_eq!(report.metrics["mean_average_precision"], 1.);
    repo.promote_metrics(
        "detector",
        0,
        &report.id,
        policy("recall/0", Some(1.), None),
        2002,
    )
    .unwrap();
    let (_dir, repo, artifact) = fixture(Annotation::Mask {
        width: 2,
        height: 2,
        classes: vec![0, 0, 1, 1],
    });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Segmentation {
            width: 2,
            height: 2,
            classes: vec![0, 0, 1, 1],
        },
    );
    let report = repo
        .evaluate_task(
            &artifact.id,
            EvaluationTask::Segmentation { classes: 2 },
            2001,
        )
        .unwrap();
    assert_eq!(report.metrics["mean_iou"], 1.);
    assert_eq!(report.metrics["mean_dice"], 1.);
    repo.promote_metrics(
        "segmentation",
        0,
        &report.id,
        policy("mean_iou", Some(0.99), None),
        2002,
    )
    .unwrap();
}

#[test]
fn adding_evidence_invalidates_previous_promotion_proof() {
    let (_dir, repo, artifact) = fixture(Annotation::Scalar { value: 4. });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Regression { values: vec![4.] },
    );
    let report = repo
        .evaluate_task(&artifact.id, EvaluationTask::Regression, 2001)
        .unwrap();
    repo.record_prediction(&PredictionRecord {
        sample_id: "teacher-probe".into(),
        artifact_id: artifact.id.clone(),
        predicted_label: Some("good".into()),
        teacher_label: Some("bad".into()),
        actual_label: None,
        actual_source: None,
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: 2002,
        details: json!({}),
    })
    .unwrap();
    assert!(matches!(
        repo.promote_metrics(
            "production",
            0,
            &report.id,
            policy("rmse", None, Some(0.)),
            2003
        ),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn outcome_horizon_and_feature_overlap_are_purged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("ledger.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "p".into(),
        stream_id: "s".into(),
        inspection_version: "1".into(),
    };
    for (id, start, end, target_end) in [
        (0, 0, 10, 15),
        (1, 21, 30, 35),
        (2, 41, 50, 55),
        (3, 18, 25, 30),
        (4, 10, 18, 25),
    ] {
        repo.record_sample(&stream,&TrainingSample{id:format!("s{id}"),annotation_revision:1,group_id:format!("g{id}"),captured_at_ms:end,label_available_at_ms:target_end,content_digest:format!("{id:064x}"),source:LabelSource::ObservedOutcome,accepted:true,payload:json!({"window_start_ms":start,"window_end_ms":end,"outcome":{"available_at_ms":target_end,"target_end_ms":target_end}})}).unwrap();
    }
    let snapshot = repo
        .snapshot(
            &stream,
            SplitPolicy::Time {
                train_end_ms: 20,
                validation_end_ms: 40,
                embargo_ms: 0,
            },
            100,
        )
        .unwrap();
    assert_eq!(snapshot.train.len(), 1);
    assert_eq!(snapshot.validation.len(), 1);
    assert_eq!(snapshot.test.len(), 1);
    assert_eq!(snapshot.excluded.len(), 2);
}

#[test]
fn readiness_does_not_consume_a_training_trigger_when_class_support_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("ledger.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "p".into(),
        stream_id: "s".into(),
        inspection_version: "1".into(),
    };
    for i in 0..30 {
        repo.record_sample(
            &stream,
            &TrainingSample {
                id: format!("s{i}"),
                annotation_revision: 1,
                group_id: format!("g{i}"),
                captured_at_ms: i,
                label_available_at_ms: i,
                content_digest: format!("{i:064x}"),
                source: LabelSource::Reviewed,
                accepted: true,
                payload: json!({"annotation":{"kind":"class","class_id":0},"label":"good"}),
            },
        )
        .unwrap();
    }
    let split = SplitPolicy::Group {
        train_fraction: 0.6,
        validation_fraction: 0.2,
        seed: 42,
    };
    let request = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"labels":["good","bad"],"minimum_examples":10,"minimum_examples_per_class":1}),
        compute: json!({}),
    };
    assert!(
        repo.trigger_after_n(&stream, 30, request.clone(), split.clone(), 1000)
            .unwrap()
            .is_none()
    );
    assert!(repo.list_jobs(&stream).unwrap().is_empty());
    for i in 0..30 {
        repo.record_sample(&stream,&TrainingSample{id:format!("s{i}"),annotation_revision:2,group_id:format!("g{i}"),captured_at_ms:i,label_available_at_ms:i,content_digest:format!("{i:064x}"),source:LabelSource::Reviewed,accepted:true,payload:json!({"annotation":{"kind":"class","class_id":i%2},"label":if i%2==0 {"good"} else {"bad"}})}).unwrap();
    }
    assert!(
        repo.trigger_after_n(&stream, 30, request, split, 1000)
            .unwrap()
            .is_some(),
        "failed readiness must not advance the accepted-sample trigger"
    );
}

fn record_one(
    repo: &TrainingRepository,
    artifact: &ModelArtifact,
    sample: &TrainingSample,
    evidence: EvidencePrediction,
) {
    repo.record_prediction(&PredictionRecord {
        sample_id: sample.id.clone(),
        artifact_id: artifact.id.clone(),
        predicted_label: None,
        teacher_label: None,
        actual_label: None,
        actual_source: Some(LabelSource::Reviewed),
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: 2000,
        details: serde_json::to_value(evidence).unwrap(),
    })
    .unwrap();
}

#[test]
fn anomaly_probabilities_have_audited_metrics_and_truth_revisions_invalidate_proofs() {
    let (_dir, repo, artifact) = fixture(Annotation::Anomaly { is_anomaly: false });
    let samples = repo.get_snapshot(&artifact.snapshot_id).unwrap().test;
    for (index, sample) in samples.iter().enumerate() {
        let mut revised = sample.clone();
        revised.annotation_revision += 1;
        revised.label_available_at_ms = 2100;
        revised.payload["annotation"] = json!({"kind":"anomaly","is_anomaly":index%2==0});
        revised.payload["provenance"] =
            json!({"kind":"reviewed","reviewer":"operator","reviewed_at_ms":2100});
        repo.record_sample(&artifact.stream, &revised).unwrap();
        record_one(
            &repo,
            &artifact,
            sample,
            EvidencePrediction::Anomaly {
                probability: if index % 2 == 0 { 0.9 } else { 0.1 },
            },
        );
    }
    let task = EvaluationTask::Anomaly { threshold: 0.5 };
    assert!(
        repo.evaluate_task(&artifact.id, task.clone(), 2050)
            .is_err(),
        "future review labels cannot enter an earlier evaluation"
    );
    let report = repo
        .evaluate_task(&artifact.id, task.clone(), 2101)
        .unwrap();
    for metric in [
        "auroc",
        "average_precision",
        "accuracy",
        "precision",
        "recall",
    ] {
        assert_eq!(report.metrics[metric], 1.);
    }
    assert_eq!(report.metrics["false_positive_rate"], 0.);
    assert!((report.metrics["brier_score"] - 0.01).abs() < 1e-10);
    let mut insufficient = policy("auroc", Some(0.9), None);
    insufficient.minimum_audited_samples = samples.len() + 1;
    assert!(
        repo.promote_metrics("too-small", 0, &report.id, insufficient, 2102)
            .is_err()
    );
    assert!(
        repo.promote_metrics(
            "overconfident",
            0,
            &report.id,
            policy("brier_score", None, Some(0.001)),
            2102
        )
        .is_err()
    );
    repo.promote_metrics(
        "anomaly",
        0,
        &report.id,
        policy("brier_score", None, Some(0.02)),
        2102,
    )
    .unwrap();
    let mut correction = samples[0].clone();
    correction.annotation_revision = 3;
    correction.label_available_at_ms = 2200;
    correction.payload["annotation"] = json!({"kind":"anomaly","is_anomaly":false});
    correction.payload["provenance"] =
        json!({"kind":"reviewed","reviewer":"operator","reviewed_at_ms":2200});
    repo.record_sample(&artifact.stream, &correction).unwrap();
    assert!(matches!(
        repo.promote_metrics(
            "stale",
            0,
            &report.id,
            policy("auroc", Some(0.9), None),
            2201
        ),
        Err(Error::Conflict(_))
    ));
    let corrected = repo.evaluate_task(&artifact.id, task, 2201).unwrap();
    assert_ne!(report.truth_digest, corrected.truth_digest);
    assert!(corrected.metrics["accuracy"] < report.metrics["accuracy"]);
}

#[test]
fn anomaly_audit_rejects_raw_scores_training_leakage_and_teacher_claims() {
    let (_dir, repo, artifact) = fixture(Annotation::Anomaly { is_anomaly: false });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Anomaly { probability: 1.1 },
    );
    assert!(
        repo.evaluate_task(
            &artifact.id,
            EvaluationTask::Anomaly { threshold: 0.5 },
            2001
        )
        .is_err()
    );
    let (_dir, repo, artifact) = fixture(Annotation::Anomaly { is_anomaly: false });
    let sample = repo
        .get_snapshot(&artifact.snapshot_id)
        .unwrap()
        .train
        .remove(0);
    record_one(
        &repo,
        &artifact,
        &sample,
        EvidencePrediction::Anomaly { probability: 0.1 },
    );
    assert!(
        repo.evaluate_task(
            &artifact.id,
            EvaluationTask::Anomaly { threshold: 0.5 },
            2001
        )
        .is_err()
    );
    let (_dir, repo, artifact) =
        fixture_adjusted(Annotation::Anomaly { is_anomaly: false }, |sample| {
            sample.provenance = LabelProvenance::Teacher {
                model: "teacher".into(),
                prompt_digest: "prompt".into(),
                confidence: Some(1.),
            };
        });
    record(
        &repo,
        &artifact,
        EvidencePrediction::Anomaly { probability: 0.1 },
    );
    assert!(
        repo.evaluate_task(
            &artifact.id,
            EvaluationTask::Anomaly { threshold: 0.5 },
            2001
        )
        .is_err()
    );
}

fn instance_truth() -> Annotation {
    Annotation::InstanceMasks {
        instances: vec![flow_like_ml_core::InstanceMask {
            instance_id: "part".into(),
            bounds: BoundingBox {
                class_id: 0,
                x_min: 0.,
                y_min: 0.,
                x_max: 0.5,
                y_max: 1.,
            },
            width: 2,
            height: 2,
            foreground: vec![true, false, true, false],
        }],
    }
}
fn instance_fixture() -> (tempfile::TempDir, TrainingRepository, ModelArtifact) {
    fixture_adjusted(instance_truth(), |sample| {
        sample.input = TensorData {
            shape: vec![1, 2, 2],
            values: vec![0., 1., 0., 1.],
        };
    })
}
fn instance_prediction(class_id: u32, confidence: f64, foreground: Vec<bool>) -> InstanceEvidence {
    InstanceEvidence {
        class_id,
        confidence,
        width: 2,
        height: 2,
        foreground,
    }
}
#[test]
fn instance_masks_match_by_class_and_score_and_fail_gates_on_false_positives() {
    let (_dir, repo, artifact) = instance_fixture();
    record(
        &repo,
        &artifact,
        EvidencePrediction::Instances {
            instances: vec![
                instance_prediction(0, 0.1, vec![false, true, false, true]),
                instance_prediction(0, 0.9, vec![true, false, true, false]),
            ],
        },
    );
    let task = EvaluationTask::InstanceSegmentation {
        classes: 2,
        iou_threshold: 0.5,
    };
    let report = repo
        .evaluate_task(&artifact.id, task.clone(), 2001)
        .unwrap();
    assert_eq!(
        report.metrics["mask_mean_average_precision"], 1.,
        "high-scoring true masks must be ranked before low-scoring false masks"
    );
    assert_eq!(report.metrics["mask_mean_iou"], 1.);
    assert_eq!(report.metrics["precision/0"], 0.5);
    assert_eq!(report.metrics["recall/0"], 1.);
    assert!(
        repo.promote_metrics(
            "no-false-masks",
            0,
            &report.id,
            policy("precision/0", Some(0.9), None),
            2002
        )
        .is_err()
    );
    repo.promote_metrics(
        "mask-ap",
        0,
        &report.id,
        policy("mask_mean_average_precision", Some(0.99), None),
        2002,
    )
    .unwrap();
    let (_dir, repo, artifact) = instance_fixture();
    record(
        &repo,
        &artifact,
        EvidencePrediction::Instances {
            instances: vec![instance_prediction(1, 0.99, vec![true, false, true, false])],
        },
    );
    let wrong_class = repo.evaluate_task(&artifact.id, task, 2001).unwrap();
    assert_eq!(wrong_class.metrics["mask_mean_average_precision"], 0.);
    assert_eq!(wrong_class.metrics["mask_mean_iou"], 0.);
    assert_eq!(wrong_class.metrics["recall/0"], 0.);
}

#[test]
fn instance_metrics_reject_malformed_grids_and_changed_review_truth() {
    let (_dir, repo, artifact) = instance_fixture();
    record(
        &repo,
        &artifact,
        EvidencePrediction::Instances {
            instances: vec![instance_prediction(0, 0.9, vec![true, false])],
        },
    );
    assert!(
        repo.evaluate_task(
            &artifact.id,
            EvaluationTask::InstanceSegmentation {
                classes: 1,
                iou_threshold: 0.5
            },
            2001
        )
        .is_err()
    );
    let (_dir, repo, artifact) = instance_fixture();
    record(
        &repo,
        &artifact,
        EvidencePrediction::Instances {
            instances: vec![instance_prediction(0, 0.9, vec![true, false, true, false])],
        },
    );
    let task = EvaluationTask::InstanceSegmentation {
        classes: 1,
        iou_threshold: 0.5,
    };
    let report = repo
        .evaluate_task(&artifact.id, task.clone(), 2001)
        .unwrap();
    let mut revised = repo
        .get_snapshot(&artifact.snapshot_id)
        .unwrap()
        .test
        .remove(0);
    revised.annotation_revision += 1;
    revised.label_available_at_ms = 2100;
    revised.payload["annotation"]["instances"][0]["foreground"] = json!([false, true, false, true]);
    repo.record_sample(&artifact.stream, &revised).unwrap();
    assert!(matches!(
        repo.promote_metrics(
            "stale-mask",
            0,
            &report.id,
            policy("mask_mean_average_precision", Some(1.), None),
            2101
        ),
        Err(Error::Conflict(_))
    ));
    let corrected = repo.evaluate_task(&artifact.id, task, 2101).unwrap();
    assert!(corrected.metrics["mask_mean_iou"] < report.metrics["mask_mean_iou"]);
}

#[test]
fn normal_training_readiness_retains_anomalies_only_for_audit() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("normal.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "p".into(),
        stream_id: "camera".into(),
        inspection_version: "v1".into(),
    };
    for i in 0..18 {
        repo.record_sample(
            &stream,
            &TrainingSample {
                id: format!("s{i}"),
                annotation_revision: 1,
                group_id: format!("g{}", i / 2),
                captured_at_ms: i,
                label_available_at_ms: i,
                content_digest: format!("{i:064x}"),
                source: LabelSource::Reviewed,
                accepted: true,
                payload: json!({"annotation":{"kind":"anomaly","is_anomaly":i%2==1}}),
            },
        )
        .unwrap();
    }
    let request = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"inspection_task":"visual_anomaly","labels":["normal","defect"],"minimum_examples":4,"minimum_examples_per_class":4}),
        compute: json!({}),
    };
    let mut waiting = request.clone();
    waiting.recipe["minimum_examples"] = json!(6);
    assert!(
        repo.trigger_after_n(
            &stream,
            18,
            waiting,
            SplitPolicy::Group {
                train_fraction: 0.6,
                validation_fraction: 0.2,
                seed: 42,
            },
            1000
        )
        .unwrap()
        .is_none()
    );
    let job = repo
        .trigger_after_n(
            &stream,
            18,
            request,
            SplitPolicy::Group {
                train_fraction: 0.6,
                validation_fraction: 0.2,
                seed: 42,
            },
            1000,
        )
        .unwrap()
        .unwrap();
    let snapshot = repo.get_snapshot(&job.snapshot_id).unwrap();
    assert!(
        snapshot
            .train
            .iter()
            .chain(&snapshot.validation)
            .all(|sample| sample.payload["annotation"]["is_anomaly"] == false)
    );
    assert!(
        snapshot
            .test
            .iter()
            .any(|sample| sample.payload["annotation"]["is_anomaly"] == true)
    );
    assert!(!snapshot.excluded.is_empty());
}

#[test]
fn instance_readiness_counts_class_ids_inside_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("instance.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "p".into(),
        stream_id: "camera".into(),
        inspection_version: "v1".into(),
    };
    for i in 0..9 {
        repo.record_sample(&stream,&TrainingSample {id:format!("s{i}"),annotation_revision:1,group_id:format!("g{i}"),captured_at_ms:i,label_available_at_ms:i,content_digest:format!("{i:064x}"),source:LabelSource::Reviewed,accepted:true,payload:json!({"annotation":{"kind":"instance_masks","instances":[{"bounds":{"class_id":0}},{"bounds":{"class_id":1}}]}})}).unwrap();
    }
    let request = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"inspection_task":"instance_segmentation","labels":["part","defect"],"minimum_examples":2,"minimum_examples_per_class":2}),
        compute: json!({}),
    };
    assert!(
        repo.trigger_after_n(
            &stream,
            9,
            request,
            SplitPolicy::Group {
                train_fraction: 0.6,
                validation_fraction: 0.2,
                seed: 42
            },
            1000
        )
        .unwrap()
        .is_some()
    );
}

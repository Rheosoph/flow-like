use super::*;
use serde_json::json;
use std::sync::{Arc, Barrier};

fn setup() -> (
    tempfile::TempDir,
    TrainingRepository,
    LearningProject,
    ExperimentBudget,
) {
    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("learning.sqlite")).unwrap();
    let project=repo.create_learning_project(LearningProjectRequest {
        stream:StreamKey {project_id:"app:board".into(),stream_id:"inspection".into(),inspection_version:"v1".into()},
        spec:json!({"task":"sensor_classification","labels":["good","bad"],"input_shape":[1]}),source:json!({"table":"source"}),
        goals:ExperimentGoals {task:EvaluationTask::Classification,primary_metric:"accuracy".into(),direction:MetricDirection::Maximize,minimum_audited_samples:2,bounds:vec![MetricBound {name:"accuracy".into(),minimum:Some(0.5),maximum:None}]},
        policy:LearningPolicy {minimum_new_reviews:2,minimum_audit_samples:2,cooldown_ms:0,review_batch_size:3,review_pool_size:8,drift_window:2,declared_slices:vec!["line".into()],..Default::default()},
        budget:LearningBudget {maximum_cycles:5,maximum_training_time_ms:10_000,maximum_artifact_bytes:1000,..Default::default()},deployment_id:"quality-live".into(),
    },100).unwrap();
    let budget = ExperimentBudget {
        maximum_trials: 1,
        maximum_training_time_ms: 1000,
        maximum_artifact_bytes: 100,
        maximum_llm_tokens: 100,
        maximum_llm_cost_micros: 100,
        worker_limits: WorkerLimits {
            maximum_duration_ms: 1000,
            maximum_artifact_bytes: 100,
            ..Default::default()
        },
        ..Default::default()
    };
    (directory, repo, project, budget)
}
fn sample(index: usize) -> TrainingSample {
    TrainingSample {
        id: format!("sample-{index}"),
        annotation_revision: 1,
        group_id: format!("lot-{index}"),
        captured_at_ms: 10,
        label_available_at_ms: 20,
        content_digest: format!("{index:064x}"),
        source: LabelSource::Reviewed,
        accepted: true,
        payload: json!({"input":{"shape":[1],"values":[index]},"label":"good","annotation":{"kind":"class","class_id":0},"provenance":{"kind":"reviewed","reviewer":"operator","reviewed_at_ms":20}}),
    }
}
fn snapshot(
    repo: &TrainingRepository,
    project: &LearningProject,
    train: &[usize],
    validation: &[usize],
    test: &[usize],
) -> DatasetSnapshot {
    let samples = |indices: &[usize]| {
        indices
            .iter()
            .map(|index| {
                let sample = sample(*index);
                repo.record_sample(&project.request.stream, &sample)
                    .unwrap();
                sample
            })
            .collect()
    };
    repo.import_experiment_snapshot(DatasetSnapshot {
        id: String::new(),
        digest: String::new(),
        stream: project.request.stream.clone(),
        as_of_ms: 100,
        cutoff_sequence: 100,
        policy: SplitPolicy::Group {
            train_fraction: 0.5,
            validation_fraction: 0.25,
            seed: 42,
        },
        train: samples(train),
        validation: samples(validation),
        test: samples(test),
        excluded: vec![],
    })
    .unwrap()
}
fn experiment(
    repo: &TrainingRepository,
    project: &LearningProject,
    cycle: &LearningCycle,
    snapshot: &DatasetSnapshot,
    time: i64,
) -> Experiment {
    repo.create_experiment(
        ExperimentRequest {
            stream: project.request.stream.clone(),
            snapshot_id: snapshot.id.clone(),
            spec: project.request.spec.clone(),
            candidates: vec![TrainingRequest {
                engine: "fixture".into(),
                recipe: json!({"labels":["good","bad"]}),
                compute: json!({}),
            }],
            goals: project.request.goals.clone(),
            budget: cycle.budget.clone(),
            source_table_versions: vec![],
            created_tables: vec![],
            updated_table_versions: vec![],
            preprocessing_manifest: Value::Null,
            context: json!({"learning_cycle_id":cycle.id}),
        },
        time,
    )
    .unwrap()
}
fn finish(
    repo: &TrainingRepository,
    experiment: &Experiment,
    snapshot: &DatasetSnapshot,
    good: bool,
    time: i64,
) -> ModelArtifact {
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "candidate", time)
        .unwrap();
    let lease = repo
        .claim_job(&trial.job_id, "worker", 100, time + 1)
        .unwrap();
    let artifact = repo
        .finish_training(
            &lease,
            b"weights",
            json!({"labels":["good","bad"]}),
            time + 2,
        )
        .unwrap();
    repo.record_experiment_trial_validation(
        &trial.id,
        &BTreeMap::from([("accuracy".into(), 1.), ("audited_samples".into(), 2.)]),
        10,
        time + 3,
    )
    .unwrap();
    repo.select_experiment_winner(&experiment.id, time + 4)
        .unwrap();
    for sample in &snapshot.test {
        prediction(repo, &artifact.id, &sample.id, good, time + 5);
    }
    let report = repo
        .evaluate_classification(&artifact.id, time + 6)
        .unwrap();
    repo.finalize_experiment(&experiment.id, Some(&report.id), "completed", time + 7)
        .unwrap();
    artifact
}
fn prediction(repo: &TrainingRepository, artifact: &str, sample_id: &str, good: bool, time: i64) {
    repo.record_prediction(&PredictionRecord {
        sample_id: sample_id.into(),
        artifact_id: artifact.into(),
        predicted_label: Some(if good { "good" } else { "bad" }.into()),
        teacher_label: None,
        actual_label: Some("good".into()),
        actual_source: Some(LabelSource::Reviewed),
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: time,
        details: Value::Null,
    })
    .unwrap();
}
fn observation(index: usize, label: &str, confidence: f64, embedding: f64) -> LearningObservation {
    LearningObservation {
        sample_id: format!("sample-{index}"),
        group_id: format!("lot-{index}"),
        captured_at_ms: 100,
        student: Some(LearningPrediction {
            artifact_id: None,
            label: Some(label.into()),
            confidence,
            probabilities: vec![],
        }),
        teacher: Some(LearningPrediction {
            artifact_id: None,
            label: Some("good".into()),
            confidence: 0.9,
            probabilities: vec![],
        }),
        embedding: vec![embedding],
        slices: BTreeMap::from([("line".into(), "A".into())]),
        sample: Value::Null,
    }
}
fn review(repo: &TrainingRepository, project: &LearningProject, index: usize, available: i64) {
    repo.record_learning_observation(&project.id, &observation(index, "good", 0.9, 0.))
        .unwrap();
    repo.record_learning_review(
        &project.id,
        &LearningReview {
            sample_id: format!("sample-{index}"),
            annotation: json!({"kind":"class","class_id":0}),
            source: LabelSource::Reviewed,
            reviewer: "operator".into(),
            available_at_ms: available,
            revision: 1,
            outcome: None,
        },
    )
    .unwrap();
}

#[test]
fn review_selection_is_durable_diverse_and_uses_only_reviewed_outcomes() {
    let (directory, repo, project, _) = setup();
    for (i, label, confidence, feature) in [
        (0, "good", 0.99, 0.),
        (1, "bad", 0.9, 0.),
        (2, "good", 0.1, 1.),
        (3, "good", 0.1, 1.),
        (4, "good", 0.1, 20.),
    ] {
        repo.record_learning_observation(&project.id, &observation(i, label, confidence, feature))
            .unwrap();
    }
    let selected = repo
        .select_learning_reviews(&project.id, "batch-1", 101)
        .unwrap();
    assert_eq!(selected.len(), 3);
    assert!(
        selected
            .iter()
            .any(|item| item.observation.sample_id == "sample-1")
    );
    assert!(
        selected
            .iter()
            .any(|item| item.observation.sample_id == "sample-4")
    );
    let reopened = TrainingRepository::open(directory.path().join("learning.sqlite")).unwrap();
    assert_eq!(
        serde_json::to_value(
            reopened
                .select_learning_reviews(&project.id, "batch-1", 102)
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&selected).unwrap()
    );
    let next = reopened
        .select_learning_reviews(&project.id, "batch-2", 102)
        .unwrap();
    assert!(next.iter().all(|item| {
        !selected
            .iter()
            .any(|old| old.observation.sample_id == item.observation.sample_id)
    }));
    let review = LearningReview {
        sample_id: "sample-1".into(),
        annotation: json!({"kind":"class","class_id":0}),
        source: LabelSource::Reviewed,
        reviewer: "operator".into(),
        available_at_ms: 102,
        revision: 1,
        outcome: None,
    };
    assert!(
        reopened
            .record_learning_review(&project.id, &review)
            .unwrap()
    );
    assert!(
        !reopened
            .record_learning_review(&project.id, &review)
            .unwrap()
    );
    let mut teacher = review.clone();
    teacher.source = LabelSource::Teacher;
    assert!(
        reopened
            .record_learning_review(&project.id, &teacher)
            .is_err()
    );
    let analysis = reopened.learning_error_analysis(&project.id).unwrap();
    assert_eq!(analysis.overall.samples, 1);
    assert_eq!(analysis.overall.errors, 1);
    assert_eq!(analysis.slices["line=A"].confusion["good"]["bad"], 1);
    assert_eq!(analysis.excluded_unreviewed, 4);
    assert!(
        reopened
            .record_learning_product_change(&project.id, "change", "new product", 103)
            .unwrap()
    );
    assert!(
        !reopened
            .record_learning_product_change(&project.id, "change", "new product", 104)
            .unwrap()
    );
}

#[test]
fn cycle_reservations_and_crash_recovery_links_are_atomic() {
    let (_directory, repo, project, budget) = setup();
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let repo = repo.clone();
            let project = project.clone();
            let budget = budget.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                repo.reserve_learning_cycle(&project.id, "cycle-1", budget, 101)
                    .unwrap()
            })
        })
        .collect();
    barrier.wait();
    let cycles: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(cycles[0].id, cycles[1].id);
    assert_eq!(
        repo.get_learning_project(&project.id)
            .unwrap()
            .usage
            .cycles_started,
        1
    );
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let experiment = experiment(&repo, &project, &cycles[0], &data, 102);
    assert!(
        repo.submit_experiment_trial(&experiment.id, 0, "early", 103)
            .is_err()
    );
    assert_eq!(
        repo.find_learning_cycle_experiment(&cycles[0].id)
            .unwrap()
            .unwrap()
            .id,
        experiment.id
    );
    assert!(
        repo.create_experiment(experiment.request.clone(), 103)
            .is_err()
    );
    repo.attach_learning_experiment(&cycles[0].id, &experiment.id, 104)
        .unwrap();
    assert_eq!(
        repo.attach_learning_experiment(&cycles[0].id, &experiment.id, 105)
            .unwrap()
            .experiment_id
            .as_deref(),
        Some(experiment.id.as_str())
    );
    assert!(
        repo.abort_learning_cycle(&cycles[0].id, "cannot release active training", 106)
            .is_err()
    );
    let project = repo.get_learning_project(&project.id).unwrap();
    repo.pause_learning_project(&project.id, project.generation, 106)
        .unwrap();
    assert!(
        repo.submit_experiment_trial(&experiment.id, 0, "candidate", 107)
            .is_err()
    );
}

#[test]
fn fresh_audit_cycles_promote_compare_same_cohort_and_rollback() {
    let (directory, repo, project, budget) = setup();
    let first = repo
        .reserve_learning_cycle(&project.id, "first", budget.clone(), 101)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let exp = experiment(&repo, &project, &first, &data, 102);
    repo.attach_learning_experiment(&first.id, &exp.id, 103)
        .unwrap();
    let champion = finish(&repo, &exp, &data, true, 104);
    repo.settle_learning_cycle(&first.id, 112).unwrap();
    let deployment = repo.promote_learning_cycle(&first.id, 0, 113).unwrap();
    assert_eq!(deployment.active_artifact_id, champion.id);
    review(&repo, &project, 8, 114);
    review(&repo, &project, 9, 114);
    assert!(
        repo.learning_next_actions(&project.id, 115)
            .unwrap()
            .ready_to_train
    );
    let second = repo
        .reserve_learning_cycle(&project.id, "second", budget, 115)
        .unwrap();
    let mut contaminated = data.clone();
    contaminated.id.clear();
    contaminated.digest.clear();
    std::mem::swap(&mut contaminated.train[0], &mut contaminated.test[0]);
    let contaminated = repo.import_experiment_snapshot(contaminated).unwrap();
    let bad_exp = experiment(&repo, &project, &second, &contaminated, 116);
    assert!(
        repo.attach_learning_experiment(&second.id, &bad_exp.id, 117)
            .is_err()
    );
    // An invalid reserved experiment remains undispatchable; its cycle can be explicitly aborted.
    repo.abort_learning_cycle(&second.id, "invalid split", 117)
        .unwrap();
    assert!(
        repo.submit_experiment_trial(&bad_exp.id, 0, "invalid", 118)
            .is_err()
    );
    let third = repo
        .reserve_learning_cycle(&project.id, "third", second.budget.clone(), 118)
        .unwrap();
    let data2 = snapshot(&repo, &project, &[0, 1], &[2, 3], &[8, 9]);
    let exp2 = experiment(&repo, &project, &third, &data2, 119);
    repo.attach_learning_experiment(&third.id, &exp2.id, 120)
        .unwrap();
    let challenger = finish(&repo, &exp2, &data2, true, 121);
    repo.settle_learning_cycle(&third.id, 129).unwrap();
    for sample in &data2.test {
        prediction(&repo, &champion.id, &sample.id, false, 130);
    }
    let comparison = repo.compare_learning_cycle(&third.id, 131).unwrap();
    assert!(comparison.eligible);
    assert_eq!(comparison.champion_metrics["accuracy"], 0.);
    assert_eq!(comparison.candidate_metrics["accuracy"], 1.);
    let promoted = repo.promote_learning_cycle(&third.id, 1, 132).unwrap();
    assert_eq!(promoted.active_artifact_id, challenger.id);
    let rolled = repo.rollback_learning_project(&project.id, 2, 133).unwrap();
    assert_eq!(rolled.active_artifact_id, champion.id);
    let reopened = TrainingRepository::open(directory.path().join("learning.sqlite")).unwrap();
    assert!(
        matches!(reopened.learning_route(&project.id,"part-1").unwrap(),LearningRoute::Champion {artifact_id} if artifact_id==champion.id)
    );
    let exclusions = reopened.learning_dataset_exclusions(&project.id).unwrap();
    assert!(
        exclusions
            .sealed_audit_sample_ids
            .contains(&"sample-4".into())
    );
    assert!(
        exclusions
            .sealed_audit_sample_ids
            .contains(&"sample-8".into())
    );
    assert_eq!(
        reopened
            .get_learning_project(&project.id)
            .unwrap()
            .usage
            .spent
            .training_time_ms,
        20
    );
}

#[test]
fn future_and_absent_reviews_are_not_consumed_by_a_finished_cycle() {
    let (_directory, repo, project, budget) = setup();
    review(&repo, &project, 0, 105);
    review(&repo, &project, 50, 500);
    review(&repo, &project, 51, 106);
    let cycle = repo
        .reserve_learning_cycle(&project.id, "first", budget, 110)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let exp = experiment(&repo, &project, &cycle, &data, 111);
    let attached = repo
        .attach_learning_experiment(&cycle.id, &exp.id, 112)
        .unwrap();
    assert_eq!(attached.reviewed_sample_ids, vec!["sample-0"]);
    finish(&repo, &exp, &data, false, 113);
    repo.settle_learning_cycle(&cycle.id, 121).unwrap();
    assert!(
        !repo
            .compare_learning_cycle(&cycle.id, 122)
            .unwrap()
            .eligible
    );
    assert_eq!(
        repo.learning_next_actions(&project.id, 123)
            .unwrap()
            .new_reviews,
        1
    );
    assert_eq!(
        repo.learning_next_actions(&project.id, 501)
            .unwrap()
            .new_reviews,
        2
    );
    repo.record_learning_review(
        &project.id,
        &LearningReview {
            sample_id: "sample-0".into(),
            annotation: json!({"kind":"class","class_id":1}),
            source: LabelSource::Reviewed,
            reviewer: "operator".into(),
            available_at_ms: 600,
            revision: 2,
            outcome: None,
        },
    )
    .unwrap();
    assert_eq!(
        repo.learning_training_reviews_at(&project.id, 550)
            .unwrap()
            .iter()
            .find(|r| r.sample_id == "sample-0")
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        repo.learning_next_actions(&project.id, 550)
            .unwrap()
            .new_reviews,
        2
    );
    assert_eq!(
        repo.learning_training_reviews_at(&project.id, 600)
            .unwrap()
            .iter()
            .find(|r| r.sample_id == "sample-0")
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        repo.learning_next_actions(&project.id, 600)
            .unwrap()
            .new_reviews,
        3
    );
}

#[test]
fn stable_prepared_samples_allow_only_input_transform_changes() {
    let (_directory, repo, project, _) = setup();
    let original = sample(0);
    repo.record_sample(&project.request.stream, &original)
        .unwrap();
    let mut transformed = original.clone();
    transformed.payload["input"] = json!({"shape":[2],"values":[0.,1.]});
    assert!(
        !repo
            .record_prepared_sample(&project.request.stream, &transformed)
            .unwrap()
            .inserted
    );
    transformed.payload["label"] = json!("bad");
    assert!(
        repo.record_prepared_sample(&project.request.stream, &transformed)
            .is_err()
    );
}

#[test]
fn consultation_diagnostics_exclude_audit_groups_and_unexposed_future_holdouts() {
    let (_directory, repo, project, budget) = setup();
    for index in 0..9 {
        let mut item = observation(index, "bad", 0.9, 0.);
        let slice = match index {
            0..=3 | 8 => "training",
            4..=5 | 7 => "holdout",
            _ => "unassigned",
        };
        item.slices.insert("line".into(), slice.into());
        if index == 4 || index == 8 {
            item.group_id = "lot-0".into();
        } else if index == 7 {
            item.group_id = "lot-5".into();
        }
        repo.record_learning_observation(&project.id, &item)
            .unwrap();
        repo.record_learning_review(
            &project.id,
            &LearningReview {
                sample_id: item.sample_id,
                annotation: json!({"kind":"class","class_id":0}),
                source: LabelSource::Reviewed,
                reviewer: "operator".into(),
                available_at_ms: 101,
                revision: 1,
                outcome: None,
            },
        )
        .unwrap();
    }
    assert_eq!(
        repo.learning_consultation_error_analysis(&project.id)
            .unwrap()
            .overall
            .samples,
        0
    );
    let cycle = repo
        .reserve_learning_cycle(&project.id, "diagnostics", budget, 102)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let experiment = experiment(&repo, &project, &cycle, &data, 103);
    repo.attach_learning_experiment(&cycle.id, &experiment.id, 104)
        .unwrap();
    let consultation = repo
        .learning_consultation_error_analysis(&project.id)
        .unwrap();
    assert_eq!(consultation.overall.samples, 5);
    assert_eq!(consultation.overall.errors, 5);
    assert_eq!(consultation.slices.len(), 1);
    assert_eq!(consultation.slices["line=training"].samples, 5);
    assert_eq!(consultation.excluded_unreviewed, 0);
    let operator = repo.learning_error_analysis(&project.id).unwrap();
    assert_eq!(operator.overall.samples, 9);
    assert_eq!(operator.slices["line=holdout"].samples, 3);
    assert_eq!(operator.slices["line=unassigned"].samples, 1);
}

#[test]
fn audit_rows_can_share_the_current_cycle_group_but_cannot_reuse_prior_audit_groups() {
    let (_directory, repo, project, budget) = setup();
    let grouped_snapshot = |indices: [usize; 2]| {
        let mut value = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
        value.id.clear();
        value.digest.clear();
        value.test = indices
            .into_iter()
            .map(|index| {
                let mut row = sample(index);
                row.group_id = "shared-audit-lot".into();
                repo.record_sample(&project.request.stream, &row).unwrap();
                row
            })
            .collect();
        repo.import_experiment_snapshot(value).unwrap()
    };
    let first = repo
        .reserve_learning_cycle(&project.id, "grouped", budget.clone(), 101)
        .unwrap();
    let data = grouped_snapshot([10, 11]);
    let experiment1 = experiment(&repo, &project, &first, &data, 102);
    let attached = repo
        .attach_learning_experiment(&first.id, &experiment1.id, 103)
        .unwrap();
    assert_eq!(attached.audit_sample_ids.len(), 2);
    assert_eq!(attached.audit_group_ids, vec!["shared-audit-lot"]);
    finish(&repo, &experiment1, &data, false, 104);
    repo.settle_learning_cycle(&first.id, 112).unwrap();
    repo.compare_learning_cycle(&first.id, 113).unwrap();
    review(&repo, &project, 8, 114);
    review(&repo, &project, 9, 114);
    let second = repo
        .reserve_learning_cycle(&project.id, "reused-group", budget, 115)
        .unwrap();
    let data2 = grouped_snapshot([12, 13]);
    let experiment2 = experiment(&repo, &project, &second, &data2, 116);
    assert!(
        repo.attach_learning_experiment(&second.id, &experiment2.id, 117)
            .is_err()
    );
    assert_eq!(
        repo.get_learning_cycle(&second.id).unwrap().state,
        LearningCycleState::Reserved
    );
}

#[test]
fn settled_cycles_consume_signal_evidence_but_keep_future_evidence_fresh() {
    let (_directory, repo, project, budget) = setup();
    for index in 0..6 {
        let mut observation = observation(index, "bad", 0.9, (index / 2) as f64 * 10.);
        observation.captured_at_ms = if index < 4 { 100 + index as i64 } else { 200 };
        repo.record_learning_observation(&project.id, &observation)
            .unwrap();
        if index >= 2 {
            repo.record_learning_review(
                &project.id,
                &LearningReview {
                    sample_id: observation.sample_id,
                    annotation: json!({"kind":"class","class_id":0}),
                    source: LabelSource::Reviewed,
                    reviewer: "operator".into(),
                    available_at_ms: if index < 4 { 104 } else { 201 },
                    revision: 1,
                    outcome: None,
                },
            )
            .unwrap();
        }
    }
    let before = repo.learning_next_actions(&project.id, 110).unwrap();
    assert!(before.reasons.contains(&"embedding_drift".into()));
    assert!(before.reasons.contains(&"observed_error".into()));
    let cycle = repo
        .reserve_learning_cycle(&project.id, "signals", budget, 110)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[6, 7]);
    let experiment = experiment(&repo, &project, &cycle, &data, 111);
    repo.attach_learning_experiment(&cycle.id, &experiment.id, 112)
        .unwrap();
    finish(&repo, &experiment, &data, false, 113);
    repo.settle_learning_cycle(&cycle.id, 121).unwrap();
    repo.compare_learning_cycle(&cycle.id, 122).unwrap();
    let after = repo.learning_next_actions(&project.id, 123).unwrap();
    assert!(!after.reasons.contains(&"embedding_drift".into()));
    assert!(!after.reasons.contains(&"observed_error".into()));
    assert!(!after.ready_to_train);
    let future = repo.learning_next_actions(&project.id, 201).unwrap();
    assert!(future.reasons.contains(&"embedding_drift".into()));
    assert!(future.reasons.contains(&"observed_error".into()));
    assert!(future.ready_to_train);
}

#[test]
fn canary_requires_fresh_measured_traffic_and_failed_gate_restores_champion() {
    let (_directory, repo, initial, budget) = setup();
    let mut policy = initial.request.policy.clone();
    policy.canary_fraction = 1.;
    let project = repo
        .update_learning_project(
            &initial.id,
            initial.generation,
            initial.request.source.clone(),
            policy,
            100,
        )
        .unwrap();
    let first = repo
        .reserve_learning_cycle(&project.id, "first", budget.clone(), 101)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let exp = experiment(&repo, &project, &first, &data, 102);
    repo.attach_learning_experiment(&first.id, &exp.id, 103)
        .unwrap();
    let champion = finish(&repo, &exp, &data, true, 104);
    repo.settle_learning_cycle(&first.id, 112).unwrap();
    repo.promote_learning_cycle(&first.id, 0, 113).unwrap();
    review(&repo, &project, 8, 114);
    review(&repo, &project, 9, 114);
    let second = repo
        .reserve_learning_cycle(&project.id, "second", budget, 115)
        .unwrap();
    let data2 = snapshot(&repo, &project, &[0, 1], &[2, 3], &[8, 9]);
    let exp2 = experiment(&repo, &project, &second, &data2, 116);
    repo.attach_learning_experiment(&second.id, &exp2.id, 117)
        .unwrap();
    let candidate = finish(&repo, &exp2, &data2, true, 118);
    repo.settle_learning_cycle(&second.id, 126).unwrap();
    for sample in &data2.test {
        prediction(&repo, &champion.id, &sample.id, false, 127);
    }
    let old_deployment = repo.promote_learning_cycle(&second.id, 1, 128).unwrap();
    assert_eq!(old_deployment.active_artifact_id, champion.id);
    assert_eq!(
        repo.get_learning_project(&project.id).unwrap().state,
        LearningState::Canary
    );
    assert!(
        matches!(repo.learning_route(&project.id,"same").unwrap(),LearningRoute::Canary {artifact_id,..} if artifact_id==candidate.id)
    );
    assert!(!repo.learning_canary_ready(&project.id, 129).unwrap());
    assert!(matches!(
        repo.promote_learning_cycle(&second.id, 1, 129),
        Err(Error::Conflict(_))
    ));
    for index in [100, 101] {
        let mut sample = sample(index);
        sample.captured_at_ms = 130;
        sample.label_available_at_ms = 131;
        repo.record_sample(&project.request.stream, &sample)
            .unwrap();
        let mut observation = observation(index, "bad", 0.9, index as f64);
        observation.captured_at_ms = 130;
        observation.student.as_mut().unwrap().artifact_id = Some(candidate.id.clone());
        repo.record_learning_observation(&project.id, &observation)
            .unwrap();
        repo.record_learning_review(
            &project.id,
            &LearningReview {
                sample_id: sample.id.clone(),
                annotation: json!({"kind":"class","class_id":0}),
                source: LabelSource::Reviewed,
                reviewer: "operator".into(),
                available_at_ms: 131,
                revision: 1,
                outcome: None,
            },
        )
        .unwrap();
        prediction(&repo, &candidate.id, &sample.id, false, 132);
        prediction(&repo, &champion.id, &sample.id, true, 132);
    }
    assert!(repo.learning_canary_ready(&project.id, 133).unwrap());
    assert!(repo.promote_learning_cycle(&second.id, 1, 133).is_err());
    let restored = repo.get_learning_project(&project.id).unwrap();
    assert_eq!(restored.state, LearningState::Active);
    assert_eq!(
        restored.champion_artifact_id.as_deref(),
        Some(champion.id.as_str())
    );
    assert!(restored.candidate_artifact_id.is_none());
    assert!(!repo.get_learning_comparison(&second.id).unwrap().eligible);
    assert_eq!(
        repo.get_learning_cycle(&second.id)
            .unwrap()
            .canary_sample_ids
            .len(),
        2
    );
    assert!(
        repo.learning_dataset_exclusions(&project.id)
            .unwrap()
            .sealed_audit_sample_ids
            .contains(&"sample-100".into())
    );
    assert_eq!(
        repo.learning_next_actions(&project.id, 134)
            .unwrap()
            .new_reviews,
        0
    );
    repo.record_learning_review(
        &project.id,
        &LearningReview {
            sample_id: "sample-100".into(),
            annotation: json!({"kind":"class","class_id":1}),
            source: LabelSource::Reviewed,
            reviewer: "operator".into(),
            available_at_ms: 135,
            revision: 2,
            outcome: None,
        },
    )
    .unwrap();
    assert_eq!(
        repo.learning_next_actions(&project.id, 136)
            .unwrap()
            .new_reviews,
        0
    );
    let mut same_group = observation(102, "bad", 0.9, 0.);
    same_group.group_id = "lot-100".into();
    repo.record_learning_observation(&project.id, &same_group)
        .unwrap();
    repo.record_learning_review(
        &project.id,
        &LearningReview {
            sample_id: same_group.sample_id,
            annotation: json!({"kind":"class","class_id":0}),
            source: LabelSource::Reviewed,
            reviewer: "operator".into(),
            available_at_ms: 137,
            revision: 1,
            outcome: None,
        },
    )
    .unwrap();
    assert_eq!(
        repo.learning_next_actions(&project.id, 138)
            .unwrap()
            .new_reviews,
        0
    );
}

#[test]
fn learning_observation_bytes_and_cross_cycle_budget_are_enforced() {
    let (_directory, repo, project, mut budget) = setup();
    budget.maximum_training_time_ms = project.request.budget.maximum_training_time_ms + 1;
    assert!(
        repo.reserve_learning_cycle(&project.id, "too-much", budget, 101)
            .is_err()
    );
    assert_eq!(
        repo.get_learning_project(&project.id)
            .unwrap()
            .usage
            .cycles_started,
        0
    );
    let mut request = project.request.clone();
    request.stream.stream_id = "bounded".into();
    request.deployment_id = "bounded".into();
    request.budget.maximum_observation_bytes = 1;
    let bounded = repo.create_learning_project(request, 101).unwrap();
    assert!(
        repo.record_learning_observation(&bounded.id, &observation(1, "good", 0.9, 0.))
            .is_err()
    );
    assert!(repo.learning_observations(&bounded.id).unwrap().is_empty());
    assert_eq!(
        repo.get_learning_project(&bounded.id)
            .unwrap()
            .usage
            .observation_bytes,
        0
    );
}

fn canary_review_fixture(
    bind_review: bool,
) -> (
    tempfile::TempDir,
    TrainingRepository,
    LearningProject,
    LearningCycle,
    ModelArtifact,
    ModelArtifact,
) {
    let (directory, repo, initial, budget) = setup();
    let mut policy = initial.request.policy.clone();
    policy.canary_fraction = 1.;
    let project = repo
        .update_learning_project(
            &initial.id,
            initial.generation,
            initial.request.source.clone(),
            policy,
            100,
        )
        .unwrap();
    let first = repo
        .reserve_learning_cycle(&project.id, "first", budget.clone(), 101)
        .unwrap();
    let data = snapshot(&repo, &project, &[0, 1], &[2, 3], &[4, 5]);
    let exp = experiment(&repo, &project, &first, &data, 102);
    repo.attach_learning_experiment(&first.id, &exp.id, 103)
        .unwrap();
    let champion = finish(&repo, &exp, &data, true, 104);
    repo.settle_learning_cycle(&first.id, 112).unwrap();
    repo.promote_learning_cycle(&first.id, 0, 113).unwrap();
    review(&repo, &project, 8, 114);
    review(&repo, &project, 9, 114);
    let second = repo
        .reserve_learning_cycle(&project.id, "second", budget, 115)
        .unwrap();
    let data2 = snapshot(&repo, &project, &[0, 1], &[2, 3], &[8, 9]);
    let exp2 = experiment(&repo, &project, &second, &data2, 116);
    repo.attach_learning_experiment(&second.id, &exp2.id, 117)
        .unwrap();
    let candidate = finish(&repo, &exp2, &data2, true, 118);
    repo.settle_learning_cycle(&second.id, 126).unwrap();
    for sample in &data2.test {
        prediction(&repo, &champion.id, &sample.id, false, 127);
    }
    let old_deployment = repo.promote_learning_cycle(&second.id, 1, 128).unwrap();
    assert_eq!(old_deployment.active_artifact_id, champion.id);
    assert_eq!(
        repo.get_learning_project(&project.id).unwrap().state,
        LearningState::Canary
    );
    assert!(
        matches!(repo.learning_route(&project.id,"same").unwrap(),LearningRoute::Canary {artifact_id,..} if artifact_id==candidate.id)
    );
    assert!(!repo.learning_canary_ready(&project.id, 129).unwrap());
    assert!(matches!(
        repo.promote_learning_cycle(&second.id, 1, 129),
        Err(Error::Conflict(_))
    ));
    for index in [100, 101] {
        let mut sample = sample(index);
        sample.captured_at_ms = 130;
        sample.label_available_at_ms = 131;
        repo.record_sample(&project.request.stream, &sample)
            .unwrap();
        let mut observation = observation(index, "bad", 0.9, index as f64);
        observation.captured_at_ms = 130;
        observation.student.as_mut().unwrap().artifact_id = Some(candidate.id.clone());
        repo.record_learning_observation(&project.id, &observation)
            .unwrap();
        repo.record_learning_review(
            &project.id,
            &LearningReview {
                sample_id: sample.id.clone(),
                annotation: json!({"kind":"class","class_id":0}),
                source: LabelSource::Reviewed,
                reviewer: "operator".into(),
                available_at_ms: 131,
                revision: 1,
                outcome: None,
            },
        )
        .unwrap();
        for (artifact, correct) in [(&candidate.id, true), (&champion.id, false)] {
            repo.record_prediction(&PredictionRecord {
                sample_id: sample.id.clone(),
                artifact_id: artifact.clone(),
                predicted_label: Some(if correct { "good" } else { "bad" }.into()),
                teacher_label: None,
                actual_label: Some("good".into()),
                actual_source: Some(LabelSource::Reviewed),
                latency_ms: 1.,
                failed: false,
                recorded_at_ms: 132,
                details: if bind_review {
                    json!({"learning_review_revision":1})
                } else {
                    Value::Null
                },
            })
            .unwrap();
        }
    }

    (directory, repo, project, second, champion, candidate)
}

fn correct_canary_reviews(
    repo: &TrainingRepository,
    project: &LearningProject,
    available_at_ms: i64,
) {
    for index in [100, 101] {
        repo.record_learning_review(
            &project.id,
            &LearningReview {
                sample_id: format!("sample-{index}"),
                annotation: json!({"kind":"class","class_id":1}),
                source: LabelSource::Reviewed,
                reviewer: "operator".into(),
                available_at_ms,
                revision: 2,
                outcome: None,
            },
        )
        .unwrap();
    }
}

#[test]
fn corrected_canary_reviews_reject_cached_evidence_and_allow_later_cycles() {
    for bind_review in [false, true] {
        let (_directory, repo, project, cycle, champion, candidate) =
            canary_review_fixture(bind_review);
        let original = serde_json::to_value(repo.predictions(&candidate.id).unwrap()).unwrap();
        correct_canary_reviews(&repo, &project, 150);
        assert!(
            repo.reconcile_learning_canary_reviews(&project.id, 134)
                .unwrap()
        );
        assert!(repo.learning_canary_ready(&project.id, 134).unwrap());
        assert!(!repo.learning_canary_ready(&project.id, 150).unwrap());
        if bind_review {
            // Host replay has synchronized the corrected truth before hitting its cache.
            for index in [100, 101] {
                let mut corrected = sample(index);
                corrected.annotation_revision = 3;
                corrected.captured_at_ms = 130;
                corrected.label_available_at_ms = 150;
                corrected.payload["annotation"] = json!({"kind":"class","class_id":1});
                corrected.payload["label"] = json!("bad");
                repo.record_prepared_sample(&project.request.stream, &corrected)
                    .unwrap();
            }
        }
        let error = repo.promote_learning_cycle(&cycle.id, 1, 150).unwrap_err();
        assert!(error.to_string().contains("canary review changed"));
        let restored = repo.get_learning_project(&project.id).unwrap();
        assert_eq!(restored.state, LearningState::Active);
        assert_eq!(
            restored.champion_artifact_id.as_deref(),
            Some(champion.id.as_str())
        );
        assert!(restored.candidate_artifact_id.is_none());
        let comparison = repo.get_learning_comparison(&cycle.id).unwrap();
        assert!(!comparison.eligible);
        assert!(comparison.reasons.contains(&"canary_review_changed".into()));
        assert!(
            repo.get_learning_cycle(&cycle.id)
                .unwrap()
                .error
                .unwrap()
                .contains("champion retained")
        );
        assert_eq!(
            serde_json::to_value(repo.predictions(&candidate.id).unwrap()).unwrap(),
            original
        );
        review(&repo, &project, 200, 151);
        review(&repo, &project, 201, 151);
        assert!(
            repo.learning_next_actions(&project.id, 152)
                .unwrap()
                .ready_to_train
        );
    }
}

#[test]
fn concurrent_review_invalidates_promotion_without_advancing_the_project_clock() {
    let (_directory, repo, project, cycle, _champion, candidate) = canary_review_fixture(true);
    let before = repo.get_learning_project(&project.id).unwrap();
    let cohort = repo
        .learning_canary_sample_ids(&project.id, 140)
        .unwrap()
        .into_iter()
        .collect();
    repo.learning_cohort_report(&candidate.id, EvaluationTask::Classification, &cohort, 140)
        .unwrap();
    correct_canary_reviews(&repo, &project, 150);
    let after = repo.get_learning_project(&project.id).unwrap();
    assert!(after.generation > before.generation);
    assert_eq!(after.updated_at_ms, before.updated_at_ms);
    let mut connection = repo.connection().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let current = super::project(&tx, &project.id).unwrap();
    assert!(matches!(
        cas(&current, before.generation, 140),
        Err(Error::Conflict(_))
    ));
    tx.commit().unwrap();
    // The corrected review remains unavailable at 140, then rejects the old
    // evidence on a retry after its availability time.
    assert!(
        repo.reconcile_learning_canary_reviews(&project.id, 140)
            .unwrap()
    );
    assert!(
        !repo
            .reconcile_learning_canary_reviews(&project.id, 150)
            .unwrap()
    );
    assert_eq!(
        repo.get_learning_project(&project.id).unwrap().state,
        LearningState::Active
    );
    assert!(!repo.get_learning_comparison(&cycle.id).unwrap().eligible);
}

#[test]
fn canary_review_comparison_preserves_exact_class_ids() {
    let accepted = json!({"kind":"class","class_id":16_777_216});
    assert!(review_annotation_matches(Some(&accepted), &accepted).unwrap());
    assert!(
        !review_annotation_matches(
            Some(&accepted),
            &json!({"kind":"class","class_id":16_777_217}),
        )
        .unwrap()
    );
}

#[cfg(any(feature = "native", feature = "burn"))]
#[test]
fn canary_review_comparison_accepts_typed_numeric_serialization() {
    for reviewed in [
        json!({"kind":"scalar","value":10}),
        json!({"kind":"values","values":[0.1,2]}),
        json!({"kind":"boxes","boxes":[{"class_id":0,"x_min":0,"y_min":0.1,"x_max":1,"y_max":1}]}),
        json!({"kind":"instance_masks","instances":[{"instance_id":"part","bounds":{"class_id":0,"x_min":0,"y_min":0.1,"x_max":1,"y_max":1},"width":1,"height":1,"foreground":[true]}]}),
    ] {
        let typed: flow_like_ml_core::Annotation =
            serde_json::from_value(reviewed.clone()).unwrap();
        let accepted = serde_json::to_value(typed).unwrap();
        assert_ne!(accepted, reviewed);
        assert!(review_annotation_matches(Some(&accepted), &reviewed).unwrap());
    }
    assert!(
        !review_annotation_matches(
            Some(&json!({"kind":"scalar","value":0.10000000000000001})),
            &json!({"kind":"scalar","value":0.10000000000000002}),
        )
        .unwrap()
    );
}

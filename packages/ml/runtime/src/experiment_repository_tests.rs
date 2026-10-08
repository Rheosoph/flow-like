use super::*;
use serde_json::json;
use std::sync::{Arc, Barrier};

fn setup() -> (
    tempfile::TempDir,
    TrainingRepository,
    DatasetSnapshot,
    ExperimentRequest,
) {
    let directory = tempfile::tempdir().unwrap();
    let repository = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "app:board".into(),
        stream_id: "inspection".into(),
        inspection_version: "v1".into(),
    };
    for index in 0..20 {
        repository.record_sample(&stream,&TrainingSample {
            id:format!("sample-{index}"),annotation_revision:1,group_id:format!("lot-{index}"),captured_at_ms:index,label_available_at_ms:index,content_digest:format!("{index:064x}"),source:LabelSource::Reviewed,accepted:true,
            payload:json!({"input":{"shape":[1],"values":[index]},"label":"good","annotation":{"kind":"class","class_id":0},"provenance":{"kind":"reviewed","reviewer":"operator"}}),
        }).unwrap();
    }
    let snapshot = repository
        .snapshot(
            &stream,
            SplitPolicy::Group {
                train_fraction: 0.5,
                validation_fraction: 0.25,
                seed: 42,
            },
            100,
        )
        .unwrap();
    let candidate = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"labels":["good"]}),
        compute: json!({}),
    };
    let request = ExperimentRequest {
        stream,
        snapshot_id: snapshot.id.clone(),
        spec: json!({"labels":["good"],"input_shape":[1]}),
        candidates: vec![candidate.clone(), candidate],
        goals: ExperimentGoals {
            task: EvaluationTask::Classification,
            primary_metric: "accuracy".into(),
            direction: MetricDirection::Maximize,
            minimum_audited_samples: 2,
            bounds: vec![MetricBound {
                name: "accuracy".into(),
                minimum: Some(0.9),
                maximum: None,
            }],
        },
        budget: ExperimentBudget {
            maximum_trials: 4,
            maximum_llm_tokens: 100,
            maximum_llm_cost_micros: 100,
            maximum_training_time_ms: 3000,
            maximum_wall_time_ms: 10000,
            maximum_artifact_bytes: 100,
            worker_limits: WorkerLimits {
                maximum_duration_ms: 1000,
                maximum_artifact_bytes: 30,
                ..Default::default()
            },
            ..Default::default()
        },
        source_table_versions: vec![json!({"table":"source","branch":"main","version":7})],
        created_tables: vec![],
        updated_table_versions: vec![],
        preprocessing_manifest: json!({"kind":"identity"}),
        context: Value::Null,
    };
    (directory, repository, snapshot, request)
}

fn trained(repo: &TrainingRepository, trial: &ExperimentTrial, time: i64) -> ModelArtifact {
    let lease = repo.claim_job(&trial.job_id, "worker", 1000, time).unwrap();
    repo.finish_training(&lease, b"weights", json!({"engine":"fixture"}), time + 1)
        .unwrap()
}

fn validate(
    repo: &TrainingRepository,
    trial: &ExperimentTrial,
    accuracy: f64,
    time: i64,
) -> ExperimentTrial {
    repo.record_experiment_trial_validation(
        &trial.id,
        &BTreeMap::from([
            ("accuracy".into(), accuracy),
            ("audited_samples".into(), 5.),
        ]),
        10,
        time,
    )
    .unwrap()
}

#[test]
fn frozen_holdout_is_hidden_from_workers_and_survives_restart() {
    let (directory, repo, snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    let job = repo.get_job(&trial.job_id).unwrap();
    let worker_data = repo.get_snapshot(&job.snapshot_id).unwrap();
    assert_eq!(worker_data.train, snapshot.train);
    assert_eq!(worker_data.validation, snapshot.validation);
    assert!(worker_data.test.is_empty());
    assert_ne!(worker_data.digest, snapshot.digest);
    assert_eq!(job.stream, snapshot.stream);
    assert_eq!(
        job.request.recipe["preprocessing_manifest"],
        json!({"kind":"identity"})
    );
    let artifact = trained(&repo, &trial, 102);
    assert_eq!(artifact.stream, snapshot.stream);
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    assert_eq!(
        repo.get_snapshot(&trial.dataset_snapshot_id).unwrap(),
        snapshot
    );
    validate(&repo, &trial, 1., 104);
    assert_eq!(
        repo.experiment_result(&experiment.id)
            .unwrap()
            .best_artifact
            .unwrap()
            .id,
        artifact.id
    );
}

#[test]
fn experiment_events_are_bounded_append_only_and_survive_finalization() {
    let (directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    assert!(
        repo.record_experiment_event(&experiment.id, "decision", json!({}), 99)
            .is_err()
    );
    assert!(
        repo.record_experiment_event(
            &experiment.id,
            "decision",
            json!("x".repeat(64 * 1024)),
            100
        )
        .is_err()
    );
    assert!(
        repo.shadow_telemetry()
            .record_experiment_event(&experiment.id, "decision", json!({}), 100)
            .is_err()
    );
    for index in 0..255 {
        repo.record_experiment_event(&experiment.id, "decision", json!({"index":index}), 100)
            .unwrap();
    }
    let finished = repo
        .finalize_experiment(&experiment.id, None, "no successful candidate", 101)
        .unwrap();
    let before = serde_json::to_value(&finished).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = vec![];
    for index in 0..2 {
        let repo = repo.clone();
        let barrier = barrier.clone();
        let id = experiment.id.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            repo.record_experiment_event(&id, "completion", json!({"index":index}), 102)
        }));
    }
    barrier.wait();
    assert_eq!(
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    let reopened = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let events = reopened.experiment_events(&experiment.id).unwrap();
    assert_eq!(events.len(), 256);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence"], index);
    }
    assert_eq!(events[255]["kind"], "completion");
    let result = reopened.experiment_result(&experiment.id).unwrap();
    assert_eq!(result.events, events);
    assert_eq!(serde_json::to_value(result.experiment).unwrap(), before);
}

#[test]
fn selection_prefers_feasible_models_then_stable_candidate_order() {
    let (_directory, repo, _snapshot, mut request) = setup();
    request.candidates.push(request.candidates[0].clone());
    request.budget.maximum_parallel_trials = 3;
    request.goals.bounds.push(MetricBound {
        name: "mean_latency_ms".into(),
        minimum: None,
        maximum: Some(10.),
    });
    let experiment = repo.create_experiment(request, 100).unwrap();
    let mut trials = vec![];
    for index in 0..3 {
        let trial = repo
            .submit_experiment_trial(&experiment.id, index, &format!("candidate-{index}"), 101)
            .unwrap();
        trained(&repo, &trial, 102);
        trials.push(trial);
    }
    for (index, accuracy, latency) in [(2, 1., 20.), (1, 0.9, 2.), (0, 0.9, 2.)] {
        repo.record_experiment_trial_validation(
            &trials[index].id,
            &BTreeMap::from([
                ("accuracy".into(), accuracy),
                ("audited_samples".into(), 5.),
                ("mean_latency_ms".into(), latency),
            ]),
            10,
            104,
        )
        .unwrap();
        assert_eq!(
            repo.get_experiment(&experiment.id)
                .unwrap()
                .best_trial_id
                .as_deref(),
            Some(trials[index].id.as_str())
        );
    }
}

#[test]
fn consultation_call_reservations_are_atomic_and_survive_restart() {
    let (directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let repo = repo.clone();
            let id = experiment.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                repo.reserve_experiment_consultation(&id, 10, 3, 1, 101)
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        handles
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    let reopened = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    assert!(
        reopened
            .reserve_experiment_consultation(&experiment.id, 10, 3, 1, 102)
            .is_err()
    );
    let events = reopened.experiment_events(&experiment.id).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "consultation_reserved");
    let usage = reopened.get_experiment(&experiment.id).unwrap().usage;
    assert_eq!(usage.llm_tokens, 10);
    assert_eq!(usage.llm_cost_micros, 3);
    assert!(
        reopened
            .reserve_experiment_consultation(&experiment.id, 100, 1, 2, 102)
            .is_err()
    );
    assert_eq!(reopened.experiment_events(&experiment.id).unwrap().len(), 1);
}

#[test]
fn request_metadata_budget_rejects_oversized_candidates_without_partial_append() {
    let (_directory, repo, _snapshot, mut request) = setup();
    request.budget.maximum_dataset_bytes = 100_000;
    let candidate = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"large":"x".repeat(100_000)}),
        compute: json!({}),
    };
    let mut oversized = request.clone();
    oversized.candidates = vec![candidate.clone()];
    assert!(repo.create_experiment(oversized, 100).is_err());
    assert!(repo.list_experiments(&request.stream).unwrap().is_empty());
    let experiment = repo.create_experiment(request, 100).unwrap();
    assert!(
        repo.append_experiment_candidates(&experiment.id, vec![candidate], 101)
            .is_err()
    );
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .request
            .candidates
            .len(),
        2
    );
}

#[test]
fn validation_keeps_its_trial_slot_until_trusted_results_settle() {
    let (_directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let first = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    trained(&repo, &first, 102);
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "second", 104)
            .is_err()
    );
    assert!(
        repo.stop_experiment_budget(&experiment.id, "budget", 104)
            .is_err()
    );
    assert!(
        repo.finalize_experiment(&experiment.id, None, "completed", 104)
            .is_err()
    );
    validate(&repo, &first, 0.95, 105);
    let second = repo
        .submit_experiment_trial(&experiment.id, 1, "second", 106)
        .unwrap();
    trained(&repo, &second, 107);
    assert!(repo.select_experiment_winner(&experiment.id, 109).is_err());
    validate(&repo, &second, 0.9, 110);
    assert_eq!(
        repo.select_experiment_winner(&experiment.id, 111)
            .unwrap()
            .selected_trial_id,
        Some(first.id)
    );
}

#[test]
fn cancelled_final_audit_resumes_only_its_frozen_winner_and_evidence() {
    let (_directory, repo, snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    let artifact = trained(&repo, &trial, 102);
    validate(&repo, &trial, 1., 104);
    let selected = repo.select_experiment_winner(&experiment.id, 105).unwrap();
    let sample = &snapshot.test[0];
    repo.record_prediction(&PredictionRecord {
        sample_id: sample.id.clone(),
        artifact_id: artifact.id.clone(),
        predicted_label: Some("good".into()),
        teacher_label: None,
        actual_label: Some("good".into()),
        actual_source: Some(LabelSource::Reviewed),
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: 106,
        details: Value::Null,
    })
    .unwrap();
    repo.cancel_experiment(&experiment.id, 107).unwrap();
    assert!(matches!(
        repo.finalize_experiment(&experiment.id, None, "completed", 108),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        repo.select_experiment_winner(&experiment.id, 108),
        Err(Error::Cancelled)
    ));
    assert!(repo.resume_experiment(&experiment.id, 108).is_err());
    let resumed = repo
        .resume_frozen_experiment_audit(&experiment.id, 108)
        .unwrap();
    assert_eq!(resumed.status, ExperimentStatus::Evaluating);
    assert_eq!(resumed.selected_trial_id, selected.selected_trial_id);
    assert_eq!(resumed.selected_at_ms, selected.selected_at_ms);
    assert_eq!(repo.predictions(&artifact.id).unwrap().len(), 1);
    assert!(
        repo.append_experiment_candidates(&experiment.id, vec![], 109)
            .is_err()
    );
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "another", 109)
            .is_err()
    );
}

#[test]
fn interrupted_attempts_consume_training_budget_before_requeue() {
    let (_directory, repo, _snapshot, mut request) = setup();
    request.budget.maximum_training_time_ms = 2000;
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "candidate", 101)
        .unwrap();
    let resource = ResourceRequest {
        key: "cpu".into(),
        estimated_bytes: 10,
        budget_bytes: 100,
    };
    let first = repo
        .claim_job_with_resources(&trial.job_id, "crashed", 10, 102, Some(resource.clone()))
        .unwrap();
    assert_eq!(
        repo.reconcile_stopped_job(&trial.job_id, 111)
            .unwrap()
            .status,
        JobStatus::Running
    );
    assert_eq!(
        repo.reconcile_stopped_job(&trial.job_id, 113)
            .unwrap()
            .status,
        JobStatus::Interrupted
    );
    assert!(
        repo.finish_training(&first, b"stale", json!({}), 113)
            .is_err()
    );
    assert_eq!(
        repo.resume_stopped_job(&trial.job_id, 114).unwrap().status,
        JobStatus::Queued
    );
    let usage = repo.get_experiment(&experiment.id).unwrap().usage;
    assert_eq!(usage.training_time_ms, 1000);
    assert_eq!(usage.reserved_training_time_ms, 1000);
    repo.claim_job_with_resources(&trial.job_id, "crashed-again", 10, 115, Some(resource))
        .unwrap();
    repo.reconcile_stopped_job(&trial.job_id, 126).unwrap();
    assert!(
        matches!(repo.resume_stopped_job(&trial.job_id,127),Err(Error::Invalid(reason)) if reason.contains("budget"))
    );
    let experiment = repo.get_experiment(&experiment.id).unwrap();
    assert_eq!(experiment.status, ExperimentStatus::BudgetExhausted);
    assert_eq!(experiment.usage.training_time_ms, 2000);
    assert_eq!(experiment.usage.reserved_training_time_ms, 0);
    assert_eq!(experiment.usage.reserved_artifact_bytes, 0);
    assert_eq!(
        repo.get_job(&trial.job_id).unwrap().status,
        JobStatus::Failed
    );
    let trial = repo.get_experiment_trial(&trial.id).unwrap();
    assert_eq!(trial.status, ExperimentTrialStatus::Failed);
    assert_eq!(trial.training_time_ms, 2000);
    assert!(repo.resume_job(&trial.job_id, 128).is_err());
    let resources: usize = repo
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM resource_reservations WHERE job_id=?",
            [&trial.job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(resources, 0);
}

#[test]
fn cancellation_and_expired_reconciliation_cannot_requeue_a_cancelled_worker() {
    let (_directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "candidate", 101)
        .unwrap();
    repo.claim_job(&trial.job_id, "stopped", 10, 102).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = vec![];
    for cancel in [true, false] {
        let repo = repo.clone();
        let job_id = trial.job_id.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            if cancel {
                repo.request_cancel(&job_id, 113)
            } else {
                repo.reconcile_stopped_job(&job_id, 113)
            }
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    assert_eq!(
        repo.get_job(&trial.job_id).unwrap().status,
        JobStatus::Cancelled
    );
    assert!(repo.resume_stopped_job(&trial.job_id, 114).is_err());
}

#[test]
fn duplicate_dispatch_and_llm_admission_are_atomic_across_connections() {
    let (_directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = vec![];
    for _ in 0..2 {
        let repo = repo.clone();
        let experiment = experiment.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            repo.submit_experiment_trial(&experiment.id, 0, "same", 101)
                .unwrap()
        }));
    }
    barrier.wait();
    let first = threads.remove(0).join().unwrap();
    let second = threads.remove(0).join().unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .usage
            .submitted_trials,
        1
    );
    assert!(
        repo.submit_experiment_trial(&experiment.id, 0, "different", 102)
            .is_err()
    );
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "parallel", 102)
            .is_err()
    );
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = vec![];
    for _ in 0..2 {
        let repo = repo.clone();
        let experiment = experiment.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            repo.consume_experiment_llm_budget(&experiment.id, 60, 60, 103)
        }));
    }
    barrier.wait();
    assert_eq!(
        threads
            .into_iter()
            .filter(|thread| thread.thread().id() != std::thread::current().id())
            .map(|thread| thread.join().unwrap().is_ok() as usize)
            .sum::<usize>(),
        1
    );
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .usage
            .llm_tokens,
        60
    );
}

#[test]
fn derived_features_preserve_split_targets_and_provenance() {
    let (_directory, repo, snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let mut derived = snapshot.clone();
    derived.id.clear();
    derived.digest.clear();
    for sample in derived
        .train
        .iter_mut()
        .chain(&mut derived.validation)
        .chain(&mut derived.test)
    {
        sample.payload["input"] = json!({"shape":[2],"values":[1.,2.]});
    }
    let dataset = repo
        .register_experiment_dataset(
            &experiment.id,
            derived.clone(),
            json!({"features":["x","x_squared"]}),
            101,
        )
        .unwrap();
    assert_ne!(dataset.digest, snapshot.digest);
    let mut moved = derived.clone();
    std::mem::swap(&mut moved.train[0], &mut moved.test[0]);
    assert!(
        repo.register_experiment_dataset(&experiment.id, moved, Value::Null, 102)
            .is_err()
    );
    let mut relabelled = derived.clone();
    relabelled.test[0].payload["label"] = json!("bad");
    assert!(
        repo.register_experiment_dataset(&experiment.id, relabelled, Value::Null, 102)
            .is_err()
    );
    let mut forged = derived;
    forged.test[0].source = LabelSource::Teacher;
    assert!(
        repo.register_experiment_dataset(&experiment.id, forged, Value::Null, 102)
            .is_err()
    );
    let candidate = TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"dataset_snapshot_id":dataset.snapshot_id}),
        compute: json!({}),
    };
    repo.append_experiment_candidates(&experiment.id, vec![candidate], 102)
        .unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 2, "features", 103)
        .unwrap();
    let job = repo.get_job(&trial.job_id).unwrap();
    assert_eq!(
        job.request.recipe["inspection_spec"]["input_shape"],
        json!([2])
    );
}

#[test]
fn cancellation_prevents_new_work_and_resume_preserves_artifact() {
    let (_directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    let artifact = trained(&repo, &trial, 102);
    assert_eq!(
        repo.cancel_experiment(&experiment.id, 104).unwrap().status,
        ExperimentStatus::Cancelled
    );
    repo.fail_experiment_trial(&trial.id, "cancelled during validation", 10, 105)
        .unwrap();
    assert_eq!(
        repo.get_experiment_trial(&trial.id).unwrap().status,
        ExperimentTrialStatus::Cancelled
    );
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "next", 105)
            .is_err()
    );
    repo.resume_experiment(&experiment.id, 106).unwrap();
    assert_eq!(
        repo.get_job(&trial.job_id).unwrap().status,
        JobStatus::Succeeded
    );
    let trial = validate(&repo, &trial, 1., 107);
    assert_eq!(trial.artifact_id.as_deref(), Some(artifact.id.as_str()));
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .usage
            .artifact_bytes,
        7
    );
}

#[test]
fn artifact_and_training_budget_are_reserved_before_dispatch() {
    let (_directory, repo, _snapshot, mut request) = setup();
    request.budget.maximum_parallel_trials = 2;
    request.budget.maximum_artifact_bytes = 30;
    let experiment = repo.create_experiment(request, 100).unwrap();
    let first = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "second", 102)
            .is_err()
    );
    let lease = repo.claim_job(&first.job_id, "worker", 1000, 102).unwrap();
    assert!(
        repo.finish_training(&lease, &[0; 31], json!({}), 103)
            .is_err()
    );
    repo.finish_training(&lease, b"weights", json!({}), 103)
        .unwrap();
    validate(&repo, &first, 1., 104);
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "second", 105)
            .is_err()
    );
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .usage
            .submitted_trials,
        1
    );
    assert!(
        repo.consume_experiment_llm_budget(&experiment.id, 1, 1, 20000)
            .is_err()
    );
}

#[test]
fn winner_is_frozen_before_final_evidence_and_goal_failure_still_returns_best() {
    let (_directory, repo, snapshot, request) = setup();
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "first", 101)
        .unwrap();
    let artifact = trained(&repo, &trial, 102);
    validate(&repo, &trial, 0.8, 104);
    repo.select_experiment_winner(&experiment.id, 105).unwrap();
    assert!(
        repo.append_experiment_candidates(&experiment.id, vec![], 106)
            .is_err()
    );
    assert!(
        repo.submit_experiment_trial(&experiment.id, 1, "too_late", 106)
            .is_err()
    );
    for sample in &snapshot.test {
        repo.record_prediction(&PredictionRecord {
            sample_id: sample.id.clone(),
            artifact_id: artifact.id.clone(),
            predicted_label: Some("bad".into()),
            teacher_label: None,
            actual_label: Some("good".into()),
            actual_source: Some(LabelSource::Reviewed),
            latency_ms: 1.,
            failed: false,
            recorded_at_ms: 106,
            details: Value::Null,
        })
        .unwrap();
    }
    let report = repo.evaluate_classification(&artifact.id, 107).unwrap();
    let finished = repo
        .finalize_experiment(&experiment.id, Some(&report.id), "completed", 108)
        .unwrap();
    assert_eq!(finished.status, ExperimentStatus::Completed);
    assert!(!finished.target_met);
    assert!(!finished.unmet_constraints.is_empty());
    let result = repo.experiment_result(&experiment.id).unwrap();
    assert_eq!(result.best_artifact.unwrap().id, artifact.id);
    assert_eq!(result.final_evaluation.unwrap().id, report.id);
    assert!(repo.resume_experiment(&experiment.id, 109).is_err());
}

#[test]
fn prepared_import_rejects_forged_labels_and_temporal_leakage() {
    let (_directory, repo, mut snapshot, _request) = setup();
    snapshot.id.clear();
    snapshot.digest = "caller_digest_is_ignored".into();
    let imported = repo.import_experiment_snapshot(snapshot.clone()).unwrap();
    assert_eq!(imported.digest.len(), 64);
    snapshot.test[0].payload["label"] = json!("forged");
    assert!(repo.import_experiment_snapshot(snapshot.clone()).is_err());
    snapshot.test[0].payload["label"] = json!("good");
    snapshot.policy = SplitPolicy::Time {
        train_end_ms: 5,
        validation_end_ms: 10,
        embargo_ms: 1,
    };
    assert!(repo.import_experiment_snapshot(snapshot).is_err());
}

#[test]
fn shadow_cannot_mutate_experiment_state() {
    let (_directory, repo, _snapshot, request) = setup();
    let experiment = repo.create_experiment(request.clone(), 100).unwrap();
    let shadow = repo.shadow_telemetry();
    assert!(matches!(
        shadow.create_experiment(request, 100),
        Err(Error::ShadowWrite)
    ));
    assert!(matches!(
        shadow.submit_experiment_trial(&experiment.id, 0, "first", 101),
        Err(Error::ShadowWrite)
    ));
    assert!(matches!(
        shadow.consume_experiment_llm_budget(&experiment.id, 1, 1, 101),
        Err(Error::ShadowWrite)
    ));
    assert!(shadow.get_experiment(&experiment.id).is_ok());
}

#[test]
fn successive_halving_continuation_is_atomic_and_preserves_optimizer_checkpoint() {
    let (_directory, repo, _snapshot, mut request) = setup();
    request.candidates[0] = TrainingRequest {
        engine: "burn".into(),
        recipe: json!({"config":{"epochs":2},"search":{"family":"mlp-small","rung":0,"max_epochs":9,"reduction_factor":3}}),
        compute: json!({}),
    };
    let experiment = repo.create_experiment(request, 100).unwrap();
    let parent = repo
        .submit_experiment_trial(&experiment.id, 0, "initial", 101)
        .unwrap();
    let lease = repo.claim_job(&parent.job_id, "worker", 1000, 102).unwrap();
    let checkpoint = repo
        .publish_checkpoint(
            &lease,
            b"optimizer-state",
            4,
            json!({"epoch":2}),
            false,
            103,
        )
        .unwrap();
    repo.finish_training(&lease, b"weights", json!({}), 104)
        .unwrap();
    validate(&repo, &parent, 0.95, 105);
    assert!(repo.continue_experiment_trial(&parent.id, 2, 106).is_err());
    assert!(repo.continue_experiment_trial(&parent.id, 10, 106).is_err());
    let child = repo.continue_experiment_trial(&parent.id, 6, 106).unwrap();
    assert_eq!(
        repo.continue_experiment_trial(&parent.id, 6, 107)
            .unwrap()
            .id,
        child.id
    );
    assert_eq!(child.dataset_snapshot_id, parent.dataset_snapshot_id);
    assert_eq!(child.snapshot_id, parent.snapshot_id);
    let job = repo.get_job(&child.job_id).unwrap();
    assert_eq!(job.request.recipe["config"]["epochs"], json!(6));
    assert_eq!(job.request.recipe["search"]["family"], json!("mlp-small"));
    assert_eq!(job.request.recipe["search"]["rung"], json!(1));
    let copied = job.checkpoint.unwrap();
    assert_eq!(copied.blob, checkpoint.blob);
    assert_eq!(
        copied.request_digest,
        digest(&serde_json::to_vec(&job.request).unwrap())
    );
    assert_ne!(copied.request_digest, checkpoint.request_digest);
    assert_eq!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .usage
            .submitted_trials,
        2
    );
}

#[test]
fn anomaly_training_filters_defects_but_search_retains_validation_classes() {
    let (_directory, repo, mut snapshot, mut request) = setup();
    for partition in [
        &mut snapshot.train,
        &mut snapshot.validation,
        &mut snapshot.test,
    ] {
        for (index, sample) in partition.iter_mut().enumerate() {
            sample.annotation_revision = 2;
            sample.payload["annotation"] = json!({"kind":"anomaly","is_anomaly":index%2==1});
            repo.record_sample(&snapshot.stream, sample).unwrap();
        }
    }
    snapshot.id.clear();
    snapshot.digest.clear();
    let snapshot = repo.import_experiment_snapshot(snapshot).unwrap();
    request.snapshot_id = snapshot.id.clone();
    request.spec["task"] = json!("sensor_anomaly");
    let experiment = repo.create_experiment(request, 100).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "anomaly", 101)
        .unwrap();
    let training = repo.get_snapshot(&trial.snapshot_id).unwrap();
    assert!(
        training
            .train
            .iter()
            .chain(&training.validation)
            .all(|sample| sample.payload["annotation"]["is_anomaly"] == json!(false))
    );
    let search = repo
        .get_experiment_search_snapshot(&trial.id, 1_000_000)
        .unwrap();
    assert_eq!(search.validation, snapshot.validation);
    assert!(search.test.is_empty());
    assert!(repo.get_experiment_search_snapshot(&trial.id, 1).is_err());
}

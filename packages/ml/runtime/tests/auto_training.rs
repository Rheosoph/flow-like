#![cfg(all(feature = "native", feature = "burn"))]
use flow_like_ml_core::{
    Annotation, ComputeBackend, ComputeConfig, InspectionSpec, LabelProvenance, Sample, TaskKind,
    TensorData,
};
use flow_like_ml_runtime::{auto_training::*, engines::*, worker::TrainingWorker, *};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn setup(repo: &TrainingRepository) -> (ExperimentRequest, InspectionSpec) {
    setup_task(repo, false)
}

fn setup_task(repo: &TrainingRepository, anomaly: bool) -> (ExperimentRequest, InspectionSpec) {
    let stream = StreamKey {
        project_id: "automl".into(),
        stream_id: "quality".into(),
        inspection_version: "v1".into(),
    };
    let spec = InspectionSpec {
        id: "v1".into(),
        description: "detect negative and positive samples".into(),
        task: if anomaly {
            TaskKind::SensorAnomaly
        } else {
            TaskKind::SensorClassification
        },
        labels: vec!["negative".into(), "positive".into()],
        input_shape: vec![1],
        prediction_horizon_ms: None,
        minimum_examples: 1,
        minimum_examples_per_class: 1,
    };
    let mut partitions: [Vec<TrainingSample>; 3] = Default::default();
    for (partition, rows) in partitions.iter_mut().enumerate() {
        for index in 0..16 {
            let sign = if index % 2 == 0 { -1.0 } else { 1.0 };
            // Final labels intentionally differ, exposing accidental reuse of validation metrics.
            let class = if partition == 2 {
                1 - index % 2
            } else {
                index % 2
            };
            let sample = Sample {
                id: format!("{partition}-{index}"),
                group_id: format!("group-{partition}-{index}"),
                stream_id: stream.stream_id.clone(),
                timestamp_ms: 100,
                window_start_ms: 100,
                window_end_ms: 100,
                input: TensorData {
                    shape: vec![1],
                    values: vec![sign * (1.0 + index as f32 * 0.01)],
                },
                annotation: if anomaly {
                    Annotation::Anomaly {
                        is_anomaly: class == 1,
                    }
                } else {
                    Annotation::Class {
                        class_id: class as u32,
                    }
                },
                provenance: LabelProvenance::Reviewed {
                    reviewer: "fixture".into(),
                    reviewed_at_ms: 100,
                },
                outcome: None,
            };
            let row = TrainingSample {
                id: sample.id.clone(),
                annotation_revision: 1,
                group_id: sample.group_id.clone(),
                captured_at_ms: 100,
                label_available_at_ms: 100,
                content_digest: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&sample).unwrap())
                ),
                source: LabelSource::Reviewed,
                accepted: true,
                payload: json!({"sample":sample,"label":spec.labels[class]}),
            };
            repo.record_sample(&stream, &row).unwrap();
            rows.push(row);
        }
    }
    let [train, validation, test] = partitions;
    let snapshot = repo
        .import_experiment_snapshot(DatasetSnapshot {
            id: "".into(),
            digest: "".into(),
            stream: stream.clone(),
            as_of_ms: 200,
            cutoff_sequence: 48,
            policy: SplitPolicy::Group {
                train_fraction: 1.0 / 3.0,
                validation_fraction: 1.0 / 3.0,
                seed: 7,
            },
            train,
            validation,
            test,
            excluded: vec![],
        })
        .unwrap();
    let mut budget = ExperimentBudget::default();
    budget.maximum_trials = 4;
    budget.worker_limits.maximum_duration_ms = 10_000;
    budget.maximum_training_time_ms = 40_000;
    let compute = ComputeConfig {
        backend: ComputeBackend::Cpu,
        ..Default::default()
    };
    let candidates = vec![TrainingRequest {
        engine: "histogram_gradient_boosting".into(),
        recipe: json!({"labels":spec.labels,"inspection_task":spec.task,"config":{"objective":"binary_log_loss","estimators":30,"max_depth":2,"max_bins":16,"min_leaf":1,"learning_rate":0.2,"l2":0.1}}),
        compute: serde_json::to_value(compute).unwrap(),
    }];
    let mut goals = default_goals(&spec);
    goals.minimum_audited_samples = 8;
    goals.bounds = vec![MetricBound {
        name: "accuracy".into(),
        minimum: Some(0.9),
        maximum: None,
    }];
    (
        ExperimentRequest {
            stream,
            snapshot_id: snapshot.id,
            spec: serde_json::to_value(&spec).unwrap(),
            candidates,
            goals,
            budget,
            source_table_versions: vec![json!({"id":"raw-v1"})],
            created_tables: vec![],
            updated_table_versions: vec![],
            preprocessing_manifest: json!({"fit_partition":"train","features":["signal"]}),
            context: json!({}),
        },
        spec,
    )
}

#[test]
fn anomaly_search_trains_only_normal_samples_but_scores_both_validation_classes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (mut request, _) = setup_task(&repo, true);
    request.candidates = vec![TrainingRequest {
        engine: "isolation_forest".into(),
        recipe: json!({"labels":["negative","positive"],"inspection_task":"sensor_anomaly","config":{"trees":32,"sample_size":8,"seed":7}}),
        compute: serde_json::to_value(ComputeConfig {
            backend: ComputeBackend::Cpu,
            ..Default::default()
        })
        .unwrap(),
    }];
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    controller.run_next(&experiment.id).unwrap();
    let trial = repo
        .list_experiment_trials(&experiment.id)
        .unwrap()
        .remove(0);
    assert_eq!(
        trial.status,
        ExperimentTrialStatus::Validated,
        "{:?}",
        trial.error
    );
    assert_eq!(trial.validation_metrics["audited_samples"], 16.0);
    assert!(trial.validation_metrics.contains_key("auroc"));
    let training = repo.get_snapshot(&trial.snapshot_id).unwrap();
    assert_eq!(training.train.len(), 8);
    assert_eq!(training.validation.len(), 8);
    assert!(training.test.is_empty());
    let result = controller.finalize(&experiment.id).unwrap();
    assert_eq!(result.status, ExperimentStatus::Completed);
    let evaluation = repo
        .get_evaluation(result.final_evaluation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(evaluation.audited_samples, 16);
    assert!(evaluation.metrics.contains_key("auroc"));
}

fn worker(repo: &TrainingRepository, limits: &WorkerLimits) -> TrainingWorker {
    let mut worker = TrainingWorker::new(repo.clone(), limits.clone()).unwrap();
    register_native_engines(&mut worker).unwrap();
    register_burn_engine(&mut worker).unwrap();
    worker
}

#[test]
fn search_uses_validation_then_freezes_one_winner_for_real_final_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    let running = controller.run_next(&experiment.id).unwrap();
    assert!(running.selected_trial_id.is_none());
    assert!(running.final_evaluation_id.is_none());
    let trials = repo.list_experiment_trials(&experiment.id).unwrap();
    assert_eq!(trials.len(), 1);
    assert_eq!(trials[0].validation_metrics["accuracy"], 1.0);
    let artifact = repo
        .get_artifact(trials[0].artifact_id.as_ref().unwrap())
        .unwrap();
    assert!(
        repo.get_snapshot(&artifact.snapshot_id)
            .unwrap()
            .test
            .is_empty()
    );
    assert!(repo.predictions(&artifact.id).unwrap().is_empty());
    assert_eq!(
        artifact.manifest["preprocessing_manifest"]["fit_partition"],
        "train"
    );
    let completed = controller.finalize(&experiment.id).unwrap();
    assert_eq!(completed.status, ExperimentStatus::Completed);
    assert!(!completed.target_met);
    let evaluation = repo
        .get_evaluation(completed.final_evaluation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(evaluation.metrics["accuracy"], 0.0);
    assert_eq!(evaluation.audited_samples, 16);
    assert_eq!(repo.predictions(&artifact.id).unwrap().len(), 16);
    assert!(
        repo.append_experiment_candidates(
            &experiment.id,
            vec![experiment.request.candidates[0].clone()],
            now_ms()
        )
        .is_err()
    );
    assert_eq!(
        controller
            .finalize(&experiment.id)
            .unwrap()
            .final_evaluation_id,
        completed.final_evaluation_id
    );
}

#[test]
fn completed_trial_recovers_without_retraining_and_cancelled_queue_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "candidate-0", now_ms())
        .unwrap();
    repo.cancel_experiment(&experiment.id, now_ms()).unwrap();
    assert_eq!(
        repo.get_job(&trial.job_id).unwrap().status,
        JobStatus::Cancelled
    );
    repo.resume_experiment(&experiment.id, now_ms()).unwrap();
    let artifact = worker(&repo, &limits).run(&trial.job_id).unwrap();
    let generation = repo.get_job(&trial.job_id).unwrap().generation;
    let reopened = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let controller = AutoTrainingController::new(reopened.clone(), worker(&reopened, &limits));
    controller.run_next(&experiment.id).unwrap();
    let validated = reopened.get_experiment_trial(&trial.id).unwrap();
    assert_eq!(validated.artifact_id.as_deref(), Some(artifact.id.as_str()));
    assert_eq!(
        reopened.get_job(&trial.job_id).unwrap().generation,
        generation
    );
    assert_eq!(validated.status, ExperimentTrialStatus::Validated);
}

struct InterceptFactory {
    experiment_id: String,
    limits: WorkerLimits,
    cancel: bool,
    triggered: Arc<AtomicBool>,
}
struct InterceptPredictor {
    inner: Box<dyn SearchPredictor>,
    repository: TrainingRepository,
    experiment_id: String,
    cancel: bool,
    triggered: Arc<AtomicBool>,
}
impl SearchPredictor for InterceptPredictor {
    fn predict(&mut self, input: &TensorData) -> Result<serde_json::Value> {
        if self.cancel && !self.triggered.swap(true, Ordering::SeqCst) {
            self.repository
                .cancel_experiment(&self.experiment_id, now_ms())?;
        }
        self.inner.predict(input)
    }
}
impl SearchPredictorFactory for InterceptFactory {
    fn load(
        &self,
        repository: &TrainingRepository,
        artifact_id: &str,
        compute: &ComputeConfig,
    ) -> Result<Box<dyn SearchPredictor>> {
        if !self.cancel {
            let concurrent =
                AutoTrainingController::new(repository.clone(), worker(repository, &self.limits));
            let before = repository.get_experiment(&self.experiment_id)?;
            assert_eq!(
                concurrent.run_next(&self.experiment_id)?.generation,
                before.generation
            );
            assert_eq!(
                concurrent.finalize(&self.experiment_id)?.generation,
                before.generation
            );
            self.triggered.store(true, Ordering::SeqCst);
        }
        Ok(Box::new(InterceptPredictor {
            inner: NativeSearchPredictorFactory.load(repository, artifact_id, compute)?,
            repository: repository.clone(),
            experiment_id: self.experiment_id.clone(),
            cancel: self.cancel,
            triggered: self.triggered.clone(),
        }))
    }
}

#[test]
fn concurrent_controller_cannot_repeat_validation_or_open_test_early() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let triggered = Arc::new(AtomicBool::new(false));
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits))
        .with_predictor_factory(Arc::new(InterceptFactory {
            experiment_id: experiment.id.clone(),
            limits,
            cancel: false,
            triggered: triggered.clone(),
        }));
    controller.run_next(&experiment.id).unwrap();
    assert!(triggered.load(Ordering::SeqCst));
    let trial = repo
        .list_experiment_trials(&experiment.id)
        .unwrap()
        .remove(0);
    assert_eq!(trial.status, ExperimentTrialStatus::Validated);
    assert!(
        repo.predictions(trial.artifact_id.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        repo.get_experiment(&experiment.id)
            .unwrap()
            .selected_trial_id
            .is_none()
    );
    assert_eq!(
        controller.finalize(&experiment.id).unwrap().status,
        ExperimentStatus::Completed
    );
}

#[test]
fn cancellation_during_validation_resumes_the_published_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits))
        .with_predictor_factory(Arc::new(InterceptFactory {
            experiment_id: experiment.id.clone(),
            limits: limits.clone(),
            cancel: true,
            triggered: Arc::new(AtomicBool::new(false)),
        }));
    let stopped = controller.run_next(&experiment.id).unwrap();
    assert_eq!(stopped.status, ExperimentStatus::Cancelled);
    let trial = repo
        .list_experiment_trials(&experiment.id)
        .unwrap()
        .remove(0);
    let job = repo.get_job(&trial.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(trial.status, ExperimentTrialStatus::Cancelled);
    repo.resume_experiment(&experiment.id, now_ms()).unwrap();
    let resumed = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    resumed.run_next(&experiment.id).unwrap();
    let validated = repo.get_experiment_trial(&trial.id).unwrap();
    assert_eq!(validated.status, ExperimentTrialStatus::Validated);
    assert_eq!(validated.artifact_id, job.artifact_id);
    assert_eq!(
        repo.get_job(&trial.job_id).unwrap().generation,
        job.generation
    );
}

#[test]
fn cancellation_after_controller_interruption_settles_the_completed_worker() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "candidate-0", now_ms())
        .unwrap();
    let artifact = worker(&repo, &limits).run(&trial.job_id).unwrap();
    let generation = repo.get_job(&trial.job_id).unwrap().generation;
    drop(repo);

    let reopened = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    reopened
        .cancel_experiment(&experiment.id, now_ms())
        .unwrap();
    let controller = AutoTrainingController::new(reopened.clone(), worker(&reopened, &limits));
    let cancelled = controller.run(&experiment.id).unwrap();
    assert_eq!(cancelled.status, ExperimentStatus::Cancelled);
    assert_eq!(cancelled.usage.reserved_training_time_ms, 0);
    assert_eq!(cancelled.usage.reserved_artifact_bytes, 0);
    assert_eq!(cancelled.usage.training_time_ms, limits.maximum_duration_ms);
    assert_eq!(cancelled.usage.artifact_bytes, artifact.blob.bytes);
    assert_eq!(
        reopened.get_experiment_trial(&trial.id).unwrap().status,
        ExperimentTrialStatus::Cancelled
    );

    let repeated = reopened
        .cancel_experiment(&experiment.id, now_ms())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&repeated.usage).unwrap(),
        serde_json::to_value(&cancelled.usage).unwrap()
    );
    reopened
        .resume_experiment(&experiment.id, now_ms())
        .unwrap();
    controller.run_next(&experiment.id).unwrap();
    let validated = reopened.get_experiment_trial(&trial.id).unwrap();
    assert_eq!(validated.status, ExperimentTrialStatus::Validated);
    assert_eq!(validated.artifact_id.as_deref(), Some(artifact.id.as_str()));
    assert_eq!(
        reopened.get_job(&trial.job_id).unwrap().generation,
        generation
    );
    let resumed = reopened.get_experiment(&experiment.id).unwrap();
    assert_eq!(resumed.usage.artifact_bytes, cancelled.usage.artifact_bytes);
    assert_eq!(
        resumed.usage.training_time_ms,
        cancelled.usage.training_time_ms
    );
}

#[test]
fn forecast_search_checks_each_target_width_before_flattening() {
    for ragged_partition in [Some(0), Some(1), None] {
        let dir = tempfile::tempdir().unwrap();
        let repo = TrainingRepository::open(dir.path().join("forecast.sqlite")).unwrap();
        let stream = StreamKey {
            project_id: "forecast".into(),
            stream_id: "readings".into(),
            inspection_version: "v1".into(),
        };
        let spec = InspectionSpec {
            id: "forecast".into(),
            description: "Forecast the next two readings".into(),
            task: TaskKind::SequenceForecast,
            labels: vec![],
            input_shape: vec![1],
            prediction_horizon_ms: Some(1),
            minimum_examples: 1,
            minimum_examples_per_class: 1,
        };
        let mut partitions: [Vec<TrainingSample>; 3] = Default::default();
        for (partition, rows) in partitions.iter_mut().enumerate() {
            for index in 0..2 {
                let timestamp = (partition * 2 + index) as i64 * 10;
                let targets = if ragged_partition == Some(partition) {
                    if index == 0 {
                        vec![10.]
                    } else {
                        vec![20., 30., 40.]
                    }
                } else {
                    vec![10., 20.]
                };
                let sample = Sample {
                    id: format!("{partition}-{index}"),
                    group_id: format!("group-{partition}-{index}"),
                    stream_id: stream.stream_id.clone(),
                    timestamp_ms: timestamp,
                    window_start_ms: timestamp,
                    window_end_ms: timestamp,
                    input: TensorData {
                        shape: vec![1],
                        values: vec![index as f32],
                    },
                    annotation: Annotation::Values { values: targets },
                    provenance: LabelProvenance::Reviewed {
                        reviewer: "fixture".into(),
                        reviewed_at_ms: timestamp + 2,
                    },
                    outcome: Some(flow_like_ml_core::Outcome {
                        target_start_ms: timestamp + 1,
                        target_end_ms: timestamp + 1,
                        available_at_ms: timestamp + 2,
                    }),
                };
                spec.validate_sample_at(&sample, 100).unwrap();
                let row = TrainingSample {
                    id: sample.id.clone(),
                    annotation_revision: 1,
                    group_id: sample.group_id.clone(),
                    captured_at_ms: timestamp,
                    label_available_at_ms: timestamp + 2,
                    content_digest: format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&sample).unwrap())
                    ),
                    source: LabelSource::Reviewed,
                    accepted: true,
                    payload: serde_json::to_value(sample).unwrap(),
                };
                repo.record_sample(&stream, &row).unwrap();
                rows.push(row);
            }
        }
        let [train, validation, test] = partitions;
        let snapshot = repo
            .import_experiment_snapshot(DatasetSnapshot {
                id: String::new(),
                digest: String::new(),
                stream: stream.clone(),
                as_of_ms: 100,
                cutoff_sequence: 6,
                policy: SplitPolicy::Group {
                    train_fraction: 0.4,
                    validation_fraction: 0.3,
                    seed: 42,
                },
                train,
                validation,
                test,
                excluded: vec![],
            })
            .unwrap();
        let mut goals = default_goals(&spec);
        goals.bounds = vec![MetricBound {
            name: "rmse".into(),
            minimum: None,
            maximum: Some(100.),
        }];
        let budget = ExperimentBudget::default();
        let limits = budget.worker_limits.clone();
        let experiment = repo.create_experiment(ExperimentRequest {
            stream,
            snapshot_id: snapshot.id,
            spec: serde_json::to_value(spec).unwrap(),
            candidates: vec![TrainingRequest {
                engine: "burn".into(),
                recipe: json!({"config":{"recipe":{"architecture":"mlp","input_features":1,"hidden":4,"outputs":2,"objective":"regression"},"backend":{"backend":"cpu"},"epochs":1,"batch_size":2,"learning_rate":0.01,"seed":42,"gradient_clip":5.},"labels":[],"inspection_task":"sequence_forecast"}),
                compute: json!({"backend":"cpu"}),
            }],
            goals,
            budget,
            source_table_versions: vec![],
            created_tables: vec![],
            updated_table_versions: vec![],
            preprocessing_manifest: json!({}),
            context: json!({}),
        }, now_ms()).unwrap();
        let result = AutoTrainingController::new(repo.clone(), worker(&repo, &limits))
            .run(&experiment.id)
            .unwrap();
        let trial = repo
            .list_experiment_trials(&experiment.id)
            .unwrap()
            .remove(0);
        if ragged_partition.is_some() {
            assert_eq!(result.status, ExperimentStatus::Failed);
            assert!(
                trial
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("target width 1; model requires 2")
            );
            assert!(trial.artifact_id.is_none());
            assert!(repo.get_job(&trial.job_id).unwrap().checkpoint.is_none());
        } else {
            assert_eq!(result.status, ExperimentStatus::Completed);
            assert!(result.target_met);
            assert!(trial.artifact_id.is_some());
            assert!(result.final_evaluation_id.is_some());
        }
    }
}

#[test]
fn controller_recovers_an_expired_worker_after_process_restart() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (request, _) = setup(&repo);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "restart", now_ms())
        .unwrap();
    let lease = repo
        .claim_job(&trial.job_id, "old-process", 1000, now_ms() - 2000)
        .unwrap();
    let reopened = TrainingRepository::open(repo.path()).unwrap();
    let controller = AutoTrainingController::new(reopened.clone(), worker(&reopened, &limits));
    controller.run_next(&experiment.id).unwrap();
    assert_eq!(
        reopened.get_experiment_trial(&trial.id).unwrap().status,
        ExperimentTrialStatus::Validated
    );
    assert!(reopened.get_job(&trial.job_id).unwrap().generation > lease.generation);
    assert!(
        reopened
            .get_experiment(&experiment.id)
            .unwrap()
            .usage
            .training_time_ms
            >= limits.maximum_duration_ms,
        "recovery must charge the interrupted worker slice before training again"
    );
}

#[test]
fn recovery_cannot_restart_a_worker_after_its_training_budget_is_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (mut request, _) = setup(&repo);
    request.budget.maximum_training_time_ms = request.budget.worker_limits.maximum_duration_ms;
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let trial = repo
        .submit_experiment_trial(&experiment.id, 0, "exhausted-restart", now_ms())
        .unwrap();
    repo.claim_job(&trial.job_id, "old-process", 1000, now_ms() - 2000)
        .unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    let result = controller.run_next(&experiment.id).unwrap();
    let stopped = repo.get_job(&trial.job_id).unwrap();
    assert_eq!(stopped.status, JobStatus::Failed);
    assert!(stopped.artifact_id.is_none());
    assert!(result.usage.training_time_ms >= limits.maximum_duration_ms);
    assert_eq!(result.usage.reserved_training_time_ms, 0);
    assert_eq!(result.status, ExperimentStatus::BudgetExhausted);
}

#[test]
fn artifact_size_can_be_the_primary_goal_with_final_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (mut request, _) = setup(&repo);
    request.goals.primary_metric = "artifact_bytes".into();
    request.goals.direction = MetricDirection::Minimize;
    request.goals.bounds = vec![MetricBound {
        name: "artifact_bytes".into(),
        minimum: None,
        maximum: Some(1_000_000.0),
    }];
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    let result = controller.run(&experiment.id).unwrap();
    assert!(result.target_met, "{:?}", result.unmet_constraints);
    let trial = repo
        .get_experiment_trial(result.selected_trial_id.as_ref().unwrap())
        .unwrap();
    let artifact = repo
        .get_artifact(trial.artifact_id.as_ref().unwrap())
        .unwrap();
    let evaluation = repo
        .get_evaluation(result.final_evaluation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        trial.validation_metrics["artifact_bytes"],
        artifact.blob.bytes as f64
    );
    assert_eq!(
        evaluation.metrics["artifact_bytes"],
        artifact.blob.bytes as f64
    );
    assert!(evaluation.metrics["mean_latency_ms"] >= 0.0);
}

#[test]
fn generated_candidates_are_seeded_and_include_real_temporal_architectures() {
    let spec = InspectionSpec {
        id: "sensor".into(),
        description: "classify sensor windows".into(),
        task: TaskKind::SensorClassification,
        labels: vec!["a".into(), "b".into(), "c".into()],
        input_shape: vec![8, 3],
        prediction_horizon_ms: None,
        minimum_examples: 1,
        minimum_examples_per_class: 1,
    };
    let config = AutoSearchConfig {
        maximum_candidates: 32,
        ..Default::default()
    };
    let a = generate_candidates(&spec, &config, &ComputeConfig::default()).unwrap();
    let b = generate_candidates(&spec, &config, &ComputeConfig::default()).unwrap();
    assert_eq!(a, b);
    for name in ["lstm", "gru", "tcn", "cnn1d"] {
        assert!(
            a.iter()
                .any(|r| r.recipe["config"]["recipe"]["architecture"] == name),
            "missing {name}"
        );
    }
}

#[test]
fn explicit_candidate_tasks_still_validate_search_and_compute_limits() {
    let spec = InspectionSpec {
        id: "fusion".into(),
        description: "classify combined image and sensor features".into(),
        task: TaskKind::Fusion,
        labels: vec!["pass".into(), "fail".into()],
        input_shape: vec![16],
        prediction_horizon_ms: None,
        minimum_examples: 1,
        minimum_examples_per_class: 1,
    };
    let search = AutoSearchConfig::default();
    let compute = ComputeConfig::default();
    validate_search(&spec, &search, &compute).unwrap();
    assert!(
        generate_candidates(&spec, &search, &compute).is_err(),
        "fusion still needs an explicit recipe"
    );
    for invalid in [
        AutoSearchConfig {
            maximum_candidates: 0,
            ..search.clone()
        },
        AutoSearchConfig {
            max_epochs: 0,
            ..search.clone()
        },
        AutoSearchConfig {
            reduction_factor: 1,
            ..search.clone()
        },
        AutoSearchConfig {
            batch_size: 129,
            ..search.clone()
        },
        AutoSearchConfig {
            output_features: 0,
            ..search.clone()
        },
    ] {
        assert!(validate_search(&spec, &invalid, &compute).is_err());
    }
    assert!(
        validate_search(
            &spec,
            &search,
            &ComputeConfig {
                memory_limit_bytes: 0,
                ..compute
            }
        )
        .is_err()
    );
}

#[test]
fn successive_halving_resumes_only_the_selected_neural_survivor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (mut request, spec) = setup(&repo);
    request.budget.maximum_trials = 4;
    request.budget.maximum_training_time_ms = 120_000;
    request.budget.worker_limits.maximum_duration_ms = 30_000;
    let search = AutoSearchConfig {
        maximum_candidates: 32,
        min_epochs: 1,
        max_epochs: 2,
        reduction_factor: 3,
        batch_size: 8,
        ..Default::default()
    };
    request.candidates = generate_candidates(
        &spec,
        &search,
        &ComputeConfig {
            backend: ComputeBackend::Cpu,
            ..Default::default()
        },
    )
    .unwrap()
    .into_iter()
    .filter(|candidate| candidate.engine == "burn")
    .collect();
    assert_eq!(request.candidates.len(), 3);
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    let result = controller.run(&experiment.id).unwrap();
    assert_eq!(result.status, ExperimentStatus::Completed);
    let trials = repo.list_experiment_trials(&experiment.id).unwrap();
    assert_eq!(trials.len(), 4);
    let child = trials
        .iter()
        .find(|trial| repo.get_job(&trial.job_id).unwrap().request.recipe["search"]["rung"] == 1)
        .unwrap();
    assert_eq!(child.status, ExperimentTrialStatus::Validated);
    let job = repo.get_job(&child.job_id).unwrap();
    assert_eq!(job.request.recipe["config"]["epochs"], 2);
    let parent = repo
        .get_experiment_trial(
            job.request.recipe["search"]["parent_trial_id"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert!(repo.get_job(&parent.job_id).unwrap().checkpoint.is_some());
    assert!(
        job.checkpoint.as_ref().unwrap().step
            > repo
                .get_job(&parent.job_id)
                .unwrap()
                .checkpoint
                .unwrap()
                .step
    );
    assert_eq!(child.snapshot_id, parent.snapshot_id);
}

#[test]
fn halving_finishes_each_survivor_and_preserves_distinct_learning_rates() {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let (mut request, spec) = setup(&repo);
    request.budget.maximum_trials = 8;
    request.budget.maximum_training_time_ms = 240_000;
    request.budget.worker_limits.maximum_duration_ms = 30_000;
    let search = AutoSearchConfig {
        maximum_candidates: 32,
        min_epochs: 1,
        max_epochs: 4,
        reduction_factor: 2,
        batch_size: 8,
        ..Default::default()
    };
    let template = generate_candidates(
        &spec,
        &search,
        &ComputeConfig {
            backend: ComputeBackend::Cpu,
            ..Default::default()
        },
    )
    .unwrap()
    .into_iter()
    .find(|candidate| candidate.engine == "burn")
    .unwrap();
    request.candidates = [0.001, 0.003, 0.01]
        .into_iter()
        .map(|rate| {
            let mut candidate = template.clone();
            candidate.recipe["config"]["learning_rate"] = json!(rate);
            candidate
        })
        .collect();
    request.candidates.push(request.candidates[0].clone());
    let limits = request.budget.worker_limits.clone();
    let experiment = repo.create_experiment(request, now_ms()).unwrap();
    let controller = AutoTrainingController::new(repo.clone(), worker(&repo, &limits));
    assert_eq!(
        controller.run(&experiment.id).unwrap().status,
        ExperimentStatus::Completed
    );
    let trials = repo.list_experiment_trials(&experiment.id).unwrap();
    assert_eq!(
        trials.len(),
        7,
        "four roots, two distinct survivors, one final continuation"
    );
    let mut rungs = [0usize; 3];
    let mut first_survivor_rates = vec![];
    for trial in &trials {
        assert_eq!(
            trial.status,
            ExperimentTrialStatus::Validated,
            "{:?}",
            trial.error
        );
        let job = repo.get_job(&trial.job_id).unwrap();
        let rung = job.request.recipe["search"]["rung"].as_u64().unwrap() as usize;
        rungs[rung] += 1;
        if rung == 1 {
            first_survivor_rates.push(
                job.request.recipe["config"]["learning_rate"]
                    .as_f64()
                    .unwrap(),
            );
        }
        if let Some(parent_id) = job.request.recipe["search"]["parent_trial_id"].as_str() {
            let parent = repo.get_experiment_trial(parent_id).unwrap();
            let parent_job = repo.get_job(&parent.job_id).unwrap();
            let mut parent_config = parent_job.request.recipe["config"].clone();
            let mut child_config = job.request.recipe["config"].clone();
            parent_config.as_object_mut().unwrap().remove("epochs");
            child_config.as_object_mut().unwrap().remove("epochs");
            assert_eq!(
                parent_config, child_config,
                "optimizer continuation must retain architecture, learning rate, and seed"
            );
            assert!(job.checkpoint.unwrap().step > parent_job.checkpoint.unwrap().step);
            assert_eq!(trial.dataset_snapshot_id, parent.dataset_snapshot_id);
        }
    }
    assert_eq!(rungs, [4, 2, 1]);
    assert_ne!(
        first_survivor_rates[0], first_survivor_rates[1],
        "duplicate candidates must not displace a distinct learning rate"
    );
}

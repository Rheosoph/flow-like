#![cfg(feature = "execution")]
use flow_like_ml_runtime::worker::*;
use flow_like_ml_runtime::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier},
};

fn stream() -> StreamKey {
    StreamKey {
        project_id: "factory".into(),
        stream_id: "line-1".into(),
        inspection_version: "v1".into(),
    }
}
fn sample(i: usize) -> TrainingSample {
    TrainingSample {
        id: format!("sample-{i}"),
        annotation_revision: 1,
        group_id: format!("lot-{i}"),
        captured_at_ms: i as i64,
        label_available_at_ms: i as i64,
        content_digest: format!("{i:064x}"),
        source: LabelSource::Reviewed,
        accepted: true,
        payload: json!({"label":if i%2==0 {"defect"} else {"good"},"values":[i as f32,1.0]}),
    }
}
fn policy() -> SplitPolicy {
    SplitPolicy::Group {
        train_fraction: 0.5,
        validation_fraction: 0.25,
        seed: 42,
    }
}
fn request() -> TrainingRequest {
    TrainingRequest {
        engine: "fixture".into(),
        recipe: json!({"epochs":2,"seed":42}),
        compute: json!({"memory_limit_bytes":1_000_000}),
    }
}
fn repo() -> (tempfile::TempDir, TrainingRepository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    (dir, repo)
}
fn populate(repo: &TrainingRepository, range: std::ops::Range<usize>) {
    for i in range {
        repo.record_sample(&stream(), &sample(i)).unwrap();
    }
}
fn queued(repo: &TrainingRepository) -> TrainingJob {
    repo.trigger_after_n(&stream(), 3, request(), policy(), 1000)
        .unwrap()
        .unwrap()
}
fn candidate(repo: &TrainingRepository) -> ModelArtifact {
    let job = queued(repo);
    let lease = repo.claim_job(&job.id, "worker", 1000, 1000).unwrap();
    repo.finish_training(
        &lease,
        b"trained-model",
        json!({"architecture":"fixture","input_shape":[2]}),
        1001,
    )
    .unwrap()
}
fn evaluate(repo: &TrainingRepository, artifact: &ModelArtifact) -> EvaluationReport {
    let snapshot = repo.get_snapshot(&artifact.snapshot_id).unwrap();
    for sample in snapshot.test {
        let label = sample.payload["label"].as_str().unwrap().to_owned();
        repo.record_prediction(&PredictionRecord {
            sample_id: sample.id,
            artifact_id: artifact.id.clone(),
            predicted_label: Some(label.clone()),
            teacher_label: Some(label.clone()),
            actual_label: Some(label),
            actual_source: Some(LabelSource::Reviewed),
            latency_ms: 5.,
            failed: false,
            recorded_at_ms: 1100,
            details: json!({}),
        })
        .unwrap();
    }
    repo.evaluate_classification(&artifact.id, 1101).unwrap()
}
fn promotion_policy() -> PromotionPolicy {
    PromotionPolicy {
        minimum_audited_samples: 1,
        minimum_accuracy: 0.9,
        minimum_class_recall: BTreeMap::new(),
        maximum_mean_latency_ms: 10.,
        maximum_failure_rate: 0.,
    }
}

#[test]
fn immutable_samples_and_snapshots_survive_restart() {
    let (dir, repo) = repo();
    populate(&repo, 0..12);
    assert!(!repo.record_sample(&stream(), &sample(0)).unwrap().inserted);
    let mut altered = sample(0);
    altered.payload = json!({"label":"good"});
    assert!(matches!(
        repo.record_sample(&stream(), &altered),
        Err(Error::Conflict(_))
    ));
    let frozen = repo.snapshot(&stream(), policy(), 1000).unwrap();
    altered.annotation_revision = 2;
    repo.record_sample(&stream(), &altered).unwrap();
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    assert_eq!(frozen, repo.get_snapshot(&frozen.id).unwrap());
    let next = repo.snapshot(&stream(), policy(), 1000).unwrap();
    assert_ne!(frozen.digest, next.digest);
    assert_eq!(
        repo.record_sample(&stream(), &altered)
            .unwrap()
            .accepted_sequence,
        Some(1)
    );
}

#[test]
fn concurrent_connections_create_exactly_one_job() {
    let (_dir, repo) = repo();
    populate(&repo, 0..12);
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let path = repo.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let independent = TrainingRepository::open(path).unwrap();
                barrier.wait();
                independent
                    .trigger_after_n(&stream(), 12, request(), policy(), 1000)
                    .unwrap()
            })
        })
        .collect();
    let jobs: Vec<_> = threads
        .into_iter()
        .filter_map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(jobs.len(), 1);
    assert_eq!(repo.list_jobs(&stream()).unwrap().len(), 1);
    let lease = repo.claim_job(&jobs[0].id, "worker", 1000, 1000).unwrap();
    repo.fail_job(&lease, "expected failure", 1001).unwrap();
    assert!(
        repo.trigger_after_n(&stream(), 1, request(), policy(), 1002)
            .unwrap()
            .is_none(),
        "failed data must not trigger again on every frame"
    );
    repo.resume_job(&jobs[0].id, 1003).unwrap();
}

#[test]
fn child_process_actor() {
    let Some(path) = std::env::var_os("FLOW_ML_TEST_LEDGER") else {
        return;
    };
    let repo = TrainingRepository::open(path).unwrap();
    for i in 0..12 {
        repo.record_sample(&stream(), &sample(i)).unwrap();
    }
    let _ = repo
        .trigger_after_n(&stream(), 12, request(), policy(), 1000)
        .unwrap();
}

#[test]
fn multiple_processes_deduplicate_and_claim_atomically() {
    let (_dir, repo) = repo();
    let mut children: Vec<_> = (0..3)
        .map(|_| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "child_process_actor", "--nocapture"])
                .env("FLOW_ML_TEST_LEDGER", repo.path())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(repo.list_jobs(&stream()).unwrap().len(), 1);
    let snapshot = repo
        .get_snapshot(&repo.list_jobs(&stream()).unwrap()[0].snapshot_id)
        .unwrap();
    assert_eq!(
        snapshot.train.len() + snapshot.validation.len() + snapshot.test.len(),
        12
    );
}

#[test]
fn expired_workers_are_fenced_and_checkpoint_resume_is_durable() {
    let (dir, repo) = repo();
    populate(&repo, 0..12);
    let job = queued(&repo);
    let stale = repo.claim_job(&job.id, "old", 10, 1000).unwrap();
    let checkpoint = repo
        .publish_checkpoint(
            &stale,
            b"optimizer-and-rng",
            4,
            json!({"engine":"fixture","optimizer":"sgd","rng":42}),
            false,
            1001,
        )
        .unwrap();
    assert!(matches!(
        repo.claim_job(&job.id, "new", 10, 1011),
        Err(Error::Conflict(_))
    ));
    repo.mark_interrupted(&job.id, stale.generation, 1011)
        .unwrap();
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    assert_eq!(
        repo.get_job(&job.id).unwrap().checkpoint,
        Some(checkpoint.clone())
    );
    repo.resume_job(&job.id, 1012).unwrap();
    let current = repo.claim_job(&job.id, "new", 1000, 1013).unwrap();
    assert!(matches!(
        repo.finish_training(&stale, b"stale-model", json!({}), 1014),
        Err(Error::LeaseLost)
    ));
    assert_eq!(
        repo.read_blob(&checkpoint.blob).unwrap(),
        b"optimizer-and-rng"
    );
    repo.finish_training(&current, b"resumed-model", json!({}), 1014)
        .unwrap();
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Succeeded);
}

#[test]
fn interrupted_artifact_write_cannot_replace_checkpoint() {
    let (dir, repo) = repo();
    populate(&repo, 0..12);
    let job = queued(&repo);
    let lease = repo.claim_job(&job.id, "worker", 1000, 1000).unwrap();
    let checkpoint = repo
        .publish_checkpoint(
            &lease,
            b"valid-checkpoint",
            1,
            json!({"optimizer":"sgd"}),
            false,
            1001,
        )
        .unwrap();
    std::fs::write(
        repo.path().with_extension("artifacts").join("crash.part"),
        b"partial",
    )
    .unwrap();
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    assert_eq!(
        repo.get_job(&job.id).unwrap().checkpoint,
        Some(checkpoint.clone())
    );
    std::fs::write(
        repo.path()
            .with_extension("artifacts")
            .join(&checkpoint.blob.path),
        b"corrupted",
    )
    .unwrap();
    assert!(matches!(
        repo.read_blob(&checkpoint.blob),
        Err(Error::Invalid(_))
    ));
    let mut traversal = checkpoint.blob;
    traversal.path = "../outside".into();
    assert!(repo.read_blob(&traversal).is_err());
}

#[test]
fn rejected_and_delayed_labels_are_not_lost_or_counted_early() {
    let (_dir, repo) = repo();
    let mut delayed = sample(0);
    delayed.label_available_at_ms = 2000;
    repo.record_sample(&stream(), &delayed).unwrap();
    populate(&repo, 1..13);
    let artifact = candidate(&repo);
    assert!(
        !repo
            .get_snapshot(&artifact.snapshot_id)
            .unwrap()
            .train
            .iter()
            .any(|s| s.id == delayed.id)
    );
    assert!(
        repo.trigger_after_n(&stream(), 1, request(), policy(), 1999)
            .unwrap()
            .is_none()
    );
    let next = repo
        .trigger_after_n(&stream(), 1, request(), policy(), 2000)
        .unwrap();
    assert!(
        next.is_some(),
        "a delayed outcome preceding the trigger watermark must remain eligible"
    );
}

#[test]
fn grouped_time_split_excludes_boundary_crossing_lots_and_future_truth() {
    let (_dir, repo) = repo();
    for (i, time, group) in [
        (0, 10, "train"),
        (1, 12, "train"),
        (2, 30, "validation"),
        (3, 50, "test"),
        (4, 19, "cross"),
        (5, 23, "cross"),
    ] {
        let mut sample = sample(i);
        sample.captured_at_ms = time;
        sample.label_available_at_ms = time;
        sample.group_id = group.into();
        repo.record_sample(&stream(), &sample).unwrap();
    }
    let snapshot = repo
        .snapshot(
            &stream(),
            SplitPolicy::Time {
                train_end_ms: 20,
                validation_end_ms: 40,
                embargo_ms: 3,
            },
            100,
        )
        .unwrap();
    assert_eq!(snapshot.train.len(), 2);
    assert_eq!(snapshot.validation.len(), 1);
    assert_eq!(snapshot.test.len(), 1);
    assert_eq!(snapshot.excluded.len(), 2);
    let sets: Vec<std::collections::HashSet<_>> =
        [&snapshot.train, &snapshot.validation, &snapshot.test]
            .iter()
            .map(|part| part.iter().map(|s| &s.group_id).collect())
            .collect();
    assert!(
        sets[0].is_disjoint(&sets[1])
            && sets[0].is_disjoint(&sets[2])
            && sets[1].is_disjoint(&sets[2])
    );
}

#[test]
fn teacher_only_agreement_cannot_promote() {
    let (_dir, repo) = repo();
    populate(&repo, 0..12);
    let artifact = candidate(&repo);
    repo.record_prediction(&PredictionRecord {
        sample_id: "unknown-stream-sample".into(),
        artifact_id: artifact.id.clone(),
        predicted_label: Some("good".into()),
        teacher_label: Some("good".into()),
        actual_label: None,
        actual_source: None,
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: 1100,
        details: json!({}),
    })
    .unwrap();
    let report = repo.evaluate_classification(&artifact.id, 1101).unwrap();
    assert_eq!(report.metrics["teacher_agreement"], 1.);
    assert_eq!(report.audited_samples, 0);
    assert!(matches!(
        repo.promote("line-production", 0, &report.id, promotion_policy(), 1102),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn evaluation_rejects_training_group_leakage_and_forged_truth() {
    let (_dir, repo) = repo();
    populate(&repo, 0..12);
    let artifact = candidate(&repo);
    let snapshot = repo.get_snapshot(&artifact.snapshot_id).unwrap();
    let sample = &snapshot.train[0];
    let mut prediction = PredictionRecord {
        sample_id: sample.id.clone(),
        artifact_id: artifact.id.clone(),
        predicted_label: Some("good".into()),
        teacher_label: None,
        actual_label: Some("forged".into()),
        actual_source: Some(LabelSource::Reviewed),
        latency_ms: 1.,
        failed: false,
        recorded_at_ms: 1100,
        details: json!({}),
    };
    assert!(repo.record_prediction(&prediction).is_err());
    prediction.actual_label = sample.payload["label"].as_str().map(str::to_owned);
    repo.record_prediction(&prediction).unwrap();
    assert!(matches!(
        repo.evaluate_classification(&artifact.id, 1101),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn promotion_generation_races_and_rollback_survive_restart() {
    let (dir, repo) = repo();
    populate(&repo, 0..12);
    let first = candidate(&repo);
    let report = evaluate(&repo, &first);
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let repo = repo.clone();
            let report = report.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                repo.promote("production", 0, &report.id, promotion_policy(), 1200)
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
    populate(&repo, 12..24);
    let second = candidate(&repo);
    let report = evaluate(&repo, &second);
    repo.promote("production", 1, &report.id, promotion_policy(), 1201)
        .unwrap();
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    assert_eq!(
        repo.get_deployment("production")
            .unwrap()
            .active_artifact_id,
        second.id
    );
    let rolled = repo.rollback("production", 2, 1202).unwrap();
    assert_eq!(rolled.active_artifact_id, first.id);
    assert_eq!(rolled.generation, 3);
    assert!(repo.rollback("production", 2, 1203).is_err());
}

#[test]
fn growing_stream_waits_for_independent_partitions_without_consuming_trigger() {
    let (_dir, repo) = repo();
    populate(&repo, 0..1);
    assert!(
        repo.trigger_after_n(&stream(), 1, request(), policy(), 1000)
            .unwrap()
            .is_none()
    );
    populate(&repo, 1..3);
    assert!(
        repo.trigger_after_n(&stream(), 3, request(), policy(), 1000)
            .unwrap()
            .is_some()
    );

    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("time.sqlite")).unwrap();
    let policy = SplitPolicy::Time {
        train_end_ms: 2,
        validation_end_ms: 5,
        embargo_ms: 1,
    };
    populate(&repo, 0..3);
    assert!(
        repo.trigger_after_n(&stream(), 1, request(), policy.clone(), 1000)
            .unwrap()
            .is_none()
    );
    populate(&repo, 3..9);
    assert!(
        repo.trigger_after_n(&stream(), 9, request(), policy, 1000)
            .unwrap()
            .is_some()
    );
}

#[test]
fn suspension_fences_promotion_and_preserves_artifact_provenance() {
    let (dir, repo) = repo();
    populate(&repo, 0..12);
    let first = candidate(&repo);
    let first_report = evaluate(&repo, &first);
    repo.promote("production", 0, &first_report.id, promotion_policy(), 1200)
        .unwrap();
    let paused = repo
        .set_deployment_paused("production", 1, true, 1201)
        .unwrap();
    assert!(paused.paused);
    assert_eq!(paused.active_artifact_id, first.id);
    assert_eq!(paused.evaluation_id, first_report.id);
    assert!(paused.previous_artifact_id.is_none());
    assert!(
        repo.rollback("production", paused.generation, 1202)
            .is_err()
    );
    assert!(matches!(
        repo.shadow_telemetry()
            .set_deployment_paused("production", 2, false, 1202),
        Err(Error::ShadowWrite)
    ));
    populate(&repo, 12..24);
    let second = candidate(&repo);
    let report = evaluate(&repo, &second);
    let barrier = Arc::new(Barrier::new(2));
    let pause_repo = repo.clone();
    let pause_barrier = barrier.clone();
    let pauser = std::thread::spawn(move || {
        pause_barrier.wait();
        pause_repo.set_deployment_paused("production", 2, true, 1203)
    });
    let promote_repo = repo.clone();
    let promoter = std::thread::spawn(move || {
        barrier.wait();
        promote_repo.promote("production", 2, &report.id, promotion_policy(), 1203)
    });
    let results = [pauser.join().unwrap(), promoter.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .any(|result| matches!(result, Err(Error::Conflict(_))))
    );
    let state = repo.get_deployment("production").unwrap();
    assert_eq!(state.generation, 3);
    assert_eq!(state.stream, stream());
    assert_eq!(
        state.active_artifact_id,
        if state.paused {
            first.id.clone()
        } else {
            second.id.clone()
        }
    );
    drop(repo);
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    assert_eq!(repo.get_deployment("production").unwrap(), state);
    if state.active_artifact_id == second.id {
        let rolled = repo.rollback("production", 3, 1204).unwrap();
        assert_eq!(rolled.active_artifact_id, first.id);
        assert!(
            rolled.paused,
            "rollback restores the prior suspended routing state"
        );
    }
}

#[test]
fn shadow_handle_cannot_mutate_training_or_deployment() {
    let (_dir, repo) = repo();
    populate(&repo, 0..12);
    let artifact = candidate(&repo);
    let shadow = repo.shadow_telemetry();
    assert!(matches!(
        shadow.record_sample(&stream(), &sample(99)),
        Err(Error::ShadowWrite)
    ));
    assert!(matches!(
        shadow.snapshot(&stream(), policy(), 1200),
        Err(Error::ShadowWrite)
    ));
    assert!(matches!(
        shadow.trigger_after_n(&stream(), 3, request(), policy(), 1200),
        Err(Error::ShadowWrite)
    ));
    shadow
        .record_prediction(&PredictionRecord {
            sample_id: "shadow-sample".into(),
            artifact_id: artifact.id.clone(),
            predicted_label: Some("good".into()),
            teacher_label: Some("good".into()),
            actual_label: None,
            actual_source: None,
            latency_ms: 2.,
            failed: false,
            recorded_at_ms: 1200,
            details: json!({}),
        })
        .unwrap();
    assert_eq!(repo.predictions(&artifact.id).unwrap().len(), 1);
    assert!(matches!(
        shadow.evaluate_classification(&artifact.id, 1201),
        Err(Error::ShadowWrite)
    ));
}

struct FixtureEngine {
    cancel: Option<TrainingRepository>,
    memory: u64,
}
impl TrainingEngine for FixtureEngine {
    fn estimated_memory_bytes(&self, _: &TrainingWork) -> Result<u64> {
        Ok(self.memory)
    }
    fn train(&self, work: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput> {
        control.checkpoint(
            b"state",
            1,
            json!({"engine":"fixture","optimizer":"sgd","sampler":1}),
        )?;
        control.progress(json!({"step":1}))?;
        if let Some(repo) = &self.cancel {
            repo.request_cancel(&work.job.id, now_ms())?;
            control.check_cancel()?;
        }
        Ok(EngineOutput {
            model_bytes: b"actual-native-output".to_vec(),
            manifest: json!({"engine":"fixture"}),
        })
    }
}

#[test]
fn worker_checkpoint_cancel_resume_and_memory_budget_are_real() {
    let (_dir, repo) = repo();
    populate(&repo, 0..12);
    let job = queued(&repo);
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    worker
        .register(
            "fixture",
            Arc::new(FixtureEngine {
                cancel: None,
                memory: 2_000_000,
            }),
        )
        .unwrap();
    assert!(matches!(worker.run(&job.id), Err(Error::Engine(_))));
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Queued);
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    worker
        .register(
            "fixture",
            Arc::new(FixtureEngine {
                cancel: Some(repo.clone()),
                memory: 100,
            }),
        )
        .unwrap();
    assert!(matches!(worker.run(&job.id), Err(Error::Cancelled)));
    let cancelled = repo.get_job(&job.id).unwrap();
    assert_eq!(cancelled.status, JobStatus::Cancelled);
    assert!(cancelled.checkpoint.is_some());
    repo.resume_job(&job.id, now_ms()).unwrap();
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    worker
        .register(
            "fixture",
            Arc::new(FixtureEngine {
                cancel: None,
                memory: 100,
            }),
        )
        .unwrap();
    let artifact = worker.run(&job.id).unwrap();
    assert_eq!(
        repo.read_blob(&artifact.blob).unwrap(),
        b"actual-native-output"
    );
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Succeeded);
}

#[test]
fn resource_admission_is_shared_across_connections_and_survives_restart() {
    let (directory, repo) = repo();
    populate(&repo, 0..12);
    let first = queued(&repo);
    let second_stream = StreamKey {
        stream_id: "line-2".into(),
        ..stream()
    };
    for i in 0..12 {
        repo.record_sample(&second_stream, &sample(i)).unwrap();
    }
    let second = repo
        .trigger_after_n(&second_stream, 12, request(), policy(), 1000)
        .unwrap()
        .unwrap();
    let resource = ResourceRequest {
        key: "cpu:0".into(),
        estimated_bytes: 600,
        budget_bytes: 1000,
    };
    let lease = repo
        .claim_job_with_resources(&first.id, "one", 1000, 1000, Some(resource.clone()))
        .unwrap();
    drop(repo);
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    assert!(matches!(
        repo.claim_job_with_resources(&second.id, "two", 1000, 1001, Some(resource.clone())),
        Err(Error::Conflict(_))
    ));
    assert_eq!(repo.pending_jobs().unwrap().len(), 1);
    assert_eq!(repo.expired_jobs(2001).unwrap().len(), 1);
    repo.mark_interrupted(&first.id, lease.generation, 2001)
        .unwrap();
    assert!(
        repo.claim_job_with_resources(&second.id, "two", 1000, 2002, Some(resource.clone()))
            .is_err(),
        "expired worker retains memory until reconciled"
    );
    repo.request_cancel(&first.id, 2003).unwrap();
    repo.claim_job_with_resources(&second.id, "two", 1000, 2004, Some(resource))
        .unwrap();
}

#[test]
fn snapshot_byte_limit_precedes_deserialization_and_worker_admission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct AdmissionProbe(Arc<AtomicUsize>);
    impl TrainingEngine for AdmissionProbe {
        fn estimated_memory_bytes(&self, _: &TrainingWork) -> Result<u64> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(1)
        }
        fn train(&self, _: &TrainingWork, _: &WorkerControl) -> Result<EngineOutput> {
            panic!("oversized snapshots must never reach training")
        }
    }
    let (_dir, repo) = repo();
    for index in 0..6 {
        let mut record = sample(index);
        record.payload["unicode"] = json!("é".repeat(512));
        repo.record_sample(&stream(), &record).unwrap();
    }
    let job = queued(&repo);
    let snapshot = repo.get_snapshot(&job.snapshot_id).unwrap();
    let bytes = serde_json::to_vec(&snapshot).unwrap().len() as u64;
    assert!(repo.get_snapshot_limited(&snapshot.id, bytes).is_ok());
    assert!(matches!(
        repo.get_snapshot_limited(&snapshot.id, bytes - 1),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        repo.get_snapshot_limited("missing", bytes),
        Err(Error::NotFound(_))
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut worker = TrainingWorker::new(
        repo.clone(),
        WorkerLimits {
            memory_budget_bytes: bytes - 1,
            ..WorkerLimits::default()
        },
    )
    .unwrap();
    worker
        .register("fixture", Arc::new(AdmissionProbe(calls.clone())))
        .unwrap();
    assert!(matches!(worker.run(&job.id), Err(Error::Invalid(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Queued);
    let connection = rusqlite::Connection::open(repo.path()).unwrap();
    connection
        .execute(
            "UPDATE snapshots SET body=? WHERE id=?",
            rusqlite::params!["{".repeat(bytes as usize), snapshot.id],
        )
        .unwrap();
    assert!(
        matches!(
            repo.get_snapshot_limited(&snapshot.id, bytes - 1),
            Err(Error::Invalid(_))
        ),
        "oversize must be rejected before attempting to parse corrupt JSON"
    );
    assert!(matches!(
        repo.get_snapshot_limited(&snapshot.id, bytes),
        Err(Error::Json(_))
    ));
}

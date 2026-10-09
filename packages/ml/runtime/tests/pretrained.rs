#![cfg(feature = "burn")]

use flow_like_ml_burn as burn;
use flow_like_ml_core::{
    Annotation, ComputeBackend, ComputeConfig, LabelProvenance, Sample, TensorData,
};
use flow_like_ml_runtime::{engines::*, worker::TrainingWorker, *};
use serde_json::json;

fn config(classes: usize) -> burn::TrainingConfig {
    burn::TrainingConfig {
        recipe: burn::Recipe::ResNet18 {
            input_channels: 1,
            classes,
            base_channels: 2,
        },
        backend: burn::BackendChoice::Cpu,
        epochs: 1,
        batch_size: 2,
        learning_rate: 0.001,
        seed: 17,
        gradient_clip: 5.,
    }
}

fn job(
    repo: &TrainingRepository,
    project: &str,
    name: &str,
    classes: usize,
    pretrained: Option<PretrainedSourceRef>,
) -> TrainingJob {
    job_with_epochs(repo, project, name, classes, pretrained, 1)
}

fn job_with_epochs(
    repo: &TrainingRepository,
    project: &str,
    name: &str,
    classes: usize,
    pretrained: Option<PretrainedSourceRef>,
    epochs: usize,
) -> TrainingJob {
    let stream = StreamKey {
        project_id: project.into(),
        stream_id: name.into(),
        inspection_version: name.into(),
    };
    let labels: Vec<_> = (0..classes)
        .map(|class| format!("{name}-{class}"))
        .collect();
    for index in 0..12 {
        let sample = Sample {
            id: format!("{name}-{index}"),
            group_id: format!("{name}-group-{index}"),
            stream_id: name.into(),
            timestamp_ms: index,
            window_start_ms: index,
            window_end_ms: index,
            input: TensorData {
                shape: vec![1, 8, 8],
                values: vec![index as f32 / 12.; 64],
            },
            annotation: Annotation::Class {
                class_id: index as u32 % classes as u32,
            },
            provenance: LabelProvenance::Reviewed {
                reviewer: "fixture".into(),
                reviewed_at_ms: index,
            },
            outcome: None,
        };
        repo.record_sample(
            &stream,
            &TrainingSample {
                id: sample.id.clone(),
                annotation_revision: 1,
                group_id: sample.group_id.clone(),
                captured_at_ms: index,
                label_available_at_ms: index,
                content_digest: format!("{index:064x}"),
                source: LabelSource::Reviewed,
                accepted: true,
                payload: serde_json::to_value(sample).unwrap(),
            },
        )
        .unwrap();
    }
    let mut training_config = config(classes);
    training_config.epochs = epochs;
    repo.trigger_after_n(
        &stream,
        12,
        TrainingRequest {
            engine: "burn".into(),
            recipe: json!({"config":training_config,"labels":labels,"pretrained":pretrained}),
            compute: json!(ComputeConfig {
                backend: ComputeBackend::Cpu,
                ..Default::default()
            }),
        },
        SplitPolicy::Time {
            train_end_ms: 5,
            validation_end_ms: 8,
            embargo_ms: 0,
        },
        100,
    )
    .unwrap()
    .unwrap()
}

fn worker(repo: &TrainingRepository) -> TrainingWorker {
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    register_burn_engine(&mut worker).unwrap();
    worker
}

#[test]
fn registered_sources_fine_tune_new_datasets_and_heads_without_training_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let source_job = job(&repo, "project", "source", 2, None);
    let original = worker(&repo).run(&source_job.id).unwrap();
    let bytes = repo.read_blob(&original.blob).unwrap();
    let imported = repo
        .register_pretrained_source(
            "project",
            &bytes,
            original.manifest.clone(),
            json!({"source":"fixture bundle"}),
            bytes.len() as u64,
            now_ms(),
        )
        .unwrap();
    assert_eq!(imported.source_kind, PretrainedSourceKind::Imported);
    assert!(matches!(
        repo.get_artifact(&imported.id),
        Err(Error::NotFound(_))
    ));
    assert!(repo.pending_jobs().unwrap().is_empty());
    assert_eq!(
        repo.list_pretrained_sources("project", 16).unwrap().len(),
        2
    );
    assert!(
        repo.list_pretrained_sources("other", 16)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.get_pretrained_source(&original.id)
            .unwrap()
            .source_kind,
        PretrainedSourceKind::TrainingArtifact
    );

    let reference = PretrainedSourceRef {
        source_id: imported.id.clone(),
        replace_head: true,
        freeze_backbone: true,
    };
    let target = job(&repo, "project", "new-task", 3, Some(reference.clone()));
    let artifact = worker(&repo).run(&target.id).unwrap();
    assert_ne!(artifact.stream, original.stream);
    assert_ne!(artifact.dataset_digest, original.dataset_digest);
    assert_eq!(
        artifact.manifest["labels"],
        json!(["new-task-0", "new-task-1", "new-task-2"])
    );
    assert_eq!(artifact.manifest["pretrained"]["source_id"], imported.id);
    assert_eq!(
        artifact.manifest["pretrained"]["source_blob"]["sha256"],
        original.blob.sha256
    );
    assert_eq!(artifact.manifest["pretrained"]["source_kind"], "imported");
    assert!(repo.get_job(&target.id).unwrap().checkpoint.is_some());
    let predictions = load_artifact_predictions(
        &repo,
        &artifact.id,
        &TensorData {
            shape: vec![1, 1, 8, 8],
            values: vec![0.5; 64],
        },
        &ComputeConfig {
            backend: ComputeBackend::Cpu,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(predictions["output"]["shape"], json!([1, 3]));

    let foreign = job(&repo, "other", "foreign", 3, Some(reference));
    assert!(
        worker(&repo)
            .run(&foreign.id)
            .unwrap_err()
            .to_string()
            .contains("different project")
    );
    assert!(repo.get_job(&foreign.id).unwrap().artifact_id.is_none());
}

#[test]
fn pretrained_sources_enforce_head_identity_integrity_and_admission_budgets() {
    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let source_job = job(&repo, "project", "source", 2, None);
    let source = worker(&repo).run(&source_job.id).unwrap();
    let reference = PretrainedSourceRef {
        source_id: source.id.clone(),
        replace_head: false,
        freeze_backbone: false,
    };
    let labels = vec!["different-0".into(), "different-1".into()];
    assert!(
        validate_burn_pretrained_source(
            &repo,
            "project",
            &reference,
            &config(2),
            &labels,
            u64::MAX
        )
        .unwrap_err()
        .to_string()
        .contains("same label order")
    );
    let labels = vec!["source-0".into(), "source-1".into()];
    assert!(
        validate_burn_pretrained_source(
            &repo,
            "project",
            &reference,
            &config(2),
            &labels,
            source.blob.bytes - 1
        )
        .is_err()
    );
    let reference = PretrainedSourceRef {
        replace_head: true,
        ..reference
    };
    let target = job(&repo, "project", "target", 3, Some(reference));
    let mut bytes = repo.read_blob(&source.blob).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(
        repo.path()
            .with_extension("artifacts")
            .join(&source.blob.path),
        bytes,
    )
    .unwrap();
    assert!(
        worker(&repo)
            .run(&target.id)
            .unwrap_err()
            .to_string()
            .contains("digest or size mismatch")
    );
    let stopped = repo.get_job(&target.id).unwrap();
    assert_eq!(stopped.status, JobStatus::Failed);
    assert!(stopped.artifact_id.is_none());
}

#[test]
fn fine_tuning_resumes_its_own_checkpoint_without_reloading_the_source_blob() {
    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let source_job = job(&repo, "project", "source", 2, None);
    let source = worker(&repo).run(&source_job.id).unwrap();
    let target = job_with_epochs(
        &repo,
        "project",
        "resume",
        3,
        Some(PretrainedSourceRef {
            source_id: source.id.clone(),
            replace_head: true,
            freeze_backbone: true,
        }),
        10,
    );
    let worker_repo = repo.clone();
    let target_id = target.id.clone();
    let running = std::thread::spawn(move || worker(&worker_repo).run(&target_id));
    // Interrupt after a durable epoch checkpoint instead of relying on a short timeout.
    let timeout = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let current = repo.get_job(&target.id).unwrap();
        assert!(matches!(
            current.status,
            JobStatus::Queued | JobStatus::Running
        ));
        if current.checkpoint.is_some() {
            repo.request_cancel(&target.id, now_ms()).unwrap();
            break;
        }
        assert!(
            std::time::Instant::now() < timeout,
            "training did not publish a checkpoint"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(matches!(running.join().unwrap(), Err(Error::Cancelled)));
    let checkpoint = repo.get_job(&target.id).unwrap().checkpoint.unwrap();
    std::fs::remove_file(
        repo.path()
            .with_extension("artifacts")
            .join(&source.blob.path),
    )
    .unwrap();
    repo.resume_job(&target.id, now_ms()).unwrap();
    let artifact = worker(&repo).run(&target.id).unwrap();
    assert_eq!(artifact.manifest["pretrained"]["source_id"], source.id);
    assert!(repo.get_job(&target.id).unwrap().checkpoint.unwrap().step > checkpoint.step);
    assert_eq!(artifact.manifest["report"]["completed_epochs"], 10);
}

#[test]
fn imported_feature_sources_enforce_scope_budget_contract_and_digest_without_training() {
    let directory = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let original = worker(&repo)
        .run(&job(&repo, "project", "source", 2, None).id)
        .unwrap();
    let bytes = repo.read_blob(&original.blob).unwrap();
    let imported = repo
        .register_pretrained_source(
            "project",
            &bytes,
            original.manifest.clone(),
            json!({"source":"fixture weights"}),
            bytes.len() as u64,
            now_ms(),
        )
        .unwrap();
    let input = TensorData {
        shape: vec![2, 1, 8, 8],
        values: (0..128).map(|i| i as f32 / 128.).collect(),
    };
    let compute = ComputeConfig {
        backend: ComputeBackend::Cpu,
        ..Default::default()
    };
    let expected = artifact_features(&repo, &original.id, &input, &compute).unwrap();
    let actual =
        pretrained_source_features(&repo, "project", &imported.id, &input, &compute).unwrap();
    assert_eq!(actual.shape, expected.shape);
    assert_eq!(actual.values, expected.values);
    assert_eq!(actual.shape[0], 2);
    assert_eq!(actual.shape.len(), 4);
    let single = TensorData {
        shape: vec![1, 1, 8, 8],
        values: input.values[..64].to_vec(),
    };
    let unbatched = TensorData {
        shape: vec![1, 8, 8],
        values: single.values.clone(),
    };
    let explicit =
        pretrained_source_features(&repo, "project", &imported.id, &single, &compute).unwrap();
    let implicit =
        pretrained_source_features(&repo, "project", &imported.id, &unbatched, &compute).unwrap();
    assert_eq!(implicit.shape, explicit.shape);
    assert_eq!(implicit.values, explicit.values);
    assert_eq!(implicit.shape[0], 1);
    assert!(matches!(
        repo.get_artifact(&imported.id),
        Err(Error::NotFound(_))
    ));
    assert!(repo.pending_jobs().unwrap().is_empty());
    assert_eq!(
        repo.list_pretrained_sources("project", 16).unwrap().len(),
        2
    );

    assert!(
        pretrained_source_features(&repo, "other", &imported.id, &input, &compute)
            .unwrap_err()
            .to_string()
            .contains("different project")
    );
    assert!(
        pretrained_source_features(
            &repo,
            "project",
            &imported.id,
            &input,
            &ComputeConfig {
                memory_limit_bytes: imported.blob.bytes * 4 - 1,
                ..compute.clone()
            }
        )
        .is_err()
    );
    // The bundle fits, but its forward workspace must also fit before loading it.
    assert!(
        pretrained_source_features(
            &repo,
            "project",
            &imported.id,
            &input,
            &ComputeConfig {
                memory_limit_bytes: imported.blob.bytes * 4,
                ..compute.clone()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("Feature extraction needs")
    );
    let wrong_shape = TensorData {
        shape: vec![1, 1, 16, 16],
        values: vec![0.; 256],
    };
    assert!(
        pretrained_source_features(&repo, "project", &imported.id, &wrong_shape, &compute)
            .unwrap_err()
            .to_string()
            .contains("input contract")
    );
    let mut changed_manifest = original.manifest.clone();
    changed_manifest["config"]["seed"] = json!(99);
    let forged = repo
        .register_pretrained_source(
            "project",
            &bytes,
            changed_manifest,
            json!({"source":"mismatched metadata"}),
            bytes.len() as u64,
            now_ms(),
        )
        .unwrap();
    assert!(
        pretrained_source_features(&repo, "project", &forged.id, &input, &compute)
            .unwrap_err()
            .to_string()
            .contains("packed model state")
    );
    let mut corrupt = bytes;
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(
        repo.path()
            .with_extension("artifacts")
            .join(&imported.blob.path),
        corrupt,
    )
    .unwrap();
    assert!(
        pretrained_source_features(&repo, "project", &imported.id, &input, &compute)
            .unwrap_err()
            .to_string()
            .contains("digest or size mismatch")
    );
}

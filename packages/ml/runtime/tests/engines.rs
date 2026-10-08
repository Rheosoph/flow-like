#![cfg(all(feature = "native", feature = "burn"))]
use flow_like_ml_core::{Annotation, ComputeConfig, LabelProvenance, Sample, TensorData};
use flow_like_ml_runtime::{engines::*, worker::TrainingWorker, *};
use serde_json::json;

fn populated() -> (tempfile::TempDir, TrainingRepository, StreamKey) {
    let directory = tempfile::tempdir().unwrap();
    let repository = TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
    let stream = StreamKey {
        project_id: "factory".into(),
        stream_id: "sensor".into(),
        inspection_version: "v1".into(),
    };
    for i in 0..30 {
        let x = (i as f32 - 15.) / 15.;
        let sample = Sample {
            id: format!("sample-{i}"),
            group_id: format!("batch-{i}"),
            stream_id: stream.stream_id.clone(),
            timestamp_ms: i,
            window_start_ms: i,
            window_end_ms: i,
            input: TensorData {
                shape: vec![1],
                values: vec![x],
            },
            annotation: Annotation::Scalar {
                value: 2. * x as f64,
            },
            provenance: LabelProvenance::Reviewed {
                reviewer: "operator".into(),
                reviewed_at_ms: i,
            },
            outcome: None,
        };
        repository
            .record_sample(
                &stream,
                &TrainingSample {
                    id: sample.id.clone(),
                    annotation_revision: 1,
                    group_id: sample.group_id.clone(),
                    captured_at_ms: i,
                    label_available_at_ms: i,
                    content_digest: format!("{i:064x}"),
                    source: LabelSource::Reviewed,
                    accepted: true,
                    payload: serde_json::to_value(sample).unwrap(),
                },
            )
            .unwrap();
    }
    (directory, repository, stream)
}
fn split() -> SplitPolicy {
    SplitPolicy::Group {
        train_fraction: 0.6,
        validation_fraction: 0.2,
        seed: 7,
    }
}

#[test]
fn native_worker_trains_a_model_that_roundtrips_through_registry() {
    let (_directory, repository, stream) = populated();
    let request = TrainingRequest {
        engine: "histogram_gradient_boosting".into(),
        recipe: json!({"config":{"objective":"squared_error","estimators":40,"max_depth":3,"max_bins":16,"min_leaf":1,"learning_rate":0.15,"l2":1.},"labels":[]}),
        compute: serde_json::to_value(ComputeConfig::default()).unwrap(),
    };
    let job = repository
        .trigger_after_n(&stream, 30, request, split(), 1000)
        .unwrap()
        .unwrap();
    let mut worker = TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
    register_native_engines(&mut worker).unwrap();
    let artifact = worker.run(&job.id).unwrap();
    let output = load_artifact_predictions(
        &repository,
        &artifact.id,
        &TensorData {
            shape: vec![2, 1],
            values: vec![-0.8, 0.8],
        },
        &ComputeConfig::default(),
    )
    .unwrap();
    let scores = output["scores"].as_array().unwrap();
    assert!(scores[0].as_f64().unwrap() < -1.0);
    assert!(scores[1].as_f64().unwrap() > 1.0);
    assert_eq!(
        repository.get_job(&job.id).unwrap().status,
        JobStatus::Succeeded
    );
}

#[test]
fn burn_worker_resumes_optimizer_checkpoint_and_loads_prediction_artifact() {
    let (_directory, repository, stream) = populated();
    let request = TrainingRequest {
        engine: "burn".into(),
        recipe: json!({"config":{"recipe":{"architecture":"mlp","input_features":1,"hidden":8,"outputs":1,"objective":"regression"},"backend":{"backend":"cpu"},"epochs":12,"batch_size":6,"learning_rate":0.04,"seed":42,"gradient_clip":5.},"labels":[]}),
        compute: serde_json::to_value(ComputeConfig::default()).unwrap(),
    };
    let job = repository
        .trigger_after_n(&stream, 30, request, split(), 1000)
        .unwrap()
        .unwrap();
    let mut worker = TrainingWorker::new(
        repository.clone(),
        WorkerLimits {
            maximum_duration_ms: 1,
            ..WorkerLimits::default()
        },
    )
    .unwrap();
    register_burn_engine(&mut worker).unwrap();
    assert!(worker.run(&job.id).is_err());
    let stopped = repository.get_job(&job.id).unwrap();
    assert_eq!(stopped.status, JobStatus::Failed);
    assert!(
        stopped.checkpoint.is_some(),
        "last completed Burn batch must be retained"
    );
    repository.resume_job(&job.id, now_ms()).unwrap();
    let mut worker = TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
    register_burn_engine(&mut worker).unwrap();
    let artifact = worker.run(&job.id).unwrap();
    let output = load_artifact_predictions(
        &repository,
        &artifact.id,
        &TensorData {
            shape: vec![2, 1],
            values: vec![-0.8, 0.8],
        },
        &ComputeConfig::default(),
    )
    .unwrap();
    let values = output["output"]["values"].as_array().unwrap();
    assert_eq!(values.len(), 2);
    assert!(values[0].as_f64().unwrap() < -0.5);
    assert!(values[1].as_f64().unwrap() > 0.5);
    let report = &artifact.manifest["report"];
    assert!(report["final_loss"].as_f64().unwrap() < report["initial_loss"].as_f64().unwrap());
    let checkpoint = repository.get_job(&job.id).unwrap().checkpoint.unwrap();
    assert!(
        checkpoint.blob.bytes > artifact.blob.bytes,
        "training checkpoint includes optimizer while inference artifact does not"
    );
}

#[test]
fn explicit_uncompiled_native_gpu_request_is_rejected() {
    let (_directory, repository, stream) = populated();
    let request = TrainingRequest {
        engine: "isolation_forest".into(),
        recipe: json!({"config":{"trees":8,"sample_size":8,"seed":42}}),
        compute: json!({"backend":"cuda","allow_cpu_fallback":false,"memory_limit_bytes":100_000_000}),
    };
    let job = repository
        .trigger_after_n(&stream, 30, request, split(), 1000)
        .unwrap()
        .unwrap();
    let mut worker = TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
    register_native_engines(&mut worker).unwrap();
    assert!(worker.run(&job.id).is_err());
    assert!(repository.get_job(&job.id).unwrap().artifact_id.is_none());
}

#[test]
fn separate_process_worker_executes_a_durable_job() {
    let (_directory, repository, stream) = populated();
    let request = TrainingRequest {
        engine: "isolation_forest".into(),
        recipe: json!({"config":{"trees":8,"sample_size":8,"seed":42}}),
        compute: serde_json::to_value(ComputeConfig::default()).unwrap(),
    };
    let job = repository
        .trigger_after_n(&stream, 30, request, split(), 1000)
        .unwrap()
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_flow-like-ml-worker"))
        .arg("--repository")
        .arg(repository.path())
        .arg("--job")
        .arg(&job.id)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact: ModelArtifact = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(artifact.job_id, job.id);
    assert_eq!(
        repository.get_job(&job.id).unwrap().status,
        JobStatus::Succeeded
    );
}

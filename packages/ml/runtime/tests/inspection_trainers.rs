#![cfg(all(feature = "native", feature = "burn"))]
use flow_like_ml_burn as burn;
use flow_like_ml_core::{
    Annotation, BoundingBox, ComputeBackend, ComputeConfig, InstanceMask, LabelProvenance, Sample,
    TensorData,
};
use flow_like_ml_runtime::{engines::*, worker::TrainingWorker, *};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn compute() -> ComputeConfig {
    ComputeConfig {
        backend: ComputeBackend::Cpu,
        ..ComputeConfig::default()
    }
}
fn split() -> SplitPolicy {
    SplitPolicy::Group {
        train_fraction: 0.5,
        validation_fraction: 0.25,
        seed: 73,
    }
}
fn sample(index: usize, kind: &str) -> Sample {
    let size = if kind == "efficient_ad" { 256 } else { 32 };
    let mask = (0..size * size)
        .map(|pixel| {
            pixel % size >= size / 4
                && pixel % size < size * 3 / 4
                && pixel / size >= size / 4
                && pixel / size < size * 3 / 4
        })
        .collect::<Vec<_>>();
    let values = mask
        .iter()
        .enumerate()
        .map(|(pixel, foreground)| {
            if kind == "efficient_ad" {
                ((pixel % size) as f32 * 0.03).sin()
                    + ((pixel / size) as f32 * 0.04).cos()
                    + index as f32 * 0.001
            } else {
                (if *foreground { 1.0 } else { -1.0 }) + index as f32 * 0.001
            }
        })
        .collect();
    let bounds = BoundingBox {
        class_id: 0,
        x_min: 0.25,
        y_min: 0.25,
        x_max: 0.75,
        y_max: 0.75,
    };
    let annotation = match kind {
        "yolox" => Annotation::Boxes {
            boxes: vec![bounds],
        },
        "unet" => Annotation::Mask {
            width: size,
            height: size,
            classes: mask.iter().map(|value| u32::from(*value)).collect(),
        },
        "mask_rcnn" => Annotation::InstanceMasks {
            instances: vec![InstanceMask {
                instance_id: format!("instance-{index}"),
                bounds,
                width: size,
                height: size,
                foreground: mask,
            }],
        },
        "efficient_ad" => Annotation::Anomaly { is_anomaly: false },
        _ => unreachable!(),
    };
    Sample {
        id: format!("sample-{index}"),
        group_id: format!("lot-{index}"),
        stream_id: kind.into(),
        timestamp_ms: index as i64,
        window_start_ms: index as i64,
        window_end_ms: index as i64,
        input: TensorData {
            shape: vec![1, size, size],
            values,
        },
        annotation,
        provenance: LabelProvenance::Teacher {
            model: "fixture-teacher".into(),
            prompt_digest: "fixture-prompt".into(),
            confidence: Some(0.99),
        },
        outcome: None,
    }
}
fn populate(repo: &TrainingRepository, key: &StreamKey, kind: &str) {
    for index in 0..8 {
        let sample = sample(index, kind);
        let content_digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&sample.input).unwrap())
        );
        repo.record_sample(
            key,
            &TrainingSample {
                id: sample.id.clone(),
                annotation_revision: 1,
                group_id: sample.group_id.clone(),
                captured_at_ms: sample.timestamp_ms,
                label_available_at_ms: sample.timestamp_ms,
                content_digest,
                source: LabelSource::Teacher,
                accepted: true,
                payload: serde_json::to_value(sample).unwrap(),
            },
        )
        .unwrap();
    }
}
fn request(kind: &str) -> TrainingRequest {
    let (recipe, labels, task) = match kind {
        "yolox" => (
            burn::Recipe::YoloX {
                input_channels: 1,
                classes: 1,
                width_multiplier: 0.0625,
                depth_multiplier: 0.33,
            },
            vec!["defect"],
            "object_detection",
        ),
        "unet" => (
            burn::Recipe::UNet {
                input_channels: 1,
                classes: 2,
                base_channels: 2,
                depth: 1,
            },
            vec!["background", "defect"],
            "segmentation",
        ),
        "mask_rcnn" => (
            burn::Recipe::MaskRcnn {
                config: burn::MaskRcnnConfig {
                    input_channels: 1,
                    classes: 1,
                    base_channels: 2,
                    anchor_scales: vec![0.5],
                    anchor_ratios: vec![1.0],
                    roi_size: 2,
                    mask_size: 4,
                    proposals: 4,
                    training_samples: 4,
                },
            },
            vec!["defect"],
            "instance_segmentation",
        ),
        _ => unreachable!(),
    };
    let config = burn::TrainingConfig {
        recipe,
        backend: burn::BackendChoice::Cpu,
        epochs: 2,
        batch_size: 2,
        learning_rate: 0.003,
        seed: 73,
        gradient_clip: 5.0,
    };
    TrainingRequest {
        engine: "burn".into(),
        recipe: json!({"config":config,"labels":labels,"inspection_task":task,"minimum_examples":2,"minimum_examples_per_class":1}),
        compute: json!(compute()),
    }
}
fn assert_durable_inference(
    kind: &str,
    request: TrainingRequest,
) -> (tempfile::TempDir, TrainingRepository, ModelArtifact, Value) {
    let dir = tempfile::tempdir().unwrap();
    let repo = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let key = StreamKey {
        project_id: "factory".into(),
        stream_id: kind.into(),
        inspection_version: "fixture-v1".into(),
    };
    populate(&repo, &key, kind);
    let job = repo
        .trigger_after_n(&key, 8, request, split(), 1000)
        .unwrap()
        .expect("Class/normal support must make this inspection trainable");
    let mut worker = TrainingWorker::new(repo.clone(), WorkerLimits::default()).unwrap();
    register_burn_engine(&mut worker).unwrap();
    register_efficient_ad_engine(&mut worker).unwrap();
    let artifact = worker.run(&job.id).unwrap();
    assert_eq!(repo.get_job(&job.id).unwrap().status, JobStatus::Succeeded);
    let report = &artifact.manifest["report"];
    assert!(report["final_loss"].as_f64().unwrap().is_finite());
    let mut input = sample(9, kind).input;
    input.shape.insert(0, 1);
    let first = load_artifact_predictions(&repo, &artifact.id, &input, &compute()).unwrap();
    let reopened = TrainingRepository::open(dir.path().join("training.sqlite")).unwrap();
    let second = load_artifact_predictions(&reopened, &artifact.id, &input, &compute()).unwrap();
    assert_eq!(first, second, "Reloaded adapter output changed");
    (dir, reopened, artifact, first)
}

#[test]
fn dense_masks_boxes_and_instance_masks_reach_their_native_losses() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for kind in ["yolox", "unet", "mask_rcnn"] {
        let (_, _, artifact, prediction) = assert_durable_inference(kind, request(kind));
        assert_eq!(artifact.manifest["engine"], "burn");
        match kind {
            "unet" => {
                assert_eq!(prediction["output"]["shape"], json!([1, 2, 32, 32]));
                assert_eq!(prediction["classes"].as_array().unwrap().len(), 32 * 32);
            }
            "yolox" => {
                assert_eq!(prediction["detections"].as_array().unwrap().len(), 1);
                assert_eq!(prediction["output"]["shape"], json!([1, 21, 6]));
            }
            "mask_rcnn" => {
                assert_eq!(prediction["instances"].as_array().unwrap().len(), 1);
                assert_eq!(prediction["detections"].as_array().unwrap().len(), 1);
            }
            _ => unreachable!(),
        }
    }
}

fn teacher_weights() -> burn::PdnTeacherWeights {
    let convolutions = [[2, 1, 4, 4], [4, 2, 4, 4], [4, 4, 3, 3], [2, 4, 4, 4]]
        .into_iter()
        .map(|shape| {
            let count = shape.iter().product::<usize>();
            burn::ConvWeights {
                weight: burn::TensorData {
                    shape: shape.to_vec(),
                    values: (0..count).map(|i| ((i % 7) as f32 - 3.0) * 0.02).collect(),
                },
                bias: burn::TensorData {
                    shape: vec![shape[0]],
                    values: vec![0.01; shape[0]],
                },
            }
        })
        .collect();
    burn::PdnTeacherWeights {
        input_channels: 1,
        base_channels: 2,
        feature_channels: 2,
        convolutions,
    }
}
#[test]
fn efficient_ad_adapter_preserves_normal_only_calibration_and_durable_maps() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let config = burn::EfficientAdConfig {
        teacher: teacher_weights(),
        backend: burn::BackendChoice::Cpu,
        epochs: 1,
        batch_size: 2,
        learning_rate: 0.001,
        seed: 73,
        autoencoder_channels: 2,
        hard_quantile: 0.99,
        gradient_clip: 5.0,
    };
    let request = TrainingRequest {
        engine: "efficient_ad".into(),
        recipe: json!({"config":config,"labels":["normal","defect"],"inspection_task":"visual_anomaly","minimum_examples":2,"minimum_examples_per_class":1}),
        compute: json!(compute()),
    };
    let (_, _, artifact, prediction) = assert_durable_inference("efficient_ad", request);
    assert_eq!(artifact.manifest["engine"], "efficient_ad");
    assert!(
        artifact.manifest["calibration"]["teacher_student_high"]
            .as_f64()
            .unwrap()
            .is_finite()
    );
    assert_eq!(prediction["anomaly_maps"]["shape"], json!([1, 1, 256, 256]));
    assert_eq!(
        prediction["teacher_student_maps"]["shape"],
        prediction["autoencoder_maps"]["shape"]
    );
    assert!(prediction["scores"][0].as_f64().unwrap().is_finite());
}

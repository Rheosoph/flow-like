use super::*;
use std::sync::Mutex;
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(feature = "onnx-export")]
#[test]
fn onnx_export_requires_a_supported_graph_and_matching_shape() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = sequence_data();
    for (recipe, supported) in [
        (
            Recipe::Lstm {
                input_features: 1,
                hidden: 4,
                outputs: 2,
                objective: Objective::Classification,
            },
            false,
        ),
        (
            Recipe::Mlp {
                input_features: 4,
                hidden: 4,
                outputs: 2,
                objective: Objective::Classification,
            },
            true,
        ),
    ] {
        let mut dataset = data.clone();
        if supported {
            dataset.inputs.shape = vec![16, 4];
        }
        let dir = tempfile::tempdir().unwrap();
        train(
            &config(recipe, 1),
            &dataset,
            None,
            dir.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        let predictor = Predictor::load(dir.path(), BackendChoice::Cpu).unwrap();
        let path = dir.path().join("model.onnx");
        let result = predictor.export_onnx(&dataset.inputs, &path);
        assert_eq!(result.is_ok(), supported);
        assert_eq!(
            path.is_file(),
            supported,
            "Unsupported exports must not publish an artifact"
        );
        if supported {
            assert!(std::fs::metadata(&path).unwrap().len() > 100);
            let mut malformed = dataset.inputs.clone();
            malformed.shape = vec![8, 8];
            assert!(
                predictor
                    .export_onnx(&malformed, dir.path().join("bad.onnx"))
                    .is_err()
            );
            assert!(!dir.path().join("bad.onnx").exists());
        }
    }
}

fn config(recipe: Recipe, epochs: usize) -> TrainingConfig {
    TrainingConfig {
        recipe,
        backend: BackendChoice::Cpu,
        epochs,
        batch_size: 8,
        learning_rate: 0.03,
        seed: 73,
        gradient_clip: 5.0,
    }
}
fn sequence_data() -> TensorDataset {
    let mut values = Vec::new();
    let mut labels = Vec::new();
    for i in 0..16 {
        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
        labels.push((i % 2) as i64);
        for t in 0..4 {
            values.push(sign * (1.0 + t as f32 * 0.1));
        }
    }
    TensorDataset {
        inputs: TensorData {
            shape: vec![16, 4, 1],
            values,
        },
        targets: Targets::Classes { values: labels },
    }
}
fn fit_and_check(recipe: Recipe, data: &TensorDataset, epochs: usize) -> TrainingReport {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = config(recipe, epochs);
    if matches!(
        settings.recipe,
        Recipe::ResNet18 { .. } | Recipe::UNet { .. }
    ) {
        settings.learning_rate = 0.005;
    }
    let report = train(
        &settings,
        data,
        Some(data),
        dir.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(
        report.final_loss < report.initial_loss,
        "loss did not decrease: {report:?}"
    );
    let first = Predictor::load(dir.path(), BackendChoice::Cpu)
        .unwrap()
        .predict(&data.inputs)
        .unwrap();
    let second = Predictor::load(dir.path(), BackendChoice::Cpu)
        .unwrap()
        .predict(&data.inputs)
        .unwrap();
    assert_eq!(
        first.output, second.output,
        "Saved weights must reload deterministically"
    );
    if let (Some(actual), Targets::Classes { values: expected }) = (first.classes, &data.targets) {
        let correct = actual.iter().zip(expected).filter(|(a, b)| a == b).count();
        assert!(
            correct * 10 >= expected.len() * 9,
            "classifier did not learn fixture: {actual:?}"
        );
    }
    report
}
#[test]
fn temporal_models_learn_and_roundtrip_on_cpu() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = sequence_data();
    let objective = Objective::Classification;
    let recipes = vec![
        Recipe::Lstm {
            input_features: 1,
            hidden: 4,
            outputs: 2,
            objective,
        },
        Recipe::Gru {
            input_features: 1,
            hidden: 4,
            outputs: 2,
            objective,
        },
        Recipe::Cnn1d {
            input_features: 1,
            hidden: 4,
            outputs: 2,
            objective,
        },
        Recipe::Tcn {
            input_features: 1,
            hidden: 4,
            levels: 2,
            outputs: 2,
            objective,
        },
    ];
    for recipe in recipes {
        fit_and_check(recipe, &data, 12);
    }
}

#[test]
fn sequence_reconstruction_video_and_fusion_learn_on_cpu() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        probe_backend(&BackendChoice::Cpu)
            .unwrap()
            .max_absolute_error
            < 1e-6
    );
    let mut reconstruction = sequence_data();
    reconstruction.targets = Targets::Reconstruction;
    fit_and_check(
        Recipe::LstmAutoencoder {
            input_features: 1,
            hidden: 6,
            latent: 3,
        },
        &reconstruction,
        20,
    );
    let inputs = TensorData {
        shape: vec![16, 2, 1, 4, 4],
        values: (0..16)
            .flat_map(|i| vec![if i % 2 == 0 { -1.0 } else { 1.0 }; 32])
            .collect(),
    };
    let video = TensorDataset {
        inputs,
        targets: Targets::Classes {
            values: (0..16).map(|i| i % 2).collect(),
        },
    };
    fit_and_check(
        Recipe::CnnLstm {
            input_channels: 1,
            cnn_channels: 2,
            hidden: 4,
            outputs: 2,
            objective: Objective::Classification,
        },
        &video,
        20,
    );
    let fusion = TensorDataset {
        inputs: TensorData {
            shape: vec![16, 17],
            values: (0..16)
                .flat_map(|i| {
                    let image = if i % 2 == 0 { -1.0 } else { 1.0 };
                    let sensor = if i % 4 < 2 { -1.0 } else { 1.0 };
                    let mut values = vec![image; 16];
                    values.push(sensor);
                    values
                })
                .collect(),
        },
        targets: Targets::Dense {
            tensor: TensorData {
                shape: vec![16, 1],
                values: (0..16)
                    .map(|i| {
                        (if i % 2 == 0 { -1.0 } else { 1.0 }) + (if i % 4 < 2 { -2.0 } else { 2.0 })
                    })
                    .collect(),
            },
        },
    };
    let report = fit_and_check(
        Recipe::ImageSensorFusion {
            input_channels: 1,
            height: 4,
            width: 4,
            sensor_features: 1,
            cnn_channels: 2,
            hidden: 8,
            outputs: 1,
            objective: Objective::Regression,
        },
        &fusion,
        30,
    );
    assert!(
        report.final_loss < report.initial_loss * 0.1,
        "Both modalities should explain the target: {report:?}"
    );
}
#[test]
fn regression_and_autoencoders_learn_on_cpu() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let inputs = TensorData {
        shape: vec![16, 2],
        values: (0..16)
            .flat_map(|i| {
                let v = i as f32 / 16.0;
                [v, 1.0 - v]
            })
            .collect(),
    };
    let regression = TensorDataset {
        inputs: inputs.clone(),
        targets: Targets::Dense {
            tensor: TensorData {
                shape: vec![16, 1],
                values: (0..16).map(|i| i as f32 / 8.0 + 0.25).collect(),
            },
        },
    };
    fit_and_check(
        Recipe::Mlp {
            input_features: 2,
            hidden: 8,
            outputs: 1,
            objective: Objective::Regression,
        },
        &regression,
        20,
    );
    fit_and_check(
        Recipe::DenseAutoencoder {
            input_features: 2,
            hidden: 4,
            latent: 1,
        },
        &TensorDataset {
            inputs,
            targets: Targets::Reconstruction,
        },
        20,
    );
    let mut sequence = sequence_data();
    sequence.targets = Targets::Reconstruction;
    fit_and_check(
        Recipe::Conv1dAutoencoder {
            input_features: 1,
            hidden: 4,
            latent: 2,
        },
        &sequence,
        15,
    );
}
#[test]
fn cancellation_checkpoints_and_resume_matches_continuous_training() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = sequence_data();
    data.inputs.shape = vec![16, 4];
    let config = config(
        Recipe::Mlp {
            input_features: 4,
            hidden: 6,
            outputs: 2,
            objective: Objective::Classification,
        },
        8,
    );
    let complete = tempfile::tempdir().unwrap();
    let interrupted = tempfile::tempdir().unwrap();
    train(
        &config,
        &data,
        None,
        complete.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    let token = CancellationToken::new();
    let err = train(&config, &data, None, interrupted.path(), &token, |p| {
        if p.steps == 3 {
            token.cancel()
        }
    })
    .unwrap_err();
    assert!(matches!(err, Error::Cancelled));
    Predictor::load(interrupted.path(), BackendChoice::Cpu)
        .unwrap()
        .predict(&data.inputs)
        .unwrap();
    let mut automatic_resume = config.clone();
    automatic_resume.backend = BackendChoice::Auto;
    let report = resume(
        &automatic_resume,
        &data,
        None,
        interrupted.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(report.steps, 16);
    assert_eq!(report.backend, Some(BackendChoice::Cpu));
    let a = Predictor::load(complete.path(), BackendChoice::Cpu)
        .unwrap()
        .predict(&data.inputs)
        .unwrap();
    let b = Predictor::load(interrupted.path(), BackendChoice::Cpu)
        .unwrap()
        .predict(&data.inputs)
        .unwrap();
    for (a, b) in a.output.values.iter().zip(&b.output.values) {
        assert!(
            (a - b).abs() < 1e-5,
            "resume changed optimizer trajectory: {a} vs {b}"
        );
    }
    let mut changed = data.clone();
    changed.inputs.values[0] += 0.1;
    assert!(
        resume(
            &config,
            &changed,
            None,
            interrupted.path(),
            &CancellationToken::new(),
            |_| {}
        )
        .is_err()
    );
}

#[test]
fn automatic_training_records_actual_backend_and_preserves_requested_mode() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = sequence_data();
    data.inputs.shape = vec![16, 4];
    let mut settings = config(
        Recipe::Mlp {
            input_features: 4,
            hidden: 4,
            outputs: 2,
            objective: Objective::Classification,
        },
        1,
    );
    settings.backend = BackendChoice::Auto;
    let directory = tempfile::tempdir().unwrap();
    let report = train(
        &settings,
        &data,
        None,
        directory.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    let selected = report.backend.unwrap();
    eprintln!("Automatic training backend: {selected:?}");
    assert_ne!(selected, BackendChoice::Auto);
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join(MANIFEST_FILE)).unwrap())
            .unwrap();
    assert_eq!(
        state["config"]["backend"],
        serde_json::to_value(&selected).unwrap()
    );
    assert_eq!(
        state["requested_backend"],
        serde_json::to_value(BackendChoice::Auto).unwrap()
    );
    let predictor = Predictor::load(directory.path(), BackendChoice::Auto).unwrap();
    assert_eq!(predictor.backend(), &selected);
    assert_eq!(
        predictor.predict(&data.inputs).unwrap().output.shape,
        vec![16, 2]
    );
}
#[test]
fn image_models_train_on_cpu_and_roundtrip() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut inputs = Vec::new();
    let mut labels = Vec::new();
    let mut masks = Vec::new();
    for i in 0..8 {
        let class = (i % 2) as i64;
        labels.push(class);
        for y in 0..8 {
            for x in 0..8 {
                let foreground = if class == 0 { x >= 4 } else { y >= 4 };
                inputs.push(if foreground { 1.0 } else { -1.0 });
                masks.push(i64::from(foreground));
                let _ = y;
            }
        }
    }
    let inputs = TensorData {
        shape: vec![8, 1, 8, 8],
        values: inputs,
    };
    let data = TensorDataset {
        inputs: inputs.clone(),
        targets: Targets::Classes { values: labels },
    };
    fit_and_check(
        Recipe::ResNet18 {
            input_channels: 1,
            classes: 2,
            base_channels: 4,
        },
        &data,
        50,
    );
    let data = TensorDataset {
        inputs,
        targets: Targets::Segmentation {
            shape: vec![8, 8, 8],
            values: masks,
        },
    };
    fit_and_check(
        Recipe::UNet {
            input_channels: 1,
            classes: 2,
            base_channels: 4,
            depth: 1,
        },
        &data,
        30,
    );
}
#[test]
fn rejects_bad_shapes_labels_and_disabled_backends() {
    let recipe = Recipe::Gru {
        input_features: 1,
        hidden: 4,
        outputs: 2,
        objective: Objective::Classification,
    };
    let mut data = sequence_data();
    data.inputs.values[0] = f32::NAN;
    assert!(data.validate(&recipe).is_err());
    let mut data = sequence_data();
    data.targets = Targets::Classes {
        values: vec![-1; 16],
    };
    assert!(data.validate(&recipe).is_err());
    assert!(
        TensorData {
            shape: vec![usize::MAX, 2],
            values: vec![]
        }
        .validate()
        .is_err()
    );
    #[cfg(not(ml_backend_cuda))]
    assert!(crate::backend::device(&BackendChoice::Cuda { device: 0 }, false).is_err());
}

#[test]
fn yolox_trains_boxes_and_applies_inference_thresholds_on_cpu() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut pixels = Vec::new();
    for _ in 0..4 {
        for y in 0..32 {
            for x in 0..32 {
                pixels.push(if (8..24).contains(&x) && (8..24).contains(&y) {
                    1.0
                } else {
                    0.0
                });
            }
        }
    }
    let data = TensorDataset {
        inputs: TensorData {
            shape: vec![4, 1, 32, 32],
            values: pixels,
        },
        targets: Targets::Boxes {
            values: vec![
                vec![BoundingBox {
                    class_id: 0,
                    x_min: 0.25,
                    y_min: 0.25,
                    x_max: 0.75,
                    y_max: 0.75
                }];
                4
            ],
        },
    };
    let recipe = Recipe::YoloX {
        input_channels: 1,
        classes: 1,
        width_multiplier: 0.0625,
        depth_multiplier: 0.33,
    };
    let mut cfg = config(recipe, 20);
    cfg.batch_size = 4;
    cfg.learning_rate = 0.005;
    let dir = tempfile::tempdir().unwrap();
    let report = train(
        &cfg,
        &data,
        None,
        dir.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(
        report.final_loss < report.initial_loss,
        "Detector did not learn: {report:?}"
    );
    let predictor = Predictor::load(dir.path(), BackendChoice::Cpu).unwrap();
    let output = predictor
        .predict_with_options(
            &data.inputs,
            &DetectionOptions {
                confidence_threshold: 0.0,
                iou_threshold: 0.45,
                max_detections: 2,
            },
        )
        .unwrap();
    assert!(
        output
            .detections
            .unwrap()
            .iter()
            .all(|boxes| !boxes.is_empty() && boxes.len() <= 2)
    );
    let rejected = predictor
        .predict_with_options(
            &data.inputs,
            &DetectionOptions {
                confidence_threshold: 1.0,
                iou_threshold: 0.45,
                max_detections: 2,
            },
        )
        .unwrap();
    assert!(rejected.detections.unwrap().iter().all(Vec::is_empty));
}

#[test]
fn mobile_classifier_recipes_support_cpu_backward_and_reload() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let inputs = TensorData {
        shape: vec![2, 1, 32, 32],
        values: (0..2048)
            .map(|i| if i < 1024 { 0.25 } else { 0.75 })
            .collect(),
    };
    let data = TensorDataset {
        inputs,
        targets: Targets::Classes { values: vec![0, 1] },
    };
    for recipe in [
        Recipe::MobileNetV2 {
            input_channels: 1,
            classes: 2,
            width_multiplier: 0.125,
        },
        Recipe::EfficientNet {
            input_channels: 1,
            classes: 2,
            width_multiplier: 0.125,
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config(recipe, 2);
        cfg.batch_size = 2;
        cfg.learning_rate = 0.001;
        let report = train(
            &cfg,
            &data,
            None,
            dir.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(report.final_loss.is_finite());
        let result = Predictor::load(dir.path(), BackendChoice::Cpu)
            .unwrap()
            .predict(&data.inputs)
            .unwrap();
        assert_eq!(result.output.shape, vec![2, 2]);
    }
}

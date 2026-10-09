use super::*;
use crate::{
    BackendChoice, CancellationToken, FineTuneOptions, Predictor, Targets, TensorDataset,
    TrainingConfig, resume, train, train_from_pretrained,
};
use burn::module::{Module, ModuleVisitor, Param};
use safetensors::tensor::{TensorView, serialize_to_file};
use std::collections::BTreeMap;

fn config(classes: usize, epochs: usize) -> TrainingConfig {
    TrainingConfig {
        recipe: Recipe::ResNet18 {
            input_channels: 1,
            classes,
            base_channels: 2,
        },
        backend: BackendChoice::Cpu,
        epochs,
        batch_size: 2,
        learning_rate: 0.01,
        seed: 103,
        gradient_clip: 5.0,
    }
}
fn dataset(classes: usize) -> TensorDataset {
    TensorDataset {
        inputs: crate::TensorData {
            shape: vec![classes * 2, 1, 8, 8],
            values: (0..classes * 2 * 64)
                .map(|index| ((index % 64) as f32 * 0.19 + (index / 64 % classes) as f32).sin())
                .collect(),
        },
        targets: Targets::Classes {
            values: (0..classes * 2).map(|i| (i % classes) as i64).collect(),
        },
    }
}
fn load_model(path: &Path, recipe: &Recipe, device: &Device) -> Model {
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join(crate::MANIFEST_FILE)).unwrap()).unwrap();
    Model::new(recipe, device)
        .unwrap()
        .try_load_file(
            path.join(state["checkpoint"].as_str().unwrap())
                .join("model.bpk"),
        )
        .unwrap()
}
fn parameters(model: &Model) -> BTreeMap<String, Vec<f32>> {
    #[derive(Default)]
    struct Values {
        tensors: BTreeMap<String, Vec<f32>>,
        path: Vec<String>,
    }
    impl ModuleVisitor for Values {
        fn enter_module(&mut self, name: &str, _: &str) {
            self.path.push(name.to_owned());
        }
        fn exit_module(&mut self, _: &str, _: &str) {
            self.path.pop();
        }
        fn visit_float<const D: usize>(&mut self, tensor: &Param<Tensor<D>>) {
            let previous = self.tensors.insert(
                self.path.join("."),
                tensor.val().into_data().iter::<f32>().collect(),
            );
            assert!(previous.is_none(), "Parameter paths must be unique");
        }
    }
    let mut values = Values::default();
    model.visit(&mut values);
    assert!(!values.tensors.is_empty());
    values.tensors
}

#[test]
fn rng_phases_isolate_initialization_dropout_and_allow_predictor_loads_in_callbacks() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let _lock = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut config = config(2, 2);
    config.recipe = Recipe::MobileNetV2 {
        input_channels: 1,
        classes: 2,
        width_multiplier: 0.125,
    };
    config.learning_rate = 0.001;
    let data = TensorDataset {
        inputs: crate::TensorData {
            shape: vec![2, 1, 32, 32],
            values: (0..2048)
                .map(|index| if index < 1024 { 0.25 } else { 0.75 })
                .collect(),
        },
        targets: Targets::Classes { values: vec![0, 1] },
    };
    let baseline = tempfile::tempdir().unwrap();
    let expected = train(
        &config,
        &data,
        None,
        baseline.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    let competing = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let draws = Arc::new(AtomicUsize::new(0));
    let worker_stop = stop.clone();
    let worker_draws = draws.clone();
    let worker = std::thread::spawn(move || {
        let device = Device::flex();
        while !worker_stop.load(Ordering::Acquire) {
            {
                let _guard = crate::execution::lock(None).unwrap();
                device.seed(987_654);
                let _ = Tensor::<1>::random([128], burn::tensor::Distribution::Default, &device)
                    .into_data();
                worker_draws.fetch_add(1, Ordering::Release);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    while draws.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }
    let result = train(
        &config,
        &data,
        None,
        competing.path(),
        &CancellationToken::new(),
        |_| {
            // Loading a model also takes the RNG guard; progress must run after releasing it.
            let (tx, rx) = std::sync::mpsc::channel();
            let directory = baseline.path().to_path_buf();
            std::thread::spawn(move || {
                let loaded = Predictor::load(directory, BackendChoice::Cpu).is_ok();
                let _ = tx.send(loaded);
            });
            assert!(
                rx.recv_timeout(std::time::Duration::from_secs(10))
                    .expect("Progress callback must not hold the RNG guard")
            );
        },
    );
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    let actual = result.unwrap();
    assert!(draws.load(Ordering::Acquire) > 1);
    assert_eq!(actual.final_loss, expected.final_loss);
    let device = Device::flex();
    assert_eq!(
        parameters(&load_model(competing.path(), &config.recipe, &device)),
        parameters(&load_model(baseline.path(), &config.recipe, &device))
    );
}

#[test]
fn final_progress_cancellation_keeps_a_resumable_generic_checkpoint() {
    let _lock = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let config = config(2, 1);
    let data = dataset(2);
    let directory = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    assert!(matches!(
        train(
            &config,
            &data,
            None,
            directory.path(),
            &cancellation,
            |event| {
                if event.steps == 2 {
                    cancellation.cancel();
                }
            }
        ),
        Err(Error::Cancelled)
    ));
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join(crate::MANIFEST_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(state["next_epoch"], 0);
    assert_eq!(state["next_batch"], 2);
    let cancellation = CancellationToken::new();
    assert!(matches!(
        resume(
            &config,
            &data,
            None,
            directory.path(),
            &cancellation,
            |_| cancellation.cancel()
        ),
        Err(Error::Cancelled)
    ));
    let completed = resume(
        &config,
        &data,
        None,
        directory.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(completed.steps, 2);
    assert_eq!(completed.completed_epochs, 1);
    assert_eq!(completed.history.len(), 1);
}

#[test]
fn pretrained_new_labels_preserve_frozen_backbone_and_resume_the_new_optimizer() {
    let _lock = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let source = tempfile::tempdir().unwrap();
    let source_config = config(2, 1);
    let source_data = dataset(2);
    train(
        &source_config,
        &source_data,
        None,
        source.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    let device = Device::flex().autodiff();
    let source_weights = parameters(&load_model(source.path(), &source_config.recipe, &device));
    let target_config = config(3, 3);
    let data = dataset(3);
    let options = FineTuneOptions {
        replace_head: true,
        freeze_backbone: true,
    };
    let full = tempfile::tempdir().unwrap();
    let report = train_from_pretrained(
        &target_config,
        &data,
        Some(&data),
        source.path(),
        &options,
        full.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(report.steps, 9, "Fine-tuning starts a fresh step cursor");
    let full_model = load_model(full.path(), &target_config.recipe, &device);
    let full_weights = parameters(&full_model);
    for (name, values) in &source_weights {
        if !name.contains("classifier") {
            assert_eq!(
                full_weights.get(name).unwrap(),
                values,
                "Frozen parameter or running state changed: {name}"
            );
        }
    }
    let loaded_source = load_model(source.path(), &source_config.recipe, &device);
    device.seed(target_config.seed);
    let initial_model = loaded_source
        .initialize_fine_tuning(&target_config.recipe, &options, &device)
        .unwrap();
    let initial_weights = parameters(&initial_model);
    assert!(
        full_weights
            .iter()
            .any(|(name, values)| name.contains("classifier") && values != &initial_weights[name])
    );
    let source_features = Predictor::load(source.path(), BackendChoice::Cpu)
        .unwrap()
        .features(&data.inputs)
        .unwrap();
    let predictor = Predictor::load(full.path(), BackendChoice::Cpu).unwrap();
    assert_eq!(predictor.features(&data.inputs).unwrap(), source_features);
    assert_eq!(
        predictor.predict(&data.inputs).unwrap().output.shape,
        vec![6, 3]
    );
    let origin = predictor.fine_tune_provenance().unwrap();
    assert_eq!(origin.source_recipe, source_config.recipe);
    assert_eq!(origin.options, options);
    assert_eq!(origin.source_model_sha256.len(), 64);
    assert_eq!(
        parameters(&load_model(source.path(), &source_config.recipe, &device)),
        source_weights
    );

    let partial = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    assert!(matches!(
        train_from_pretrained(
            &target_config,
            &data,
            Some(&data),
            source.path(),
            &options,
            partial.path(),
            &cancellation,
            |progress| {
                if progress.steps == 2 {
                    cancellation.cancel();
                }
            }
        ),
        Err(Error::Cancelled)
    ));
    let resumed = resume(
        &target_config,
        &data,
        Some(&data),
        partial.path(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(resumed.steps, report.steps);
    assert!((resumed.final_loss - report.final_loss).abs() < 1e-6);
    assert_eq!(
        parameters(&load_model(partial.path(), &target_config.recipe, &device)),
        full_weights
    );
    let mut changed = data.clone();
    changed.inputs.values[0] += 0.01;
    assert!(
        resume(
            &target_config,
            &changed,
            Some(&data),
            partial.path(),
            &CancellationToken::new(),
            |_| {}
        )
        .is_err()
    );
    assert!(
        crate::validate_fine_tune(
            &source_config.recipe,
            &target_config.recipe,
            &FineTuneOptions {
                replace_head: false,
                freeze_backbone: false
            }
        )
        .is_err()
    );
}

type Weight = (Dtype, Vec<usize>, Vec<u8>);
fn f32_weight(
    weights: &mut BTreeMap<String, Weight>,
    name: String,
    shape: Vec<usize>,
    values: Vec<f32>,
) {
    weights.insert(
        name,
        (
            Dtype::F32,
            shape,
            values.into_iter().flat_map(f32::to_le_bytes).collect(),
        ),
    );
}
fn conv_norm(
    weights: &mut BTreeMap<String, Weight>,
    conv: &str,
    norm: &str,
    input: usize,
    output: usize,
    kernel: usize,
) {
    f32_weight(
        weights,
        format!("{conv}.weight"),
        vec![output, input, kernel, kernel],
        vec![0.0; output * input * kernel * kernel],
    );
    for suffix in ["weight", "bias", "running_mean", "running_var"] {
        let value = if suffix == "weight" || suffix == "running_var" {
            1.0
        } else {
            0.0
        };
        f32_weight(
            weights,
            format!("{norm}.{suffix}"),
            vec![output],
            vec![value; output],
        );
    }
    weights.insert(
        format!("{norm}.num_batches_tracked"),
        (Dtype::I64, vec![], 0i64.to_le_bytes().to_vec()),
    );
}
fn torchvision_fixture() -> BTreeMap<String, Weight> {
    let mut weights = BTreeMap::new();
    conv_norm(&mut weights, "conv1", "bn1", 3, 64, 7);
    let mut input = 64;
    for stage in 1..=4 {
        let output = 64 << (stage - 1);
        for block in 0..2 {
            let prefix = format!("layer{stage}.{block}");
            conv_norm(
                &mut weights,
                &format!("{prefix}.conv1"),
                &format!("{prefix}.bn1"),
                input,
                output,
                3,
            );
            conv_norm(
                &mut weights,
                &format!("{prefix}.conv2"),
                &format!("{prefix}.bn2"),
                output,
                output,
                3,
            );
            if stage > 1 && block == 0 {
                conv_norm(
                    &mut weights,
                    &format!("{prefix}.downsample.0"),
                    &format!("{prefix}.downsample.1"),
                    input,
                    output,
                    1,
                );
            }
            input = output;
        }
    }
    f32_weight(
        &mut weights,
        "layer4.1.bn2.bias".into(),
        vec![512],
        (0..512).map(|i| 1.0 + i as f32 / 512.0).collect(),
    );
    f32_weight(
        &mut weights,
        "layer4.1.bn2.running_mean".into(),
        vec![512],
        vec![0.5; 512],
    );
    f32_weight(
        &mut weights,
        "layer4.1.bn2.running_var".into(),
        vec![512],
        vec![4.0; 512],
    );
    let mut head = vec![0.0; 2 * 512];
    head[0] = 2.0;
    head[2 * 512 - 1] = 3.0;
    f32_weight(&mut weights, "fc.weight".into(), vec![2, 512], head);
    f32_weight(&mut weights, "fc.bias".into(), vec![2], vec![0.25, -0.5]);
    weights
}
fn write_weights(weights: &BTreeMap<String, Weight>, path: &Path) {
    serialize_to_file(
        weights.iter().map(|(name, (dtype, shape, data))| {
            (name, TensorView::new(*dtype, shape.clone(), data).unwrap())
        }),
        None,
        path,
    )
    .unwrap();
}
fn metadata() -> PretrainedWeightsMetadata {
    PretrainedWeightsMetadata {
        source: "test fixture with torchvision ResNet-18 state_dict layout".into(),
        license: Some("MIT".into()),
        weights_license: None,
        revision: Some("fixture-v1".into()),
        expected_sha256: None,
    }
}

#[test]
fn torchvision_safetensors_import_maps_batch_norm_and_transposed_classifier() {
    let _lock = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let weights = torchvision_fixture();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("resnet18.safetensors");
    write_weights(&weights, &path);
    let mut metadata = metadata();
    metadata.expected_sha256 = Some(file_sha256(&path).unwrap());
    let output = tempfile::tempdir().unwrap();
    let info = import_torchvision_resnet18_safetensors(&path, output.path(), &metadata).unwrap();
    assert_eq!(
        info.recipe,
        Recipe::ResNet18 {
            input_channels: 3,
            classes: 2,
            base_channels: 64
        }
    );
    let device = Device::flex();
    let model = load_model(output.path(), &info.recipe, &device).valid();
    let input = crate::TensorData {
        shape: vec![1, 3, 8, 8],
        values: vec![0.0; 3 * 64],
    };
    let output = model
        .forward(&input, &device)
        .unwrap()
        .prediction(&info.recipe, &input, &crate::DetectionOptions::default())
        .unwrap();
    let shift = 0.5f32 / (4.0f32 + 1e-5).sqrt();
    let logits = [
        2.0 * (1.0 - shift) + 0.25,
        3.0 * (1.0 + 511.0 / 512.0 - shift) - 0.5,
    ];
    let expected = 1.0 / (1.0 + (logits[1] - logits[0]).exp());
    assert!((output.output.values[0] - expected).abs() < 1e-6);
    assert_eq!(info.provenance.sha256, metadata.expected_sha256.unwrap());
}

#[test]
fn safetensors_import_rejects_wrong_digest_nonfinite_weights_and_unknown_keys() {
    let _lock = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.safetensors");
    std::fs::write(&path, b"not a checkpoint").unwrap();
    let mut origin = metadata();
    origin.expected_sha256 = Some("0".repeat(64));
    let destination = directory.path().join("artifact");
    assert!(
        import_torchvision_resnet18_safetensors(&path, &destination, &origin)
            .unwrap_err()
            .to_string()
            .contains("SHA256")
    );
    assert!(!destination.join(crate::MANIFEST_FILE).exists());
    let device = Device::flex();
    let mut weights = BTreeMap::new();
    f32_weight(&mut weights, "value".into(), vec![1], vec![f32::NAN]);
    write_weights(&weights, &path);
    let bytes = std::fs::read(&path).unwrap();
    let mut reader = SafeTensorReader::new(&bytes).unwrap();
    assert!(reader.tensor("value", [1], &device).is_err());
    f32_weight(&mut weights, "value".into(), vec![1], vec![1.0]);
    write_weights(&weights, &path);
    let bytes = std::fs::read(&path).unwrap();
    assert!(SafeTensorReader::new(&bytes).unwrap().finish().is_err());
    assert!(
        SafeTensorReader::new(&bytes)
            .unwrap()
            .tensor("value", [2], &device)
            .is_err()
    );
}

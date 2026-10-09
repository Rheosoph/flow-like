use crate::{
    backend,
    engine::{DirectoryLock, shuffled_indices, sync_directory, sync_record},
    pretrained::{SafeTensorReader, file_sha256, read_import_weights},
    *,
};
use burn::{
    grad_clipping::GradientClippingConfig,
    module::Module,
    optim::{AdamConfig, GradientsParams, ModuleOptimizer},
    tensor::{Device, Tensor, TensorData as BurnData},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const SAM2_MANIFEST_FILE: &str = "sam2-state.json";
static CHECKPOINT_ID: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    version: u32,
    model: Sam2Config,
    provenance: Sam2WeightsProvenance,
    source_model_sha256: Option<String>,
    training: Option<Sam2TrainingConfig>,
    dataset_sha256: Option<String>,
    checkpoint: String,
    next_epoch: usize,
    next_batch: usize,
    report: TrainingReport,
}
fn record_error(error: impl std::fmt::Display) -> Error {
    Error::Record(error.to_string())
}
fn read_state(directory: &Path) -> Result<State> {
    let state: State =
        serde_json::from_reader(fs::File::open(directory.join(SAM2_MANIFEST_FILE))?)?;
    if state.version != 1 {
        return Err(Error::Invalid("Unsupported SAM artifact version".into()));
    }
    state.model.validate()?;
    state.provenance.validate()?;
    if let Some(config) = &state.training {
        config.validate()?;
        if config.model != state.model {
            return Err(Error::Invalid(
                "SAM model and training configuration differ".into(),
            ));
        }
    }
    for digest in [&state.source_model_sha256, &state.dataset_sha256]
        .into_iter()
        .flatten()
    {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("Invalid SAM artifact digest".into()));
        }
    }
    checked_checkpoint(directory, &state.checkpoint)?;
    Ok(state)
}
fn checked_checkpoint(directory: &Path, name: &str) -> Result<PathBuf> {
    if !name.starts_with("sam2-checkpoint-") || name.contains(['/', '\\']) || name.contains("..") {
        return Err(Error::Invalid("Invalid SAM checkpoint name".into()));
    }
    Ok(directory.join(name))
}
fn checkpoint(
    directory: &Path,
    model: &Sam2,
    optimizer: Option<&ModuleOptimizer>,
    state: &mut State,
) -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(record_error)?
        .as_nanos();
    let name = format!(
        "sam2-checkpoint-{}-{stamp}-{}",
        state.report.steps,
        CHECKPOINT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let target = directory.join(&name);
    fs::create_dir(&target)?;
    model
        .clone()
        .save_file(target.join("model.bpk"))
        .map_err(record_error)?;
    sync_record(&target.join("model.bpk"))?;
    if let Some(optimizer) = optimizer {
        optimizer
            .save(target.join("optimizer.bpk"))
            .map_err(record_error)?;
        sync_record(&target.join("optimizer.bpk"))?;
    }
    sync_directory(&target)?;
    sync_directory(directory)?;
    state.checkpoint = name;
    let temporary = directory.join("sam2-state.json.pending");
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&file, state)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, directory.join(SAM2_MANIFEST_FILE))?;
    sync_directory(directory)?;
    Ok(())
}

/// Import the image path of the official SAM 2.1 Hiera-T checkpoint without executing Python.
pub fn import_sam2_weights(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    metadata: &PretrainedWeightsMetadata,
) -> Result<Sam2ImportedModelInfo> {
    let directory = artifact_dir.as_ref();
    let _lock = DirectoryLock::acquire(directory)?;
    if directory.join(SAM2_MANIFEST_FILE).exists() {
        return Err(Error::Invalid(
            "SAM artifact directory already contains a model".into(),
        ));
    }
    let (bytes, sha256, source_format) =
        read_import_weights(source_path.as_ref(), metadata, 512 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let config = Sam2Config {
        variant: Sam2Variant::HieraTiny,
    };
    let _rng = crate::execution::lock(None)?;
    let device = backend::device(&backend::resolve_backend(&BackendChoice::Auto)?, false)?;
    let model = Sam2::new(&config, &device)?.import(&mut reader, &device)?;
    consume_video_weights(&mut reader)?;
    reader.finish()?;
    let provenance = Sam2WeightsProvenance {
        metadata: metadata.clone(),
        format: format!("sam2_1_hiera_tiny_image:{source_format}"),
        sha256,
    };
    let mut state = State {
        version: 1,
        model: config.clone(),
        provenance: provenance.clone(),
        source_model_sha256: None,
        training: None,
        dataset_sha256: None,
        checkpoint: String::new(),
        next_epoch: 0,
        next_batch: 0,
        report: TrainingReport {
            backend: None,
            completed_epochs: 0,
            steps: 0,
            initial_loss: 0.0,
            final_loss: 0.0,
            validation_loss: None,
            history: Vec::new(),
        },
    };
    checkpoint(directory, &model, None, &mut state)?;
    Ok(Sam2ImportedModelInfo { config, provenance })
}

// The image predictor does not use temporal memory. Accept only the named, shaped tensors
// from the published Hiera-T video branch; every other extra key remains an import error.
fn consume_video_weights(reader: &mut SafeTensorReader<'_>) -> Result<()> {
    let mut expected = std::collections::BTreeMap::<String, Vec<usize>>::new();
    let mut add = |name: String, shape: Vec<usize>| {
        expected.insert(name, shape);
    };
    for (name, shape) in [
        ("maskmem_tpos_enc", vec![7, 1, 1, 64]),
        ("no_mem_pos_enc", vec![1, 1, 256]),
        ("no_obj_ptr", vec![1, 256]),
        ("no_obj_embed_spatial", vec![1, 64]),
    ] {
        add(name.into(), shape);
    }
    for layer in 0..4 {
        let prefix = format!("memory_attention.layers.{layer}");
        for norm in ["norm1", "norm2", "norm3"] {
            for suffix in ["weight", "bias"] {
                add(format!("{prefix}.{norm}.{suffix}"), vec![256]);
            }
        }
        for (name, input, output) in [("linear1", 256, 2048), ("linear2", 2048, 256)] {
            add(format!("{prefix}.{name}.weight"), vec![output, input]);
            add(format!("{prefix}.{name}.bias"), vec![output]);
        }
        for attention in ["self_attn", "cross_attn_image"] {
            for projection in ["q_proj", "k_proj", "v_proj", "out_proj"] {
                let input = if attention == "cross_attn_image"
                    && matches!(projection, "k_proj" | "v_proj")
                {
                    64
                } else {
                    256
                };
                add(
                    format!("{prefix}.{attention}.{projection}.weight"),
                    vec![256, input],
                );
                add(format!("{prefix}.{attention}.{projection}.bias"), vec![256]);
            }
        }
    }
    for suffix in ["weight", "bias"] {
        add(format!("memory_attention.norm.{suffix}"), vec![256]);
    }
    for stage in 0..4 {
        let input = 4usize.pow(stage);
        let output = input * 4;
        let index = stage * 3;
        add(
            format!("memory_encoder.mask_downsampler.encoder.{index}.weight"),
            vec![output, input, 3, 3],
        );
        add(
            format!("memory_encoder.mask_downsampler.encoder.{index}.bias"),
            vec![output],
        );
        for suffix in ["weight", "bias"] {
            add(
                format!(
                    "memory_encoder.mask_downsampler.encoder.{}.{suffix}",
                    index + 1
                ),
                vec![output],
            );
        }
    }
    for (name, input, output, kernel) in [
        ("memory_encoder.mask_downsampler.encoder.12", 256, 256, 1),
        ("memory_encoder.pix_feat_proj", 256, 256, 1),
        ("memory_encoder.out_proj", 256, 64, 1),
        ("mask_downsample", 1, 1, 4),
    ] {
        add(
            format!("{name}.weight"),
            vec![output, input, kernel, kernel],
        );
        add(format!("{name}.bias"), vec![output]);
    }
    for index in 0..2 {
        let p = format!("memory_encoder.fuser.layers.{index}");
        add(format!("{p}.dwconv.weight"), vec![256, 1, 7, 7]);
        add(format!("{p}.dwconv.bias"), vec![256]);
        for suffix in ["weight", "bias"] {
            add(format!("{p}.norm.{suffix}"), vec![256]);
        }
        add(format!("{p}.gamma"), vec![256]);
        for (name, input, output) in [("pwconv1", 256, 1024), ("pwconv2", 1024, 256)] {
            add(format!("{p}.{name}.weight"), vec![output, input]);
            add(format!("{p}.{name}.bias"), vec![output]);
        }
    }
    for index in 0..3 {
        add(
            format!("obj_ptr_proj.layers.{index}.weight"),
            vec![256, 256],
        );
        add(format!("obj_ptr_proj.layers.{index}.bias"), vec![256]);
    }
    add("obj_ptr_tpos_proj.weight".into(), vec![64, 256]);
    add("obj_ptr_tpos_proj.bias".into(), vec![64]);
    for (name, shape) in expected {
        if !reader.has(&name) {
            continue;
        }
        let (dtype, actual, bytes) = reader.raw(&name)?;
        if dtype != safetensors::Dtype::F32
            || actual != shape
            || bytes
                .chunks_exact(4)
                .any(|chunk| !f32::from_le_bytes(chunk.try_into().expect("F32 chunk")).is_finite())
        {
            return Err(Error::Invalid(format!(
                "Invalid SAM video-only tensor {name}; expected finite F32 {shape:?}"
            )));
        }
    }
    Ok(())
}

pub struct Sam2Predictor {
    model: Sam2,
    device: Device,
    state: State,
}
impl Sam2Predictor {
    pub fn load(directory: impl AsRef<Path>, backend: BackendChoice) -> Result<Self> {
        let directory = directory.as_ref();
        let state = read_state(directory)?;
        let device = backend::device(&backend::resolve_backend(&backend)?, false)?;
        let _rng = crate::execution::lock(None)?;
        let model = Sam2::new(&state.model, &device)?
            .try_load_file(checked_checkpoint(directory, &state.checkpoint)?.join("model.bpk"))
            .map_err(record_error)?
            .valid();
        crate::execution::materialize(&model);
        Ok(Self {
            model,
            device,
            state,
        })
    }
    pub fn config(&self) -> &Sam2Config {
        &self.state.model
    }
    pub fn provenance(&self) -> &Sam2WeightsProvenance {
        &self.state.provenance
    }
    pub fn predict(&self, images: &TensorData, prompts: &[Sam2Prompt]) -> Result<Sam2Prediction> {
        images.validate()?;
        self.state.model.validate_input_shape(&images.shape)?;
        let output = self
            .model
            .forward(image_tensor(images, &self.device), prompts)?;
        Ok(Sam2Prediction {
            mask_logits: host(output.mask_logits)?,
            iou: host(output.iou)?,
            object_score_logits: host(output.object_score_logits)?,
        })
    }
}
fn host<const D: usize>(value: Tensor<D>) -> Result<TensorData> {
    let shape = value.dims().to_vec();
    let values = value
        .into_data()
        .try_to_vec::<f32>()
        .map_err(record_error)?;
    let output = TensorData { shape, values };
    output.validate()?;
    Ok(output)
}
fn image_tensor(data: &TensorData, device: &Device) -> Tensor<4> {
    Tensor::from_data(
        BurnData::new(
            data.values.clone(),
            [data.shape[0], data.shape[1], data.shape[2], data.shape[3]],
        ),
        device,
    )
}

pub fn train_sam2(
    config: &Sam2TrainingConfig,
    data: &Sam2TrainingDataset,
    pretrained_dir: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        Some(pretrained_dir.as_ref()),
        artifact_dir.as_ref(),
        cancellation,
        progress,
    )
}
pub fn resume_sam2(
    config: &Sam2TrainingConfig,
    data: &Sam2TrainingDataset,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        None,
        artifact_dir.as_ref(),
        cancellation,
        progress,
    )
}
fn run(
    config: &Sam2TrainingConfig,
    data: &Sam2TrainingDataset,
    source: Option<&Path>,
    directory: &Path,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    config.validate()?;
    data.validate(&config.model)?;
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let _lock = DirectoryLock::acquire(directory)?;
    if source.is_some() && directory.join(SAM2_MANIFEST_FILE).exists() {
        return Err(Error::Invalid(
            "Use a new SAM artifact directory or resume its own checkpoint".into(),
        ));
    }
    let previous = if source.is_none() {
        Some(read_state(directory)?)
    } else {
        None
    };
    let mut config = config.clone();
    config.backend = backend::resolve_for_resume(
        &config.backend,
        previous
            .as_ref()
            .and_then(|s| s.training.as_ref().map(|c| &c.backend)),
    )?;
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(data)?));
    let count = data.images.shape[0];
    let batches = count.div_ceil(config.batch_size);
    let device = backend::device(&config.backend, true)?;
    let initialization_guard = crate::execution::lock(Some(cancellation))?;
    device.seed(config.seed);
    let mut optimizer = AdamConfig::new()
        .init()
        .with_grad_clipping(GradientClippingConfig::Norm(config.gradient_clip).init());
    let (mut model, mut state) = if let Some(previous) = previous {
        let mut expected = previous.training.clone().ok_or_else(|| {
            Error::Invalid("An imported SAM source cannot be resumed; start fine-tuning".into())
        })?;
        expected.epochs = config.epochs;
        expected.backend = config.backend.clone();
        if expected != config
            || previous.dataset_sha256.as_ref() != Some(&digest)
            || config.epochs < previous.next_epoch
            || previous.next_batch > batches
        {
            return Err(Error::Invalid(
                "SAM resume configuration or dataset differs from its checkpoint".into(),
            ));
        }
        let checkpoint = checked_checkpoint(directory, &previous.checkpoint)?;
        let model = Sam2::new(&config.model, &device)?
            .try_load_file(checkpoint.join("model.bpk"))
            .map_err(record_error)?
            .configure_fine_tuning(config.freeze_image_encoder);
        optimizer = optimizer
            .load(checkpoint.join("optimizer.bpk"))
            .map_err(record_error)?;
        (model, previous)
    } else {
        let source = source.expect("new fine-tuning has a source");
        let origin = read_state(source)?;
        if origin.model != config.model {
            return Err(Error::Invalid(
                "SAM fine-tuning must retain its source architecture".into(),
            ));
        }
        let path = checked_checkpoint(source, &origin.checkpoint)?.join("model.bpk");
        let source_digest = file_sha256(&path)?;
        let model = Sam2::new(&config.model, &device)?
            .try_load_file(path)
            .map_err(record_error)?
            .configure_fine_tuning(config.freeze_image_encoder);
        let initial = evaluate(&model, data, &device, config.batch_size)?;
        let state = State {
            version: 1,
            model: config.model.clone(),
            provenance: origin.provenance,
            source_model_sha256: Some(source_digest),
            training: Some(config.clone()),
            dataset_sha256: Some(digest),
            checkpoint: String::new(),
            next_epoch: 0,
            next_batch: 0,
            report: TrainingReport {
                backend: Some(config.backend.clone()),
                completed_epochs: 0,
                steps: 0,
                initial_loss: initial,
                final_loss: initial,
                validation_loss: None,
                history: Vec::new(),
            },
        };
        (model, state)
    };
    state.training = Some(config.clone());
    state.report.backend = Some(config.backend.clone());
    checkpoint(directory, &model, Some(&optimizer), &mut state)?;
    drop(initialization_guard);
    for epoch in state.next_epoch..config.epochs {
        let indices = shuffled_indices(count, config.seed.wrapping_add(epoch as u64));
        let first = if epoch == state.next_epoch {
            state.next_batch
        } else {
            0
        };
        for batch_index in first..batches {
            if cancellation.is_cancelled() {
                checkpoint(directory, &model, Some(&optimizer), &mut state)?;
                return Err(Error::Cancelled);
            }
            let indices = &indices[batch_index * config.batch_size
                ..((batch_index + 1) * config.batch_size).min(count)];
            let (images, prompts, masks) = batch(data, indices, &device);
            let batch_guard = match crate::execution::lock(Some(cancellation)) {
                Ok(guard) => guard,
                Err(error) => {
                    checkpoint(directory, &model, Some(&optimizer), &mut state)?;
                    return Err(error);
                }
            };
            device.seed(
                config
                    .seed
                    .wrapping_add(state.report.steps as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15),
            );
            let loss = model.forward(images, &prompts)?.loss(masks);
            let loss_value = scalar(loss.clone())?;
            if !loss_value.is_finite() {
                return Err(Error::Invalid(
                    "SAM training produced non-finite loss; last checkpoint retained".into(),
                ));
            }
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            model = optimizer.step(config.learning_rate, model, gradients);
            drop(batch_guard);
            state.report.steps += 1;
            state.next_epoch = epoch;
            state.next_batch = batch_index + 1;
            progress(TrainingProgress {
                epoch: epoch + 1,
                batch: batch_index + 1,
                steps: state.report.steps,
                training_loss: loss_value,
                validation_loss: None,
            });
            if cancellation.is_cancelled() {
                checkpoint(directory, &model, Some(&optimizer), &mut state)?;
                return Err(Error::Cancelled);
            }
        }
        state.next_epoch = epoch + 1;
        state.next_batch = 0;
        state.report.completed_epochs = epoch + 1;
        state.report.final_loss = evaluate(&model, data, &device, config.batch_size)?;
        let event = TrainingProgress {
            epoch: epoch + 1,
            batch: batches,
            steps: state.report.steps,
            training_loss: state.report.final_loss,
            validation_loss: None,
        };
        state.report.history.push(event.clone());
        checkpoint(directory, &model, Some(&optimizer), &mut state)?;
        progress(event);
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(state.report)
}
fn batch(
    data: &Sam2TrainingDataset,
    indices: &[usize],
    device: &Device,
) -> (Tensor<4>, Vec<Sam2Prompt>, Tensor<4>) {
    let image_len = 3 * 1024 * 1024;
    let mask_len = 256 * 256;
    let images = indices
        .iter()
        .flat_map(|i| {
            data.images.values[i * image_len..(i + 1) * image_len]
                .iter()
                .copied()
        })
        .collect::<Vec<_>>();
    let masks = indices
        .iter()
        .flat_map(|i| {
            data.masks.values[i * mask_len..(i + 1) * mask_len]
                .iter()
                .copied()
        })
        .collect::<Vec<_>>();
    let prompts = indices.iter().map(|i| data.prompts[*i].clone()).collect();
    (
        Tensor::from_data(
            BurnData::new(images, [indices.len(), 3, 1024, 1024]),
            device,
        ),
        prompts,
        Tensor::from_data(BurnData::new(masks, [indices.len(), 1, 256, 256]), device),
    )
}
fn scalar(value: Tensor<1>) -> Result<f32> {
    value
        .into_data()
        .try_to_vec::<f32>()
        .map_err(record_error)?
        .first()
        .copied()
        .ok_or_else(|| Error::Record("Expected scalar loss".into()))
}
fn evaluate(
    model: &Sam2,
    data: &Sam2TrainingDataset,
    device: &Device,
    batch_size: usize,
) -> Result<f32> {
    let model = model.valid();
    let count = data.images.shape[0];
    let mut sum = 0.0f64;
    for start in (0..count).step_by(batch_size) {
        let indices = (start..(start + batch_size).min(count)).collect::<Vec<_>>();
        let (images, prompts, masks) = batch(data, &indices, device);
        sum += scalar(model.forward(images, &prompts)?.loss(masks))? as f64 * indices.len() as f64;
    }
    let loss = (sum / count as f64) as f32;
    if !loss.is_finite() {
        return Err(Error::Invalid(
            "SAM evaluation produced non-finite loss".into(),
        ));
    }
    Ok(loss)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use safetensors::{
        Dtype,
        tensor::{TensorView, serialize},
    };

    #[test]
    fn sam2_resume_rejects_changed_data_freezing_and_import_sources_before_loading_weights() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let data = Sam2TrainingDataset {
            images: TensorData {
                shape: vec![1, 3, 1024, 1024],
                values: vec![0.0; 3 * 1024 * 1024],
            },
            prompts: vec![Sam2Prompt::default()],
            masks: TensorData {
                shape: vec![1, 1, 256, 256],
                values: vec![0.0; 256 * 256],
            },
        };
        let config = Sam2TrainingConfig {
            model: Sam2Config {
                variant: Sam2Variant::HieraTiny,
            },
            backend: BackendChoice::Cpu,
            epochs: 2,
            batch_size: 1,
            learning_rate: 1e-4,
            seed: 1,
            gradient_clip: 5.0,
            freeze_image_encoder: true,
        };
        let mut state = State {
            version: 1,
            model: config.model.clone(),
            provenance: Sam2WeightsProvenance {
                metadata: PretrainedWeightsMetadata {
                    source: "unit-test".into(),
                    license: None,
                    weights_license: None,
                    revision: None,
                    expected_sha256: None,
                },
                format: "sam2_image_test".into(),
                sha256: "0".repeat(64),
            },
            source_model_sha256: Some("0".repeat(64)),
            training: Some(config.clone()),
            dataset_sha256: Some(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&data).unwrap())
            )),
            checkpoint: "sam2-checkpoint-0-test".into(),
            next_epoch: 0,
            next_batch: 0,
            report: TrainingReport {
                backend: Some(BackendChoice::Cpu),
                completed_epochs: 0,
                steps: 0,
                initial_loss: 0.0,
                final_loss: 0.0,
                validation_loss: None,
                history: Vec::new(),
            },
        };
        let directory = tempfile::tempdir().unwrap();
        let write = |state: &State| {
            fs::write(
                directory.path().join(SAM2_MANIFEST_FILE),
                serde_json::to_vec(state).unwrap(),
            )
            .unwrap()
        };
        write(&state);
        let mut different = config.clone();
        different.freeze_image_encoder = false;
        let error = resume_sam2(
            &different,
            &data,
            directory.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("configuration or dataset differs")
        );
        state.dataset_sha256 = Some("1".repeat(64));
        write(&state);
        let error = resume_sam2(
            &config,
            &data,
            directory.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("configuration or dataset differs")
        );
        state.training = None;
        write(&state);
        let error = resume_sam2(
            &config,
            &data,
            directory.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("imported SAM source cannot be resumed")
        );
    }

    #[test]
    fn sam2_import_checks_unused_video_tensors_by_name_shape_and_values() {
        let good = vec![0u8; 256 * 4];
        let view = TensorView::new(Dtype::F32, vec![1, 1, 256], &good).unwrap();
        let bytes = serialize([("no_mem_pos_enc", view)], None).unwrap();
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        consume_video_weights(&mut reader).unwrap();
        reader.finish().unwrap();
        let view = TensorView::new(Dtype::F32, vec![256], &good).unwrap();
        let bytes = serialize([("no_mem_pos_enc", view)], None).unwrap();
        assert!(consume_video_weights(&mut SafeTensorReader::new(&bytes).unwrap()).is_err());
        let mut nonfinite = good.clone();
        nonfinite[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        let view = TensorView::new(Dtype::F32, vec![1, 1, 256], &nonfinite).unwrap();
        let bytes = serialize([("no_mem_pos_enc", view)], None).unwrap();
        assert!(consume_video_weights(&mut SafeTensorReader::new(&bytes).unwrap()).is_err());
        let view = TensorView::new(Dtype::F32, vec![256], &good).unwrap();
        let bytes = serialize([("memory_attention.unrecognized.weight", view)], None).unwrap();
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        consume_video_weights(&mut reader).unwrap();
        assert!(reader.finish().is_err());
    }

    #[test]
    #[ignore = "requires the official SAM 2.1 Hiera-T checkpoint"]
    fn sam2_public_checkpoint_matches_upstream_image_prompt_decoder() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let source = std::env::var("FLOW_LIKE_SAM2_WEIGHTS").expect("FLOW_LIKE_SAM2_WEIGHTS");
        let metadata = PretrainedWeightsMetadata {
            source: "https://dl.fbaipublicfiles.com/segment_anything_2/092824/sam2.1_hiera_tiny.pt"
                .into(),
            license: Some("Apache-2.0".into()),
            weights_license: Some("Apache-2.0".into()),
            revision: Some("092824/sam2.1_hiera_tiny".into()),
            expected_sha256: Some(
                "7402e0d864fa82708a20fbd15bc84245c2f26dff0eb43a4b5b93452deb34be69".into(),
            ),
        };
        let directory = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let info = import_sam2_weights(source, directory.path(), &metadata).unwrap();
        eprintln!("SAM2 native import completed in {:?}", started.elapsed());
        assert_eq!(info.config.variant, Sam2Variant::HieraTiny);
        let predictor = Sam2Predictor::load(directory.path(), BackendChoice::Cpu).unwrap();
        eprintln!(
            "SAM2 native artifact load completed in {:?}",
            started.elapsed()
        );
        let input = TensorData {
            shape: vec![1, 3, 1024, 1024],
            values: (0..3 * 1024 * 1024)
                .map(|index| (index % 251) as f32 / 250.0)
                .collect(),
        };
        let prompt = Sam2Prompt {
            points: vec![Sam2Point {
                x: 512.0,
                y: 512.0,
                label: 1,
            }],
            ..Default::default()
        };
        let result = predictor.predict(&input, &[prompt]).unwrap();
        eprintln!(
            "SAM2 native full-resolution prediction completed in {:?}",
            started.elapsed()
        );
        assert_eq!(result.mask_logits.shape, [1, 4, 256, 256]);
        assert_eq!(result.iou.shape, [1, 4]);
        assert!(
            result
                .mask_logits
                .values
                .iter()
                .all(|value| value.is_finite())
        );
        assert!(
            result
                .iou
                .values
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
        let maximum = result
            .mask_logits
            .values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let minimum = result
            .mask_logits
            .values
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        assert!(
            maximum - minimum > 1e-3,
            "Imported image decoder must produce spatially varying mask logits"
        );
        // Upstream Meta SAM2 commit 2b90b9f5ceec907a1c18123530e92e794ad901a4,
        // CPU float32 image_encoder + prompt_encoder + mask_decoder.predict_masks.
        let expected_masks = [
            -0.8641753,
            -1.1327356,
            -1.1971129,
            -1.4958999,
            -0.91926694,
            -1.1988467,
            -0.9452483,
            -1.2582783,
            -0.9374117,
            -1.2510197,
            -0.7912205,
            -1.0109665,
            -0.5682224,
            -0.9037082,
            -0.600565,
            -0.92058146,
        ];
        for (index, (actual, expected)) in result
            .mask_logits
            .values
            .iter()
            .zip(expected_masks)
            .enumerate()
        {
            assert!(
                (actual - expected).abs() < 1e-3,
                "Mask logit {index}: native {actual}, upstream {expected}"
            );
        }
        for (actual, expected) in
            result
                .iou
                .values
                .iter()
                .zip([0.4082553, 0.8920166, 0.005734258, 0.23028949])
        {
            assert!(
                (actual - expected).abs() < 1e-5,
                "IoU: native {actual}, upstream {expected}"
            );
        }
        assert!((result.object_score_logits.values[0] - 18.271763).abs() < 1e-4);
        if let Ok(directory) = std::env::var("FLOW_LIKE_SAM2_REFERENCE_DIR") {
            for (name, actual, tolerance) in [
                ("mask_logits", &result.mask_logits.values, 1e-3f32),
                ("iou", &result.iou.values, 1e-5),
                (
                    "object_score_logits",
                    &result.object_score_logits.values,
                    1e-4,
                ),
            ] {
                let bytes = fs::read(
                    Path::new(&directory).join(format!("flow-like-sam2-reference-{name}.bin")),
                )
                .unwrap();
                assert_eq!(bytes.len(), actual.len() * 4);
                let mut max_error = 0.0f32;
                for (actual, expected) in actual.iter().zip(bytes.chunks_exact(4)) {
                    max_error = max_error
                        .max((actual - f32::from_le_bytes(expected.try_into().unwrap())).abs());
                }
                eprintln!(
                    "SAM2 {name}: {} values, max absolute error {max_error}",
                    actual.len()
                );
                assert!(
                    max_error < tolerance,
                    "SAM2 {name} differs from upstream by {max_error}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires the official SAM 2.1 Hiera-T checkpoint"]
    fn sam2_public_checkpoint_callback_cancellation_retains_resume_cursor() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let source = std::env::var("FLOW_LIKE_SAM2_WEIGHTS").expect("FLOW_LIKE_SAM2_WEIGHTS");
        let metadata = PretrainedWeightsMetadata {
            source: "https://dl.fbaipublicfiles.com/segment_anything_2/092824/sam2.1_hiera_tiny.pt"
                .into(),
            license: Some("Apache-2.0".into()),
            weights_license: Some("Apache-2.0".into()),
            revision: Some("092824/sam2.1_hiera_tiny".into()),
            expected_sha256: Some(
                "7402e0d864fa82708a20fbd15bc84245c2f26dff0eb43a4b5b93452deb34be69".into(),
            ),
        };
        let source_directory = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let info = import_sam2_weights(source, source_directory.path(), &metadata).unwrap();
        let source_state = fs::read(source_directory.path().join(SAM2_MANIFEST_FILE)).unwrap();
        let config = Sam2TrainingConfig {
            model: info.config,
            backend: BackendChoice::Cpu,
            epochs: 1,
            batch_size: 1,
            learning_rate: 1e-4,
            seed: 17,
            gradient_clip: 5.0,
            freeze_image_encoder: true,
        };
        let data = Sam2TrainingDataset {
            images: TensorData {
                shape: vec![1, 3, 1024, 1024],
                values: (0..3 * 1024 * 1024)
                    .map(|i| (i % 251) as f32 / 250.0)
                    .collect(),
            },
            prompts: vec![Sam2Prompt {
                points: vec![Sam2Point {
                    x: 512.0,
                    y: 512.0,
                    label: 1,
                }],
                ..Default::default()
            }],
            masks: TensorData {
                shape: vec![1, 1, 256, 256],
                values: (0..256 * 256)
                    .map(|i| {
                        if (64..192).contains(&(i % 256)) && (64..192).contains(&(i / 256)) {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            },
        };
        let cancellation = CancellationToken::new();
        let result = train_sam2(
            &config,
            &data,
            source_directory.path(),
            target.path(),
            &cancellation,
            |event| {
                if event.steps == 1 {
                    cancellation.cancel();
                }
            },
        );
        assert!(matches!(result, Err(Error::Cancelled)));
        let state = read_state(target.path()).unwrap();
        assert_eq!(
            (state.next_epoch, state.next_batch, state.report.steps),
            (0, 1, 1)
        );
        assert_eq!(
            fs::read(source_directory.path().join(SAM2_MANIFEST_FILE)).unwrap(),
            source_state
        );
        let cancellation = CancellationToken::new();
        let result = resume_sam2(&config, &data, target.path(), &cancellation, |event| {
            if event.epoch == 1 {
                cancellation.cancel();
            }
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        let state = read_state(target.path()).unwrap();
        assert_eq!(
            (state.next_epoch, state.next_batch, state.report.steps),
            (1, 0, 1)
        );
        assert_eq!(state.report.history.len(), 1);
        let completed = resume_sam2(
            &config,
            &data,
            target.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(completed.completed_epochs, 1);
        assert_eq!(
            completed.steps, 1,
            "Resuming a completed batch must not apply another optimizer step"
        );
        assert_eq!(completed.history.len(), 1);
        assert!(completed.final_loss.is_finite());
    }
}

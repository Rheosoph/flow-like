use crate::{backend, model::Model, *};
use burn::grad_clipping::GradientClippingConfig;
use burn::{
    module::Module,
    optim::{AdamConfig, GradientsParams, ModuleOptimizer},
    tensor::{Device, Tensor},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const MANIFEST_FILE: &str = "state.json";
const FORMAT_VERSION: u32 = 1;
static CHECKPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    version: u32,
    config: TrainingConfig,
    #[serde(default)]
    requested_backend: BackendChoice,
    input_shape: Vec<usize>,
    dataset_fingerprint: String,
    checkpoint: String,
    next_epoch: usize,
    next_batch: usize,
    report: TrainingReport,
}

pub(crate) struct DirectoryLock {
    _file: fs::File,
}
impl DirectoryLock {
    pub(crate) fn acquire(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(".training.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.try_lock()
            .map_err(|e| Error::Invalid(format!("Cannot acquire training directory lock: {e}")))?;
        Ok(Self { _file: file })
    }
}

/// Run training on the caller's worker thread. Cancellation is observed between batches.
pub fn train(
    config: &TrainingConfig,
    data: &TensorDataset,
    validation: Option<&TensorDataset>,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        validation,
        artifact_dir.as_ref(),
        cancellation,
        progress,
        false,
    )
}

/// Resume the model, optimizer and batch cursor from the last durable checkpoint.
/// The recipe, data, seed, batch size and optimizer settings must match the original run.
pub fn resume(
    config: &TrainingConfig,
    data: &TensorDataset,
    validation: Option<&TensorDataset>,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        validation,
        artifact_dir.as_ref(),
        cancellation,
        progress,
        true,
    )
}

fn run(
    config: &TrainingConfig,
    data: &TensorDataset,
    validation: Option<&TensorDataset>,
    dir: &Path,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(TrainingProgress),
    resuming: bool,
) -> Result<TrainingReport> {
    config.validate()?;
    data.validate(&config.recipe)?;
    if let Some(v) = validation {
        v.validate(&config.recipe)?;
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let _lock = DirectoryLock::acquire(dir)?;
    if !resuming && dir.join(MANIFEST_FILE).exists() {
        return Err(Error::Invalid(
            "Artifact directory already contains a model; use resume or a new directory".into(),
        ));
    }
    let previous = if resuming {
        Some(read_state(dir)?)
    } else {
        None
    };
    let requested_backend = config.backend.clone();
    let mut resolved_config = config.clone();
    resolved_config.backend = backend::resolve_for_resume(
        &requested_backend,
        previous.as_ref().map(|state| &state.config.backend),
    )?;
    let config = &resolved_config;
    let device = backend::device(&config.backend, true)?;
    device.seed(config.seed);
    let mut model = Model::new(&config.recipe, &device)?;
    let mut optimizer = AdamConfig::new()
        .init()
        .with_grad_clipping(GradientClippingConfig::Norm(config.gradient_clip).init());
    let fingerprint = match validation {
        Some(validation) => format!(
            "{}:{}",
            dataset_fingerprint(data)?,
            dataset_fingerprint(validation)?
        ),
        None => format!("{}:none", dataset_fingerprint(data)?),
    };
    let mut state = if let Some(previous) = previous {
        let mut expected = previous.config.clone();
        expected.epochs = config.epochs;
        expected.backend = config.backend.clone();
        if expected != *config
            || previous.dataset_fingerprint != fingerprint
            || config.epochs < previous.next_epoch
        {
            return Err(Error::Invalid(
                "Resume configuration or dataset differs from its checkpoint".into(),
            ));
        }
        let checkpoint = checked_checkpoint(dir, &previous.checkpoint)?;
        model = model
            .try_load_file(checkpoint.join("model.bpk"))
            .map_err(record_error)?
            .train();
        optimizer = optimizer
            .load(checkpoint.join("optimizer.bpk"))
            .map_err(record_error)?;
        previous
    } else {
        let initial_loss = evaluate_loss(&model, data, &device, config.batch_size)?;
        State {
            version: FORMAT_VERSION,
            config: config.clone(),
            requested_backend: requested_backend.clone(),
            input_shape: data.inputs.shape[1..].to_vec(),
            dataset_fingerprint: fingerprint,
            checkpoint: String::new(),
            next_epoch: 0,
            next_batch: 0,
            report: TrainingReport {
                backend: Some(config.backend.clone()),
                completed_epochs: 0,
                steps: 0,
                initial_loss,
                final_loss: initial_loss,
                validation_loss: None,
                history: Vec::new(),
            },
        }
    };
    // Establish a complete checkpoint before doing work so a cancellation always leaves a loadable model.
    state.config = config.clone();
    state.requested_backend = requested_backend;
    state.report.backend = Some(config.backend.clone());
    checkpoint(dir, &model, &optimizer, &mut state)?;
    let batches = data.inputs.shape[0].div_ceil(config.batch_size);
    for epoch in state.next_epoch..config.epochs {
        let indices =
            shuffled_indices(data.inputs.shape[0], config.seed.wrapping_add(epoch as u64));
        let first = if epoch == state.next_epoch {
            state.next_batch
        } else {
            0
        };
        for batch_index in first..batches {
            if cancellation.is_cancelled() {
                checkpoint(dir, &model, &optimizer, &mut state)?;
                return Err(Error::Cancelled);
            }
            let start = batch_index * config.batch_size;
            let end = (start + config.batch_size).min(indices.len());
            let batch = data.batch(&indices[start..end]);
            device.seed(
                config
                    .seed
                    .wrapping_add(state.report.steps as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15),
            );
            let loss = model.loss(&batch, &device)?;
            let loss_value = scalar(loss.clone())?;
            if !loss_value.is_finite() {
                return Err(Error::Invalid(
                    "Training produced a non-finite loss; last checkpoint is retained".into(),
                ));
            }
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            model = optimizer.step(config.learning_rate, model, gradients);
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
        }
        state.next_epoch = epoch + 1;
        state.next_batch = 0;
        state.report.completed_epochs = epoch + 1;
        state.report.final_loss = evaluate_loss(&model, data, &device, config.batch_size)?;
        state.report.validation_loss = validation
            .map(|v| evaluate_loss(&model, v, &device, config.batch_size))
            .transpose()?;
        let event = TrainingProgress {
            epoch: epoch + 1,
            batch: batches,
            steps: state.report.steps,
            training_loss: state.report.final_loss,
            validation_loss: state.report.validation_loss,
        };
        state.report.history.push(event.clone());
        checkpoint(dir, &model, &optimizer, &mut state)?;
        progress(event);
    }
    Ok(state.report)
}

fn evaluate_loss(
    model: &Model,
    data: &TensorDataset,
    device: &Device,
    batch_size: usize,
) -> Result<f32> {
    let model = model.valid();
    let mut total = 0.0f64;
    for start in (0..data.inputs.shape[0]).step_by(batch_size) {
        let end = (start + batch_size).min(data.inputs.shape[0]);
        let batch = data.batch(&(start..end).collect::<Vec<_>>());
        total += scalar(model.loss(&batch, device)?)? as f64 * (end - start) as f64;
    }
    let result = (total / data.inputs.shape[0] as f64) as f32;
    if !result.is_finite() {
        return Err(Error::Invalid("Evaluation produced non-finite loss".into()));
    }
    Ok(result)
}
fn scalar(tensor: Tensor<1>) -> Result<f32> {
    let values = tensor
        .into_data()
        .try_to_vec::<f32>()
        .map_err(record_error)?;
    values
        .first()
        .copied()
        .ok_or_else(|| Error::Record("Expected scalar tensor".into()))
}
fn record_error(error: impl std::fmt::Display) -> Error {
    Error::Record(error.to_string())
}

fn checkpoint(
    dir: &Path,
    model: &Model,
    optimizer: &ModuleOptimizer,
    state: &mut State,
) -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(record_error)?
        .as_nanos();
    let name = format!(
        "checkpoint-{}-{}-{}",
        state.report.steps,
        stamp,
        CHECKPOINT_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let target = dir.join(&name);
    fs::create_dir(&target)?;
    model
        .clone()
        .save_file(target.join("model.bpk"))
        .map_err(record_error)?;
    optimizer
        .save(target.join("optimizer.bpk"))
        .map_err(record_error)?;
    sync_record(&target.join("model.bpk"))?;
    sync_record(&target.join("optimizer.bpk"))?;
    sync_directory(&target)?;
    sync_directory(dir)?;
    state.checkpoint = name;
    let temporary = dir.join("state.json.pending");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&file, state)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, dir.join(MANIFEST_FILE))?;
    sync_directory(dir)?;
    Ok(())
}

pub(crate) fn sync_record(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    })?
    .sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn read_state(dir: &Path) -> Result<State> {
    let state: State = serde_json::from_reader(fs::File::open(dir.join(MANIFEST_FILE))?)?;
    if state.version != FORMAT_VERSION {
        return Err(Error::Invalid(format!(
            "Unsupported model artifact version {}",
            state.version
        )));
    }
    state.config.validate()?;
    Ok(state)
}
pub(crate) fn checked_checkpoint(dir: &Path, name: &str) -> Result<PathBuf> {
    if !name.starts_with("checkpoint-")
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(Error::Invalid("Invalid checkpoint name".into()));
    }
    Ok(dir.join(name))
}
fn dataset_fingerprint(data: &TensorDataset) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for d in &data.inputs.shape {
        digest.update((*d as u64).to_le_bytes());
    }
    for v in &data.inputs.values {
        digest.update(v.to_bits().to_le_bytes());
    }
    digest.update(serde_json::to_vec(&data.targets)?);
    Ok(format!("{:x}", digest.finalize()))
}
pub(crate) fn shuffled_indices(n: usize, seed: u64) -> Vec<usize> {
    let mut result = (0..n).collect::<Vec<_>>();
    let mut state = seed ^ 0x9e3779b97f4a7c15;
    for i in (1..n).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        result.swap(i, (state as usize) % (i + 1));
    }
    result
}

pub struct Predictor {
    model: Model,
    device: Device,
    recipe: Recipe,
    input_shape: Vec<usize>,
    backend: BackendChoice,
}
impl Predictor {
    pub fn load(artifact_dir: impl AsRef<Path>, backend: BackendChoice) -> Result<Self> {
        let dir = artifact_dir.as_ref();
        let state = read_state(dir)?;
        let backend = backend::resolve_backend(&backend)?;
        let device = backend::device(&backend, false)?;
        let model = Model::new(&state.config.recipe, &device)?
            .try_load_file(checked_checkpoint(dir, &state.checkpoint)?.join("model.bpk"))
            .map_err(record_error)?
            .valid();
        Ok(Self {
            model,
            device,
            recipe: state.config.recipe,
            input_shape: state.input_shape,
            backend,
        })
    }
    pub fn backend(&self) -> &BackendChoice {
        &self.backend
    }
    /// Export the captured inference graph with fixed input dimensions.
    /// Unsupported operators return an error; the native model remains usable.
    #[cfg(feature = "onnx-export")]
    pub fn export_onnx(
        &self,
        input: &TensorData,
        path: impl AsRef<Path>,
    ) -> Result<crate::OnnxExportReport> {
        self.recipe.validate_input(input)?;
        if input.shape[1..] != self.input_shape {
            return Err(Error::Invalid(
                "Export input differs from trained preprocessing contract".into(),
            ));
        }
        let (model, report) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.model.export_onnx(input, &self.device, &self.recipe)
        }))
        .map_err(|_| {
            Error::Record("ONNX graph capture rejected an unsupported operation".into())
        })??;
        let path = path.as_ref();
        let name = path
            .file_name()
            .ok_or_else(|| Error::Invalid("ONNX output must name a file".into()))?
            .to_string_lossy();
        let temporary = path.with_file_name(format!(
            ".{name}.{}-{}.pending",
            std::process::id(),
            CHECKPOINT_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let saved = (|| -> Result<()> {
            model.save(&temporary).map_err(record_error)?;
            sync_record(&temporary)?;
            fs::rename(&temporary, path)?;
            sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))?;
            Ok(())
        })();
        if saved.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        saved?;
        Ok(report)
    }
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }
    /// Return the final convolutional feature map in NCHW order from a trained image classifier.
    pub fn features(&self, input: &TensorData) -> Result<TensorData> {
        self.recipe.validate_input(input)?;
        if input.shape[1..] != self.input_shape {
            return Err(Error::Invalid(
                "Feature input differs from trained preprocessing contract".into(),
            ));
        }
        let tensor = || {
            Tensor::<4>::from_data(
                burn::tensor::TensorData::new(input.values.clone(), input.shape.clone()),
                &self.device,
            )
        };
        let output = match &self.model {
            Model::ResNet18(m) => m.features(tensor()),
            Model::MobileNetV2(m) => m.features(tensor()),
            Model::EfficientNet(m) => m.features(tensor()),
            _ => {
                return Err(Error::Invalid(
                    "Spatial features require ResNet18, MobileNetV2 or EfficientNet".into(),
                ));
            }
        };
        let shape = output.dims().to_vec();
        let values = output
            .into_data()
            .try_to_vec::<f32>()
            .map_err(record_error)?;
        Ok(TensorData { shape, values })
    }
    pub fn predict(&self, input: &TensorData) -> Result<PredictionBatch> {
        self.predict_with_options(input, &DetectionOptions::default())
    }
    pub fn predict_with_options(
        &self,
        input: &TensorData,
        options: &DetectionOptions,
    ) -> Result<PredictionBatch> {
        options.validate()?;
        self.recipe.validate_input(input)?;
        if input.shape[1..] != self.input_shape {
            return Err(Error::Invalid(
                "Input dimensions differ from the trained preprocessing contract".into(),
            ));
        }
        self.model
            .forward(input, &self.device)?
            .prediction(&self.recipe, input, options)
    }
}

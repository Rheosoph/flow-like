use crate::{backend, efficient_ad_types::validate_images, *};
use burn::{
    grad_clipping::GradientClippingConfig,
    module::{Module, Param},
    nn::{
        Dropout, DropoutConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
        interpolate::{Interpolate2dConfig, InterpolateMode},
        pool::{AvgPool2d, AvgPool2dConfig},
    },
    optim::{AdamConfig, GradientsParams, ModuleOptimizer},
    tensor::{Device, Tensor, TensorData as BurnData, activation::relu},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const MANIFEST: &str = "efficient-ad-state.json";
static CHECKPOINT_ID: AtomicU64 = AtomicU64::new(0);
fn record(error: impl std::fmt::Display) -> Error {
    Error::Record(error.to_string())
}

#[derive(Module, Debug)]
struct PdnSmall {
    convolutions: Vec<Conv2d>,
    pool: AvgPool2d,
}
impl PdnSmall {
    fn new(input: usize, width: usize, features: usize, device: &Device) -> Self {
        let layers = [
            (input, width, 4),
            (width, width * 2, 4),
            (width * 2, width * 2, 3),
            (width * 2, features, 4),
        ];
        Self {
            convolutions: layers
                .into_iter()
                .map(|(a, b, k)| Conv2dConfig::new([a, b], [k, k]).init(device))
                .collect(),
            pool: AvgPool2dConfig::new([2, 2]).with_strides([2, 2]).init(),
        }
    }
    fn import(&mut self, weights: &PdnTeacherWeights, device: &Device) {
        for (conv, source) in self.convolutions.iter_mut().zip(&weights.convolutions) {
            conv.weight = Param::from_tensor(tensor(&source.weight, device));
            conv.bias = Some(Param::from_tensor(tensor(&source.bias, device)));
        }
    }
    fn forward(&self, mut x: Tensor<4>) -> Tensor<4> {
        for (i, conv) in self.convolutions.iter().enumerate() {
            x = conv.forward(x);
            if i < 3 {
                x = relu(x);
            }
            if i < 2 {
                x = self.pool.forward(x);
            }
        }
        x
    }
}

#[derive(Module, Debug)]
struct FeatureAutoencoder {
    encoder: Vec<Conv2d>,
    decoder: Vec<Conv2d>,
    final_conv: Conv2d,
    dropout: Dropout,
}
impl FeatureAutoencoder {
    fn new(input: usize, width: usize, features: usize, device: &Device) -> Self {
        let channels = [input, width / 2, width / 2, width, width, width, width];
        let encoder = (0..6)
            .map(|i| {
                Conv2dConfig::new(
                    [channels[i], channels[i + 1]],
                    if i == 5 { [8, 8] } else { [4, 4] },
                )
                .with_stride(if i == 5 { [1, 1] } else { [2, 2] })
                .with_padding(if i == 5 {
                    PaddingConfig2d::Valid
                } else {
                    PaddingConfig2d::Explicit(1, 1, 1, 1)
                })
                .init(device)
            })
            .collect();
        let decoder = (0..7)
            .map(|i| {
                Conv2dConfig::new([width, width], if i == 6 { [3, 3] } else { [4, 4] })
                    .with_padding(if i == 6 {
                        PaddingConfig2d::Explicit(1, 1, 1, 1)
                    } else {
                        PaddingConfig2d::Explicit(2, 2, 2, 2)
                    })
                    .init(device)
            })
            .collect();
        Self {
            encoder,
            decoder,
            final_conv: Conv2dConfig::new([width, features], [3, 3])
                .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
                .init(device),
            dropout: DropoutConfig::new(0.2).init(),
        }
    }
    fn forward(&self, mut x: Tensor<4>) -> Tensor<4> {
        for (i, conv) in self.encoder.iter().enumerate() {
            x = conv.forward(x);
            if i < 5 {
                x = relu(x);
            }
        }
        for (i, (conv, size)) in self
            .decoder
            .iter()
            .zip([3, 8, 15, 32, 63, 127, 56])
            .enumerate()
        {
            x = relu(conv.forward(resize(x, size)));
            if i < 6 {
                x = self.dropout.forward(x);
            }
        }
        self.final_conv.forward(x)
    }
}
fn resize(x: Tensor<4>, size: usize) -> Tensor<4> {
    Interpolate2dConfig::new()
        .with_output_size(Some([size, size]))
        .with_mode(InterpolateMode::Linear)
        .with_align_corners(false)
        .init()
        .forward(x)
}

#[derive(Module, Debug)]
struct EfficientAdModel {
    teacher: PdnSmall,
    student: PdnSmall,
    autoencoder: FeatureAutoencoder,
    features: usize,
}
impl EfficientAdModel {
    fn new(architecture: &Architecture, device: &Device) -> Self {
        Self {
            teacher: PdnSmall::new(
                architecture.input,
                architecture.width,
                architecture.features,
                device,
            ),
            student: PdnSmall::new(
                architecture.input,
                architecture.width,
                architecture.features * 2,
                device,
            ),
            autoencoder: FeatureAutoencoder::new(
                architecture.input,
                architecture.autoencoder_width,
                architecture.features,
                device,
            ),
            features: architecture.features,
        }
    }
    fn teacher_features(&self, images: Tensor<4>, normalization: &Normalization) -> Tensor<4> {
        let device = images.device();
        let output = self.teacher.forward(images).detach();
        let mean = Tensor::<4>::from_data(
            BurnData::new(normalization.mean.clone(), [1, self.features, 1, 1]),
            &device,
        );
        let std = Tensor::<4>::from_data(
            BurnData::new(normalization.std.clone(), [1, self.features, 1, 1]),
            &device,
        );
        (output - mean) / std
    }
    fn loss(
        &self,
        data: &EfficientAdDataset,
        normalization: &Normalization,
        quantile: f32,
        device: &Device,
    ) -> Result<Tensor<1>> {
        let images = tensor::<4>(&data.images, device);
        let teacher = self.teacher_features(images.clone(), normalization);
        let student = self.student.forward(images).narrow(1, 0, self.features);
        let distance = (teacher - student).square();
        let threshold = quantile_value(
            distance
                .clone()
                .detach()
                .into_data()
                .try_to_vec::<f32>()
                .map_err(record)?,
            quantile,
        )?;
        let mask = distance
            .clone()
            .detach()
            .greater_equal_elem(threshold)
            .float();
        let hard = (distance * mask.clone()).sum() / mask.sum();
        let augmented = tensor::<4>(
            data.autoencoder_images.as_ref().unwrap_or(&data.images),
            device,
        );
        let teacher_ae = self.teacher_features(augmented.clone(), normalization);
        let autoencoder = self.autoencoder.forward(augmented.clone());
        let student_ae = self
            .student
            .forward(augmented)
            .narrow(1, self.features, self.features);
        let mut loss = hard
            + (teacher_ae - autoencoder.clone()).square().mean()
            + (autoencoder - student_ae).square().mean();
        if let Some(penalty) = &data.penalty_images {
            loss = loss
                + self
                    .student
                    .forward(tensor(penalty, device))
                    .narrow(1, 0, self.features)
                    .square()
                    .mean();
        }
        Ok(loss)
    }
    fn maps(&self, images: Tensor<4>, normalization: &Normalization) -> (Tensor<4>, Tensor<4>) {
        let teacher = self.teacher_features(images.clone(), normalization);
        let student = self.student.forward(images.clone());
        let autoencoder = self.autoencoder.forward(images);
        (
            (teacher - student.clone().narrow(1, 0, self.features))
                .square()
                .mean_dim(1),
            (autoencoder - student.narrow(1, self.features, self.features))
                .square()
                .mean_dim(1),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Architecture {
    input: usize,
    width: usize,
    features: usize,
    autoencoder_width: usize,
}
impl From<&EfficientAdConfig> for Architecture {
    fn from(c: &EfficientAdConfig) -> Self {
        Self {
            input: c.teacher.input_channels,
            width: c.teacher.base_channels,
            features: c.teacher.feature_channels,
            autoencoder_width: c.autoencoder_channels,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Normalization {
    mean: Vec<f32>,
    std: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    version: u32,
    #[serde(default)]
    requested_backend: BackendChoice,
    #[serde(default)]
    backend: Option<BackendChoice>,
    architecture: Architecture,
    settings_fingerprint: String,
    dataset_fingerprint: String,
    normalization: Normalization,
    calibration: Option<EfficientAdCalibration>,
    checkpoint: String,
    next_epoch: usize,
    next_batch: usize,
    report: TrainingReport,
}

pub fn train_efficient_ad(
    config: &EfficientAdConfig,
    data: &EfficientAdDataset,
    validation_normal: &TensorData,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        validation_normal,
        artifact_dir.as_ref(),
        cancellation,
        progress,
        false,
    )
}
pub fn resume_efficient_ad(
    config: &EfficientAdConfig,
    data: &EfficientAdDataset,
    validation_normal: &TensorData,
    artifact_dir: impl AsRef<Path>,
    cancellation: &CancellationToken,
    progress: impl FnMut(TrainingProgress),
) -> Result<TrainingReport> {
    run(
        config,
        data,
        validation_normal,
        artifact_dir.as_ref(),
        cancellation,
        progress,
        true,
    )
}
fn run(
    config: &EfficientAdConfig,
    data: &EfficientAdDataset,
    validation_normal: &TensorData,
    dir: &Path,
    cancellation: &CancellationToken,
    mut progress: impl FnMut(TrainingProgress),
    resuming: bool,
) -> Result<TrainingReport> {
    config.validate()?;
    data.validate(config.teacher.input_channels)?;
    validate_images(validation_normal, config.teacher.input_channels)?;
    let _lock = crate::engine::DirectoryLock::acquire(dir)?;
    if !resuming && dir.join(MANIFEST).exists() {
        return Err(Error::Invalid(
            "EfficientAD artifact exists; use resume".into(),
        ));
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
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
        previous.as_ref().and_then(|state| state.backend.as_ref()),
    )?;
    let config = &resolved_config;
    let device = backend::device(&config.backend, true)?;
    device.seed(config.seed);
    let architecture = Architecture::from(config);
    let mut model = EfficientAdModel::new(&architecture, &device);
    let mut optimizer = AdamConfig::new()
        .init()
        .with_grad_clipping(GradientClippingConfig::Norm(config.gradient_clip).init());
    let mut fingerprint_config = config.clone();
    fingerprint_config.epochs = 1;
    fingerprint_config.backend = BackendChoice::Cpu;
    let settings_fingerprint = fingerprint(&fingerprint_config)?;
    let dataset_fingerprint = fingerprint(&(data, validation_normal))?;
    let mut state = if let Some(state) = previous {
        if state.settings_fingerprint != settings_fingerprint
            || state.dataset_fingerprint != dataset_fingerprint
            || config.epochs < state.next_epoch
        {
            return Err(Error::Invalid(
                "EfficientAD resume configuration or data differs".into(),
            ));
        }
        let path = crate::engine::checked_checkpoint(dir, &state.checkpoint)?;
        model = model
            .try_load_file(path.join("model.bpk"))
            .map_err(record)?
            .train();
        optimizer = optimizer.load(path.join("optimizer.bpk")).map_err(record)?;
        state
    } else {
        model.teacher.import(&config.teacher, &device);
        let normalization = teacher_normalization(
            &model,
            &data.images,
            config.batch_size,
            &device,
            cancellation,
        )?;
        let initial_loss = evaluate(&model, data, &normalization, config, &device, cancellation)?;
        State {
            version: 1,
            requested_backend: requested_backend.clone(),
            backend: Some(config.backend.clone()),
            architecture,
            settings_fingerprint,
            dataset_fingerprint,
            normalization,
            calibration: None,
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
    state.requested_backend = requested_backend;
    state.backend = Some(config.backend.clone());
    state.report.backend = Some(config.backend.clone());
    state.calibration = None;
    checkpoint(dir, &model, &optimizer, &mut state)?;
    let batches = data.images.shape[0].div_ceil(config.batch_size);
    for epoch in state.next_epoch..config.epochs {
        let indices = crate::engine::shuffled_indices(
            data.images.shape[0],
            config.seed.wrapping_add(epoch as u64),
        );
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
            let loss = model.loss(&batch, &state.normalization, config.hard_quantile, &device)?;
            let value = scalar(loss.clone())?;
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            model = optimizer.step(config.learning_rate, model, gradients);
            state.report.steps += 1;
            state.next_epoch = epoch;
            state.next_batch = batch_index + 1;
            progress(TrainingProgress {
                epoch: epoch + 1,
                batch: batch_index + 1,
                steps: state.report.steps,
                training_loss: value,
                validation_loss: None,
            });
        }
        let final_loss = evaluate(
            &model,
            data,
            &state.normalization,
            config,
            &device,
            cancellation,
        );
        let final_loss = match final_loss {
            Ok(value) => value,
            Err(error) => {
                checkpoint(dir, &model, &optimizer, &mut state)?;
                return Err(error);
            }
        };
        let validation = EfficientAdDataset {
            images: validation_normal.clone(),
            autoencoder_images: None,
            penalty_images: None,
        };
        let validation_loss = evaluate(
            &model,
            &validation,
            &state.normalization,
            config,
            &device,
            cancellation,
        );
        let validation_loss = match validation_loss {
            Ok(value) => value,
            Err(error) => {
                checkpoint(dir, &model, &optimizer, &mut state)?;
                return Err(error);
            }
        };
        state.next_epoch = epoch + 1;
        state.next_batch = 0;
        state.report.completed_epochs = epoch + 1;
        state.report.final_loss = final_loss;
        state.report.validation_loss = Some(validation_loss);
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
    state.calibration = Some(calibrate(
        &model,
        validation_normal,
        &state.normalization,
        config.batch_size,
        &device,
        cancellation,
    )?);
    checkpoint(dir, &model, &optimizer, &mut state)?;
    Ok(state.report)
}

fn teacher_normalization(
    model: &EfficientAdModel,
    images: &TensorData,
    batch: usize,
    device: &Device,
    cancel: &CancellationToken,
) -> Result<Normalization> {
    let mut sums = vec![0.0f64; model.features];
    let mut squares = vec![0.0f64; model.features];
    let mut count = 0usize;
    for start in (0..images.shape[0]).step_by(batch) {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let indices = (start..(start + batch).min(images.shape[0])).collect::<Vec<_>>();
        let output = model
            .teacher
            .forward(tensor(&images.batch(&indices), device))
            .detach();
        let [n, c, h, w] = output.dims();
        let values = output.into_data().try_to_vec::<f32>().map_err(record)?;
        for sample in 0..n {
            for channel in 0..c {
                for value in
                    &values[(sample * c + channel) * h * w..(sample * c + channel + 1) * h * w]
                {
                    sums[channel] += *value as f64;
                    squares[channel] += (*value as f64).powi(2);
                }
            }
        }
        count += n * h * w;
    }
    let mean = sums
        .iter()
        .map(|x| (*x / count as f64) as f32)
        .collect::<Vec<_>>();
    let std = squares
        .iter()
        .zip(&sums)
        .map(|(q, s)| {
            (q / count as f64 - (s / count as f64).powi(2))
                .max(1e-12)
                .sqrt() as f32
        })
        .collect::<Vec<_>>();
    if mean.iter().chain(&std).any(|x| !x.is_finite()) {
        return Err(Error::Invalid("Non-finite teacher normalization".into()));
    }
    Ok(Normalization { mean, std })
}
fn evaluate(
    model: &EfficientAdModel,
    data: &EfficientAdDataset,
    normalization: &Normalization,
    config: &EfficientAdConfig,
    device: &Device,
    cancel: &CancellationToken,
) -> Result<f32> {
    let model = model.valid();
    let mut sum = 0.0f64;
    for start in (0..data.images.shape[0]).step_by(config.batch_size) {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let indices =
            (start..(start + config.batch_size).min(data.images.shape[0])).collect::<Vec<_>>();
        sum += scalar(model.loss(
            &data.batch(&indices),
            normalization,
            config.hard_quantile,
            device,
        )?)? as f64
            * indices.len() as f64;
    }
    Ok((sum / data.images.shape[0] as f64) as f32)
}
fn calibrate(
    model: &EfficientAdModel,
    images: &TensorData,
    normalization: &Normalization,
    batch: usize,
    device: &Device,
    cancel: &CancellationToken,
) -> Result<EfficientAdCalibration> {
    let model = model.valid();
    let mut st = Vec::new();
    let mut ae = Vec::new();
    for start in (0..images.shape[0]).step_by(batch) {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let indices = (start..(start + batch).min(images.shape[0])).collect::<Vec<_>>();
        let (a, b) = model.maps(tensor(&images.batch(&indices), device), normalization);
        st.extend(a.into_data().try_to_vec::<f32>().map_err(record)?);
        ae.extend(b.into_data().try_to_vec::<f32>().map_err(record)?);
    }
    Ok(EfficientAdCalibration {
        teacher_student_low: quantile_value(st.clone(), 0.9)?,
        teacher_student_high: quantile_value(st, 0.995)?,
        autoencoder_low: quantile_value(ae.clone(), 0.9)?,
        autoencoder_high: quantile_value(ae, 0.995)?,
    })
}
fn quantile_value(mut values: Vec<f32>, q: f32) -> Result<f32> {
    if values.is_empty() || values.iter().any(|x| !x.is_finite()) {
        return Err(Error::Invalid(
            "Anomaly distances are empty or non-finite".into(),
        ));
    }
    values.sort_by(f32::total_cmp);
    let index = q as f64 * (values.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    Ok(values[lower] + (values[upper] - values[lower]) * (index - lower as f64) as f32)
}
fn tensor<const D: usize>(data: &TensorData, device: &Device) -> Tensor<D> {
    Tensor::from_data(
        BurnData::new(data.values.clone(), data.shape.clone()),
        device,
    )
}
fn scalar(value: Tensor<1>) -> Result<f32> {
    let values = value.into_data().try_to_vec::<f32>().map_err(record)?;
    if values.len() != 1 || !values[0].is_finite() {
        return Err(Error::Invalid(
            "EfficientAD produced a non-finite loss".into(),
        ));
    }
    Ok(values[0])
}
fn fingerprint(value: &impl Serialize) -> Result<String> {
    use sha2::{Digest, Sha256};
    struct Writer(Sha256);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Sha256::new());
    serde_json::to_writer(&mut writer, value)?;
    Ok(format!("{:x}", writer.0.finalize()))
}
fn checkpoint(
    dir: &Path,
    model: &EfficientAdModel,
    optimizer: &ModuleOptimizer,
    state: &mut State,
) -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(record)?
        .as_nanos();
    let name = format!(
        "checkpoint-{}-{}-{}",
        state.report.steps,
        stamp,
        CHECKPOINT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let target = dir.join(&name);
    fs::create_dir(&target)?;
    model
        .clone()
        .save_file(target.join("model.bpk"))
        .map_err(record)?;
    optimizer
        .save(target.join("optimizer.bpk"))
        .map_err(record)?;
    engine::sync_record(&target.join("model.bpk"))?;
    engine::sync_record(&target.join("optimizer.bpk"))?;
    engine::sync_directory(&target)?;
    engine::sync_directory(dir)?;
    state.checkpoint = name;
    let temporary = dir.join("efficient-ad-state.json.pending");
    let file = fs::File::create(&temporary)?;
    serde_json::to_writer_pretty(&file, state)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, dir.join(MANIFEST))?;
    engine::sync_directory(dir)?;
    Ok(())
}
fn read_state(dir: &Path) -> Result<State> {
    let state: State = serde_json::from_reader(fs::File::open(dir.join(MANIFEST))?)?;
    let a = &state.architecture;
    if state.version != 1
        || a.input == 0
        || a.input > 16
        || a.width == 0
        || a.width > 256
        || a.features == 0
        || a.features > 1024
        || a.autoencoder_width < 2
        || a.autoencoder_width > 256
        || state.normalization.mean.len() != a.features
        || state.normalization.std.len() != a.features
        || state.normalization.mean.iter().any(|x| !x.is_finite())
        || state
            .normalization
            .std
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.0)
    {
        return Err(Error::Invalid(
            "Invalid EfficientAD artifact metadata".into(),
        ));
    }
    if let Some(c) = &state.calibration {
        if [
            c.teacher_student_low,
            c.teacher_student_high,
            c.autoencoder_low,
            c.autoencoder_high,
        ]
        .iter()
        .any(|x| !x.is_finite())
            || c.teacher_student_high < c.teacher_student_low
            || c.autoencoder_high < c.autoencoder_low
        {
            return Err(Error::Invalid("Invalid EfficientAD map calibration".into()));
        }
    }
    Ok(state)
}

pub struct EfficientAdPredictor {
    model: EfficientAdModel,
    device: Device,
    normalization: Normalization,
    calibration: EfficientAdCalibration,
    input_channels: usize,
    backend: BackendChoice,
}
impl EfficientAdPredictor {
    pub fn load(artifact_dir: impl AsRef<Path>, backend: BackendChoice) -> Result<Self> {
        let dir = artifact_dir.as_ref();
        let state = read_state(dir)?;
        let backend = backend::resolve_backend(&backend)?;
        let device = backend::device(&backend, false)?;
        let calibration = state.calibration.ok_or_else(|| {
            Error::Invalid("EfficientAD training has not completed validation calibration".into())
        })?;
        let model = EfficientAdModel::new(&state.architecture, &device)
            .try_load_file(
                crate::engine::checked_checkpoint(dir, &state.checkpoint)?.join("model.bpk"),
            )
            .map_err(record)?
            .valid();
        Ok(Self {
            model,
            device,
            normalization: state.normalization,
            calibration,
            input_channels: state.architecture.input,
            backend,
        })
    }
    pub fn backend(&self) -> &BackendChoice {
        &self.backend
    }
    pub fn calibration(&self) -> &EfficientAdCalibration {
        &self.calibration
    }
    pub fn predict(&self, images: &TensorData) -> Result<EfficientAdPrediction> {
        validate_images(images, self.input_channels)?;
        let (st, ae) = self
            .model
            .maps(tensor(images, &self.device), &self.normalization);
        let c = &self.calibration;
        let st = (st - c.teacher_student_low)
            * (0.1 / (c.teacher_student_high - c.teacher_student_low).max(1e-6));
        let ae =
            (ae - c.autoencoder_low) * (0.1 / (c.autoencoder_high - c.autoencoder_low).max(1e-6));
        let combined = (st.clone() + ae.clone()) * 0.5;
        let display = |x: Tensor<4>| resize(x.pad([(0, 0), (0, 0), (4, 4), (4, 4)], 0.0), 256);
        let anomaly_maps = to_data(display(combined))?;
        let scores = anomaly_maps
            .values
            .chunks_exact(256 * 256)
            .map(|x| x.iter().copied().fold(f32::NEG_INFINITY, f32::max))
            .collect();
        Ok(EfficientAdPrediction {
            anomaly_maps,
            teacher_student_maps: to_data(display(st))?,
            autoencoder_maps: to_data(display(ae))?,
            scores,
        })
    }
}
fn to_data<const D: usize>(tensor: Tensor<D>) -> Result<TensorData> {
    let shape = tensor.dims().to_vec();
    let values = tensor.into_data().try_to_vec::<f32>().map_err(record)?;
    let data = TensorData { shape, values };
    data.validate()?;
    Ok(data)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    fn fixture() -> (EfficientAdConfig, EfficientAdDataset, TensorData) {
        let device = Device::flex();
        device.seed(73);
        let teacher = PdnSmall::new(1, 2, 2, &device);
        let convolutions = teacher
            .convolutions
            .iter()
            .map(|conv| ConvWeights {
                weight: to_data(conv.weight.val()).unwrap(),
                bias: to_data(conv.bias.as_ref().unwrap().val()).unwrap(),
            })
            .collect();
        let teacher = PdnTeacherWeights {
            input_channels: 1,
            base_channels: 2,
            feature_channels: 2,
            convolutions,
        };
        let values = (0..2 * 256 * 256)
            .map(|i| {
                ((i % 256) as f32 * 0.03).sin()
                    + (((i / 256) % 256) as f32 * 0.04).cos()
                    + (i / (256 * 256)) as f32 * 0.05
            })
            .collect::<Vec<_>>();
        let images = TensorData {
            shape: vec![2, 1, 256, 256],
            values,
        };
        let augment = TensorData {
            shape: images.shape.clone(),
            values: images.values.iter().map(|x| x * 0.98 + 0.02).collect(),
        };
        let validation = TensorData {
            shape: vec![1, 1, 256, 256],
            values: images.values[..256 * 256]
                .iter()
                .map(|x| x + 0.03)
                .collect(),
        };
        (
            EfficientAdConfig {
                teacher,
                backend: BackendChoice::Cpu,
                epochs: 4,
                batch_size: 1,
                learning_rate: 0.001,
                seed: 73,
                autoencoder_channels: 2,
                hard_quantile: 0.99,
                gradient_clip: 5.0,
            },
            EfficientAdDataset {
                images,
                autoencoder_images: Some(augment),
                penalty_images: None,
            },
            validation,
        )
    }
    #[test]
    fn efficient_ad_trains_dual_branches_calibrates_and_resumes_on_cpu() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (config, data, validation) = fixture();
        let initial_device = Device::flex();
        initial_device.seed(config.seed);
        let initial = EfficientAdModel::new(&Architecture::from(&config), &initial_device);
        let initial_student = to_data(initial.student.convolutions[3].weight.val()).unwrap();
        let initial_autoencoder = to_data(initial.autoencoder.final_conv.weight.val()).unwrap();
        let full = tempfile::tempdir().unwrap();
        let report = train_efficient_ad(
            &config,
            &data,
            &validation,
            full.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(
            report.final_loss < report.initial_loss,
            "Anomaly loss should decrease: {report:?}"
        );
        let first = EfficientAdPredictor::load(full.path(), BackendChoice::Cpu).unwrap();
        let trained_student = to_data(first.model.student.convolutions[3].weight.val()).unwrap();
        let branch_width = trained_student.values.len() / 2;
        assert_ne!(
            trained_student.values[..branch_width],
            initial_student.values[..branch_width],
            "Teacher/student branch must update"
        );
        assert_ne!(
            trained_student.values[branch_width..],
            initial_student.values[branch_width..],
            "Autoencoder/student branch must update"
        );
        assert_ne!(
            to_data(first.model.autoencoder.final_conv.weight.val()).unwrap(),
            initial_autoencoder,
            "Autoencoder must update"
        );
        let predicted = first.predict(&validation).unwrap();
        let reloaded = EfficientAdPredictor::load(full.path(), BackendChoice::Cpu)
            .unwrap()
            .predict(&validation)
            .unwrap();
        assert_eq!(predicted.anomaly_maps, reloaded.anomaly_maps);
        assert_eq!(predicted.anomaly_maps.shape, vec![1, 1, 256, 256]);
        assert!(predicted.scores.iter().all(|x| x.is_finite()));
        let c = first.calibration();
        assert!(
            c.teacher_student_high >= c.teacher_student_low
                && c.autoencoder_high >= c.autoencoder_low
        );
        for ((a, b), combined) in predicted
            .teacher_student_maps
            .values
            .iter()
            .zip(&predicted.autoencoder_maps.values)
            .zip(&predicted.anomaly_maps.values)
        {
            assert!(((a + b) * 0.5 - combined).abs() < 1e-5);
        }
        for (actual, expected) in first
            .model
            .teacher
            .convolutions
            .iter()
            .zip(&config.teacher.convolutions)
        {
            assert_eq!(
                to_data(actual.weight.val()).unwrap(),
                expected.weight,
                "Teacher must remain frozen"
            );
        }
        let partial = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let control = cancel.clone();
        let result =
            train_efficient_ad(&config, &data, &validation, partial.path(), &cancel, |p| {
                if p.steps == 3 {
                    control.cancel();
                }
            });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(
            EfficientAdPredictor::load(partial.path(), BackendChoice::Cpu).is_err(),
            "Uncalibrated checkpoint must not be deployed"
        );
        let mut automatic_resume = config.clone();
        automatic_resume.backend = BackendChoice::Auto;
        let resumed = resume_efficient_ad(
            &automatic_resume,
            &data,
            &validation,
            partial.path(),
            &CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(resumed.steps, report.steps);
        assert_eq!(resumed.backend, Some(BackendChoice::Cpu));
        let actual = EfficientAdPredictor::load(partial.path(), BackendChoice::Cpu)
            .unwrap()
            .predict(&validation)
            .unwrap();
        let error = actual
            .anomaly_maps
            .values
            .iter()
            .zip(&predicted.anomaly_maps.values)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            error < 1e-5,
            "Resume must restore optimizer and random stream: {error}"
        );
        let mut changed = config.clone();
        changed.teacher.convolutions[0].weight.values[0] += 0.01;
        assert!(
            resume_efficient_ad(
                &changed,
                &data,
                &validation,
                partial.path(),
                &CancellationToken::new(),
                |_| {}
            )
            .is_err()
        );
    }
    #[test]
    fn efficient_ad_rejects_incompatible_teacher_and_image_shapes() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (mut config, data, _) = fixture();
        config.teacher.convolutions[0].weight.shape[0] += 1;
        assert!(config.validate().is_err());
        assert!(data.validate(3).is_err());
        assert_eq!(quantile_value(vec![0.0, 2.0, 4.0], 0.75).unwrap(), 3.0);
    }
}

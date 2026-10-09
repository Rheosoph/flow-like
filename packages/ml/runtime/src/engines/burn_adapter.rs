use super::*;
use flow_like_ml_burn as burn;
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct BurnEngineRecipe {
    pub config: burn::TrainingConfig,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub preprocessing: Vec<flow_like_ml_core::PreprocessingStep>,
    #[serde(default)]
    pub pretrained: Option<PretrainedSourceRef>,
}

fn fine_tune_options(source: &PretrainedSourceRef) -> burn::FineTuneOptions {
    burn::FineTuneOptions {
        replace_head: source.replace_head,
        freeze_backbone: source.freeze_backbone,
    }
}

fn authorized_source(
    repository: &TrainingRepository,
    project_id: &str,
    source_id: &str,
    maximum_bytes: u64,
) -> Result<PretrainedSource> {
    if project_id.trim().is_empty() || source_id.trim().is_empty() {
        return Err(invalid(
            "pretrained source requires a project and source ID",
        ));
    }
    let source = repository.get_pretrained_source(source_id)?;
    if source.project_id != project_id {
        return Err(invalid("pretrained source belongs to a different project"));
    }
    if source.manifest["engine"] != "burn"
        || source.manifest["format_version"].as_u64() != Some(1)
        || source.blob.bytes == 0
        || source.blob.bytes > maximum_bytes
    {
        return Err(invalid(
            "pretrained source format or byte budget is incompatible",
        ));
    }
    Ok(source)
}

fn unpack_source(
    repository: &TrainingRepository,
    source: &PretrainedSource,
    maximum_bytes: u64,
) -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    let bytes = repository.read_blob_limited(&source.blob, maximum_bytes)?;
    unpack(&bytes, directory.path(), maximum_bytes)?;
    let state: Value =
        serde_json::from_slice(&fs::read(directory.path().join(burn::MANIFEST_FILE))?)?;
    if state["config"] != source.manifest["config"]
        || state["input_shape"] != source.manifest["input_shape"]
        || state.get("imported").filter(|value| !value.is_null())
            != source
                .manifest
                .get("imported")
                .filter(|value| !value.is_null())
    {
        return Err(invalid(
            "pretrained manifest differs from its packed model state",
        ));
    }
    Ok(directory)
}

/// Extract spatial features from project-owned weights without creating training evidence.
pub fn pretrained_source_features(
    repository: &TrainingRepository,
    project_id: &str,
    source_id: &str,
    input: &TensorData,
    compute: &ComputeConfig,
) -> Result<TensorData> {
    compute.validate().map_err(engine_error)?;
    let source = authorized_source(repository, project_id, source_id, source_limit(compute))?;
    let config: burn::TrainingConfig = serde_json::from_value(source.manifest["config"].clone())?;
    config.validate().map_err(engine_error)?;
    if !matches!(
        config.recipe,
        burn::Recipe::ResNet18 { .. }
            | burn::Recipe::MobileNetV2 { .. }
            | burn::Recipe::EfficientNet { .. }
            | burn::Recipe::DinoV2 { .. }
    ) {
        return Err(invalid(
            "Spatial features require ResNet18, MobileNetV2, EfficientNet or DINOv2",
        ));
    }
    let input_limit = (compute.memory_limit_bytes / 64).min(64 * 1024 * 1024) as usize;
    input.validate(input_limit).map_err(engine_error)?;
    let expected: Vec<usize> = serde_json::from_value(source.manifest["input_shape"].clone())?;
    let batch_shape = image_batch_shape(&input.shape, &expected)?;
    config
        .recipe
        .validate_shape(&batch_shape)
        .map_err(engine_error)?;
    let workspace = match &config.recipe {
        burn::Recipe::DinoV2 { config } => {
            // Inference retains one transformer block at a time. Reserve attention
            // matrices, projected tokens, normalization and input/output copies.
            let batch = batch_shape[0] as u128;
            let tokens = (batch_shape[2] as u128 / 14) * (batch_shape[3] as u128 / 14) + 1;
            let bytes = batch
                * (tokens * tokens * config.variant.heads() as u128 * 16
                    + tokens * config.variant.embedding_dim() as u128 * 128)
                + input.values.len() as u128 * 16;
            u64::try_from(bytes).map_err(|_| invalid("feature workspace estimate overflows"))?
        }
        _ => config
            .recipe
            .estimated_training_bytes(&expected, batch_shape[0])
            .map_err(engine_error)?,
    };
    let estimate = source
        .blob
        .bytes
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(workspace))
        .ok_or_else(|| invalid("feature memory estimate overflows"))?;
    if estimate > compute.memory_limit_bytes {
        return Err(invalid(format!(
            "Feature extraction needs an estimated {estimate} bytes, exceeding the configured {} byte memory limit",
            compute.memory_limit_bytes
        )));
    }
    let directory = unpack_source(repository, &source, source_limit(compute))?;
    let predictor =
        burn::Predictor::load(directory.path(), backend(compute)?).map_err(engine_error)?;
    let output = predictor
        .features(&burn::TensorData {
            shape: batch_shape,
            values: input.values.clone(),
        })
        .map_err(engine_error)?;
    let output = TensorData {
        shape: output.shape,
        values: output.values,
    };
    output.validate(input_limit).map_err(engine_error)?;
    Ok(output)
}

/// Resolve an initialization model within the owning project before admitting training.
pub fn validate_burn_pretrained_source(
    repository: &TrainingRepository,
    project_id: &str,
    reference: &PretrainedSourceRef,
    config: &burn::TrainingConfig,
    labels: &[String],
    maximum_bytes: u64,
) -> Result<PretrainedSource> {
    let source = authorized_source(repository, project_id, &reference.source_id, maximum_bytes)?;
    let source_config: burn::TrainingConfig =
        serde_json::from_value(source.manifest["config"].clone())?;
    source_config.validate().map_err(engine_error)?;
    burn::validate_fine_tune(
        &source_config.recipe,
        &config.recipe,
        &fine_tune_options(reference),
    )
    .map_err(engine_error)?;
    let source_labels: Vec<String> = serde_json::from_value(
        source
            .manifest
            .get("labels")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    flow_like_ml_core::validate_labels(&source_labels).map_err(engine_error)?;
    if !reference.replace_head && source_labels != labels {
        return Err(invalid(
            "retaining a pretrained head requires the same label order",
        ));
    }
    if !reference.replace_head
        && source
            .manifest
            .pointer("/imported/classification_head_pretrained")
            .and_then(Value::as_bool)
            == Some(false)
    {
        return Err(invalid(
            "imported backbone requires a new classification head",
        ));
    }
    Ok(source)
}

/// Register a converted Burn model as initialization weights without inventing training evidence.
pub fn publish_burn_pretrained_source(
    repository: &TrainingRepository,
    project_id: &str,
    directory: &Path,
    labels: Vec<String>,
    origin: Value,
    maximum_bytes: u64,
    at_ms: i64,
) -> Result<PretrainedSource> {
    flow_like_ml_core::validate_labels(&labels).map_err(engine_error)?;
    let model_bytes = pack(directory, false, maximum_bytes)?;
    let state: Value = serde_json::from_slice(&fs::read(directory.join(burn::MANIFEST_FILE))?)?;
    let config: burn::TrainingConfig = serde_json::from_value(state["config"].clone())?;
    config.validate().map_err(engine_error)?;
    let input_shape: Vec<usize> = serde_json::from_value(state["input_shape"].clone())?;
    let mut batch_shape = vec![1];
    batch_shape.extend(&input_shape);
    config
        .recipe
        .validate_shape(&batch_shape)
        .map_err(engine_error)?;
    if !labels.is_empty() && labels.len() != config.recipe.outputs() {
        return Err(invalid(
            "pretrained labels must match the model output count",
        ));
    }
    let recommended_preprocessing = match state.get("imported").filter(|value| !value.is_null()) {
        Some(provenance) => burn::ImportedModelInfo {
            recipe: config.recipe.clone(),
            input_shape: input_shape.clone(),
            provenance: serde_json::from_value(provenance.clone())?,
        }
        .recommended_preprocessing(),
        None => Vec::new(),
    };
    repository.register_pretrained_source(
        project_id,
        &model_bytes,
        serde_json::json!({
            "engine":"burn", "format_version":1, "config":config,
            "input_shape":input_shape, "labels":labels,
            "imported":state["imported"], "pretrained":state["pretrained"],
            "recommended_preprocessing":recommended_preprocessing,
        }),
        origin,
        maximum_bytes,
        at_ms,
    )
}

fn source_limit(compute: &ComputeConfig) -> u64 {
    (compute.memory_limit_bytes / 4).min(512 * 1024 * 1024)
}

pub fn register_burn_engine(worker: &mut TrainingWorker) -> Result<()> {
    worker.register("burn", Arc::new(BurnEngine))
}

struct BurnEngine;
pub(super) fn resource_key(compute: &ComputeConfig) -> Result<String> {
    Ok(match backend(compute)? {
        burn::BackendChoice::Auto => return Err(engine_error("backend resolution returned Auto")),
        burn::BackendChoice::Cpu => "cpu:0".into(),
        burn::BackendChoice::Wgpu => "wgpu:0".into(),
        burn::BackendChoice::WgpuAdapter { kind, device } => format!("wgpu:{kind:?}:{device}"),
        burn::BackendChoice::Cuda { device } => format!("cuda:{device}"),
        burn::BackendChoice::Rocm { device } => format!("rocm:{device}"),
    })
}
impl TrainingEngine for BurnEngine {
    fn resource_key(&self, work: &TrainingWork) -> Result<String> {
        let compute = compute_config(&work.job.request.compute)?;
        resource_key(&compute)
    }
    fn estimated_memory_bytes(&self, work: &TrainingWork) -> Result<u64> {
        let recipe: BurnEngineRecipe = serde_json::from_value(work.job.request.recipe.clone())?;
        recipe.config.validate().map_err(engine_error)?;
        let sample = work
            .snapshot
            .train
            .first()
            .ok_or_else(|| invalid("empty tensor partition"))?;
        let sample = payload_sample(sample, recipe.labels.len())?;
        let model = recipe
            .config
            .recipe
            .estimated_training_bytes(&sample.input.shape, recipe.config.batch_size)
            .map_err(engine_error)?;
        let dataset = serialized_size(&work.snapshot)?;
        let source_memory = if let Some(reference) = &recipe.pretrained {
            let compute = compute_config(&work.job.request.compute)?;
            let source = validate_burn_pretrained_source(
                &work.repository,
                &work.job.stream.project_id,
                reference,
                &recipe.config,
                &recipe.labels,
                source_limit(&compute),
            )?;
            if work.resume_checkpoint.is_some() {
                0
            } else {
                let config: burn::TrainingConfig =
                    serde_json::from_value(source.manifest["config"].clone())?;
                let shape: Vec<usize> =
                    serde_json::from_value(source.manifest["input_shape"].clone())?;
                source.blob.bytes.saturating_mul(4).saturating_add(
                    config
                        .recipe
                        .estimated_training_bytes(&shape, 1)
                        .map_err(engine_error)?,
                )
            }
        } else {
            0
        };
        Ok(model
            .saturating_add(dataset.saturating_mul(12))
            .saturating_add(source_memory))
    }
    fn train(&self, work: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput> {
        let mut recipe: BurnEngineRecipe = serde_json::from_value(work.job.request.recipe.clone())?;
        flow_like_ml_core::validate_labels(&recipe.labels).map_err(engine_error)?;
        if recipe.config.recipe.objective() == Some(burn::Objective::Classification)
            && recipe.labels.len() != recipe.config.recipe.outputs()
        {
            return Err(invalid(
                "classification labels must match the model output count",
            ));
        }
        let compute = compute_config(&work.job.request.compute)?;
        recipe.config.backend = backend(&compute)?;
        let training = dataset(
            &work.snapshot.train,
            &recipe.config.recipe,
            recipe.labels.len(),
        )?;
        let validation = dataset(
            &work.snapshot.validation,
            &recipe.config.recipe,
            recipe.labels.len(),
        )?;
        let directory = tempfile::tempdir()?;
        if let Some(bytes) = &work.resume_checkpoint {
            unpack(
                bytes,
                directory.path(),
                control.limits().maximum_checkpoint_bytes,
            )?;
        }
        let pretrained = recipe
            .pretrained
            .as_ref()
            .map(|reference| {
                validate_burn_pretrained_source(
                    &work.repository,
                    &work.job.stream.project_id,
                    reference,
                    &recipe.config,
                    &recipe.labels,
                    source_limit(&compute),
                )
            })
            .transpose()?;
        let source_directory = if work.resume_checkpoint.is_none() {
            pretrained
                .as_ref()
                .map(|source| unpack_source(&work.repository, source, source_limit(&compute)))
                .transpose()?
        } else {
            None
        };
        let cancellation = burn::CancellationToken::new();
        let cancellation_monitor = BurnCancellationMonitor::start(control, &cancellation)?;
        let mut interruption = None;
        let mut checkpoint_error = None;
        let mut latest_published = 0;
        let mut callback = |progress: burn::TrainingProgress| {
            if let Err(error) =
                control.progress(serde_json::to_value(&progress).unwrap_or(Value::Null))
            {
                interruption = Some(error);
                cancellation.cancel();
                return;
            }
            // Burn publishes its epoch checkpoint before emitting validation progress.
            if progress.validation_loss.is_some() && progress.steps > latest_published {
                let result=pack(directory.path(),true,control.limits().maximum_checkpoint_bytes)
                    .and_then(|bytes|control.checkpoint(&bytes,progress.steps as u64,serde_json::json!({"engine":"burn","format_version":1,"epoch":progress.epoch,"steps":progress.steps})).map(|_|()));
                if let Err(error) = result {
                    checkpoint_error = Some(error);
                    cancellation.cancel();
                } else {
                    latest_published = progress.steps;
                }
            }
        };
        let trained = if work.resume_checkpoint.is_some() {
            burn::resume(
                &recipe.config,
                &training,
                Some(&validation),
                directory.path(),
                &cancellation,
                &mut callback,
            )
        } else if let Some(source_directory) = &source_directory {
            burn::train_from_pretrained(
                &recipe.config,
                &training,
                Some(&validation),
                source_directory.path(),
                &fine_tune_options(recipe.pretrained.as_ref().unwrap()),
                directory.path(),
                &cancellation,
                &mut callback,
            )
        } else {
            burn::train(
                &recipe.config,
                &training,
                Some(&validation),
                directory.path(),
                &cancellation,
                &mut callback,
            )
        };
        let cancellation_reason = cancellation_monitor.finish();
        if trained.is_err() && directory.path().join(burn::MANIFEST_FILE).exists() {
            // Burn checkpoints the last completed batch on cooperative cancellation.
            let state: Value =
                serde_json::from_slice(&fs::read(directory.path().join(burn::MANIFEST_FILE))?)?;
            let step = state
                .pointer("/report/steps")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let bytes = pack(
                directory.path(),
                true,
                control.limits().maximum_checkpoint_bytes,
            )?;
            let _ = control.checkpoint_after_stop(
                &bytes,
                step,
                serde_json::json!({"engine":"burn","format_version":1,"steps":step}),
            );
        }
        if let Some(error) = interruption.or(checkpoint_error).or(cancellation_reason) {
            return Err(error);
        }
        let report = trained.map_err(|error| {
            if matches!(error, burn::Error::Cancelled) {
                Error::Cancelled
            } else {
                engine_error(error)
            }
        })?;
        control.check_cancel()?;
        let bytes = pack(
            directory.path(),
            true,
            control.limits().maximum_checkpoint_bytes,
        )?;
        control.checkpoint(
            &bytes,
            report.steps as u64,
            serde_json::json!({"engine":"burn","format_version":1,"steps":report.steps}),
        )?;
        let model_bytes = pack(
            directory.path(),
            false,
            control.limits().maximum_artifact_bytes,
        )?;
        let state: Value =
            serde_json::from_slice(&fs::read(directory.path().join(burn::MANIFEST_FILE))?)?;
        let initialization = pretrained.map(|source| {
            serde_json::json!({
                "source_id":source.id, "source_kind":source.source_kind,
                "source_blob":source.blob, "source_origin":source.origin,
                "options":recipe.pretrained, "training":state["pretrained"],
            })
        });
        Ok(EngineOutput {
            model_bytes,
            manifest: serde_json::json!({"engine":"burn","format_version":1,"config":recipe.config,"labels":recipe.labels,"input_shape":training.inputs.shape[1..],"preprocessing":recipe.preprocessing,"report":report,"pretrained":initialization}),
        })
    }
}

fn dataset(
    samples: &[TrainingSample],
    recipe: &burn::Recipe,
    classes: usize,
) -> Result<burn::TensorDataset> {
    let samples = samples
        .iter()
        .map(|sample| payload_sample(sample, classes))
        .collect::<Result<Vec<_>>>()?;
    let first = samples
        .first()
        .ok_or_else(|| invalid("empty tensor partition"))?;
    if samples
        .iter()
        .any(|sample| sample.input.shape != first.input.shape)
    {
        return Err(invalid("training samples have inconsistent shapes"));
    }
    let mut shape = vec![samples.len()];
    shape.extend_from_slice(&first.input.shape);
    let inputs = burn::TensorData {
        shape,
        values: samples
            .iter()
            .flat_map(|sample| sample.input.values.iter().copied())
            .collect(),
    };
    let box_target = |bounds: &flow_like_ml_core::BoundingBox| burn::BoundingBox {
        class_id: bounds.class_id as usize,
        x_min: bounds.x_min,
        y_min: bounds.y_min,
        x_max: bounds.x_max,
        y_max: bounds.y_max,
    };
    let targets = if matches!(recipe, burn::Recipe::MaskRcnn { .. }) {
        burn::Targets::Instances {
            values: samples
                .iter()
                .map(|sample| {
                    let Annotation::InstanceMasks { instances } = &sample.annotation else {
                        return Err(invalid("Mask R-CNN requires instance masks"));
                    };
                    Ok(instances
                        .iter()
                        .map(|instance| burn::InstanceTarget {
                            bbox: box_target(&instance.bounds),
                            mask: burn::TensorData {
                                shape: vec![instance.height, instance.width],
                                values: instance
                                    .foreground
                                    .iter()
                                    .map(|value| if *value { 1.0 } else { 0.0 })
                                    .collect(),
                            },
                        })
                        .collect())
                })
                .collect::<Result<_>>()?,
        }
    } else if recipe.is_detection() {
        burn::Targets::Boxes {
            values: samples
                .iter()
                .map(|sample| {
                    let Annotation::Boxes { boxes } = &sample.annotation else {
                        return Err(invalid("Object detection requires box annotations"));
                    };
                    Ok(boxes.iter().map(&box_target).collect())
                })
                .collect::<Result<_>>()?,
        }
    } else if recipe.is_segmentation() {
        let mut shape = vec![samples.len()];
        let mut values = Vec::new();
        let mut mask_shape = None;
        for sample in &samples {
            let Annotation::Mask {
                width,
                height,
                classes,
            } = &sample.annotation
            else {
                return Err(invalid("segmentation requires dense masks"));
            };
            if mask_shape.is_some_and(|dims| dims != (*height, *width)) {
                return Err(invalid("segmentation mask dimensions differ"));
            }
            mask_shape = Some((*height, *width));
            values.extend(classes.iter().map(|c| *c as i64));
        }
        let (height, width) = mask_shape.unwrap();
        shape.extend([height, width]);
        burn::Targets::Segmentation { shape, values }
    } else {
        match recipe.objective() {
            Some(burn::Objective::Classification) => burn::Targets::Classes {
                values: samples
                    .iter()
                    .map(|sample| match sample.annotation {
                        Annotation::Class { class_id } => Ok(class_id as i64),
                        _ => Err(invalid("classification requires class annotations")),
                    })
                    .collect::<Result<_>>()?,
            },
            Some(burn::Objective::Regression) => {
                let values = samples
                    .iter()
                    .map(|sample| {
                        let values = match &sample.annotation {
                            Annotation::Scalar { value } => vec![*value as f32],
                            Annotation::Values { values } => values.clone(),
                            _ => {
                                return Err(invalid(
                                    "regression requires scalar/dense annotations",
                                ));
                            }
                        };
                        if values.len() != recipe.outputs() {
                            return Err(invalid(format!(
                                "regression sample {} has target width {}; model requires {}",
                                sample.id,
                                values.len(),
                                recipe.outputs()
                            )));
                        }
                        Ok(values)
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                burn::Targets::Dense {
                    tensor: burn::TensorData {
                        shape: vec![samples.len(), recipe.outputs()],
                        values,
                    },
                }
            }
            None => {
                if samples.iter().any(|sample| {
                    matches!(sample.annotation, Annotation::Anomaly { is_anomaly: true })
                }) {
                    return Err(invalid(
                        "reconstruction anomaly training requires normal examples",
                    ));
                }
                burn::Targets::Reconstruction
            }
        }
    };
    let dataset = burn::TensorDataset { inputs, targets };
    dataset.validate(recipe).map_err(engine_error)?;
    Ok(dataset)
}

pub(super) fn compute_config(value: &Value) -> Result<ComputeConfig> {
    let mut default = serde_json::to_value(ComputeConfig::default())?;
    let incoming = value
        .as_object()
        .ok_or_else(|| invalid("compute must be an object"))?;
    for (key, value) in incoming {
        default[key] = value.clone();
    }
    Ok(serde_json::from_value(default)?)
}
pub(super) fn backend(compute: &ComputeConfig) -> Result<burn::BackendChoice> {
    backend_with(compute, burn::resolve_backend_with_fallback)
}

pub(super) fn concrete_compute(
    requested: &ComputeConfig,
    choice: &burn::BackendChoice,
) -> Result<ComputeConfig> {
    let (backend, device_index) = match choice {
        burn::BackendChoice::Auto => return Err(engine_error("backend resolution returned Auto")),
        burn::BackendChoice::Cpu => (ComputeBackend::Cpu, 0),
        burn::BackendChoice::Wgpu => (ComputeBackend::Wgpu, 0),
        burn::BackendChoice::WgpuAdapter { .. } => {
            return Err(engine_error(
                "runtime compute cannot describe a typed WGPU adapter",
            ));
        }
        burn::BackendChoice::Cuda { device } => (ComputeBackend::Cuda, *device),
        burn::BackendChoice::Rocm { device } => (ComputeBackend::Rocm, *device),
    };
    Ok(ComputeConfig {
        backend,
        device_index,
        allow_cpu_fallback: false,
        memory_limit_bytes: requested.memory_limit_bytes,
    })
}

fn backend_with(
    compute: &ComputeConfig,
    resolve: impl FnOnce(&burn::BackendChoice, bool) -> burn::Result<burn::BackendChoice>,
) -> Result<burn::BackendChoice> {
    compute.validate().map_err(engine_error)?;
    if matches!(
        compute.backend,
        ComputeBackend::Wgpu | ComputeBackend::Auto | ComputeBackend::Cpu
    ) && compute.device_index != 0
    {
        return Err(invalid(
            "generic CPU/WGPU/Auto selection requires device_index 0; use an explicit Burn WgpuAdapter with the lower-level engine for adapter selection",
        ));
    }
    let choice = match compute.backend {
        ComputeBackend::Cpu => burn::BackendChoice::Cpu,
        ComputeBackend::Wgpu => burn::BackendChoice::Wgpu,
        ComputeBackend::Cuda => burn::BackendChoice::Cuda {
            device: compute.device_index,
        },
        ComputeBackend::Rocm => burn::BackendChoice::Rocm {
            device: compute.device_index,
        },
        ComputeBackend::Auto => burn::BackendChoice::Auto,
    };
    resolve(&choice, compute.allow_cpu_fallback).map_err(engine_error)
}

#[cfg(test)]
pub(super) fn predict(bytes: &[u8], input: &TensorData, compute: &ComputeConfig) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    unpack(bytes, directory.path(), compute.memory_limit_bytes)?;
    let predictor =
        burn::Predictor::load(directory.path(), backend(compute)?).map_err(engine_error)?;
    let prediction = predictor
        .predict(&burn::TensorData {
            shape: input.shape.clone(),
            values: input.values.clone(),
        })
        .map_err(engine_error)?;
    Ok(serde_json::to_value(prediction)?)
}

#[cfg(feature = "onnx-export")]
pub(super) fn export(
    bytes: &[u8],
    input: &TensorData,
    compute: &ComputeConfig,
) -> Result<(Vec<u8>, burn::OnnxExportReport)> {
    let directory = tempfile::tempdir()?;
    unpack(bytes, directory.path(), compute.memory_limit_bytes)?;
    let predictor =
        burn::Predictor::load(directory.path(), backend(compute)?).map_err(engine_error)?;
    let path = directory.path().join("model.onnx");
    let report = predictor
        .export_onnx(
            &burn::TensorData {
                shape: input.shape.clone(),
                values: input.values.clone(),
            },
            &path,
        )
        .map_err(engine_error)?;
    if fs::metadata(&path)?.len() > compute.memory_limit_bytes {
        return Err(invalid(
            "ONNX artifact exceeds the configured memory budget",
        ));
    }
    Ok((fs::read(path)?, report))
}

// The envelope contains three fixed payloads, never archive paths. Its state points at
// one fixed directory on extraction, so model-supplied traversal cannot escape a temp root.
const MAGIC: &[u8; 8] = b"FLMLB001";
fn pack(directory: &Path, optimizer: bool, limit: u64) -> Result<Vec<u8>> {
    pack_named(directory, burn::MANIFEST_FILE, optimizer, limit)
}
pub(super) fn pack_named(
    directory: &Path,
    manifest: &str,
    optimizer: bool,
    limit: u64,
) -> Result<Vec<u8>> {
    let state_path = directory.join(manifest);
    if fs::metadata(&state_path)?.len() > 16 * 1024 * 1024 {
        return Err(invalid("model state exceeds metadata size limit"));
    }
    let mut state: Value = serde_json::from_slice(&fs::read(state_path)?)?;
    let checkpoint = state["checkpoint"]
        .as_str()
        .ok_or_else(|| invalid("checkpoint pointer is missing"))?;
    let checkpoint_prefix = if manifest == burn::SAM2_MANIFEST_FILE {
        "sam2-checkpoint-"
    } else {
        "checkpoint-"
    };
    if !checkpoint.starts_with(checkpoint_prefix)
        || checkpoint.contains('/')
        || checkpoint.contains('\\')
        || checkpoint.contains("..")
    {
        return Err(invalid("invalid checkpoint directory"));
    }
    let checkpoint = directory.join(checkpoint);
    let model_path = checkpoint.join("model.bpk");
    let optimizer_path = checkpoint.join("optimizer.bpk");
    let model_len = fs::metadata(&model_path)?.len();
    let optimizer_len = if optimizer {
        fs::metadata(&optimizer_path)?.len()
    } else {
        0
    };
    state["checkpoint"] = Value::String(format!("{checkpoint_prefix}runtime"));
    let state = serde_json::to_vec(&state)?;
    let total = 32u64
        .checked_add(state.len() as u64)
        .and_then(|n| n.checked_add(model_len))
        .and_then(|n| n.checked_add(optimizer_len))
        .ok_or_else(|| invalid("artifact length overflow"))?;
    if total > limit || total > usize::MAX as u64 {
        return Err(invalid("artifact exceeds byte budget"));
    }
    let mut output = Vec::with_capacity(total as usize);
    output.extend_from_slice(MAGIC);
    for length in [state.len() as u64, model_len, optimizer_len] {
        output.extend_from_slice(&length.to_le_bytes());
    }
    output.extend_from_slice(&state);
    output.extend_from_slice(&fs::read(model_path)?);
    if optimizer {
        output.extend_from_slice(&fs::read(optimizer_path)?);
    }
    Ok(output)
}
pub(super) fn unpack(bytes: &[u8], directory: &Path, limit: u64) -> Result<()> {
    unpack_named(bytes, directory, burn::MANIFEST_FILE, limit)
}
pub(super) fn unpack_named(
    bytes: &[u8],
    directory: &Path,
    manifest: &str,
    limit: u64,
) -> Result<()> {
    if bytes.len() < 32 || bytes.len() as u64 > limit || &bytes[..8] != MAGIC {
        return Err(invalid("invalid or oversized Burn artifact envelope"));
    }
    let lengths: [u64; 3] = std::array::from_fn(|i| {
        u64::from_le_bytes(bytes[8 + i * 8..16 + i * 8].try_into().unwrap())
    });
    let total = lengths
        .iter()
        .try_fold(32u64, |n, length| n.checked_add(*length))
        .ok_or_else(|| invalid("artifact length overflow"))?;
    if total != bytes.len() as u64 || lengths[0] > 16 * 1024 * 1024 || lengths[1] == 0 {
        return Err(invalid("artifact lengths are inconsistent"));
    }
    let state_end = 32 + lengths[0] as usize;
    let model_end = state_end + lengths[1] as usize;
    let mut state: Value = serde_json::from_slice(&bytes[32..state_end])?;
    let checkpoint_name = if manifest == burn::SAM2_MANIFEST_FILE {
        "sam2-checkpoint-runtime"
    } else {
        "checkpoint-runtime"
    };
    state["checkpoint"] = Value::String(checkpoint_name.into());
    let checkpoint = directory.join(checkpoint_name);
    fs::create_dir_all(&checkpoint)?;
    fs::write(checkpoint.join("model.bpk"), &bytes[state_end..model_end])?;
    if lengths[2] > 0 {
        fs::write(checkpoint.join("optimizer.bpk"), &bytes[model_end..])?;
    }
    fs::write(directory.join(manifest), serde_json::to_vec(&state)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_backend_selection_preserves_auto_fallback_and_device_contracts() {
        let compute = ComputeConfig {
            backend: ComputeBackend::Auto,
            allow_cpu_fallback: false,
            ..Default::default()
        };
        let result = backend_with(&compute, |choice, fallback| {
            assert_eq!(choice, &burn::BackendChoice::Auto);
            assert!(!fallback);
            Ok(burn::BackendChoice::Cpu)
        })
        .unwrap();
        assert_eq!(result, burn::BackendChoice::Cpu);

        for (backend, expected) in [
            (
                ComputeBackend::Cuda,
                burn::BackendChoice::Cuda { device: 2 },
            ),
            (
                ComputeBackend::Rocm,
                burn::BackendChoice::Rocm { device: 2 },
            ),
        ] {
            let mut compute = ComputeConfig {
                backend,
                device_index: 2,
                allow_cpu_fallback: false,
                ..Default::default()
            };
            assert!(
                backend_with(&compute, |choice, fallback| {
                    assert_eq!(choice, &expected);
                    assert!(!fallback);
                    Err(burn::Error::Backend("driver unavailable".into()))
                })
                .is_err()
            );
            compute.allow_cpu_fallback = true;
            assert_eq!(
                backend_with(&compute, |choice, fallback| {
                    assert_eq!(choice, &expected);
                    assert!(fallback);
                    Ok(burn::BackendChoice::Wgpu)
                })
                .unwrap(),
                burn::BackendChoice::Wgpu
            );
        }
        for backend in [
            ComputeBackend::Cpu,
            ComputeBackend::Wgpu,
            ComputeBackend::Auto,
        ] {
            assert!(
                backend_with(
                    &ComputeConfig {
                        backend,
                        device_index: 1,
                        ..Default::default()
                    },
                    |_, _| {
                        panic!("invalid generic device indices must fail before initialization")
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn cpu_selection_and_resource_reservation_resolve_the_same_backend() {
        let compute = ComputeConfig {
            backend: ComputeBackend::Cpu,
            ..Default::default()
        };
        assert_eq!(backend(&compute).unwrap(), burn::BackendChoice::Cpu);
        assert_eq!(resource_key(&compute).unwrap(), "cpu:0");
        assert_eq!(backend(&compute).unwrap(), burn::BackendChoice::Cpu);
        if burn::compiled_backends() == vec!["cpu"] {
            let automatic = ComputeConfig {
                backend: ComputeBackend::Auto,
                allow_cpu_fallback: false,
                ..Default::default()
            };
            assert_eq!(backend(&automatic).unwrap(), burn::BackendChoice::Cpu);
            assert_eq!(resource_key(&automatic).unwrap(), "cpu:0");
        }
    }

    #[test]
    fn artifact_envelope_rejects_truncation_overflow_and_extra_bytes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(unpack(b"FLMLB001", dir.path(), 1024).is_err());
        let mut corrupt = MAGIC.to_vec();
        for _ in 0..3 {
            corrupt.extend_from_slice(&u64::MAX.to_le_bytes());
        }
        assert!(unpack(&corrupt, dir.path(), 1024).is_err());
    }
}

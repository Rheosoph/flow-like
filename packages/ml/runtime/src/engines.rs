//! Adapters between the durable worker contract and native algorithms/Burn models.
use crate::{worker::*, *};
#[cfg(feature = "burn")]
use flow_like_ml_core::ComputeBackend;
use flow_like_ml_core::{Annotation, ComputeConfig, Sample, TensorData};
use std::sync::Arc;
mod inference;
pub use inference::LoadedArtifactPredictor;

#[cfg(feature = "burn")]
mod burn_adapter;
#[cfg(feature = "burn")]
mod efficient_ad_adapter;
#[cfg(feature = "burn")]
pub use burn_adapter::{BurnEngineRecipe, register_burn_engine};
#[cfg(feature = "burn")]
pub use efficient_ad_adapter::{EfficientAdEngineRecipe, register_efficient_ad_engine};

#[cfg(feature = "burn")]
pub fn artifact_features(
    repository: &TrainingRepository,
    artifact_id: &str,
    input: &TensorData,
    compute: &ComputeConfig,
) -> Result<TensorData> {
    LoadedArtifactPredictor::load(repository, artifact_id, compute)?.features(input)
}

#[cfg(feature = "onnx-export")]
pub fn export_artifact_onnx(
    repository: &TrainingRepository,
    artifact_id: &str,
    input: &TensorData,
    compute: &ComputeConfig,
) -> Result<(Vec<u8>, flow_like_ml_burn::OnnxExportReport)> {
    input.validate(16_777_216).map_err(engine_error)?;
    compute.validate().map_err(engine_error)?;
    let artifact = repository.get_artifact(artifact_id)?;
    if artifact.manifest["engine"] != "burn" {
        return Err(invalid("ONNX export requires a Burn model artifact"));
    }
    let limit = (compute.memory_limit_bytes / 4).min(512 * 1024 * 1024);
    burn_adapter::export(
        &repository.read_blob_limited(&artifact.blob, limit)?,
        input,
        compute,
    )
}

fn engine_error(error: impl std::fmt::Display) -> Error {
    Error::Engine(error.to_string())
}

fn serialized_size<T: Serialize + ?Sized>(value: &T) -> Result<u64> {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| std::io::Error::other("serialized size exceeds u64"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

fn payload_sample(sample: &TrainingSample, classes: usize) -> Result<Sample> {
    let value = sample
        .payload
        .get("sample")
        .unwrap_or(&sample.payload)
        .clone();
    let parsed: Sample = serde_json::from_value(value)?;
    parsed
        .validate(classes, 64 * 1024 * 1024)
        .map_err(engine_error)?;
    if parsed.id != sample.id || parsed.group_id != sample.group_id {
        return Err(invalid(
            "sample payload identity/group differs from durable ledger",
        ));
    }
    if parsed
        .outcome
        .as_ref()
        .is_some_and(|outcome| outcome.available_at_ms > sample.label_available_at_ms)
    {
        return Err(invalid(
            "outcome was unavailable at the recorded label availability",
        ));
    }
    Ok(parsed)
}

#[cfg(feature = "native")]
fn labels(recipe: &Value) -> Result<Vec<String>> {
    let labels: Vec<String> = recipe
        .get("labels")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    flow_like_ml_core::validate_labels(&labels).map_err(engine_error)?;
    Ok(labels)
}

/// Native algorithms currently use CPU. Refuse a GPU request unless the caller opted into
/// CPU fallback, so a backend pin cannot silently become a different execution contract.
#[cfg(feature = "native")]
fn validate_native_compute(value: &Value) -> Result<()> {
    let compute: ComputeConfig = serde_json::from_value(value.clone())?;
    compute.validate().map_err(engine_error)?;
    if !matches!(
        compute.backend,
        flow_like_ml_core::ComputeBackend::Cpu | flow_like_ml_core::ComputeBackend::Auto
    ) && !compute.allow_cpu_fallback
    {
        return Err(invalid(
            "native classical models require CPU or explicit CPU fallback",
        ));
    }
    Ok(())
}

#[cfg(feature = "native")]
pub fn register_native_engines(worker: &mut TrainingWorker) -> Result<()> {
    for name in [
        "isolation_forest",
        "histogram_gradient_boosting",
        "patchcore",
        "padim",
    ] {
        worker.register(name, Arc::new(NativeEngine { name }))?;
    }
    Ok(())
}

#[cfg(feature = "native")]
#[derive(Serialize, Deserialize)]
#[serde(tag = "engine", content = "model", rename_all = "snake_case")]
enum NativeModel {
    IsolationForest(flow_like_ml_native::models::IsolationForest),
    HistogramGradientBoosting(flow_like_ml_native::models::HistogramGradientBoosting),
    Patchcore(flow_like_ml_native::models::PatchCore),
    Padim(flow_like_ml_native::models::Padim),
}

#[cfg(feature = "native")]
struct NativeEngine {
    name: &'static str,
}
#[cfg(feature = "native")]
impl TrainingEngine for NativeEngine {
    fn resource_key(&self, _: &TrainingWork) -> Result<String> {
        Ok("cpu:0".into())
    }
    fn estimated_memory_bytes(&self, work: &TrainingWork) -> Result<u64> {
        let base = serialized_size(&work.snapshot.train)?;
        let model = match self.name {
            "isolation_forest" => work
                .job
                .request
                .recipe
                .pointer("/config/trees")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .saturating_mul(65536),
            "histogram_gradient_boosting" => {
                let config = &work.job.request.recipe["config"];
                let depth = config["max_depth"].as_u64().unwrap_or(3).min(16);
                config["estimators"]
                    .as_u64()
                    .unwrap_or(100)
                    .saturating_mul(1u64 << depth)
                    .saturating_mul(64)
            }
            "padim" => {
                let sample = payload_sample(
                    work.snapshot
                        .train
                        .first()
                        .ok_or_else(|| invalid("empty training set"))?,
                    2,
                )?;
                if sample.input.shape.len() != 2 {
                    return Err(invalid("PaDiM input must be [patches, embedding channels]"));
                }
                (sample.input.shape[0] as u64)
                    .saturating_mul(sample.input.shape[1] as u64)
                    .saturating_mul(sample.input.shape[1] as u64)
                    .saturating_mul(16)
            }
            _ => base.saturating_mul(2),
        };
        Ok(base.saturating_mul(8).saturating_add(model))
    }
    fn train(&self, work: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput> {
        use flow_like_ml_native::models::*;
        validate_native_compute(&work.job.request.compute)?;
        control.check_cancel()?;
        let labels = labels(&work.job.request.recipe)?;
        let samples: Vec<_> = work
            .snapshot
            .train
            .iter()
            .map(|sample| payload_sample(sample, labels.len().max(2)))
            .collect::<Result<_>>()?;
        let input_shape = samples
            .first()
            .ok_or_else(|| invalid("empty training data"))?
            .input
            .shape
            .clone();
        if samples.iter().any(|s| s.input.shape != input_shape) {
            return Err(invalid("training inputs have different shapes"));
        }
        let rows: Vec<Vec<f64>> = samples
            .iter()
            .map(|s| s.input.values.iter().map(|v| *v as f64).collect())
            .collect();
        let progress = |step| {
            control
                .progress(serde_json::json!({"step":step}))
                .map_err(|error| flow_like_ml_core::Error::Invalid(error.to_string()))
        };
        let trained: std::result::Result<NativeModel, flow_like_ml_core::Error> = match self.name {
            "isolation_forest" => {
                let config: IsolationForestConfig = serde_json::from_value(
                    work.job
                        .request
                        .recipe
                        .get("config")
                        .cloned()
                        .unwrap_or_else(
                            || serde_json::json!({"trees":100,"sample_size":256,"seed":42}),
                        ),
                )?;
                IsolationForest::fit_with_progress(&rows, &config, progress)
                    .map(NativeModel::IsolationForest)
            }
            "histogram_gradient_boosting" => {
                let config: BoostConfig =
                    serde_json::from_value(work.job.request.recipe["config"].clone())?;
                let targets = samples
                    .iter()
                    .map(|s| match s.annotation {
                        Annotation::Class { class_id } => Ok(class_id as f64),
                        Annotation::Scalar { value } => Ok(value),
                        _ => Err(invalid("boosting needs class or scalar annotations")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                HistogramGradientBoosting::fit_with_progress(&rows, &targets, &config, progress)
                    .map(NativeModel::HistogramGradientBoosting)
            }
            "patchcore" => {
                ensure_normal(&samples)?;
                let images = patch_images(&samples)?;
                let patches = images.into_iter().flatten().collect::<Vec<_>>();
                let count = work.job.request.recipe["coreset_size"]
                    .as_u64()
                    .unwrap_or(256) as usize;
                PatchCore::fit_with_progress(&patches, count, progress).map(NativeModel::Patchcore)
            }
            "padim" => {
                ensure_normal(&samples)?;
                let images = patch_images(&samples)?;
                let regularization = work.job.request.recipe["regularization"]
                    .as_f64()
                    .unwrap_or(0.01);
                Padim::fit_with_progress(&images, regularization, progress).map(NativeModel::Padim)
            }
            _ => return Err(invalid("unsupported native engine")),
        };
        // Preserve cancellation/lease failure rather than wrapping it as a fit error.
        control.check_cancel()?;
        let trained = trained.map_err(engine_error)?;
        let model_bytes = serde_json::to_vec(&trained)?;
        control.checkpoint(
            &model_bytes,
            1,
            serde_json::json!({"engine":self.name,"resume_mode":"completed_model"}),
        )?;
        Ok(EngineOutput {
            model_bytes,
            manifest: serde_json::json!({"engine":self.name,"format_version":1,"labels":labels,"input_shape":input_shape,"recipe":work.job.request.recipe,"backend":"cpu"}),
        })
    }
}

#[cfg(feature = "native")]
fn ensure_normal(samples: &[Sample]) -> Result<()> {
    if samples
        .iter()
        .any(|sample| !matches!(sample.annotation, Annotation::Anomaly { is_anomaly: false }))
    {
        return Err(invalid(
            "PatchCore/PaDiM fit requires explicitly normal embeddings",
        ));
    }
    Ok(())
}
#[cfg(feature = "native")]
fn patch_images(samples: &[Sample]) -> Result<Vec<Vec<Vec<f64>>>> {
    samples
        .iter()
        .map(|sample| {
            if sample.input.shape.len() != 2 {
                return Err(invalid(
                    "image embeddings must have shape [patches, embedding channels]",
                ));
            }
            Ok(sample
                .input
                .values
                .chunks(sample.input.shape[1])
                .map(|patch| patch.iter().map(|v| *v as f64).collect())
                .collect())
        })
        .collect()
}

/// Input includes the batch dimension for dense/temporal/image models. Patch anomaly
/// models accept one image as [patches, channels] or a batch as [batch, patches, channels].
pub fn load_artifact_predictions(
    repository: &TrainingRepository,
    artifact_id: &str,
    input: &TensorData,
    compute: &ComputeConfig,
) -> Result<Value> {
    LoadedArtifactPredictor::load(repository, artifact_id, compute)?.predict(input)
}

#[cfg(all(test, feature = "native"))]
mod native_compute_tests {
    use super::*;

    #[test]
    fn automatic_and_defaulted_native_compute_use_cpu_with_explicit_strictness() {
        let request: TrainingRequest =
            serde_json::from_value(serde_json::json!({"engine":"isolation_forest","recipe":{}}))
                .unwrap();
        assert_eq!(request.compute, serde_json::json!({}));
        validate_native_compute(&request.compute).unwrap();
        for value in [
            serde_json::json!({}),
            serde_json::json!({"backend":"auto","allow_cpu_fallback":false}),
            serde_json::json!({"backend":"cuda"}),
            serde_json::json!({"backend":"rocm","allow_cpu_fallback":true}),
        ] {
            validate_native_compute(&value).unwrap();
        }
        for backend in ["cuda", "rocm", "wgpu"] {
            assert!(
                validate_native_compute(
                    &serde_json::json!({"backend":backend,"allow_cpu_fallback":false})
                )
                .is_err()
            );
        }
        assert!(validate_native_compute(&serde_json::json!({"backend":"unknown"})).is_err());
        assert!(validate_native_compute(&serde_json::json!({"memory_limit_bytes":0})).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_size_counts_utf8_escapes_and_nested_values() {
        let value = serde_json::json!({
            "text": "é😀\n\"\\",
            "nested": [null, true, -123.456, {"empty": []}],
            "samples": [1, 2, 3]
        });
        assert_eq!(
            serialized_size(&value).unwrap(),
            serde_json::to_vec(&value).unwrap().len() as u64
        );
    }
}

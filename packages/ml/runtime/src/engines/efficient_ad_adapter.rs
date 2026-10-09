use super::burn_adapter::compute_config;
use super::*;
use flow_like_ml_burn as burn;
use std::fs;
const MANIFEST: &str = "efficient-ad-state.json";

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EfficientAdEngineRecipe {
    pub config: burn::EfficientAdConfig,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub preprocessing: Vec<flow_like_ml_core::PreprocessingStep>,
}
pub fn register_efficient_ad_engine(worker: &mut TrainingWorker) -> Result<()> {
    worker.register("efficient_ad", Arc::new(EfficientAdEngine))
}
struct EfficientAdEngine;
impl TrainingEngine for EfficientAdEngine {
    fn resource_key(&self, work: &TrainingWork) -> Result<String> {
        burn_adapter::resource_key(&compute_config(&work.job.request.compute)?)
    }
    fn estimated_memory_bytes(&self, work: &TrainingWork) -> Result<u64> {
        let recipe: EfficientAdEngineRecipe =
            serde_json::from_value(work.job.request.recipe.clone())?;
        let model = recipe
            .config
            .estimated_training_bytes()
            .map_err(engine_error)?;
        let dataset = serialized_size(&work.snapshot)?;
        Ok(model.saturating_add(dataset.saturating_mul(12)))
    }
    fn train(&self, work: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput> {
        let mut recipe: EfficientAdEngineRecipe =
            serde_json::from_value(work.job.request.recipe.clone())?;
        let compute = compute_config(&work.job.request.compute)?;
        recipe.config.backend = burn_adapter::backend(&compute)?;
        let training_images = images(&work.snapshot.train, recipe.labels.len())?;
        let validation = images(&work.snapshot.validation, recipe.labels.len())?;
        let data = burn::EfficientAdDataset {
            images: training_images,
            autoencoder_images: None,
            penalty_images: None,
        };
        let directory = tempfile::tempdir()?;
        if let Some(bytes) = &work.resume_checkpoint {
            burn_adapter::unpack_named(
                bytes,
                directory.path(),
                MANIFEST,
                control.limits().maximum_checkpoint_bytes,
            )?;
        }
        let cancellation = burn::CancellationToken::new();
        let cancellation_monitor = BurnCancellationMonitor::start(control, &cancellation)?;
        let mut error = None;
        let mut callback = |progress: burn::TrainingProgress| {
            let update = (|| -> Result<()> {
                control.progress(serde_json::to_value(&progress)?)?;
                if progress.validation_loss.is_some() {
                    let bytes = burn_adapter::pack_named(
                        directory.path(),
                        MANIFEST,
                        true,
                        control.limits().maximum_checkpoint_bytes,
                    )?;
                    control.checkpoint(
                        &bytes,
                        progress.steps as u64,
                        serde_json::json!({"engine":"efficient_ad","steps":progress.steps}),
                    )?;
                }
                Ok(())
            })();
            if let Err(e) = update {
                error = Some(e);
                cancellation.cancel();
            }
        };
        let trained = if work.resume_checkpoint.is_some() {
            burn::resume_efficient_ad(
                &recipe.config,
                &data,
                &validation,
                directory.path(),
                &cancellation,
                &mut callback,
            )
        } else {
            burn::train_efficient_ad(
                &recipe.config,
                &data,
                &validation,
                directory.path(),
                &cancellation,
                &mut callback,
            )
        };
        let cancellation_reason = cancellation_monitor.finish();
        if trained.is_err() && directory.path().join(MANIFEST).exists() {
            let state: Value = serde_json::from_slice(&fs::read(directory.path().join(MANIFEST))?)?;
            let step = state
                .pointer("/report/steps")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let bytes = burn_adapter::pack_named(
                directory.path(),
                MANIFEST,
                true,
                control.limits().maximum_checkpoint_bytes,
            )?;
            control.checkpoint_after_stop(
                &bytes,
                step,
                serde_json::json!({"engine":"efficient_ad","steps":step}),
            )?;
        }
        if let Some(error) = error.or(cancellation_reason) {
            return Err(error);
        }
        let report = trained.map_err(|e| {
            if matches!(e, burn::Error::Cancelled) {
                Error::Cancelled
            } else {
                engine_error(e)
            }
        })?;
        control.check_cancel()?;
        let checkpoint = burn_adapter::pack_named(
            directory.path(),
            MANIFEST,
            true,
            control.limits().maximum_checkpoint_bytes,
        )?;
        control.checkpoint(
            &checkpoint,
            report.steps as u64,
            serde_json::json!({"engine":"efficient_ad","steps":report.steps}),
        )?;
        let model_bytes = burn_adapter::pack_named(
            directory.path(),
            MANIFEST,
            false,
            control.limits().maximum_artifact_bytes,
        )?;
        let state: Value = serde_json::from_slice(&fs::read(directory.path().join(MANIFEST))?)?;
        Ok(EngineOutput {
            model_bytes,
            manifest: serde_json::json!({"engine":"efficient_ad","format_version":1,"architecture":state["architecture"],"calibration":state["calibration"],"backend":recipe.config.backend,"labels":recipe.labels,"preprocessing":recipe.preprocessing,"input_shape":data.images.shape[1..],"report":report}),
        })
    }
}
fn images(samples: &[TrainingSample], classes: usize) -> Result<burn::TensorData> {
    let mut shape = Vec::new();
    let mut values = Vec::new();
    if samples.is_empty() {
        return Err(invalid(
            "EfficientAD needs nonempty independent training and validation normal images",
        ));
    }
    for sample in samples {
        let sample = payload_sample(sample, classes)?;
        if !matches!(sample.annotation, Annotation::Anomaly { is_anomaly: false }) {
            return Err(invalid(
                "EfficientAD trains and calibrates on explicitly normal images",
            ));
        }
        if shape.is_empty() {
            shape = sample.input.shape.clone();
        } else if shape != sample.input.shape {
            return Err(invalid("EfficientAD image dimensions differ"));
        }
        if values.len().saturating_add(sample.input.values.len()) > 67_108_864 {
            return Err(invalid("EfficientAD dataset exceeds tensor budget"));
        }
        values.extend(sample.input.values);
    }
    shape.insert(0, samples.len());
    let tensor = burn::TensorData { shape, values };
    tensor.validate().map_err(engine_error)?;
    Ok(tensor)
}
#[cfg(test)]
pub(super) fn predict(bytes: &[u8], input: &TensorData, compute: &ComputeConfig) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    burn_adapter::unpack_named(
        bytes,
        directory.path(),
        MANIFEST,
        compute.memory_limit_bytes,
    )?;
    let model = burn::EfficientAdPredictor::load(directory.path(), burn_adapter::backend(compute)?)
        .map_err(engine_error)?;
    serde_json::to_value(
        model
            .predict(&burn::TensorData {
                shape: input.shape.clone(),
                values: input.values.clone(),
            })
            .map_err(engine_error)?,
    )
    .map_err(Error::from)
}

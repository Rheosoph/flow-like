use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
use flow_like_ml_core::{ComputeConfig, InspectionSpec, Sample, TensorData};
#[cfg(feature = "training")]
use flow_like_ml_runtime as runtime;
use flow_like_ml_runtime::*;
#[cfg(feature = "training")]
use flow_like_types::json::json;
use flow_like_types::{Result, Value, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A changed specification creates a new training stream, including changed label order.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectionContext {
    pub stream_id: String,
    pub spec: InspectionSpec,
}
#[cfg(any(feature = "training", test))]
impl InspectionContext {
    pub(crate) fn key(&self, project_id: String) -> Result<StreamKey> {
        self.spec.validate()?;
        if self.stream_id.trim().is_empty() {
            return Err(anyhow!("Inspection stream ID is required"));
        }
        Ok(StreamKey {
            project_id,
            stream_id: self.stream_id.clone(),
            inspection_version: flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(
                &self.spec,
            )?),
        })
    }
}

#[cfg(feature = "execute")]
pub(crate) fn local_storage_directory(
    context: &ExecutionContext,
    suffix: &str,
) -> Result<std::path::PathBuf> {
    use flow_like_storage::{Path, files::store::FlowLikeStore};
    let cache = context
        .execution_cache
        .as_ref()
        .ok_or_else(|| anyhow!("Execution storage context is unavailable"))?;
    if cache.shadow {
        return Err(anyhow!(
            "Training lifecycle operations require a live workflow. Run model comparisons through the explicit prediction recording node in a live workflow"
        ));
    }
    let Some(FlowLikeStore::Local(local)) = &cache.stores.app_storage_store else {
        return Err(anyhow!(
            "Persistent training requires local durable app storage on this executor"
        ));
    };
    let root = local
        .directory_to_filesystem(&Path::from(""))?
        .canonicalize()?;
    let path = local
        .directory_to_filesystem(&cache.get_storage(false)?.join("inspection").join(suffix))?;
    let mut ancestor = path.as_path();
    while !ancestor.try_exists()? {
        if std::fs::symlink_metadata(ancestor).is_ok() {
            return Err(anyhow!("Training path contains a dangling symlink"));
        }
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow!("Training path has no parent"))?;
    }
    if !ancestor.canonicalize()?.starts_with(root) {
        return Err(anyhow!("Training path escapes app storage"));
    }
    Ok(path)
}
#[cfg(feature = "training")]
pub(crate) fn repository(context: &ExecutionContext) -> Result<(TrainingRepository, String)> {
    let path = local_storage_directory(context, "jobs.sqlite")?;
    let cache = context
        .execution_cache
        .as_ref()
        .ok_or_else(|| anyhow!("Execution storage context is unavailable"))?;
    Ok((
        TrainingRepository::open(path)?,
        format!("{}:{}", cache.app_id, cache.board_id),
    ))
}

#[cfg(any(feature = "training", test))]
fn check_stream(expected: &StreamKey, actual: &StreamKey) -> Result<()> {
    if expected != actual {
        return Err(anyhow!(
            "The requested resource belongs to a different inspection stream or specification"
        ));
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordSampleRequest {
    pub inspection: InspectionContext,
    pub sample: Sample,
    pub annotation_revision: u64,
    pub accepted: bool,
    pub label_available_at_ms: i64,
}
#[cfg(feature = "training")]
fn record_sample(
    repo: TrainingRepository,
    project: String,
    input: RecordSampleRequest,
) -> Result<SampleReceipt> {
    input.inspection.spec.validate_sample(&input.sample)?;
    if input.accepted
        && matches!(
            input.sample.annotation,
            flow_like_ml_core::Annotation::Unlabeled
        )
    {
        return Err(anyhow!(
            "An unlabelled sample cannot count as an accepted training label"
        ));
    }
    let key = input.inspection.key(project)?;
    if input.sample.stream_id != key.stream_id {
        return Err(anyhow!("Sample and inspection stream IDs differ"));
    }
    let (source, available) = match &input.sample.provenance {
        flow_like_ml_core::LabelProvenance::Teacher { .. } => {
            (LabelSource::Teacher, input.sample.timestamp_ms)
        }
        flow_like_ml_core::LabelProvenance::Reviewed { reviewed_at_ms, .. } => {
            (LabelSource::Reviewed, *reviewed_at_ms)
        }
        flow_like_ml_core::LabelProvenance::Measured { .. } => (
            LabelSource::ObservedOutcome,
            input
                .sample
                .outcome
                .as_ref()
                .map_or(input.sample.timestamp_ms, |o| o.available_at_ms),
        ),
        _ => {
            return Err(anyhow!(
                "Only labelled samples with provenance can enter training storage"
            ));
        }
    };
    if input.label_available_at_ms < available || input.label_available_at_ms > runtime::now_ms() {
        return Err(anyhow!(
            "Label availability must follow capture/review/outcome and cannot be in the future"
        ));
    }
    let digest = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(
        &input.sample.stream_id,
        input.sample.window_start_ms,
        input.sample.window_end_ms,
        &input.sample.input,
    ))?);
    let mut payload = json!(input.sample);
    if let flow_like_ml_core::Annotation::Class { class_id } = input.sample.annotation {
        payload["label"] = json!(input.inspection.spec.labels[class_id as usize]);
    }
    Ok(repo.record_sample(
        &key,
        &TrainingSample {
            id: input.sample.id,
            annotation_revision: input.annotation_revision,
            group_id: input.sample.group_id,
            captured_at_ms: input.sample.timestamp_ms,
            label_available_at_ms: input.label_available_at_ms,
            content_digest: digest,
            source,
            accepted: input.accepted,
            payload,
        },
    )?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScheduleTrainingRequest {
    pub inspection: InspectionContext,
    pub after_new_samples: usize,
    pub training: TrainingRequest,
    pub split: SplitPolicy,
    pub worker_limits: WorkerLimits,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrainBurnRequest {
    pub inspection: InspectionContext,
    pub after_new_samples: usize,
    pub config: flow_like_ml_burn::TrainingConfig,
    #[serde(default)]
    pub compute: ComputeConfig,
    pub split: SplitPolicy,
    pub worker_limits: WorkerLimits,
    #[serde(default)]
    pub preprocessing: Vec<flow_like_ml_core::PreprocessingStep>,
}
#[cfg(any(feature = "training", test))]
fn validate_native_schedule_compute(engine: &str, compute: &ComputeConfig) -> Result<()> {
    if matches!(
        engine,
        "isolation_forest" | "histogram_gradient_boosting" | "patchcore" | "padim"
    ) && !matches!(
        compute.backend,
        flow_like_ml_core::ComputeBackend::Cpu | flow_like_ml_core::ComputeBackend::Auto
    ) && !compute.allow_cpu_fallback
    {
        return Err(anyhow!(
            "Native classical models require CPU or explicit CPU fallback"
        ));
    }
    Ok(())
}
#[cfg(feature = "training")]
fn schedule(
    repo: TrainingRepository,
    project: String,
    mut input: ScheduleTrainingRequest,
) -> Result<Option<TrainingJob>> {
    let key = input.inspection.key(project)?;
    let compute: ComputeConfig = flow_like_types::json::from_value(input.training.compute.clone())?;
    compute.validate()?;
    validate_native_schedule_compute(&input.training.engine, &compute)?;
    runtime::worker::TrainingWorker::new(repo.clone(), input.worker_limits.clone())?;
    match input.training.engine.as_str() {
        "burn" => {
            let recipe: runtime::engines::BurnEngineRecipe =
                flow_like_types::json::from_value(input.training.recipe.clone())?;
            flow_like_ml_burn::validate_inspection_config(&input.inspection.spec, &recipe.config)?;
        }
        "efficient_ad" => {
            let recipe: runtime::engines::EfficientAdEngineRecipe =
                flow_like_types::json::from_value(input.training.recipe.clone())?;
            recipe.config.validate()?;
            if input.inspection.spec.task != flow_like_ml_core::TaskKind::VisualAnomaly
                || input.inspection.spec.input_shape
                    != [recipe.config.teacher.input_channels, 256, 256]
            {
                return Err(anyhow!(
                    "EfficientAD requires a visual anomaly inspection with normalized [teacher channels,256,256] images"
                ));
            }
        }
        "isolation_forest" => {
            if !matches!(
                input.inspection.spec.task,
                flow_like_ml_core::TaskKind::SensorAnomaly
                    | flow_like_ml_core::TaskKind::VisualAnomaly
            ) {
                return Err(anyhow!("Isolation Forest requires an anomaly inspection"));
            }
        }
        "patchcore" | "padim" => {
            if input.inspection.spec.task != flow_like_ml_core::TaskKind::VisualAnomaly
                || input.inspection.spec.input_shape.len() != 2
            {
                return Err(anyhow!(
                    "PatchCore and PaDiM require visual anomaly embeddings shaped [patches,channels]"
                ));
            }
        }
        "histogram_gradient_boosting" => {
            let config: flow_like_ml_native::models::BoostConfig =
                flow_like_types::json::from_value(input.training.recipe["config"].clone())?;
            let compatible = match config.objective {
                flow_like_ml_native::models::BoostObjective::BinaryLogLoss => {
                    input.inspection.spec.task == flow_like_ml_core::TaskKind::SensorClassification
                        && input.inspection.spec.labels.len() == 2
                }
                flow_like_ml_native::models::BoostObjective::SquaredError => {
                    input.inspection.spec.task == flow_like_ml_core::TaskKind::SensorRegression
                }
            };
            if !compatible {
                return Err(anyhow!(
                    "Histogram boosting objective must match binary sensor classification or scalar sensor regression"
                ));
            }
        }
        _ => return Err(anyhow!("Unsupported inspection training engine")),
    }
    let labels = input
        .training
        .recipe
        .get("labels")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Training recipe must include labels in inspection order"))?;
    if *labels
        != input
            .inspection
            .spec
            .labels
            .iter()
            .map(|s| json!(s))
            .collect::<Vec<_>>()
    {
        return Err(anyhow!(
            "Training labels differ from the inspection specification"
        ));
    }
    input.training.recipe["minimum_examples"] = json!(input.inspection.spec.minimum_examples);
    input.training.recipe["inspection_task"] = json!(input.inspection.spec.task);
    input.training.recipe["minimum_examples_per_class"] =
        json!(input.inspection.spec.minimum_examples_per_class);
    for pending in repo.list_jobs(&key)? {
        if pending.status == JobStatus::Queued
            || (pending.status == JobStatus::Running
                && pending
                    .lease_expires_at_ms
                    .is_some_and(|t| t < runtime::now_ms()))
        {
            enqueue(
                repo.clone(),
                pending.id.clone(),
                input.worker_limits.clone(),
            )?;
            return Ok(Some(pending));
        }
    }
    let job = repo.trigger_after_n(
        &key,
        input.after_new_samples,
        input.training,
        input.split,
        runtime::now_ms(),
    )?;
    if let Some(job) = &job {
        enqueue(repo, job.id.clone(), input.worker_limits)?;
    }
    Ok(job)
}
#[cfg(feature = "training")]
fn train_burn(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    flow_like_ml_burn::validate_inspection_config(&input.inspection.spec, &input.config)?;
    let labels = input.inspection.spec.labels.clone();
    let request = TrainingRequest {
        engine: "burn".into(),
        recipe: json!({"config":input.config,"labels":labels,"preprocessing":input.preprocessing}),
        compute: json!(input.compute),
    };
    schedule(
        repo,
        project,
        ScheduleTrainingRequest {
            inspection: input.inspection,
            after_new_samples: input.after_new_samples,
            training: request,
            split: input.split,
            worker_limits: input.worker_limits,
        },
    )
}

#[cfg(feature = "training")]
fn enqueue(repo: TrainingRepository, job: String, limits: WorkerLimits) -> Result<()> {
    use std::sync::{
        LazyLock, Mutex,
        mpsc::{SyncSender, sync_channel},
    };
    type Work = (TrainingRepository, String, WorkerLimits, String);
    static QUEUED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
    // One worker limits aggregate CPU/GPU memory use; SQLite remains the durable queue.
    static WORKER: LazyLock<std::result::Result<SyncSender<Work>, String>> =
        LazyLock::new(|| {
            let (sender, receiver) = sync_channel::<Work>(32);
            std::thread::Builder::new()
            .name("inspection-training".into())
            .spawn(move || {
                while let Ok((repo, id, limits,key)) = receiver.recv() {
                    let result = (|| -> runtime::Result<()> {
                        let mut worker = runtime::worker::TrainingWorker::new(repo, limits)?;
                        runtime::engines::register_native_engines(&mut worker)?;
                    runtime::engines::register_burn_engine(&mut worker)?;
                    runtime::engines::register_efficient_ad_engine(&mut worker)?;
                        worker.run(&id)?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        tracing::warn!(job_id = %id, %error, "Inspection training worker stopped");
                    }
                    if let Ok(mut queued)=QUEUED.lock(){queued.remove(&key);}
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(sender)
        });
    let sender = WORKER
        .as_ref()
        .map_err(|e| anyhow!("Cannot start training worker: {e}"))?;
    let key =
        flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(repo.path(), &job))?);
    let mut queued = QUEUED
        .lock()
        .map_err(|_| anyhow!("Training queue lock is poisoned"))?;
    if !queued.insert(key.clone()) {
        return Ok(());
    }
    if let Err(error) = sender.try_send((repo, job, limits, key.clone())) {
        queued.remove(&key);
        return Err(anyhow!(
            "Training job is saved, but the worker queue is unavailable. Recover Training Jobs can retry it: {error}"
        ));
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRequest {
    pub inspection: InspectionContext,
    pub job_id: String,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrainEfficientAdRequest {
    pub inspection: InspectionContext,
    pub after_new_samples: usize,
    pub config: flow_like_ml_burn::EfficientAdConfig,
    #[serde(default)]
    pub compute: ComputeConfig,
    pub split: SplitPolicy,
    pub worker_limits: WorkerLimits,
    #[serde(default)]
    pub preprocessing: Vec<flow_like_ml_core::PreprocessingStep>,
}
#[cfg(feature = "training")]
fn train_efficient_ad(
    repo: TrainingRepository,
    project: String,
    input: TrainEfficientAdRequest,
) -> Result<Option<TrainingJob>> {
    input.config.validate()?;
    if input.inspection.spec.task != flow_like_ml_core::TaskKind::VisualAnomaly {
        return Err(anyhow!("EfficientAD requires a visual anomaly inspection"));
    }
    let request = TrainingRequest {
        engine: "efficient_ad".into(),
        recipe: json!({"config":input.config,"labels":input.inspection.spec.labels,"preprocessing":input.preprocessing}),
        compute: json!(input.compute),
    };
    schedule(
        repo,
        project,
        ScheduleTrainingRequest {
            inspection: input.inspection,
            after_new_samples: input.after_new_samples,
            training: request,
            split: input.split,
            worker_limits: input.worker_limits,
        },
    )
}
#[cfg(feature = "training")]
fn job(repo: TrainingRepository, project: String, input: JobRequest) -> Result<TrainingJob> {
    let job = repo.get_job(&input.job_id)?;
    check_stream(&input.inspection.key(project)?, &job.stream)?;
    Ok(job)
}
#[cfg(feature = "training")]
fn cancel(repo: TrainingRepository, project: String, input: JobRequest) -> Result<TrainingJob> {
    job(repo.clone(), project, input.clone())?;
    Ok(repo.request_cancel(&input.job_id, runtime::now_ms())?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResumeRequest {
    pub inspection: InspectionContext,
    pub job_id: String,
    pub worker_limits: WorkerLimits,
}
#[cfg(feature = "training")]
fn resume(repo: TrainingRepository, project: String, input: ResumeRequest) -> Result<TrainingJob> {
    job(
        repo.clone(),
        project,
        JobRequest {
            inspection: input.inspection,
            job_id: input.job_id.clone(),
        },
    )?;
    let job = repo.resume_job(&input.job_id, runtime::now_ms())?;
    enqueue(repo, input.job_id, input.worker_limits)?;
    Ok(job)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecoverRequest {
    pub inspection: InspectionContext,
    pub worker_limits: WorkerLimits,
}
#[cfg(feature = "training")]
fn recover(
    repo: TrainingRepository,
    project: String,
    input: RecoverRequest,
) -> Result<Vec<TrainingJob>> {
    let key = input.inspection.key(project)?;
    for job in repo.list_jobs(&key)? {
        if job.status == JobStatus::Queued
            || (job.status == JobStatus::Running
                && job
                    .lease_expires_at_ms
                    .is_some_and(|t| t < runtime::now_ms()))
        {
            enqueue(repo.clone(), job.id, input.worker_limits.clone())?;
        }
    }
    Ok(repo.list_jobs(&key)?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactRequest {
    pub inspection: InspectionContext,
    pub artifact_id: String,
}
#[cfg(feature = "training")]
fn artifact(
    repo: TrainingRepository,
    project: String,
    input: ArtifactRequest,
) -> Result<ModelArtifact> {
    let artifact = repo.get_artifact(&input.artifact_id)?;
    check_stream(&input.inspection.key(project)?, &artifact.stream)?;
    Ok(artifact)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PredictRequest {
    pub inspection: InspectionContext,
    pub artifact_id: String,
    pub input: TensorData,
    #[serde(default)]
    pub compute: ComputeConfig,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordPredictionRequest {
    pub inspection: InspectionContext,
    pub prediction: PredictionRecord,
}
#[cfg(feature = "training")]
fn record_prediction(
    repo: TrainingRepository,
    project: String,
    input: RecordPredictionRequest,
) -> Result<bool> {
    artifact(
        repo.clone(),
        project,
        ArtifactRequest {
            inspection: input.inspection,
            artifact_id: input.prediction.artifact_id.clone(),
        },
    )?;
    Ok(repo.record_prediction(&input.prediction)?)
}
#[cfg(feature = "training")]
fn evaluate(
    repo: TrainingRepository,
    project: String,
    input: ArtifactRequest,
) -> Result<EvaluationReport> {
    artifact(repo.clone(), project, input.clone())?;
    Ok(repo.evaluate_classification(&input.artifact_id, runtime::now_ms())?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromoteRequest {
    pub inspection: InspectionContext,
    pub deployment_id: String,
    pub expected_generation: i64,
    pub evaluation_id: String,
    pub policy: PromotionPolicy,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateTaskRequest {
    pub inspection: InspectionContext,
    pub artifact_id: String,
    pub task: EvaluationTask,
}
#[cfg(feature = "training")]
fn evaluate_task(
    repo: TrainingRepository,
    project: String,
    input: EvaluateTaskRequest,
) -> Result<EvaluationReport> {
    artifact(
        repo.clone(),
        project,
        ArtifactRequest {
            inspection: input.inspection,
            artifact_id: input.artifact_id.clone(),
        },
    )?;
    Ok(repo.evaluate_task(&input.artifact_id, input.task, runtime::now_ms())?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromoteMetricsRequest {
    pub inspection: InspectionContext,
    pub deployment_id: String,
    pub expected_generation: i64,
    pub evaluation_id: String,
    pub policy: MetricPromotionPolicy,
}
#[cfg(feature = "training")]
fn promote_metrics(
    repo: TrainingRepository,
    project: String,
    input: PromoteMetricsRequest,
) -> Result<Deployment> {
    let report = repo.get_evaluation(&input.evaluation_id)?;
    artifact(
        repo.clone(),
        project,
        ArtifactRequest {
            inspection: input.inspection,
            artifact_id: report.artifact_id,
        },
    )?;
    Ok(repo.promote_metrics(
        &input.deployment_id,
        input.expected_generation,
        &input.evaluation_id,
        input.policy,
        runtime::now_ms(),
    )?)
}
#[cfg(feature = "training")]
fn promote(repo: TrainingRepository, project: String, input: PromoteRequest) -> Result<Deployment> {
    let report = repo.get_evaluation(&input.evaluation_id)?;
    artifact(
        repo.clone(),
        project,
        ArtifactRequest {
            inspection: input.inspection,
            artifact_id: report.artifact_id,
        },
    )?;
    Ok(repo.promote(
        &input.deployment_id,
        input.expected_generation,
        &input.evaluation_id,
        input.policy,
        runtime::now_ms(),
    )?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentRequest {
    pub inspection: InspectionContext,
    pub deployment_id: String,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InspectionMode {
    TeacherOnly,
    Shadow,
    StudentOnly,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectionRoute {
    pub mode: InspectionMode,
    pub active_artifact_id: Option<String>,
    pub candidate_artifact_id: Option<String>,
    pub deployment_generation: Option<i64>,
}
#[cfg(feature = "training")]
fn routing(
    repo: TrainingRepository,
    project: String,
    input: DeploymentRequest,
) -> Result<InspectionRoute> {
    let key = input.inspection.key(project.clone())?;
    let deployment = deployment(repo.clone(), project, input)?;
    if let Some(active) = &deployment {
        if !active.paused {
            return Ok(InspectionRoute {
                mode: InspectionMode::StudentOnly,
                active_artifact_id: Some(active.active_artifact_id.clone()),
                candidate_artifact_id: None,
                deployment_generation: Some(active.generation),
            });
        }
    }
    let candidate = repo
        .list_jobs(&key)?
        .into_iter()
        .filter(|j| j.status == JobStatus::Succeeded)
        .max_by_key(|j| j.updated_at_ms)
        .and_then(|j| j.artifact_id);
    Ok(InspectionRoute {
        mode: if candidate.is_some() {
            InspectionMode::Shadow
        } else {
            InspectionMode::TeacherOnly
        },
        active_artifact_id: None,
        candidate_artifact_id: candidate,
        deployment_generation: deployment.map(|d| d.generation),
    })
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PauseDeploymentRequest {
    pub inspection: InspectionContext,
    pub deployment_id: String,
    pub expected_generation: i64,
    pub paused: bool,
}
#[cfg(feature = "training")]
fn pause_deployment(
    repo: TrainingRepository,
    project: String,
    input: PauseDeploymentRequest,
) -> Result<Deployment> {
    deployment(
        repo.clone(),
        project,
        DeploymentRequest {
            inspection: input.inspection,
            deployment_id: input.deployment_id.clone(),
        },
    )?;
    Ok(repo.set_deployment_paused(
        &input.deployment_id,
        input.expected_generation,
        input.paused,
        runtime::now_ms(),
    )?)
}
#[cfg(feature = "training")]
fn deployment(
    repo: TrainingRepository,
    project: String,
    input: DeploymentRequest,
) -> Result<Option<Deployment>> {
    match repo.get_deployment(&input.deployment_id) {
        Ok(deployment) => {
            check_stream(&input.inspection.key(project)?, &deployment.stream)?;
            Ok(Some(deployment))
        }
        Err(runtime::Error::NotFound(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackRequest {
    pub inspection: InspectionContext,
    pub deployment_id: String,
    pub expected_generation: i64,
}
#[cfg(feature = "training")]
fn rollback(
    repo: TrainingRepository,
    project: String,
    input: RollbackRequest,
) -> Result<Deployment> {
    deployment(
        repo.clone(),
        project,
        DeploymentRequest {
            inspection: input.inspection,
            deployment_id: input.deployment_id.clone(),
        },
    )?;
    Ok(repo.rollback(
        &input.deployment_id,
        input.expected_generation,
        runtime::now_ms(),
    )?)
}

macro_rules! lifecycle_node {
    ($node:ident, $id:literal, $name:literal, $description:literal, $function:literal, $input:ty, $output:ty, $handler:path) => {
        #[flow_like_types::async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> flow_like::flow::node::Node {
                super::operation_node::<$input, $output>(
                    $id,
                    $name,
                    $description,
                    $function,
                    "AI/ML/Lifecycle",
                )
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                #[cfg(feature = "training")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let input: $input = context.evaluate_pin("request").await?;
                    let (repo, project) = repository(context)?;
                    let result: $output =
                        tokio::task::spawn_blocking(move || $handler(repo, project, input))
                            .await??;
                    context.set_pin_value("result", json!(result)).await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "training"))]
                {
                    let _ = context;
                    Err(anyhow!("This executor needs the ML training feature"))
                }
            }
        }
    };
}
#[crate::register_node]
#[derive(Default)]
pub struct RecordTrainingSampleNode;
lifecycle_node!(
    RecordTrainingSampleNode,
    "ml_record_training_sample",
    "Record Training Sample",
    "Persist a versioned annotation with capture time and label provenance",
    "recordSample",
    RecordSampleRequest,
    SampleReceipt,
    record_sample
);

#[crate::register_node]
#[derive(Default)]
pub struct ScheduleTrainingNode;
lifecycle_node!(
    ScheduleTrainingNode,
    "ml_schedule_training",
    "Train After N Samples",
    "Atomically snapshot new labels and start a durable background training job",
    "scheduleTraining",
    ScheduleTrainingRequest,
    Option<TrainingJob>,
    schedule
);

#[crate::register_node]
#[derive(Default)]
pub struct TrainBurnModelNode;
lifecycle_node!(
    TrainBurnModelNode,
    "ml_train_burn_model",
    "Train Burn Model",
    "Train a selected neural architecture on CPU or an optional GPU backend",
    "trainBurn",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_burn
);

#[crate::register_node]
#[derive(Default)]
pub struct TrainingJobStatusNode;
lifecycle_node!(
    TrainingJobStatusNode,
    "ml_training_job_status",
    "Training Job Status",
    "Read progress, checkpoint, failure and artifact state",
    "trainingStatus",
    JobRequest,
    TrainingJob,
    job
);

#[crate::register_node]
#[derive(Default)]
pub struct CancelTrainingNode;
lifecycle_node!(
    CancelTrainingNode,
    "ml_cancel_training",
    "Cancel Training",
    "Request cooperative cancellation and retain the last completed checkpoint",
    "cancelTraining",
    JobRequest,
    TrainingJob,
    cancel
);

#[crate::register_node]
#[derive(Default)]
pub struct ResumeTrainingNode;
lifecycle_node!(
    ResumeTrainingNode,
    "ml_resume_training",
    "Resume Training",
    "Resume an interrupted or cancelled job from its saved checkpoint",
    "resumeTraining",
    ResumeRequest,
    TrainingJob,
    resume
);

#[crate::register_node]
#[derive(Default)]
pub struct RecoverTrainingJobsNode;
lifecycle_node!(
    RecoverTrainingJobsNode,
    "ml_recover_training_jobs",
    "Recover Training Jobs",
    "Restart queued jobs or reclaim expired worker leases after process restart",
    "recoverTraining",
    RecoverRequest,
    Vec<TrainingJob>,
    recover
);

#[crate::register_node]
#[derive(Default)]
pub struct GetModelArtifactNode;
lifecycle_node!(
    GetModelArtifactNode,
    "ml_get_model_artifact",
    "Get Model Artifact",
    "Resolve a durable artifact and verify its inspection scope",
    "artifact",
    ArtifactRequest,
    ModelArtifact,
    artifact
);

#[crate::register_node]
#[derive(Default)]
pub struct StudentInferenceNode;

#[cfg(feature = "training")]
type CachedPredictor = std::sync::Arc<CatalogLoadedPredictor>;
#[cfg(feature = "training")]
pub(crate) enum CatalogLoadedPredictor {
    Builtin(runtime::engines::LoadedArtifactPredictor),
    Classical {
        artifact: ModelArtifact,
        predictor: std::sync::Mutex<super::auto_training_legacy::LegacyPredictor>,
    },
}
#[cfg(feature = "training")]
impl CatalogLoadedPredictor {
    pub(crate) fn load(
        repo: &TrainingRepository,
        artifact_id: &str,
        compute: &ComputeConfig,
    ) -> Result<Self> {
        let artifact = repo.get_artifact(artifact_id)?;
        if artifact.manifest["engine"] == "legacy_classical" {
            let predictor =
                super::auto_training_legacy::LegacyPredictor::load(repo, artifact_id, compute)?;
            Ok(Self::Classical {
                artifact,
                predictor: std::sync::Mutex::new(predictor),
            })
        } else {
            Ok(Self::Builtin(
                runtime::engines::LoadedArtifactPredictor::load(repo, artifact_id, compute)?,
            ))
        }
    }
    fn artifact(&self) -> &ModelArtifact {
        match self {
            Self::Builtin(predictor) => predictor.artifact(),
            Self::Classical { artifact, .. } => artifact,
        }
    }
    pub(crate) fn predict(&self, input: &TensorData) -> Result<Value> {
        match self {
            Self::Builtin(predictor) => Ok(predictor.predict(input)?),
            Self::Classical { predictor, .. } => {
                use runtime::auto_training::SearchPredictor;
                let mut predictor = predictor
                    .lock()
                    .map_err(|_| anyhow!("Classical predictor lock is poisoned"))?;
                Ok(predictor.predict(input)?)
            }
        }
    }
    pub(crate) fn features(&self, input: &TensorData) -> Result<TensorData> {
        match self {
            Self::Builtin(predictor) => Ok(predictor.features(input)?),
            Self::Classical { .. } => {
                Err(anyhow!("Classical models do not expose image feature maps"))
            }
        }
    }
}
#[cfg(feature = "training")]
#[derive(Clone, Default)]
struct PredictionCache {
    models: std::sync::Arc<tokio::sync::Mutex<Vec<(String, CachedPredictor, u64)>>>,
}
#[cfg(feature = "training")]
fn trim_prediction_cache(
    models: &mut Vec<(String, CachedPredictor, u64)>,
    incoming: u64,
    budget: u64,
) {
    while !models.is_empty()
        && (models.len() >= 4
            || models
                .iter()
                .fold(incoming, |total, entry| total.saturating_add(entry.2))
                > budget)
    {
        models.remove(0);
    }
}
#[cfg(feature = "training")]
impl flow_like_types::Cacheable for PredictionCache {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
#[cfg(feature = "training")]
async fn cached_predictor(
    context: &mut ExecutionContext,
    input: &PredictRequest,
) -> Result<CachedPredictor> {
    let (repo, project) = repository(context)?;
    let model = artifact(
        repo.clone(),
        project,
        ArtifactRequest {
            inspection: input.inspection.clone(),
            artifact_id: input.artifact_id.clone(),
        },
    )?;
    input.compute.validate()?;
    let key = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(
        repo.path(),
        &model.id,
        &model.blob.sha256,
        &input.compute,
    ))?);
    let cache = {
        let mut run_cache = context.cache.write().await;
        let value = run_cache
            .entry("inspection-predictors-v1".into())
            .or_insert_with(|| std::sync::Arc::new(PredictionCache::default()));
        value
            .as_any()
            .downcast_ref::<PredictionCache>()
            .ok_or_else(|| anyhow!("Inspection predictor cache has an unexpected type"))?
            .clone()
    };
    let mut models = cache.models.lock().await;
    if let Some(index) = models.iter().position(|(id, _, _)| id == &key) {
        let entry = models.remove(index);
        let predictor = entry.1.clone();
        trim_prediction_cache(&mut models, entry.2, input.compute.memory_limit_bytes);
        models.push(entry);
        return Ok(predictor);
    }
    let reserved = model
        .blob
        .bytes
        .checked_mul(4)
        .ok_or_else(|| anyhow!("Model cache size overflows"))?;
    let budget = input.compute.memory_limit_bytes;
    if reserved >= budget {
        return Err(anyhow!("Model exceeds the inference memory budget"));
    }
    trim_prediction_cache(&mut models, reserved, budget);
    let artifact_id = model.id;
    let compute = input.compute.clone();
    let predictor = std::sync::Arc::new(
        tokio::task::spawn_blocking(move || {
            CatalogLoadedPredictor::load(&repo, &artifact_id, &compute)
        })
        .await??,
    );
    models.push((key, predictor.clone(), reserved));
    Ok(predictor)
}

#[flow_like_types::async_trait]
impl NodeLogic for StudentInferenceNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<PredictRequest, Value>(
            "ml_student_inference",
            "Student Model Inference",
            "Predict with a versioned model reused within this workflow execution",
            "predict",
            "AI/ML/Lifecycle",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let mut input: PredictRequest = context.evaluate_pin("request").await?;
            let predictor = cached_predictor(context, &input).await?;
            if matches!(
                predictor.artifact().manifest["engine"].as_str(),
                Some("burn" | "efficient_ad" | "legacy_classical")
            ) && input.input.shape == input.inspection.spec.input_shape
            {
                input.input.shape.insert(0, 1);
            }
            let output =
                tokio::task::spawn_blocking(move || predictor.predict(&input.input)).await??;
            context.set_pin_value("result", json!(output)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!("Student inference requires the training feature"))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RecordModelComparisonNode;
lifecycle_node!(
    RecordModelComparisonNode,
    "ml_record_model_comparison",
    "Record Model Comparison",
    "Store aligned teacher and student predictions with audited outcomes",
    "recordComparison",
    RecordPredictionRequest,
    bool,
    record_prediction
);

#[crate::register_node]
#[derive(Default)]
pub struct EvaluateStudentNode;
lifecycle_node!(
    EvaluateStudentNode,
    "ml_evaluate_student",
    "Evaluate Student",
    "Measure audited classification quality separately from teacher agreement",
    "evaluateStudent",
    ArtifactRequest,
    EvaluationReport,
    evaluate
);

#[crate::register_node]
#[derive(Default)]
pub struct PromoteStudentNode;
lifecycle_node!(
    PromoteStudentNode,
    "ml_promote_student",
    "Promote Student",
    "Atomically promote a model that passes the audited quality and latency policy",
    "promoteStudent",
    PromoteRequest,
    Deployment,
    promote
);

#[crate::register_node]
#[derive(Default)]
pub struct GetDeploymentNode;
lifecycle_node!(
    GetDeploymentNode,
    "ml_get_deployment",
    "Get Active Student",
    "Resolve the promoted artifact; an empty result means the teacher still owns decisions",
    "deployment",
    DeploymentRequest,
    Option<Deployment>,
    deployment
);

#[crate::register_node]
#[derive(Default)]
pub struct RollbackStudentNode;
lifecycle_node!(
    RollbackStudentNode,
    "ml_rollback_student",
    "Rollback Student",
    "Atomically restore the previous model and its evaluation policy",
    "rollbackStudent",
    RollbackRequest,
    Deployment,
    rollback
);

#[crate::register_node]
#[derive(Default)]
pub struct EvaluateInspectionTaskNode;
lifecycle_node!(
    EvaluateInspectionTaskNode,
    "ml_evaluate_inspection_task",
    "Evaluate Inspection Task",
    "Derive audited regression, detection, segmentation or event metrics from held-out evidence",
    "evaluateTask",
    EvaluateTaskRequest,
    EvaluationReport,
    evaluate_task
);
#[crate::register_node]
#[derive(Default)]
pub struct PromoteByMetricsNode;
lifecycle_node!(
    PromoteByMetricsNode,
    "ml_promote_by_metrics",
    "Promote by Audited Metrics",
    "Promote an artifact only when its sealed evaluation meets named metric bounds",
    "promoteByMetrics",
    PromoteMetricsRequest,
    Deployment,
    promote_metrics
);

#[crate::register_node]
#[derive(Default)]
pub struct ExtractPatchFeaturesNode;
#[flow_like_types::async_trait]
impl NodeLogic for ExtractPatchFeaturesNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<PredictRequest, TensorData>(
            "ml_extract_patch_features",
            "Extract Image Features",
            "Extract spatial features from a Burn classifier reused within this workflow execution",
            "imageFeatures",
            "AI/ML/Lifecycle",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let mut input: PredictRequest = context.evaluate_pin("request").await?;
            let predictor = cached_predictor(context, &input).await?;
            if input.input.shape == input.inspection.spec.input_shape {
                input.input.shape.insert(0, 1);
            }
            let output =
                tokio::task::spawn_blocking(move || predictor.features(&input.input)).await??;
            context.set_pin_value("result", json!(output)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!(
                "Image feature extraction requires the training feature"
            ))
        }
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportOnnxRequest {
    pub prediction: PredictRequest,
    pub destination: flow_like_catalog_core::FlowPath,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportOnnxResult {
    pub path: flow_like_catalog_core::FlowPath,
    pub sha256: String,
    pub bytes: u64,
    pub opset: u32,
    pub input_shape: Vec<usize>,
    pub output_semantics: String,
}
#[crate::register_node]
#[derive(Default)]
pub struct ExportStudentOnnxNode;
#[flow_like_types::async_trait]
impl NodeLogic for ExportStudentOnnxNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<ExportOnnxRequest, ExportOnnxResult>(
            "ml_export_student_onnx",
            "Export Student to ONNX",
            "Export a supported inference graph with fixed dimensions and explicit output semantics",
            "exportOnnx",
            "AI/ML/Export",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "onnx-export")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let input: ExportOnnxRequest = context.evaluate_pin("request").await?;
            let (repo, project) = repository(context)?;
            let mut prediction = input.prediction;
            let expected = prediction.inspection.spec.input_shape.clone();
            artifact(
                repo.clone(),
                project,
                ArtifactRequest {
                    inspection: prediction.inspection,
                    artifact_id: prediction.artifact_id.clone(),
                },
            )?;
            if prediction.input.shape == expected {
                prediction.input.shape.insert(0, 1);
            }
            let (bytes, report) = tokio::task::spawn_blocking(move || {
                runtime::engines::export_artifact_onnx(
                    &repo,
                    &prediction.artifact_id,
                    &prediction.input,
                    &prediction.compute,
                )
            })
            .await??;
            let output = ExportOnnxResult {
                path: input.destination.clone(),
                sha256: flow_like_ml_core::content_digest(&bytes),
                bytes: bytes.len() as u64,
                opset: report.opset,
                input_shape: report.input_shape,
                output_semantics: report.output_semantics,
            };
            input.destination.put(context, bytes, false).await?;
            context.set_pin_value("result", json!(output)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "onnx-export"))]
        {
            let _ = context;
            Err(anyhow!("This executor needs the onnx-export feature"))
        }
    }
}
#[cfg(feature = "training")]
fn train_mlp(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::Mlp { .. }) {
        return Err(anyhow!("Select the MLP recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainMlpNode;
lifecycle_node!(
    TrainMlpNode,
    "ml_train_mlp",
    "Train MLP",
    "Schedule MLP training with durable checkpoints and explicit device selection",
    "trainMlp",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_mlp
);

#[cfg(feature = "training")]
fn train_lstm(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::Lstm { .. }) {
        return Err(anyhow!("Select the LSTM recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainLstmNode;
lifecycle_node!(
    TrainLstmNode,
    "ml_train_lstm",
    "Train LSTM",
    "Schedule LSTM training with durable checkpoints and explicit device selection",
    "trainLstm",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_lstm
);

#[cfg(feature = "training")]
fn train_gru(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::Gru { .. }) {
        return Err(anyhow!("Select the GRU recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainGruNode;
lifecycle_node!(
    TrainGruNode,
    "ml_train_gru",
    "Train GRU",
    "Schedule GRU training with durable checkpoints and explicit device selection",
    "trainGru",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_gru
);

#[cfg(feature = "training")]
fn train_cnn1d(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::Cnn1d { .. }) {
        return Err(anyhow!("Select the 1D CNN recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainCnn1dNode;
lifecycle_node!(
    TrainCnn1dNode,
    "ml_train_cnn1d",
    "Train 1D CNN",
    "Schedule 1D CNN training with durable checkpoints and explicit device selection",
    "trainCnn1d",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_cnn1d
);

#[cfg(feature = "training")]
fn train_tcn(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::Tcn { .. }) {
        return Err(anyhow!("Select the TCN recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainTcnNode;
lifecycle_node!(
    TrainTcnNode,
    "ml_train_tcn",
    "Train TCN",
    "Schedule TCN training with durable checkpoints and explicit device selection",
    "trainTcn",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_tcn
);

#[cfg(feature = "training")]
fn train_dense_autoencoder(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::DenseAutoencoder { .. }
    ) {
        return Err(anyhow!("Select the Dense Autoencoder recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainDenseAutoencoderNode;
lifecycle_node!(
    TrainDenseAutoencoderNode,
    "ml_train_dense_autoencoder",
    "Train Dense Autoencoder",
    "Schedule Dense Autoencoder training with durable checkpoints and explicit device selection",
    "trainDenseAutoencoder",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_dense_autoencoder
);

#[cfg(feature = "training")]
fn train_conv1d_autoencoder(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::Conv1dAutoencoder { .. }
    ) {
        return Err(anyhow!(
            "Select the Convolutional Sequence Autoencoder recipe for this node"
        ));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainConv1dAutoencoderNode;
lifecycle_node!(
    TrainConv1dAutoencoderNode,
    "ml_train_conv1d_autoencoder",
    "Train Convolutional Sequence Autoencoder",
    "Schedule Convolutional Sequence Autoencoder training with durable checkpoints and explicit device selection",
    "trainConv1dAutoencoder",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_conv1d_autoencoder
);

#[cfg(feature = "training")]
fn train_res_net18(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::ResNet18 { .. }
    ) {
        return Err(anyhow!("Select the ResNet-18 recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainResNet18Node;
lifecycle_node!(
    TrainResNet18Node,
    "ml_train_res_net18",
    "Train ResNet-18",
    "Schedule ResNet-18 training with durable checkpoints and explicit device selection",
    "trainResNet18",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_res_net18
);

#[cfg(feature = "training")]
fn train_mobile_net_v2(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::MobileNetV2 { .. }
    ) {
        return Err(anyhow!("Select the MobileNetV2 recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainMobileNetV2Node;
lifecycle_node!(
    TrainMobileNetV2Node,
    "ml_train_mobile_net_v2",
    "Train MobileNetV2",
    "Schedule MobileNetV2 training with durable checkpoints and explicit device selection",
    "trainMobileNetV2",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_mobile_net_v2
);

#[cfg(feature = "training")]
fn train_efficient_net(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::EfficientNet { .. }
    ) {
        return Err(anyhow!("Select the EfficientNet recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainEfficientNetNode;
lifecycle_node!(
    TrainEfficientNetNode,
    "ml_train_efficient_net",
    "Train EfficientNet",
    "Schedule EfficientNet training with durable checkpoints and explicit device selection",
    "trainEfficientNet",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_efficient_net
);

#[cfg(feature = "training")]
fn train_yolo_x(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::YoloX { .. }) {
        return Err(anyhow!("Select the YOLOX recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainYoloXNode;
lifecycle_node!(
    TrainYoloXNode,
    "ml_train_yolo_x",
    "Train YOLOX",
    "Schedule YOLOX training with durable checkpoints and explicit device selection",
    "trainYoloX",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_yolo_x
);

#[cfg(feature = "training")]
fn train_u_net(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(input.config.recipe, flow_like_ml_burn::Recipe::UNet { .. }) {
        return Err(anyhow!("Select the U-Net recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainUNetNode;
lifecycle_node!(
    TrainUNetNode,
    "ml_train_u_net",
    "Train U-Net",
    "Schedule U-Net training with durable checkpoints and explicit device selection",
    "trainUNet",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_u_net
);
#[cfg(feature = "training")]
fn train_lstm_autoencoder(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::LstmAutoencoder { .. }
    ) {
        return Err(anyhow!("Select the LSTM Autoencoder recipe for this node"));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainLstmAutoencoderNode;
lifecycle_node!(
    TrainLstmAutoencoderNode,
    "ml_train_lstm_autoencoder",
    "Train LSTM Autoencoder",
    "Schedule LSTM Autoencoder with durable checkpoints and explicit device selection",
    "trainLstmAutoencoder",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_lstm_autoencoder
);

#[cfg(feature = "training")]
fn train_cnn_lstm(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::CnnLstm { .. }
    ) {
        return Err(anyhow!(
            "Select the CNN-LSTM Video Model recipe for this node"
        ));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainCnnLstmNode;
lifecycle_node!(
    TrainCnnLstmNode,
    "ml_train_cnn_lstm",
    "Train CNN-LSTM Video Model",
    "Schedule CNN-LSTM Video Model with durable checkpoints and explicit device selection",
    "trainCnnLstm",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_cnn_lstm
);

#[cfg(feature = "training")]
fn train_image_sensor_fusion(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::ImageSensorFusion { .. }
    ) {
        return Err(anyhow!(
            "Select the Image/Sensor Fusion Model recipe for this node"
        ));
    }
    train_burn(repo, project, input)
}
#[crate::register_node]
#[derive(Default)]
pub struct TrainImageSensorFusionNode;
lifecycle_node!(
    TrainImageSensorFusionNode,
    "ml_train_image_sensor_fusion",
    "Train Image/Sensor Fusion Model",
    "Schedule Image/Sensor Fusion Model with durable checkpoints and explicit device selection",
    "trainImageSensorFusion",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_image_sensor_fusion
);

#[crate::register_node]
#[derive(Default)]
pub struct PauseStudentNode;
#[crate::register_node]
#[derive(Default)]
pub struct TrainEfficientAdNode;
#[crate::register_node]
#[derive(Default)]
pub struct TrainMaskRcnnNode;
#[cfg(feature = "training")]
fn train_mask_rcnn(
    repo: TrainingRepository,
    project: String,
    input: TrainBurnRequest,
) -> Result<Option<TrainingJob>> {
    if !matches!(
        input.config.recipe,
        flow_like_ml_burn::Recipe::MaskRcnn { .. }
    ) {
        return Err(anyhow!("Select the Mask R-CNN recipe for this node"));
    }
    train_burn(repo, project, input)
}
lifecycle_node!(
    TrainMaskRcnnNode,
    "ml_train_mask_rcnn",
    "Train Mask R-CNN",
    "Train a two-stage detector with region proposals, ROIAlign, boxes and instance masks",
    "trainMaskRcnn",
    TrainBurnRequest,
    Option<TrainingJob>,
    train_mask_rcnn
);
lifecycle_node!(
    TrainEfficientAdNode,
    "ml_train_efficient_ad",
    "Train EfficientAD",
    "Train a PDN teacher/student and autoencoder anomaly model from supplied pretrained teacher weights and normal images",
    "trainEfficientAd",
    TrainEfficientAdRequest,
    Option<TrainingJob>,
    train_efficient_ad
);
lifecycle_node!(
    PauseStudentNode,
    "ml_pause_student",
    "Pause/Resume Student Routing",
    "Return decisions to the teacher after drift or resume an audited deployment",
    "pauseStudent",
    PauseDeploymentRequest,
    Deployment,
    pause_deployment
);

#[crate::register_node]
#[derive(Default)]
pub struct RouteInspectionNode;
#[flow_like_types::async_trait]
impl NodeLogic for RouteInspectionNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        use flow_like::flow::variable::VariableType;
        let mut node = super::operation_node::<DeploymentRequest, InspectionRoute>(
            "ml_route_inspection",
            "Route Inspection",
            "Route to the teacher during collection/shadow evaluation and to the audited student after promotion",
            "route",
            "AI/ML/Lifecycle",
        );
        node.add_output_pin(
            "teacher",
            "Teacher",
            "Teacher owns this decision",
            VariableType::Execution,
        );
        node.add_output_pin(
            "student",
            "Student",
            "Promoted student owns this decision",
            VariableType::Execution,
        );
        node.add_output_pin(
            "shadow",
            "Shadow Candidate",
            "Run the candidate for comparison while the teacher owns the decision",
            VariableType::Execution,
        );
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            for pin in ["teacher", "student", "shadow", "exec_out"] {
                context.deactivate_exec_pin(pin).await?;
            }
            let input: DeploymentRequest = context.evaluate_pin("request").await?;
            let (repo, project) = repository(context)?;
            let route =
                tokio::task::spawn_blocking(move || routing(repo, project, input)).await??;
            context.set_pin_value("result", json!(route)).await?;
            match route.mode {
                InspectionMode::StudentOnly => context.activate_exec_pin("student").await?,
                InspectionMode::TeacherOnly => context.activate_exec_pin("teacher").await?,
                InspectionMode::Shadow => {
                    context.activate_exec_pin("teacher").await?;
                    context.activate_exec_pin("shadow").await?;
                }
            }
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!("Inspection routing requires the training feature"))
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn stream_authorization_includes_project_stream_and_canonical_specification() {
        let inspection = InspectionContext {
            stream_id: "camera-1".into(),
            spec: InspectionSpec {
                id: "surface".into(),
                description: "Surface classification".into(),
                task: flow_like_ml_core::TaskKind::ImageClassification,
                labels: vec!["good".into(), "scratch".into()],
                input_shape: vec![3, 16, 16],
                prediction_horizon_ms: None,
                minimum_examples: 10,
                minimum_examples_per_class: 2,
            },
        };
        let expected = inspection.key("project-a".into()).unwrap();
        let reordered: InspectionContext = flow_like_types::json::from_value(flow_like_types::json::json!({
            "spec": {"input_shape":[3,16,16], "minimum_examples_per_class":2,"minimum_examples":10,
                "labels":["good","scratch"],"task":"image_classification","description":"Surface classification",
                "prediction_horizon_ms":null,"id":"surface"}, "stream_id":"camera-1"
        })).unwrap();
        check_stream(&expected, &reordered.key("project-a".into()).unwrap()).unwrap();
        assert!(check_stream(&expected, &inspection.key("project-b".into()).unwrap()).is_err());
        let mut changed = inspection.clone();
        changed.stream_id = "camera-2".into();
        assert!(check_stream(&expected, &changed.key("project-a".into()).unwrap()).is_err());
        changed = inspection;
        changed.spec.labels.swap(0, 1);
        assert!(check_stream(&expected, &changed.key("project-a".into()).unwrap()).is_err());
    }

    #[test]
    fn native_gpu_requests_fail_before_scheduling_unless_fallback_is_explicit() {
        for engine in [
            "isolation_forest",
            "histogram_gradient_boosting",
            "patchcore",
            "padim",
        ] {
            let mut compute = ComputeConfig {
                backend: flow_like_ml_core::ComputeBackend::Wgpu,
                allow_cpu_fallback: false,
                ..Default::default()
            };
            assert!(validate_native_schedule_compute(engine, &compute).is_err());
            compute.allow_cpu_fallback = true;
            validate_native_schedule_compute(engine, &compute).unwrap();
            validate_native_schedule_compute(engine, &ComputeConfig::default()).unwrap();
        }
    }
}

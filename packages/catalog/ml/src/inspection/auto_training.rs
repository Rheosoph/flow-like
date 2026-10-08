use super::auto_training_tables::TableDatasetRequest;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_ml_core::{ComputeConfig, TaskKind};
use flow_like_ml_runtime::experiment::*;
use flow_like_types::{Result, Value, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A search has one pinned source and one evaluation contract for every candidate.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoTrainRequest {
    pub dataset: TableDatasetRequest,
    #[serde(default)]
    pub compute: ComputeConfig,
    #[serde(default)]
    pub search: AutoSearchConfig,
    #[serde(default)]
    pub budget: ExperimentBudget,
    #[serde(default)]
    pub goals: Option<ExperimentGoals>,
    #[serde(default)]
    pub tables: super::auto_training_tables::TableWritePolicy,
    #[serde(default)]
    pub ordered_target: bool,
    /// Optional task-compatible recipes, including architectures requiring an explicit input layout.
    #[serde(default)]
    pub initial_candidates: Vec<flow_like_ml_runtime::TrainingRequest>,
    #[serde(default)]
    pub feature_plan: Option<super::feature_engineering::FeaturePlan>,
    #[serde(default)]
    pub feature_sources: Vec<super::feature_engineering::FeatureSourceInput>,
    #[serde(default)]
    pub learning_cycle_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentRequest {
    pub experiment_id: String,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PredictAutoModelRequest {
    pub experiment_id: String,
    pub rows: Vec<Value>,
    #[serde(default)]
    pub compute: ComputeConfig,
    #[serde(default)]
    pub stateful_features: bool,
    #[serde(default)]
    pub feature_state: Option<flow_like_ml_native::feature_engineering::FeatureStreamState>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoModelPredictions {
    pub experiment_id: String,
    pub model_artifact_id: String,
    pub predictions: Vec<Value>,
    pub feature_state: Option<flow_like_ml_native::feature_engineering::FeatureStreamState>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportAutoModelRequest {
    pub experiment_id: String,
    pub weights: flow_like_catalog_core::FlowPath,
    pub manifest: flow_like_catalog_core::FlowPath,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportAutoModelResult {
    pub model_artifact_id: String,
    pub weights: flow_like_catalog_core::FlowPath,
    pub manifest: flow_like_catalog_core::FlowPath,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoTrainingResult {
    pub experiment_id: String,
    pub status: ExperimentStatus,
    pub stop_reason: Option<String>,
    pub goal_met: bool,
    pub unmet_constraints: Vec<String>,
    pub best_trial_id: Option<String>,
    pub model_artifact_id: Option<String>,
    pub model_bundle_ref: Option<flow_like_ml_runtime::ModelArtifact>,
    pub preprocessing_manifest: Value,
    pub source_table_versions: Vec<Value>,
    pub created_tables: Vec<Value>,
    pub updated_table_versions: Vec<Value>,
    pub dataset_snapshot_ids: Vec<String>,
    pub leaderboard: Vec<ExperimentTrial>,
    pub validation_metrics: std::collections::BTreeMap<String, f64>,
    pub final_test_metrics: std::collections::BTreeMap<String, f64>,
    pub predictions_table: Option<Value>,
    pub other_artifacts: Vec<Value>,
    pub budget_used: ExperimentUsage,
}

impl From<ExperimentResult> for AutoTrainingResult {
    fn from(result: ExperimentResult) -> Self {
        let exp = result.experiment;
        let best = exp
            .selected_trial_id
            .as_ref()
            .or(exp.best_trial_id.as_ref());
        let trial = result.trials.iter().find(|trial| Some(&trial.id) == best);
        let validation_metrics = trial
            .map(|trial| trial.validation_metrics.clone())
            .unwrap_or_default();
        let preprocessing_manifest = trial
            .and_then(|trial| {
                result
                    .datasets
                    .iter()
                    .find(|data| data.snapshot_id == trial.dataset_snapshot_id)
            })
            .map(|data| data.preprocessing_manifest.clone())
            .unwrap_or(exp.request.preprocessing_manifest);
        let final_test_metrics = result
            .final_evaluation
            .as_ref()
            .map(|evaluation| evaluation.metrics.clone())
            .unwrap_or_default();
        let mut other_artifacts = result.events;
        if let Some(evaluation) = result.final_evaluation {
            other_artifacts.push(flow_like_types::json::json!(evaluation));
        }
        let predictions_table = exp
            .request
            .created_tables
            .iter()
            .chain(&exp.request.updated_table_versions)
            .find(|table| table.get("partition").and_then(Value::as_str) == Some("predictions"))
            .cloned();
        Self {
            experiment_id: exp.id,
            status: exp.status,
            stop_reason: exp.stop_reason,
            goal_met: exp.target_met,
            unmet_constraints: exp.unmet_constraints,
            best_trial_id: exp.selected_trial_id.or(exp.best_trial_id),
            model_artifact_id: result
                .best_artifact
                .as_ref()
                .map(|artifact| artifact.id.clone()),
            model_bundle_ref: result.best_artifact,
            preprocessing_manifest,
            source_table_versions: exp.request.source_table_versions,
            created_tables: exp.request.created_tables,
            updated_table_versions: exp.request.updated_table_versions,
            dataset_snapshot_ids: result
                .datasets
                .into_iter()
                .map(|data| data.snapshot_id)
                .collect(),
            leaderboard: result.trials,
            validation_metrics,
            final_test_metrics,
            predictions_table,
            other_artifacts,
            budget_used: exp.usage,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoTrainTask {
    Tabular,
    Forecast,
    Vision,
    Anomaly,
}

fn validate_task(facade: AutoTrainTask, task: &TaskKind) -> Result<()> {
    let allowed = match facade {
        AutoTrainTask::Tabular => matches!(
            task,
            TaskKind::SensorClassification | TaskKind::SensorRegression
        ),
        AutoTrainTask::Forecast => matches!(
            task,
            TaskKind::SequenceForecast
                | TaskKind::SensorClassification
                | TaskKind::VisualSequenceClassification
        ),
        AutoTrainTask::Vision => matches!(
            task,
            TaskKind::ImageClassification
                | TaskKind::ObjectDetection
                | TaskKind::Segmentation
                | TaskKind::InstanceSegmentation
                | TaskKind::Fusion
        ),
        AutoTrainTask::Anomaly => matches!(
            task,
            TaskKind::SensorAnomaly | TaskKind::VisualAnomaly | TaskKind::SequenceAutoencoder
        ),
    };
    if !allowed {
        return Err(anyhow!(
            "The selected auto trainer does not support this inspection task"
        ));
    }
    Ok(())
}

macro_rules! task_node {
    ($node:ident, $id:literal, $name:literal, $function:literal, $task:ident) => {
        #[flow_like_types::async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> Node {
                super::operation_node::<AutoTrainRequest, AutoTrainingResult>(
                    $id,
                    $name,
                    "Pin a dataset and start a durable, bounded model search",
                    $function,
                    "AI/ML/Auto Training",
                )
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                #[cfg(feature = "training")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let request: AutoTrainRequest = context.evaluate_pin("request").await?;
                    validate_task(AutoTrainTask::$task, &request.dataset.spec.task)?;
                    let result = start_auto_training(context, request, true).await?;
                    context
                        .set_pin_value("result", flow_like_types::json::json!(result))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "training"))]
                {
                    let _ = context;
                    Err(anyhow!("Auto training requires a local training build"))
                }
            }
        }
    };
}

#[crate::register_node]
#[derive(Default)]
pub struct AutoTrainTabularNode;
#[crate::register_node]
#[derive(Default)]
pub struct AutoTrainForecastNode;
#[crate::register_node]
#[derive(Default)]
pub struct AutoTrainVisionNode;
#[crate::register_node]
#[derive(Default)]
pub struct AutoTrainAnomalyNode;

task_node!(
    AutoTrainTabularNode,
    "ml_auto_train_tabular",
    "Auto Train Tabular",
    "autoTrainTabular",
    Tabular
);
task_node!(
    AutoTrainForecastNode,
    "ml_auto_train_forecast",
    "Auto Train Forecast",
    "autoTrainForecast",
    Forecast
);
task_node!(
    AutoTrainVisionNode,
    "ml_auto_train_vision",
    "Auto Train Vision",
    "autoTrainVision",
    Vision
);
task_node!(
    AutoTrainAnomalyNode,
    "ml_auto_train_anomaly",
    "Auto Train Anomaly",
    "autoTrainAnomaly",
    Anomaly
);

#[cfg(feature = "training")]
pub fn experiment_repository(
    context: &ExecutionContext,
    id: &str,
) -> Result<flow_like_ml_runtime::TrainingRepository> {
    let (repo, project) = super::lifecycle::repository(context)?;
    let experiment = repo.get_experiment(id)?;
    if experiment.request.stream.project_id != project {
        return Err(anyhow!(
            "The experiment belongs to a different app or board"
        ));
    }
    Ok(repo)
}

#[cfg(feature = "training")]
pub fn controller(
    repo: flow_like_ml_runtime::TrainingRepository,
    budget: &ExperimentBudget,
) -> Result<flow_like_ml_runtime::auto_training::AutoTrainingController> {
    use flow_like_ml_runtime::{
        auto_training::AutoTrainingController, engines, worker::TrainingWorker,
    };
    let mut worker = TrainingWorker::new(repo.clone(), budget.worker_limits.clone())?;
    engines::register_native_engines(&mut worker)?;
    engines::register_burn_engine(&mut worker)?;
    engines::register_efficient_ad_engine(&mut worker)?;
    super::auto_training_legacy::register_legacy_engine(&mut worker)?;
    Ok(
        AutoTrainingController::new(repo, worker).with_predictor_factory(std::sync::Arc::new(
            super::auto_training_legacy::CatalogSearchPredictorFactory,
        )),
    )
}

#[cfg(feature = "training")]
fn snapshot_from_table(
    prepared: &super::auto_training_tables::PreparedTableDataset,
    stream: flow_like_ml_runtime::StreamKey,
    policy: flow_like_ml_runtime::SplitPolicy,
) -> Result<flow_like_ml_runtime::DatasetSnapshot> {
    use flow_like_ml_core::{Annotation, LabelProvenance};
    use flow_like_ml_runtime::{DatasetSnapshot, LabelSource, TrainingSample};
    prepared.split.validate(&prepared.samples)?;
    let convert = |indices: &[usize]| -> Result<Vec<TrainingSample>> {
        indices
            .iter()
            .map(|&index| {
                let mut sample = prepared.samples[index].clone();
                sample.stream_id = stream.stream_id.clone();
                let (source, available) = match &sample.provenance {
                    LabelProvenance::Teacher { .. } => (LabelSource::Teacher, sample.timestamp_ms),
                    LabelProvenance::Reviewed { reviewed_at_ms, .. } => {
                        (LabelSource::Reviewed, *reviewed_at_ms)
                    }
                    LabelProvenance::Measured { .. } => (
                        LabelSource::ObservedOutcome,
                        sample
                            .outcome
                            .as_ref()
                            .map_or(sample.timestamp_ms, |outcome| outcome.available_at_ms),
                    ),
                    LabelProvenance::Unlabeled => {
                        return Err(anyhow!("Training rows must have labels with provenance"));
                    }
                };
                if matches!(sample.annotation, Annotation::Unlabeled) {
                    return Err(anyhow!("A training row is unlabelled"));
                }
                let mut payload = flow_like_types::json::json!(sample);
                if let Some(row) = prepared.raw_rows.get(index) {
                    payload["feature_row"] = row.clone();
                }
                if let Annotation::Class { class_id } = &sample.annotation {
                    payload["label"] = flow_like_types::json::json!(
                        prepared
                            .spec
                            .labels
                            .get(*class_id as usize)
                            .ok_or_else(|| anyhow!("Class exceeds the declared label order"))?
                    );
                }
                Ok(TrainingSample {
                    id: sample.id.clone(),
                    annotation_revision: u64::try_from(
                        prepared
                            .annotation_revisions
                            .get(&sample.id)
                            .copied()
                            .unwrap_or(1),
                    )
                    .map_err(|_| anyhow!("Annotation revision must be positive"))?,
                    group_id: sample.group_id.clone(),
                    captured_at_ms: sample.timestamp_ms,
                    label_available_at_ms: available.max(
                        prepared
                            .label_available_at_ms
                            .get(&sample.id)
                            .copied()
                            .unwrap_or(available),
                    ),
                    content_digest: prepared
                        .raw_content_digests
                        .get(&sample.id)
                        .cloned()
                        .unwrap_or(flow_like_ml_core::content_digest(
                            &flow_like_types::json::to_vec(&sample.input)?,
                        )),
                    source,
                    accepted: true,
                    payload,
                })
            })
            .collect()
    };
    Ok(DatasetSnapshot {
        id: String::new(),
        digest: String::new(),
        stream: stream.clone(),
        as_of_ms: flow_like_ml_runtime::now_ms(),
        cutoff_sequence: prepared.samples.len() as i64,
        policy,
        train: convert(&prepared.split.train)?,
        validation: convert(&prepared.split.validation)?,
        test: convert(&prepared.split.test)?,
        excluded: prepared
            .split
            .purged
            .iter()
            .map(|&index| prepared.samples[index].id.clone())
            .collect(),
    })
}

#[cfg(feature = "training")]
fn table_split_policy(
    policy: &super::auto_training_tables::TableSplitPolicy,
) -> Result<flow_like_ml_runtime::SplitPolicy> {
    use super::auto_training_tables::TableSplitPolicy;
    Ok(match policy {
        TableSplitPolicy::Group {
            train_fraction,
            validation_fraction,
            seed,
        } => flow_like_ml_runtime::SplitPolicy::Group {
            train_fraction: *train_fraction,
            validation_fraction: *validation_fraction,
            seed: *seed,
        },
        TableSplitPolicy::Time {
            train_end_ms,
            validation_end_ms,
            embargo_ms,
        } => flow_like_ml_runtime::SplitPolicy::Time {
            train_end_ms: *train_end_ms,
            validation_end_ms: *validation_end_ms,
            embargo_ms: i64::try_from(*embargo_ms)
                .map_err(|_| anyhow!("Temporal embargo exceeds the supported range"))?,
        },
    })
}

#[cfg(feature = "training")]
pub async fn start_auto_training(
    context: &mut ExecutionContext,
    request: AutoTrainRequest,
    background: bool,
) -> Result<AutoTrainingResult> {
    start_auto_training_scoped(context, request, background, None).await
}

#[cfg(feature = "training")]
pub async fn start_auto_training_scoped(
    context: &mut ExecutionContext,
    mut request: AutoTrainRequest,
    background: bool,
    stream: Option<flow_like_ml_runtime::StreamKey>,
) -> Result<AutoTrainingResult> {
    use flow_like_ml_runtime::auto_training::{
        default_goals, generate_candidates, validate_search,
    };
    let (repo, project) = super::lifecycle::repository(context)?;
    let original_spec = request.dataset.spec.clone();
    let scoped_stream = stream
        .as_ref()
        .map(|value| {
            super::lifecycle::InspectionContext {
                stream_id: value.stream_id.clone(),
                spec: original_spec.clone(),
            }
            .key(project.clone())
        })
        .transpose()?;
    if stream
        .as_ref()
        .is_some_and(|value| Some(value) != scoped_stream.as_ref())
    {
        return Err(anyhow!(
            "Training stream belongs to another project or inspection contract"
        ));
    }
    if let Some(cycle_id) = &request.learning_cycle_id {
        if background {
            return Err(anyhow!(
                "Learning cycles must attach their experiment before dispatch"
            ));
        }
        let cycle = repo.get_learning_cycle(cycle_id)?;
        let learning = repo.get_learning_project(&cycle.project_id)?;
        if scoped_stream.as_ref() != Some(&learning.request.stream) {
            return Err(anyhow!("Learning cycle does not own this training stream"));
        }
    }
    let mut prepared = if let Some(plan) = &request.feature_plan {
        super::feature_engineering::prepare_feature_dataset(
            context,
            &request.dataset,
            plan,
            &request.feature_sources,
        )
        .await?
    } else {
        super::auto_training_tables::prepare_table_dataset(context, &request.dataset).await?
    };
    if request.feature_plan.is_none() {
        prepared.feature_sources =
            super::feature_engineering::pin_feature_sources(context, &request.feature_sources)
                .await?;
    }
    request.dataset.spec = prepared.spec.clone();
    request.dataset.source = prepared.source.database.clone();
    let expected_stream = super::lifecycle::InspectionContext {
        stream_id: format!("auto:{}", uuid::Uuid::new_v4()),
        spec: prepared.spec.clone(),
    }
    .key(project)?;
    let stream = scoped_stream.unwrap_or(expected_stream);
    let snapshot = snapshot_from_table(
        &prepared,
        stream.clone(),
        table_split_policy(&request.dataset.split)?,
    )?;
    for sample in snapshot
        .train
        .iter()
        .chain(&snapshot.validation)
        .chain(&snapshot.test)
    {
        repo.record_prepared_sample(&stream, sample)?;
    }
    let snapshot = repo.import_experiment_snapshot(snapshot)?;
    request.search.maximum_candidates = request
        .search
        .maximum_candidates
        .min(request.budget.maximum_trials);
    if let Some(sample) = prepared
        .split
        .train
        .first()
        .map(|&index| &prepared.samples[index])
    {
        request.search.output_features = match &sample.annotation {
            flow_like_ml_core::Annotation::Values { values } => values.len(),
            _ => 1,
        };
    }
    validate_search(&prepared.spec, &request.search, &request.compute)?;
    let mut candidates = request.initial_candidates.clone();
    for candidate in &candidates {
        if candidate.engine == "burn" {
            let config: flow_like_ml_burn::TrainingConfig = flow_like_types::json::from_value(
                candidate
                    .recipe
                    .get("config")
                    .cloned()
                    .ok_or_else(|| anyhow!("Burn candidate needs a training config"))?,
            )?;
            config.validate_planning_budget()?;
            flow_like_ml_burn::validate_inspection_config(&prepared.spec, &config)?;
        }
    }
    if prepared.spec.task == TaskKind::SensorClassification && prepared.spec.input_shape.len() == 1
    {
        candidates.extend(legacy_candidates(
            &prepared.spec,
            &request.compute,
            request.ordered_target,
        )?);
    } else if request.ordered_target {
        return Err(anyhow!(
            "Ordered targets require a tabular classification task with declared labels"
        ));
    }
    match generate_candidates(&prepared.spec, &request.search, &request.compute) {
        Ok(generated) => candidates.extend(generated),
        Err(error) if candidates.is_empty() => return Err(error.into()),
        Err(_) => {}
    }
    candidates.truncate(request.search.maximum_candidates);
    let mut goals = request
        .goals
        .clone()
        .unwrap_or_else(|| default_goals(&prepared.spec));
    if request.ordered_target && request.goals.is_none() {
        goals.primary_metric = "mean_rank_error".into();
        goals.direction = MetricDirection::Minimize;
    }
    let experiment = repo.create_experiment(flow_like_ml_runtime::experiment::ExperimentRequest {
        stream, snapshot_id: snapshot.id, spec: flow_like_types::json::json!(prepared.spec),
        candidates, goals, budget: request.budget.clone(),
        source_table_versions: std::iter::once(flow_like_types::json::json!(prepared.source.reference)).chain(prepared.feature_sources.iter().map(|source|flow_like_types::json::json!({"alias":source.alias,"reference":source.source.reference}))).collect(),
        created_tables: vec![], updated_table_versions: vec![], preprocessing_manifest: flow_like_types::json::json!(prepared.preprocessing),
        context: flow_like_types::json::json!({"training_profile":prepared.profile,"search":request.search,"compute":request.compute,"dataset_request":request.dataset,"table_policy":request.tables,"pinned_source":prepared.source,"ordered_target":request.ordered_target,"feature_plan":request.feature_plan,"feature_sources":prepared.feature_sources,"learning_cycle_id":request.learning_cycle_id}),
    }, flow_like_ml_runtime::now_ms())?;
    let tables = super::auto_training_tables::materialize_prepared_tables(
        context,
        &prepared,
        &experiment.id,
        &request.tables,
    )
    .await?;
    let references = tables
        .into_iter()
        .map(|table| flow_like_types::json::json!(table))
        .collect();
    if request.tables.allowed_destination.is_some() {
        repo.update_experiment_tables(
            &experiment.id,
            vec![],
            references,
            flow_like_ml_runtime::now_ms(),
        )?;
    } else {
        repo.update_experiment_tables(
            &experiment.id,
            references,
            vec![],
            flow_like_ml_runtime::now_ms(),
        )?;
    }
    if background {
        enqueue_experiment(repo.clone(), experiment.id.clone())?;
    }
    Ok(repo.experiment_result(&experiment.id)?.into())
}

#[cfg(feature = "training")]
fn legacy_candidates(
    spec: &flow_like_ml_core::InspectionSpec,
    compute: &ComputeConfig,
    ordered: bool,
) -> Result<Vec<flow_like_ml_runtime::TrainingRequest>> {
    use super::auto_training_legacy::{LegacyAlgorithm, LegacyRecipe};
    let algorithms = if ordered {
        vec![
            LegacyAlgorithm::OrdinalRidge { alpha: 1.0 },
            LegacyAlgorithm::OrdinalLogistic {
                alpha: 1.0,
                max_iterations: 500,
            },
        ]
    } else {
        vec![
            LegacyAlgorithm::GaussianNaiveBayes,
            LegacyAlgorithm::DecisionTree {
                max_depth: 8,
                min_leaf: 2,
            },
            LegacyAlgorithm::Logistic {
                alpha: 1.0,
                max_iterations: 200,
            },
        ]
    };
    algorithms
        .into_iter()
        .map(|algorithm| {
            Ok(flow_like_ml_runtime::TrainingRequest {
                engine: "legacy_classical".into(),
                recipe: flow_like_types::json::to_value(LegacyRecipe {
                    algorithm,
                    labels: spec.labels.clone(),
                })?,
                compute: flow_like_types::json::json!(compute),
            })
        })
        .collect()
}

#[cfg(feature = "training")]
pub fn consultation_context(
    repo: &flow_like_ml_runtime::TrainingRepository,
    id: &str,
) -> Result<Value> {
    let state = repo.experiment_result(id)?;
    let experiment = state.experiment;
    let trials = state.trials.iter().map(|trial| -> Result<Value> {
        let job = repo.get_job(&trial.job_id)?;
        let config = if job.request.engine == "burn" {
            flow_like_types::json::from_value::<flow_like_ml_burn::TrainingConfig>(job.request.recipe["config"].clone()).ok().map(|config| flow_like_types::json::json!(config)).unwrap_or(Value::Null)
        } else {
            let mut summary = flow_like_types::json::Map::new();
            for key in ["objective","estimators","max_depth","max_bins","min_leaf","learning_rate","l2","trees","sample_size","seed","epochs","batch_size","autoencoder_channels","hard_quantile","gradient_clip"] {
                if let Some(value) = job.request.recipe["config"].get(key).filter(|value| !value.is_object() && !value.is_array()) { summary.insert(key.into(),value.clone()); }
            }
            if let Ok(recipe) = flow_like_types::json::from_value::<super::auto_training_legacy::LegacyRecipe>(job.request.recipe.clone()) {
                summary.insert("algorithm".into(),flow_like_types::json::json!(recipe.algorithm));
            }
            Value::Object(summary)
        };
        Ok(flow_like_types::json::json!({
            "trial_id":trial.id,"candidate_index":trial.candidate_index,"status":trial.status,
            "engine":job.request.engine,"config":config,"dataset_snapshot_id":trial.dataset_snapshot_id,
            "validation_metrics":trial.validation_metrics,"error":trial.error,
            "training_time_ms":trial.training_time_ms,"progress":job.progress,
        }))
    }).collect::<Result<Vec<_>>>()?;
    let datasets = state.datasets.iter().map(|dataset| {
        let preprocessing = flow_like_types::json::from_value::<super::auto_training_tables::TablePreprocessing>(dataset.preprocessing_manifest.clone()).ok();
        let (mut shape, numeric, categorical) = match preprocessing.as_ref().map(super::auto_training_tables::base_preprocessing) {
            Some(super::auto_training_tables::TablePreprocessing::Tabular { fitted }) => {
                use flow_like_ml_native::preprocessing::FittedColumn;
                let numeric:Vec<_> = fitted.columns.iter().filter_map(|column| if let FittedColumn::Numeric { name,.. } = column {Some(name)} else {None}).cloned().collect();
                let categorical:Vec<_> = fitted.columns.iter().filter_map(|column| if let FittedColumn::Categorical { name,.. } = column {Some(name)} else {None}).cloned().collect();
                (vec![fitted.output_features.len()],numeric,categorical)
            },
            Some(super::auto_training_tables::TablePreprocessing::Tensor { input_shape,.. } | super::auto_training_tables::TablePreprocessing::Sample { input_shape,.. }) => (input_shape.clone(),vec![],vec![]),
            _ => (vec![],vec![],vec![]),
        };
        let plan=if let Some(super::auto_training_tables::TablePreprocessing::Engineered {projection,plan,..})=&preprocessing {
            if let Some(projection)=projection {shape=vec![projection.components.len()];}
            plan.as_deref()
        }else{None};
        flow_like_types::json::json!({"dataset_snapshot_id":dataset.snapshot_id,"input_shape":shape,"numeric_columns":numeric,"categorical_columns":categorical,"feature_plan":plan,"digest":dataset.digest})
    }).collect::<Vec<_>>();
    // Only this explicit projection is sent to the consultant. It has no test rows or test scores.
    let result = flow_like_types::json::json!({
        "spec": experiment.request.spec,
        "goals": experiment.request.goals,
        "budget": experiment.request.budget,
        "budget_used": experiment.usage,
        "training_profile": experiment.request.context.get("training_profile"),
        "allowed_features": experiment.request.context.pointer("/dataset_request/mapping/features"),
        "feature_plan": experiment.request.context.get("feature_plan"),
        "feature_sources": experiment.request.context.get("feature_sources").and_then(Value::as_array).map(|sources|sources.iter().map(|source|flow_like_types::json::json!({"alias":source.get("alias"),"allowed_columns":source.get("allowed_columns")})).collect::<Vec<_>>()),
        "feature_operations":["derive_columns","window_features","join_sources","train_only_imputation","train_only_scaling","train_only_pca"],
        "ordered_target": experiment.request.context.get("ordered_target"),
        "trials": trials,
        "registered_datasets":datasets
    });
    if flow_like_types::json::to_vec(&result)?.len() > 128 * 1024 {
        return Err(anyhow!("Consultation report exceeds its 128 KiB limit"));
    }
    Ok(result)
}

#[cfg(feature = "training")]
pub fn append_burn_candidate(
    repo: &flow_like_ml_runtime::TrainingRepository,
    id: &str,
    config: flow_like_ml_burn::TrainingConfig,
    dataset_snapshot_id: Option<String>,
) -> Result<()> {
    let state = repo.experiment_result(id)?;
    let experiment = state.experiment;
    let dataset_id = dataset_snapshot_id
        .as_deref()
        .unwrap_or(&experiment.request.snapshot_id);
    let dataset = state
        .datasets
        .iter()
        .find(|dataset| dataset.snapshot_id == dataset_id)
        .ok_or_else(|| anyhow!("Candidate dataset is not registered in this experiment"))?;
    let training = repo.get_snapshot_limited(
        &dataset.training_snapshot_id,
        experiment.request.budget.maximum_dataset_bytes,
    )?;
    let row = training
        .train
        .first()
        .ok_or_else(|| anyhow!("Candidate dataset has no training rows"))?;
    let sample: flow_like_ml_core::Sample = flow_like_types::json::from_value(
        row.payload.get("sample").unwrap_or(&row.payload).clone(),
    )?;
    let mut spec: flow_like_ml_core::InspectionSpec =
        flow_like_types::json::from_value(experiment.request.spec.clone())?;
    spec.input_shape = sample.input.shape;
    config.validate_planning_budget()?;
    flow_like_ml_burn::validate_inspection_config(&spec, &config)?;
    let compute: ComputeConfig = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("compute")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!({})),
    )?;
    let candidate = flow_like_ml_runtime::TrainingRequest {
        engine: "burn".into(),
        recipe: flow_like_types::json::json!({"config":config,"labels":spec.labels,"preprocessing":[],"inspection_task":spec.task,"minimum_examples":spec.minimum_examples,"minimum_examples_per_class":spec.minimum_examples_per_class,"dataset_snapshot_id":dataset_id}),
        compute: flow_like_types::json::json!(compute),
    };
    repo.append_experiment_candidates(id, vec![candidate], flow_like_ml_runtime::now_ms())?;
    Ok(())
}

#[cfg(feature = "training")]
pub async fn derive_feature_candidate(
    context: &mut ExecutionContext,
    id: &str,
    numeric_columns: Vec<String>,
    categorical_columns: Vec<String>,
    standardize: bool,
) -> Result<()> {
    use super::auto_training_tables::FeatureSource;
    let repo = experiment_repository(context, id)?;
    let experiment = repo.get_experiment(id)?;
    let request: TableDatasetRequest =
        flow_like_types::json::from_value(experiment.request.context["dataset_request"].clone())?;
    let FeatureSource::Tabular { options } = request.mapping.features else {
        return Err(anyhow!("Feature selection requires a tabular mapping"));
    };
    let original_allowed: std::collections::HashSet<_> = options
        .numeric_columns
        .iter()
        .chain(&options.categorical_columns)
        .cloned()
        .collect();
    let mut plan = experiment
        .request
        .context
        .get("feature_plan")
        .filter(|value| !value.is_null())
        .map(|value| {
            flow_like_types::json::from_value::<super::feature_engineering::FeaturePlan>(
                value.clone(),
            )
        })
        .transpose()?
        .unwrap_or(super::feature_engineering::FeaturePlan {
            pipeline: Default::default(),
            numeric_columns: options.numeric_columns,
            categorical_columns: options.categorical_columns,
            imputation: options.imputation,
            standardize: options.standardize,
            pca_components: None,
            limits: Default::default(),
        });
    let requested: Vec<_> = numeric_columns.iter().chain(&categorical_columns).collect();
    if requested.iter().any(|name| {
        !original_allowed.contains(*name) && !plan.pipeline.generated_columns().contains(*name)
    }) {
        let stored =
            repo.experiment_result(id)?
                .datasets
                .into_iter()
                .filter_map(|dataset| {
                    flow_like_types::json::from_value::<
                        super::auto_training_tables::TablePreprocessing,
                    >(dataset.preprocessing_manifest)
                    .ok()
                })
                .find_map(|preprocessing| {
                    if let super::auto_training_tables::TablePreprocessing::Engineered {
                        plan: Some(plan),
                        ..
                    } = preprocessing
                    {
                        let generated = plan.pipeline.generated_columns();
                        if requested.iter().all(|name| {
                            original_allowed.contains(*name) || generated.contains(*name)
                        }) {
                            return Some(*plan);
                        }
                    }
                    None
                })
                .ok_or_else(|| {
                    anyhow!("Selected engineered columns require a registered feature pipeline")
                })?;
        plan = stored;
    }
    plan.numeric_columns = numeric_columns;
    plan.categorical_columns = categorical_columns;
    plan.standardize = standardize;
    super::feature_engineering::derive_pipeline_candidate(context, id, plan).await
}

#[cfg(feature = "training")]
pub(crate) async fn register_prepared_candidate(
    context: &mut ExecutionContext,
    id: &str,
    prepared: super::auto_training_tables::PreparedTableDataset,
) -> Result<()> {
    use super::auto_training_tables::TableWritePolicy;
    let repo = experiment_repository(context, id)?;
    let experiment = repo.get_experiment(id)?;
    let mut snapshot = repo.get_snapshot_limited(
        &experiment.request.snapshot_id,
        experiment.request.budget.maximum_dataset_bytes,
    )?;
    snapshot.id.clear();
    snapshot.digest.clear();
    let inputs: std::collections::HashMap<_, _> = prepared
        .samples
        .iter()
        .map(|sample| (sample.id.as_str(), &sample.input))
        .collect();
    for sample in snapshot
        .train
        .iter_mut()
        .chain(&mut snapshot.validation)
        .chain(&mut snapshot.test)
    {
        let input = inputs
            .get(sample.id.as_str())
            .ok_or_else(|| anyhow!("A derived dataset changed row membership"))?;
        let payload = if sample.payload.get("sample").is_some() {
            &mut sample.payload["sample"]
        } else {
            &mut sample.payload
        };
        payload["input"] = flow_like_types::json::json!(input);
    }
    let dataset = repo.register_experiment_dataset(
        id,
        snapshot,
        flow_like_types::json::json!(prepared.preprocessing),
        flow_like_ml_runtime::now_ms(),
    )?;
    let mut search: AutoSearchConfig = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("search")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!({})),
    )?;
    search.maximum_candidates = 1;
    let compute: ComputeConfig = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("compute")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!({})),
    )?;
    let mut candidates = flow_like_ml_runtime::auto_training::generate_candidates(
        &prepared.spec,
        &search,
        &compute,
    )?;
    for candidate in &mut candidates {
        candidate.recipe["dataset_snapshot_id"] = flow_like_types::json::json!(dataset.snapshot_id);
    }
    repo.append_experiment_candidates(id, candidates, flow_like_ml_runtime::now_ms())?;
    let mut policy: TableWritePolicy = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("table_policy")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!({})),
    )?;
    if policy.allowed_destination.is_some() {
        if !policy.create_tables {
            return Err(anyhow!(
                "Derived candidates require authorization to create a separate versioned table"
            ));
        }
        policy.allowed_destination = None;
        policy.expected_destination = None;
    }
    let tables = super::auto_training_tables::materialize_prepared_tables(
        context,
        &prepared,
        &format!("{id}-{}", dataset.snapshot_id),
        &policy,
    )
    .await?;
    let refs = tables
        .into_iter()
        .map(|table| flow_like_types::json::json!(table))
        .collect();
    if policy.allowed_destination.is_some() {
        repo.update_experiment_tables(id, vec![], refs, flow_like_ml_runtime::now_ms())?;
    } else {
        repo.update_experiment_tables(id, refs, vec![], flow_like_ml_runtime::now_ms())?;
    }
    Ok(())
}

#[cfg(feature = "training")]
pub async fn run_next_auto_training(
    context: &ExecutionContext,
    id: &str,
) -> Result<AutoTrainingResult> {
    let repo = experiment_repository(context, id)?;
    let experiment = repo.get_experiment(id)?;
    let controller = controller(repo.clone(), &experiment.request.budget)?;
    let owned_id = id.to_string();
    let mut work = tokio::task::spawn_blocking(move || controller.run_next(&owned_id));
    if let Some(token) = context.get_cancellation_token() {
        tokio::select! {
            result = &mut work => { result??; },
            _ = token.cancelled() => {
                repo.cancel_experiment(id, flow_like_ml_runtime::now_ms())?;
                return Err(anyhow!("Auto training was cancelled; its experiment ID can be resumed"));
            }
        }
    } else {
        work.await??;
    }
    Ok(repo.experiment_result(id)?.into())
}

#[cfg(feature = "training")]
pub fn resume_saved_experiment(
    repo: &flow_like_ml_runtime::TrainingRepository,
    id: &str,
) -> Result<()> {
    let experiment = repo.get_experiment(id)?;
    for trial in repo.list_experiment_trials(id)? {
        let job = repo.get_job(&trial.job_id)?;
        if matches!(
            job.status,
            flow_like_ml_runtime::JobStatus::Running
                | flow_like_ml_runtime::JobStatus::CancelRequested
        ) && job
            .lease_expires_at_ms
            .is_some_and(|expiry| expiry < flow_like_ml_runtime::now_ms())
        {
            flow_like_ml_runtime::worker::reconcile_expired_job(repo, &job.id)?;
        }
    }
    if matches!(
        experiment.status,
        ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
    ) {
        if experiment.selected_trial_id.is_some() {
            repo.resume_frozen_experiment_audit(id, flow_like_ml_runtime::now_ms())?;
        } else {
            repo.resume_experiment(id, flow_like_ml_runtime::now_ms())?;
        }
    }
    Ok(())
}

#[cfg(feature = "training")]
pub async fn finalize_auto_training(
    context: &ExecutionContext,
    id: &str,
) -> Result<AutoTrainingResult> {
    let repo = experiment_repository(context, id)?;
    let experiment = repo.get_experiment(id)?;
    let controller = controller(repo.clone(), &experiment.request.budget)?;
    let owned_id = id.to_string();
    let mut work = tokio::task::spawn_blocking(move || controller.finalize(&owned_id));
    if let Some(token) = context.get_cancellation_token() {
        tokio::select! {
            result = &mut work => { result??; },
            _ = token.cancelled() => {
                repo.cancel_experiment(id, flow_like_ml_runtime::now_ms())?;
                return Err(anyhow!("Final audit was cancelled; resume the experiment to continue auditing the same frozen model"));
            }
        }
    } else {
        work.await??;
    }
    Ok(repo.experiment_result(id)?.into())
}

#[cfg(feature = "training")]
pub async fn materialize_final_predictions(
    context: &mut ExecutionContext,
    id: &str,
) -> Result<AutoTrainingResult> {
    use super::auto_training_tables::{PinnedTableSource, TableWritePolicy};
    let repo = experiment_repository(context, id)?;
    let mut result: AutoTrainingResult = repo.experiment_result(id)?.into();
    if result.predictions_table.is_some() || result.final_test_metrics.is_empty() {
        return Ok(result);
    }
    let experiment = repo.get_experiment(id)?;
    let mut policy: TableWritePolicy = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("table_policy")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!({})),
    )?;
    if !policy.create_tables {
        return Ok(result);
    }
    policy.allowed_destination = None;
    policy.expected_destination = None;
    let Some(artifact_id) = &result.model_artifact_id else {
        return Ok(result);
    };
    let rows = repo.predictions(artifact_id)?.into_iter().map(|record| Ok(flow_like_types::json::json!({"sample_id":record.sample_id,"record_json":flow_like_types::json::to_string(&record)?}))).collect::<Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(result);
    }
    let mut source: PinnedTableSource = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("pinned_source")
            .cloned()
            .ok_or_else(|| anyhow!("Experiment has no source table locator"))?,
    )?;
    source.database = super::auto_training_tables::reopen_pinned_source(context, &source).await?;
    let digest = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(
        &result.preprocessing_manifest,
    )?);
    let table = super::auto_training_tables::write_experiment_rows(
        context,
        &source,
        rows,
        id,
        "predictions",
        &digest,
        &policy,
    )
    .await?;
    repo.update_experiment_tables(
        id,
        vec![flow_like_types::json::json!(table)],
        vec![],
        flow_like_ml_runtime::now_ms(),
    )?;
    result = repo.experiment_result(id)?.into();
    Ok(result)
}

#[derive(Clone, Copy)]
enum LifecycleAction {
    Status,
    Step,
    Cancel,
    Resume,
    Finalize,
}

#[cfg(feature = "training")]
async fn lifecycle(
    context: &mut ExecutionContext,
    id: &str,
    action: LifecycleAction,
) -> Result<AutoTrainingResult> {
    if matches!(action, LifecycleAction::Step) {
        return run_next_auto_training(context, id).await;
    }
    let repo = experiment_repository(context, id)?;
    match action {
        LifecycleAction::Status => {}
        LifecycleAction::Step => unreachable!(),
        LifecycleAction::Cancel => {
            repo.cancel_experiment(id, flow_like_ml_runtime::now_ms())?;
        }
        LifecycleAction::Resume => {
            resume_saved_experiment(&repo, id)?;
            enqueue_experiment(repo.clone(), id.to_string())?;
        }
        LifecycleAction::Finalize => {
            finalize_auto_training(context, id).await?;
        }
    }
    materialize_final_predictions(context, id).await
}

macro_rules! lifecycle_node {
    ($node:ident, $id:literal, $name:literal, $description:literal, $function:literal, $action:ident) => {
        #[flow_like_types::async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> Node {
                let mut node = super::operation_node::<ExperimentRequest, AutoTrainingResult>(
                    $id,
                    $name,
                    $description,
                    $function,
                    "AI/ML/Auto Training",
                );
                node.set_long_running(matches!(
                    LifecycleAction::$action,
                    LifecycleAction::Step | LifecycleAction::Finalize
                ));
                node
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                #[cfg(feature = "training")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let request: ExperimentRequest = context.evaluate_pin("request").await?;
                    let result =
                        lifecycle(context, &request.experiment_id, LifecycleAction::$action)
                            .await?;
                    context
                        .set_pin_value("result", flow_like_types::json::json!(result))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "training"))]
                {
                    let _ = context;
                    Err(anyhow!("Auto training requires a local training build"))
                }
            }
        }
    };
}

#[crate::register_node]
#[derive(Default)]
pub struct GetAutoTrainingNode;
#[crate::register_node]
#[derive(Default)]
pub struct StepAutoTrainingNode;
#[crate::register_node]
#[derive(Default)]
pub struct CancelAutoTrainingNode;
#[crate::register_node]
#[derive(Default)]
pub struct ResumeAutoTrainingNode;
#[crate::register_node]
#[derive(Default)]
pub struct FinalizeAutoTrainingNode;

lifecycle_node!(
    GetAutoTrainingNode,
    "ml_get_auto_training",
    "Get Auto Training Result",
    "Read the durable experiment, leaderboard, table versions and best model bundle",
    "getAutoTraining",
    Status
);
lifecycle_node!(
    StepAutoTrainingNode,
    "ml_step_auto_training",
    "Step Auto Training",
    "Run one candidate and return its measured validation results",
    "stepAutoTraining",
    Step
);
lifecycle_node!(
    CancelAutoTrainingNode,
    "ml_cancel_auto_training",
    "Cancel Auto Training",
    "Cancel active trials and preserve the experiment and checkpoints",
    "cancelAutoTraining",
    Cancel
);
lifecycle_node!(
    ResumeAutoTrainingNode,
    "ml_resume_auto_training",
    "Resume Auto Training",
    "Resume a saved experiment in the background within its remaining budget",
    "resumeAutoTraining",
    Resume
);
lifecycle_node!(
    FinalizeAutoTrainingNode,
    "ml_finalize_auto_training",
    "Finalize Auto Training",
    "Freeze the selected candidate and audit it against the untouched test partition",
    "finalizeAutoTraining",
    Finalize
);

#[crate::register_node]
#[derive(Default)]
pub struct PredictAutoModelNode;

#[flow_like_types::async_trait]
impl NodeLogic for PredictAutoModelNode {
    fn get_node(&self) -> Node {
        super::operation_node::<PredictAutoModelRequest, AutoModelPredictions>(
            "ml_predict_auto_model",
            "Predict Auto Model",
            "Apply the winning model's saved preprocessing to raw rows and load its durable weights",
            "predictAutoModel",
            "AI/ML/Auto Training",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: PredictAutoModelRequest = context.evaluate_pin("request").await?;
            let repo = experiment_repository(context, &request.experiment_id)?;
            let result: AutoTrainingResult = repo.experiment_result(&request.experiment_id)?.into();
            let artifact = result
                .model_artifact_id
                .ok_or_else(|| anyhow!("The experiment has no successful model yet"))?;
            if request.rows.len() > 100_000 {
                return Err(anyhow!("Prediction batch exceeds 100,000 rows"));
            }
            let result = tokio::task::spawn_blocking(move || -> Result<AutoModelPredictions> {
                let preprocessing =
                    flow_like_types::json::from_value(result.preprocessing_manifest)?;
                let (inputs, feature_state) =
                    super::auto_training_tables::transform_table_rows_with_state(
                        &preprocessing,
                        &request.rows,
                        32 * 1024 * 1024,
                        request.feature_state,
                        request.stateful_features,
                    )?;
                use flow_like_ml_runtime::auto_training::SearchPredictorFactory;
                let mut predictor = super::auto_training_legacy::CatalogSearchPredictorFactory
                    .load(&repo, &artifact, &request.compute)?;
                let predictions = inputs
                    .into_iter()
                    .map(|mut input| {
                        input.shape.insert(0, 1);
                        Ok(predictor.predict(&input)?)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(AutoModelPredictions {
                    experiment_id: request.experiment_id,
                    model_artifact_id: artifact,
                    predictions,
                    feature_state,
                })
            })
            .await??;
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!(
                "Auto model inference requires a local training build"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExportAutoModelNode;

#[flow_like_types::async_trait]
impl NodeLogic for ExportAutoModelNode {
    fn get_node(&self) -> Node {
        super::operation_node::<ExportAutoModelRequest, ExportAutoModelResult>(
            "ml_export_auto_model",
            "Export Auto Model",
            "Write native model weights and their task, preprocessing and evaluation manifest",
            "exportAutoModel",
            "AI/ML/Auto Training",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "training")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: ExportAutoModelRequest = context.evaluate_pin("request").await?;
            if flow_like_types::json::to_value(&request.weights)?
                == flow_like_types::json::to_value(&request.manifest)?
            {
                return Err(anyhow!(
                    "Weights and manifest must have different destinations"
                ));
            }
            let repo = experiment_repository(context, &request.experiment_id)?;
            let result: AutoTrainingResult = repo.experiment_result(&request.experiment_id)?.into();
            let artifact = result
                .model_bundle_ref
                .clone()
                .ok_or_else(|| anyhow!("The experiment has no model to export"))?;
            let blob = artifact.blob.clone();
            let maximum = repo
                .get_experiment(&request.experiment_id)?
                .request
                .budget
                .worker_limits
                .maximum_artifact_bytes;
            let bytes = tokio::task::spawn_blocking(move || repo.read_blob_limited(&blob, maximum))
                .await??;
            let manifest = flow_like_types::json::to_vec_pretty(&flow_like_types::json::json!({
                "format_version":1,"artifact":artifact,"preprocessing":result.preprocessing_manifest,
                "source_table_versions":result.source_table_versions,"validation_metrics":result.validation_metrics,
                "final_test_metrics":result.final_test_metrics,"goal_met":result.goal_met
            }))?;
            request.weights.put(context, bytes, false).await?;
            request.manifest.put(context, manifest, false).await?;
            context
                .set_pin_value(
                    "result",
                    flow_like_types::json::json!(ExportAutoModelResult {
                        model_artifact_id: artifact.id,
                        weights: request.weights,
                        manifest: request.manifest,
                        sha256: artifact.blob.sha256,
                        bytes: artifact.blob.bytes,
                    }),
                )
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "training"))]
        {
            let _ = context;
            Err(anyhow!("Auto model export requires a local training build"))
        }
    }
}

#[cfg(feature = "training")]
pub fn enqueue_experiment(
    repo: flow_like_ml_runtime::TrainingRepository,
    id: String,
) -> Result<()> {
    use std::sync::{
        LazyLock, Mutex,
        mpsc::{SyncSender, sync_channel},
    };
    type Work = (flow_like_ml_runtime::TrainingRepository, String, String);
    static QUEUED: LazyLock<Mutex<std::collections::HashSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
    static WORKER: LazyLock<std::result::Result<SyncSender<Work>, String>> = LazyLock::new(|| {
        let (sender, receiver) = sync_channel::<Work>(32);
        std::thread::Builder::new()
            .name("auto-training".into())
            .spawn(move || {
                while let Ok((repo, id, key)) = receiver.recv() {
                    let result = (|| -> Result<()> {
                        let experiment = repo.get_experiment(&id)?;
                        controller(repo, &experiment.request.budget)?.run(&id)?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        tracing::warn!(experiment_id = %id, %error, "Auto training worker stopped");
                    }
                    if let Ok(mut queued) = QUEUED.lock() {
                        queued.remove(&key);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(sender)
    });
    let sender = WORKER
        .as_ref()
        .map_err(|error| anyhow!("Cannot start auto training worker: {error}"))?;
    let key =
        flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(repo.path(), &id))?);
    let mut queued = QUEUED
        .lock()
        .map_err(|_| anyhow!("Auto training queue lock is poisoned"))?;
    if !queued.insert(key.clone()) {
        return Ok(());
    }
    if let Err(error) = sender.try_send((repo, id, key.clone())) {
        queued.remove(&key);
        return Err(anyhow!(
            "Experiment saved. Resume Auto Training can retry the full worker queue: {error}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_facades_reject_incompatible_objectives() {
        assert!(validate_task(AutoTrainTask::Tabular, &TaskKind::SensorRegression).is_ok());
        assert!(validate_task(AutoTrainTask::Forecast, &TaskKind::SequenceForecast).is_ok());
        assert!(validate_task(AutoTrainTask::Vision, &TaskKind::ObjectDetection).is_ok());
        assert!(validate_task(AutoTrainTask::Anomaly, &TaskKind::SequenceAutoencoder).is_ok());
        assert!(validate_task(AutoTrainTask::Tabular, &TaskKind::ObjectDetection).is_err());
    }

    #[cfg(feature = "training")]
    #[test]
    fn ordinal_table_search_returns_a_durable_bundle_with_fitted_preprocessing() {
        use super::super::auto_training_tables::{
            PinnedTableSource, prepare_rows, transform_table_rows,
        };
        use flow_like_ml_runtime::{
            MetricBound, StreamKey, TrainingRepository, auto_training::SearchPredictorFactory,
        };
        use flow_like_types::json::{from_value, json};
        let directory =
            std::env::temp_dir().join(format!("flow-like-ordinal-search-{}", uuid::Uuid::new_v4()));
        let repo = TrainingRepository::open(directory.join("training.sqlite")).unwrap();
        let source: PinnedTableSource = from_value(json!({"database":{"cache_key":"source"},"reference":{"table":"measurements","branch":"main","version":1,"read_only":true,"pinned":true},"locator":null})).unwrap();
        let request: TableDatasetRequest = from_value(json!({
            "source":{"cache_key":"source"},"stream_id":"quality",
            "spec":{"id":"ordered-quality","description":"Predict quality level","task":"sensor_classification","labels":["low","medium","high"],"input_shape":[1],"prediction_horizon_ms":null,"minimum_examples":3,"minimum_examples_per_class":1},
            "mapping":{"row_id":"id","group_id":"group","features":{"kind":"tabular","options":{"numeric_columns":["measurement"],"categorical_columns":[]}},"target":{"kind":"column","column":"target"},"provenance":{"kind":"measured","source":"gauge"}},
            "split":{"kind":"group","train_fraction":0.6,"validation_fraction":0.2,"seed":42}
        })).unwrap();
        let rows: Vec<_> = (0..90).map(|i| json!({"id":i,"group":i/3,"measurement":(i%3) as f64 * 20.0 + i as f64 / 100.0,"target":(["low","medium","high"][i%3])})).collect();
        let prepared = prepare_rows(source, &request, &rows).unwrap();
        let stream = StreamKey {
            project_id: "test".into(),
            stream_id: "quality".into(),
            inspection_version: "v1".into(),
        };
        let snapshot = snapshot_from_table(
            &prepared,
            stream.clone(),
            table_split_policy(&request.split).unwrap(),
        )
        .unwrap();
        for sample in snapshot
            .train
            .iter()
            .chain(&snapshot.validation)
            .chain(&snapshot.test)
        {
            repo.record_sample(&stream, sample).unwrap();
        }
        let snapshot = repo.import_experiment_snapshot(snapshot).unwrap();
        let mut goals = flow_like_ml_runtime::auto_training::default_goals(&prepared.spec);
        goals.primary_metric = "mean_rank_error".into();
        goals.direction = MetricDirection::Minimize;
        goals.bounds = vec![MetricBound {
            name: "mean_rank_error".into(),
            minimum: None,
            maximum: Some(0.5),
        }];
        let compute = ComputeConfig {
            backend: flow_like_ml_core::ComputeBackend::Cpu,
            ..Default::default()
        };
        let mut budget = ExperimentBudget::default();
        budget.maximum_trials = 2;
        let experiment = repo
            .create_experiment(
                flow_like_ml_runtime::experiment::ExperimentRequest {
                    stream,
                    snapshot_id: snapshot.id,
                    spec: json!(prepared.spec),
                    candidates: legacy_candidates(&prepared.spec, &compute, true).unwrap(),
                    goals,
                    budget: budget.clone(),
                    source_table_versions: vec![],
                    created_tables: vec![],
                    updated_table_versions: vec![],
                    preprocessing_manifest: json!(prepared.preprocessing),
                    context: json!({}),
                },
                flow_like_ml_runtime::now_ms(),
            )
            .unwrap();
        controller(repo.clone(), &budget)
            .unwrap()
            .run(&experiment.id)
            .unwrap();
        let reopened = TrainingRepository::open(directory.join("training.sqlite")).unwrap();
        let consultation = consultation_context(&reopened, &experiment.id).unwrap();
        assert!(consultation.get("final_test_metrics").is_none());
        assert!(consultation.get("final_evaluation").is_none());
        assert!(consultation.get("samples").is_none());
        assert!(
            consultation["trials"]
                .as_array()
                .unwrap()
                .iter()
                .all(|trial| {
                    trial.get("validation_metrics").is_some()
                        && trial.get("weights").is_none()
                        && trial.get("test").is_none()
                })
        );
        assert_eq!(
            consultation["registered_datasets"][0]["input_shape"],
            json!([1])
        );
        assert_eq!(
            consultation["registered_datasets"][0]["numeric_columns"],
            json!(["measurement"])
        );
        let result: AutoTrainingResult = reopened.experiment_result(&experiment.id).unwrap().into();
        assert!(result.goal_met, "{:?}", result.unmet_constraints);
        assert_eq!(result.status, ExperimentStatus::Completed);
        assert!(result.final_test_metrics["mean_rank_error"] <= 0.5);
        assert_eq!(result.preprocessing_manifest, json!(prepared.preprocessing));
        let artifact = result.model_bundle_ref.unwrap();
        assert_eq!(
            artifact.manifest["preprocessing_manifest"],
            json!(prepared.preprocessing)
        );
        let mut predictor = super::super::auto_training_legacy::CatalogSearchPredictorFactory
            .load(&reopened, &artifact.id, &compute)
            .unwrap();
        let mut input =
            transform_table_rows(&prepared.preprocessing, &[json!({"measurement":20.2})], 16)
                .unwrap()
                .remove(0);
        input.shape.insert(0, 1);
        assert_eq!(predictor.predict(&input).unwrap()["classes"][0], 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

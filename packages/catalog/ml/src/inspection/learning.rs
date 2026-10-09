use super::auto_training::{AutoTrainRequest, AutoTrainingResult};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_ml_runtime::learning::*;
#[cfg(any(feature = "training", test))]
use flow_like_types::json::json;
use flow_like_types::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "training")]
#[path = "learning_inference.rs"]
mod inference;

#[cfg(all(test, feature = "training"))]
#[path = "learning_tests.rs"]
mod tests;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateLearningProjectRequest {
    pub training: AutoTrainRequest,
    #[serde(default)]
    pub policy: LearningPolicy,
    #[serde(default)]
    pub budget: LearningBudget,
    /// Promote eligible candidates after the configured independent audit.
    #[serde(default)]
    pub automatic_promotion: bool,
    /// Advance temporal boundaries at each cycle while preserving sealed audit exclusions.
    #[serde(default)]
    pub rolling_time_split: Option<RollingTimeSplit>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollingTimeSplit {
    pub validation_duration_ms: u64,
    pub test_duration_ms: u64,
}

#[cfg(feature = "training")]
impl RollingTimeSplit {
    fn apply(
        &self,
        split: &mut super::auto_training_tables::TableSplitPolicy,
        at_ms: i64,
    ) -> Result<()> {
        use super::auto_training_tables::TableSplitPolicy;
        let TableSplitPolicy::Time {
            train_end_ms,
            validation_end_ms,
            ..
        } = split
        else {
            return Err(anyhow!(
                "Rolling time splits require a temporal source split"
            ));
        };
        let validation = i64::try_from(self.validation_duration_ms)
            .map_err(|_| anyhow!("Validation duration exceeds the supported range"))?;
        let test = i64::try_from(self.test_duration_ms)
            .map_err(|_| anyhow!("Test duration exceeds the supported range"))?;
        if validation <= 0 || test <= 0 {
            return Err(anyhow!(
                "Rolling validation and test durations must be positive"
            ));
        }
        let validation_end = at_ms
            .checked_sub(test)
            .ok_or_else(|| anyhow!("Rolling validation boundary overflow"))?;
        let train_end = validation_end
            .checked_sub(validation)
            .ok_or_else(|| anyhow!("Rolling training boundary overflow"))?;
        *train_end_ms = train_end;
        *validation_end_ms = validation_end;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningProjectId {
    pub project_id: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningObservationsRequest {
    pub project_id: String,
    pub observations: Vec<LearningObservation>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningReviewRequest {
    pub project_id: String,
    pub review: LearningReview,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningProductChangeRequest {
    pub project_id: String,
    pub change_id: String,
    pub description: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningReviewSelectionRequest {
    pub project_id: String,
    pub selection_key: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningRouteRequest {
    pub project_id: String,
    pub sample_id: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LearningProjectResult {
    pub project: LearningProject,
    pub next_actions: LearningNextActions,
    pub cycles: Vec<LearningCycle>,
    pub experiment: Option<AutoTrainingResult>,
    pub comparison: Option<LearningComparison>,
}

#[cfg(feature = "training")]
pub fn project_repository(
    context: &ExecutionContext,
    id: &str,
) -> Result<flow_like_ml_runtime::TrainingRepository> {
    let (repo, scope) = super::lifecycle::repository(context)?;
    if repo.get_learning_project(id)?.request.stream.project_id != scope {
        return Err(anyhow!(
            "Learning project belongs to a different app or board"
        ));
    }
    Ok(repo)
}

#[cfg(feature = "training")]
pub fn training_template(project: &LearningProject) -> Result<AutoTrainRequest> {
    Ok(flow_like_types::json::from_value(
        project.request.source["training"].clone(),
    )?)
}

#[cfg(feature = "training")]
fn result(
    repo: &flow_like_ml_runtime::TrainingRepository,
    id: &str,
) -> Result<LearningProjectResult> {
    let project = repo.get_learning_project(id)?;
    let cycles = repo.learning_cycles(id)?;
    let latest = project
        .active_cycle_id
        .as_ref()
        .and_then(|cycle_id| cycles.iter().find(|cycle| &cycle.id == cycle_id))
        .or_else(|| cycles.last());
    let experiment = latest
        .and_then(|cycle| cycle.experiment_id.as_ref())
        .map(|id| repo.experiment_result(id).map(AutoTrainingResult::from))
        .transpose()?;
    let comparison = latest
        .map(|cycle| match repo.get_learning_comparison(&cycle.id) {
            Ok(value) => Ok(Some(value)),
            Err(flow_like_ml_runtime::Error::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        })
        .transpose()?
        .flatten();
    Ok(LearningProjectResult {
        project,
        next_actions: repo.learning_next_actions(id, flow_like_ml_runtime::now_ms())?,
        cycles,
        experiment,
        comparison,
    })
}

#[cfg(feature = "training")]
async fn create(
    context: &mut ExecutionContext,
    mut request: CreateLearningProjectRequest,
) -> Result<LearningProjectResult> {
    let (repo, scope) = super::lifecycle::repository(context)?;
    if let Some(rolling) = &request.rolling_time_split {
        rolling.apply(
            &mut request.training.dataset.split,
            flow_like_ml_runtime::now_ms(),
        )?;
    }
    let spec = &request.training.dataset.spec;
    let mut goals = request
        .training
        .goals
        .clone()
        .unwrap_or_else(|| flow_like_ml_runtime::auto_training::default_goals(spec));
    if request.training.ordered_target && request.training.goals.is_none() {
        goals.primary_metric = "mean_rank_error".into();
        goals.direction = flow_like_ml_runtime::MetricDirection::Minimize;
    }
    goals.minimum_audited_samples = goals
        .minimum_audited_samples
        .max(request.policy.minimum_audit_samples);
    if request.automatic_promotion && goals.bounds.is_empty() {
        return Err(anyhow!(
            "Automatic promotion requires explicit quality bounds"
        ));
    }
    let stream = super::lifecycle::InspectionContext {
        stream_id: format!("learning:{}", uuid::Uuid::new_v4()),
        spec: spec.clone(),
    }
    .key(scope)?;
    let pinned =
        super::auto_training_tables::pin_table_source(context, &request.training.dataset.source)
            .await?;
    let pinned_feature_sources =
        super::feature_engineering::pin_feature_sources(context, &request.training.feature_sources)
            .await?;
    let project = repo.create_learning_project(LearningProjectRequest {
        deployment_id: format!("deployment:{}",stream.stream_id), stream,
        spec:json!(spec), source:json!({"training":request.training,"pinned_source":pinned,"pinned_feature_sources":pinned_feature_sources,"automatic_promotion":request.automatic_promotion,"rolling_time_split":request.rolling_time_split}),
        goals, policy:request.policy, budget:request.budget,
    }, flow_like_ml_runtime::now_ms())?;
    result(&repo, &project.id)
}

#[cfg(feature = "training")]
async fn get(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    result(
        &project_repository(context, &request.project_id)?,
        &request.project_id,
    )
}

#[cfg(feature = "training")]
async fn observe(
    context: &mut ExecutionContext,
    request: LearningObservationsRequest,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    if request.observations.len() > 4096 {
        return Err(anyhow!(
            "An observation batch may contain at most 4096 rows"
        ));
    }
    for observation in request.observations {
        repo.record_learning_observation(&request.project_id, &observation)?;
    }
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
async fn review(
    context: &mut ExecutionContext,
    request: LearningReviewRequest,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    repo.record_learning_review(&request.project_id, &request.review)?;
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
async fn select_reviews(
    context: &mut ExecutionContext,
    request: LearningReviewSelectionRequest,
) -> Result<Vec<LearningReviewItem>> {
    Ok(
        project_repository(context, &request.project_id)?.select_learning_reviews(
            &request.project_id,
            &request.selection_key,
            flow_like_ml_runtime::now_ms(),
        )?,
    )
}

#[cfg(feature = "training")]
async fn errors(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningErrorAnalysis> {
    Ok(project_repository(context, &request.project_id)?
        .learning_error_analysis(&request.project_id)?)
}

#[cfg(feature = "training")]
async fn changed(
    context: &mut ExecutionContext,
    request: LearningProductChangeRequest,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    repo.record_learning_product_change(
        &request.project_id,
        &request.change_id,
        &request.description,
        flow_like_ml_runtime::now_ms(),
    )?;
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
async fn pause(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    let project = repo.pause_learning_project(
        &request.project_id,
        repo.get_learning_project(&request.project_id)?.generation,
        flow_like_ml_runtime::now_ms(),
    )?;
    if let Some(cycle_id) = project.active_cycle_id {
        if let Some(experiment_id) = repo.get_learning_cycle(&cycle_id)?.experiment_id {
            repo.cancel_experiment(&experiment_id, flow_like_ml_runtime::now_ms())?;
        }
    }
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
async fn resume(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    repo.resume_learning_project(
        &request.project_id,
        repo.get_learning_project(&request.project_id)?.generation,
        flow_like_ml_runtime::now_ms(),
    )?;
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
async fn route(
    context: &mut ExecutionContext,
    request: LearningRouteRequest,
) -> Result<LearningRoute> {
    Ok(project_repository(context, &request.project_id)?
        .learning_route(&request.project_id, &request.sample_id)?)
}

#[cfg(feature = "training")]
async fn rollback(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    repo.rollback_learning_project(
        &request.project_id,
        deployment_generation(&repo, &repo.get_learning_project(&request.project_id)?)?,
        flow_like_ml_runtime::now_ms(),
    )?;
    result(&repo, &request.project_id)
}

#[cfg(feature = "training")]
fn deployment_generation(
    repo: &flow_like_ml_runtime::TrainingRepository,
    project: &LearningProject,
) -> Result<i64> {
    match repo.get_deployment(&project.request.deployment_id) {
        Ok(value) => Ok(value.generation),
        Err(flow_like_ml_runtime::Error::NotFound(_)) => Ok(0),
        Err(error) => Err(error.into()),
    }
}

/// Prepare at most one experiment. A workflow timer can call this again after new evidence arrives.
#[cfg(feature = "training")]
pub async fn prepare_learning_cycle(
    context: &mut ExecutionContext,
    id: &str,
) -> Result<LearningProjectResult> {
    use super::auto_training_tables::{PinnedTableSource, TableLabelOverride};
    use flow_like_ml_runtime::{LabelSource, now_ms};
    let repo = project_repository(context, id)?;
    let project = repo.get_learning_project(id)?;
    if matches!(
        project.state,
        LearningState::Paused
            | LearningState::Shadow
            | LearningState::Canary
            | LearningState::BudgetExhausted
    ) {
        return result(&repo, id);
    }
    let mut template = training_template(&project)?;
    let cycle = if let Some(cycle_id) = &project.active_cycle_id {
        repo.get_learning_cycle(cycle_id)?
    } else {
        if !repo.learning_next_actions(id, now_ms())?.ready_to_train {
            return result(&repo, id);
        }
        repo.reserve_learning_cycle(
            id,
            &format!("cycle:{}", project.usage.cycles_started),
            template.budget.clone(),
            now_ms(),
        )?
    };
    if cycle.experiment_id.is_some() {
        return result(&repo, id);
    }
    if let Some(experiment) = repo.find_learning_cycle_experiment(&cycle.id)? {
        repo.attach_learning_experiment(&cycle.id, &experiment.id, now_ms())?;
        return result(&repo, id);
    }
    let prepared: Result<String> = async {
        let pinned: PinnedTableSource =
            flow_like_types::json::from_value(project.request.source["pinned_source"].clone())?;
        template.dataset.source =
            super::auto_training_tables::reopen_latest_source(context, &pinned).await?;
        let sides: Vec<super::feature_engineering::PinnedFeatureSource> =
            flow_like_types::json::from_value(
                project
                    .request
                    .source
                    .get("pinned_feature_sources")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            )?;
        template.feature_sources.clear();
        for side in sides {
            template
                .feature_sources
                .push(super::feature_engineering::FeatureSourceInput {
                    alias: side.alias,
                    allowed_columns: side.allowed_columns,
                    source: super::auto_training_tables::reopen_latest_source(
                        context,
                        &side.source,
                    )
                    .await?,
                });
        }
        let exclusions = repo.learning_dataset_exclusions(id)?;
        merge_dataset_exclusions(&mut template.dataset.selection, exclusions);
        if let Some(value) = project
            .request
            .source
            .get("rolling_time_split")
            .filter(|value| !value.is_null())
        {
            let rolling: RollingTimeSplit = flow_like_types::json::from_value(value.clone())?;
            rolling.apply(&mut template.dataset.split, cycle.created_at_ms)?;
        }
        for review in repo.learning_training_reviews_at(id, now_ms())? {
            let provenance = match review.source {
                LabelSource::Reviewed => flow_like_ml_core::LabelProvenance::Reviewed {
                    reviewer: review.reviewer,
                    reviewed_at_ms: review.available_at_ms,
                },
                LabelSource::ObservedOutcome => flow_like_ml_core::LabelProvenance::Measured {
                    source: review.reviewer,
                },
                LabelSource::Teacher => {
                    return Err(anyhow!(
                        "Teacher labels cannot enter a reviewed outcome overlay"
                    ));
                }
            };
            template.dataset.label_overrides.insert(
                review.sample_id,
                TableLabelOverride {
                    annotation: flow_like_types::json::from_value(review.annotation)?,
                    provenance,
                    annotation_revision: i64::try_from(
                        review
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| anyhow!("Review revision overflow"))?,
                    )
                    .map_err(|_| anyhow!("Review revision exceeds the supported range"))?,
                    available_at_ms: Some(review.available_at_ms),
                    outcome: review
                        .outcome
                        .map(flow_like_types::json::from_value)
                        .transpose()?,
                },
            );
        }
        template.goals = Some(project.request.goals.clone());
        template.budget = cycle.budget.clone();
        template.learning_cycle_id = Some(cycle.id.clone());
        let experiment = super::auto_training::start_auto_training_scoped(
            context,
            template,
            false,
            Some(project.request.stream.clone()),
        )
        .await?;
        repo.attach_learning_experiment(&cycle.id, &experiment.experiment_id, now_ms())?;
        Ok(experiment.experiment_id)
    }
    .await;
    if let Err(error) = prepared {
        // A saved experiment keeps its reservation and can be attached after a restart.
        if repo.find_learning_cycle_experiment(&cycle.id)?.is_none() {
            repo.abort_learning_cycle(&cycle.id, &error.to_string(), now_ms())?;
        }
        return Err(error);
    }
    result(&repo, id)
}

#[cfg(feature = "training")]
fn merge_dataset_exclusions(
    selection: &mut super::auto_training_tables::DatasetSelection,
    exclusions: LearningDatasetExclusions,
) {
    for (existing, additional) in [
        (
            &mut selection.excluded_ids,
            exclusions.sealed_audit_sample_ids,
        ),
        (
            &mut selection.excluded_groups,
            exclusions.sealed_audit_group_ids,
        ),
        (
            &mut selection.ineligible_test_ids,
            exclusions.previously_trained_sample_ids,
        ),
        (
            &mut selection.ineligible_test_groups,
            exclusions.previously_trained_group_ids,
        ),
    ] {
        existing.extend(additional);
        existing.sort();
        existing.dedup();
    }
}

#[cfg(feature = "training")]
fn audit_champion(
    repo: &flow_like_ml_runtime::TrainingRepository,
    project: &LearningProject,
    cycle: &LearningCycle,
) -> Result<()> {
    use flow_like_ml_runtime::{
        ExperimentResult,
        auto_training::{SearchPredictorFactory, prediction_record_from_output},
    };
    let Some(artifact_id) = &project.champion_artifact_id else {
        return Ok(());
    };
    if cycle.artifact_id.as_ref() == Some(artifact_id) {
        return Ok(());
    }
    let experiment_id = cycle
        .experiment_id
        .as_ref()
        .ok_or_else(|| anyhow!("Learning cycle has no experiment"))?;
    let state: ExperimentResult = repo.experiment_result(experiment_id)?;
    let selected = state
        .experiment
        .selected_trial_id
        .as_ref()
        .ok_or_else(|| anyhow!("A candidate must be frozen before comparing its champion"))?;
    let trial = state
        .trials
        .iter()
        .find(|trial| &trial.id == selected)
        .ok_or_else(|| anyhow!("Selected learning trial is missing"))?;
    let snapshot = repo.get_snapshot_limited(
        &trial.dataset_snapshot_id,
        state.experiment.request.budget.maximum_dataset_bytes,
    )?;
    let artifact = repo.get_artifact(artifact_id)?;
    let preprocessing: super::auto_training_tables::TablePreprocessing =
        flow_like_types::json::from_value(artifact.manifest["preprocessing_manifest"].clone())?;
    let mut spec: flow_like_ml_core::InspectionSpec =
        flow_like_types::json::from_value(artifact.manifest["inspection_spec"].clone())?;
    let rows = snapshot
        .test
        .iter()
        .map(|sample| {
            sample.payload.get("feature_row").cloned().ok_or_else(|| {
                anyhow!(
                    "This dataset predates raw-feature audit replay; prepare a new learning cycle"
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let inputs = super::auto_training_tables::transform_table_rows(
        &preprocessing,
        &rows,
        training_template(project)?
            .dataset
            .budget
            .maximum_tensor_elements,
    )?;
    let compute = training_template(project)?.compute;
    let mut predictor = super::auto_training_legacy::CatalogSearchPredictorFactory.load(
        repo,
        artifact_id,
        &compute,
    )?;
    let mut anomaly_scale = 1.;
    for previous in repo.learning_cycles(&project.id)? {
        if previous.artifact_id.as_ref() != Some(artifact_id) {
            continue;
        }
        if let Some(id) = previous.experiment_id {
            if let Some(trial) = repo
                .experiment_result(&id)?
                .trials
                .iter()
                .find(|trial| trial.artifact_id.as_ref() == Some(artifact_id))
            {
                anomaly_scale = trial
                    .validation_metrics
                    .get("anomaly_scale")
                    .copied()
                    .unwrap_or(1.);
            }
        }
    }
    let recorded: std::collections::HashSet<_> = repo
        .predictions(artifact_id)?
        .into_iter()
        .map(|prediction| prediction.sample_id)
        .collect();
    for (row, input) in snapshot.test.iter().zip(inputs) {
        if recorded.contains(&row.id) {
            continue;
        }
        let mut prepared = row.clone();
        spec.input_shape = input.shape.clone();
        let sample_payload = if prepared.payload.get("sample").is_some() {
            &mut prepared.payload["sample"]
        } else {
            &mut prepared.payload
        };
        sample_payload["input"] = json!(input);
        let mut batched = input;
        batched.shape.insert(0, 1);
        let started = std::time::Instant::now();
        let output = predictor.predict(&batched)?;
        let record = prediction_record_from_output(
            &prepared,
            &spec,
            &project.request.goals.task,
            artifact_id,
            &output,
            anomaly_scale,
            started.elapsed().as_secs_f64() * 1000.,
        )?;
        repo.record_prediction(&record)?;
    }
    Ok(())
}

#[cfg(feature = "training")]
pub async fn finish_learning_cycle(
    context: &mut ExecutionContext,
    id: &str,
) -> Result<LearningProjectResult> {
    use flow_like_ml_runtime::{ExperimentStatus, now_ms};
    let repo = project_repository(context, id)?;
    let project = repo.get_learning_project(id)?;
    if project.state == LearningState::Paused {
        return result(&repo, id);
    }
    let cycles = repo.learning_cycles(id)?;
    let Some(mut cycle) = project
        .active_cycle_id
        .as_ref()
        .and_then(|cycle_id| cycles.iter().find(|cycle| &cycle.id == cycle_id))
        .or_else(|| cycles.last())
        .cloned()
    else {
        return result(&repo, id);
    };
    let Some(experiment_id) = &cycle.experiment_id else {
        return result(&repo, id);
    };
    let experiment = repo.get_experiment(experiment_id)?;
    if matches!(
        experiment.status,
        ExperimentStatus::Running
            | ExperimentStatus::Evaluating
            | ExperimentStatus::CancelRequested
    ) {
        return result(&repo, id);
    }
    if cycle.state == LearningCycleState::Attached {
        cycle = repo.settle_learning_cycle(&cycle.id, now_ms())?;
    }
    if cycle.evaluation_id.is_none() {
        return result(&repo, id);
    }
    let fresh = repo.get_learning_project(id)?;
    if fresh.candidate_artifact_id != cycle.artifact_id {
        return result(&repo, id);
    }
    let work_repo = repo.clone();
    let work_project = fresh.clone();
    let work_cycle = cycle.clone();
    tokio::task::spawn_blocking(move || audit_champion(&work_repo, &work_project, &work_cycle))
        .await??;
    let comparison = repo.compare_learning_cycle(&cycle.id, now_ms())?;
    if comparison.eligible && fresh.state == LearningState::Canary {
        let work_repo = repo.clone();
        let work_project = fresh.clone();
        tokio::task::spawn_blocking(move || inference::audit_canary(&work_repo, &work_project))
            .await??;
        if repo.get_learning_project(id)?.candidate_artifact_id != cycle.artifact_id {
            return result(&repo, id);
        }
    }
    let can_promote =
        fresh.state != LearningState::Canary || repo.learning_canary_ready(id, now_ms())?;
    if comparison.eligible
        && can_promote
        && fresh.request.source["automatic_promotion"].as_bool() == Some(true)
    {
        if let Err(error) =
            repo.promote_learning_cycle(&cycle.id, deployment_generation(&repo, &fresh)?, now_ms())
        {
            let current = repo.get_learning_project(id)?;
            let audited = repo.get_learning_comparison(&cycle.id)?;
            if current.candidate_artifact_id.is_some() || audited.eligible {
                return Err(error.into());
            }
            // A failed canary gate is a recorded decision; the champion continues serving.
            return result(&repo, id);
        }
    }
    let mut output = result(&repo, id)?;
    if comparison.eligible && !can_promote {
        output
            .next_actions
            .blocking
            .push("canary_requires_reviewed_traffic_and_both_model_predictions".into());
    }
    Ok(output)
}

#[cfg(feature = "training")]
async fn step(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    use flow_like_ml_runtime::{ExperimentStatus, now_ms};
    let prepared = prepare_learning_cycle(context, &request.project_id).await?;
    if prepared.project.state == LearningState::Paused {
        return Ok(prepared);
    }
    if let Some(experiment) = prepared.experiment {
        if matches!(
            experiment.status,
            ExperimentStatus::Running
                | ExperimentStatus::Evaluating
                | ExperimentStatus::Cancelled
                | ExperimentStatus::CancelRequested
                | ExperimentStatus::BudgetExhausted
        ) && prepared.project.active_cycle_id.is_some()
        {
            let repo = project_repository(context, &request.project_id)?;
            super::auto_training::resume_saved_experiment(&repo, &experiment.experiment_id)?;
            let budget = repo
                .get_experiment(&experiment.experiment_id)?
                .request
                .budget;
            let controller = super::auto_training::controller(repo.clone(), &budget)?;
            let id = experiment.experiment_id.clone();
            let mut work = tokio::task::spawn_blocking(move || controller.run(&id));
            if let Some(token) = context.get_cancellation_token() {
                tokio::select! {done=&mut work=>{done??;},_ = token.cancelled()=>{repo.cancel_experiment(&experiment.experiment_id,now_ms())?;return Err(anyhow!("Learning cycle cancelled; its saved project can continue it later"));}}
            } else {
                work.await??;
            }
            super::auto_training::materialize_final_predictions(context, &experiment.experiment_id)
                .await?;
        }
    }
    finish_learning_cycle(context, &request.project_id).await
}

#[cfg(feature = "training")]
async fn promote(
    context: &mut ExecutionContext,
    request: LearningProjectId,
) -> Result<LearningProjectResult> {
    let repo = project_repository(context, &request.project_id)?;
    let project = repo.get_learning_project(&request.project_id)?;
    let cycles = repo.learning_cycles(&request.project_id)?;
    let cycle = cycles
        .iter()
        .rev()
        .find(|cycle| {
            cycle.artifact_id == project.candidate_artifact_id
                && cycle.state == LearningCycleState::Settled
        })
        .ok_or_else(|| anyhow!("Project has no settled candidate"))?;
    let work_repo = repo.clone();
    let work_project = project.clone();
    let work_cycle = cycle.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        audit_champion(&work_repo, &work_project, &work_cycle)?;
        inference::audit_canary(&work_repo, &work_project)
    })
    .await??;
    if repo
        .get_learning_project(&request.project_id)?
        .candidate_artifact_id
        != cycle.artifact_id
    {
        return result(&repo, &request.project_id);
    }
    repo.compare_learning_cycle(&cycle.id, flow_like_ml_runtime::now_ms())?;
    repo.promote_learning_cycle(
        &cycle.id,
        deployment_generation(&repo, &project)?,
        flow_like_ml_runtime::now_ms(),
    )?;
    result(&repo, &request.project_id)
}

macro_rules! learning_node {
    ($type:ident,$id:literal,$title:literal,$description:literal,$function:literal,$input:ty,$output:ty,$handler:ident) => {
        #[flow_like_types::async_trait]
        impl NodeLogic for $type {
            fn get_node(&self) -> Node {
                let mut node = super::operation_node::<$input, $output>(
                    $id,
                    $title,
                    $description,
                    $function,
                    "AI/ML/Continuous Learning",
                );
                node.set_long_running(true);
                node
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                #[cfg(feature = "training")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let request: $input = context.evaluate_pin("request").await?;
                    let output = $handler(context, request).await?;
                    context.set_pin_value("result", json!(output)).await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "training"))]
                {
                    let _ = context;
                    Err(anyhow!(
                        "Continuous learning requires a local training build"
                    ))
                }
            }
        }
    };
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateLearningProjectNode;
#[crate::register_node]
#[derive(Default)]
pub struct GetLearningProjectNode;
#[crate::register_node]
#[derive(Default)]
pub struct ObserveLearningSampleNode;
#[crate::register_node]
#[derive(Default)]
pub struct ReviewLearningSampleNode;
#[crate::register_node]
#[derive(Default)]
pub struct SelectSamplesForReviewNode;
#[crate::register_node]
#[derive(Default)]
pub struct AnalyzeModelErrorsNode;
#[crate::register_node]
#[derive(Default)]
pub struct NotifyLearningProductChangeNode;
#[crate::register_node]
#[derive(Default)]
pub struct StepLearningProjectNode;
#[crate::register_node]
#[derive(Default)]
pub struct PromoteLearningModelNode;
#[crate::register_node]
#[derive(Default)]
pub struct RollbackLearningModelNode;
#[crate::register_node]
#[derive(Default)]
pub struct PauseLearningProjectNode;
#[crate::register_node]
#[derive(Default)]
pub struct ResumeLearningProjectNode;
#[crate::register_node]
#[derive(Default)]
pub struct RouteLearningProjectNode;

learning_node!(
    CreateLearningProjectNode,
    "ml_create_learning_project",
    "Create Learning Project",
    "Persist a task, dataset source, review policy and budget across training experiments",
    "createLearningProject",
    CreateLearningProjectRequest,
    LearningProjectResult,
    create
);
learning_node!(
    GetLearningProjectNode,
    "ml_get_learning_project",
    "Get Learning Project",
    "Read project state, budget, experiment history and retraining reasons",
    "getLearningProject",
    LearningProjectId,
    LearningProjectResult,
    get
);
learning_node!(
    ObserveLearningSampleNode,
    "ml_observe_learning_sample",
    "Observe Learning Samples",
    "Record sample predictions, embeddings and operating conditions for review and drift detection",
    "observeLearningSamples",
    LearningObservationsRequest,
    LearningProjectResult,
    observe
);
learning_node!(
    ReviewLearningSampleNode,
    "ml_review_learning_sample",
    "Review Learning Sample",
    "Record a reviewed annotation or measured outcome for a collected sample",
    "reviewLearningSample",
    LearningReviewRequest,
    LearningProjectResult,
    review
);
learning_node!(
    SelectSamplesForReviewNode,
    "ml_select_samples_for_review",
    "Select Samples for Review",
    "Select uncertain, disagreeing, diverse and underrepresented samples",
    "selectSamplesForReview",
    LearningReviewSelectionRequest,
    Vec<LearningReviewItem>,
    select_reviews
);
learning_node!(
    AnalyzeModelErrorsNode,
    "ml_analyze_model_errors",
    "Analyze Model Errors",
    "Measure errors by class and declared operating slices using reviewed outcomes",
    "analyzeModelErrors",
    LearningProjectId,
    LearningErrorAnalysis,
    errors
);
learning_node!(
    NotifyLearningProductChangeNode,
    "ml_notify_learning_product_change",
    "Notify Learning Product Change",
    "Record an idempotent product change that can trigger another training cycle",
    "notifyLearningProductChange",
    LearningProductChangeRequest,
    LearningProjectResult,
    changed
);
learning_node!(
    StepLearningProjectNode,
    "ml_step_learning_project",
    "Step Learning Project",
    "Prepare or continue a training cycle, compare models on fresh audit evidence and apply the deployment policy",
    "stepLearningProject",
    LearningProjectId,
    LearningProjectResult,
    step
);
learning_node!(
    PromoteLearningModelNode,
    "ml_promote_learning_model",
    "Promote Learning Model",
    "Promote an eligible model using fresh audited evidence and the project quality policy",
    "promoteLearningModel",
    LearningProjectId,
    LearningProjectResult,
    promote
);
learning_node!(
    RollbackLearningModelNode,
    "ml_rollback_learning_model",
    "Rollback Learning Model",
    "Restore the previous model in the project deployment",
    "rollbackLearningModel",
    LearningProjectId,
    LearningProjectResult,
    rollback
);
learning_node!(
    PauseLearningProjectNode,
    "ml_pause_learning_project",
    "Pause Learning Project",
    "Pause project training and route decisions through its fallback",
    "pauseLearningProject",
    LearningProjectId,
    LearningProjectResult,
    pause
);
learning_node!(
    ResumeLearningProjectNode,
    "ml_resume_learning_project",
    "Resume Learning Project",
    "Resume the project with its saved budget, datasets and model state",
    "resumeLearningProject",
    LearningProjectId,
    LearningProjectResult,
    resume
);
learning_node!(
    RouteLearningProjectNode,
    "ml_route_learning_project",
    "Route Learning Project",
    "Resolve teacher, champion, shadow or deterministic canary routing for a sample",
    "routeLearningProject",
    LearningRouteRequest,
    LearningRoute,
    route
);

use crate::{EvaluationTask, MetricBound, StreamKey, TrainingRequest, WorkerLimits};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    Maximize,
    Minimize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AutoSearchConfig {
    pub seed: u64,
    pub maximum_candidates: usize,
    pub min_epochs: usize,
    pub max_epochs: usize,
    pub reduction_factor: usize,
    pub batch_size: usize,
    pub output_features: usize,
}

impl Default for AutoSearchConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            maximum_candidates: 8,
            min_epochs: 3,
            max_epochs: 30,
            reduction_factor: 3,
            batch_size: 16,
            output_features: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentGoals {
    pub task: EvaluationTask,
    pub primary_metric: String,
    pub direction: MetricDirection,
    pub minimum_audited_samples: usize,
    pub bounds: Vec<MetricBound>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExperimentBudget {
    pub maximum_trials: usize,
    pub maximum_parallel_trials: usize,
    pub maximum_wall_time_ms: u64,
    pub maximum_training_time_ms: u64,
    pub maximum_llm_tokens: u64,
    pub maximum_llm_cost_micros: u64,
    pub maximum_dataset_bytes: u64,
    pub maximum_artifact_bytes: u64,
    pub worker_limits: WorkerLimits,
}

impl Default for ExperimentBudget {
    fn default() -> Self {
        Self {
            maximum_trials: 12,
            maximum_parallel_trials: 1,
            maximum_wall_time_ms: 1_200_000,
            maximum_training_time_ms: 600_000,
            maximum_llm_tokens: 128_000,
            maximum_llm_cost_micros: 50_000_000,
            maximum_dataset_bytes: 256 * 1024 * 1024,
            maximum_artifact_bytes: 4 * 1024 * 1024 * 1024,
            worker_limits: WorkerLimits {
                maximum_duration_ms: 60_000,
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentRequest {
    pub stream: StreamKey,
    pub snapshot_id: String,
    /// Validated task contract supplied by the host, including the fixed label order.
    pub spec: Value,
    #[serde(default)]
    pub candidates: Vec<TrainingRequest>,
    pub goals: ExperimentGoals,
    #[serde(default)]
    pub budget: ExperimentBudget,
    #[serde(default)]
    pub source_table_versions: Vec<Value>,
    #[serde(default)]
    pub created_tables: Vec<Value>,
    #[serde(default)]
    pub updated_table_versions: Vec<Value>,
    #[serde(default)]
    pub preprocessing_manifest: Value,
    #[serde(default)]
    pub context: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Running,
    CancelRequested,
    Cancelled,
    Evaluating,
    Completed,
    BudgetExhausted,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentUsage {
    pub submitted_trials: usize,
    pub training_time_ms: u64,
    pub reserved_training_time_ms: u64,
    pub llm_tokens: u64,
    pub llm_cost_micros: u64,
    pub artifact_bytes: u64,
    pub reserved_artifact_bytes: u64,
    pub dataset_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Experiment {
    pub id: String,
    pub request: ExperimentRequest,
    pub status: ExperimentStatus,
    pub generation: i64,
    pub usage: ExperimentUsage,
    pub best_trial_id: Option<String>,
    /// Frozen before the final holdout is evaluated. No further trials can be added.
    pub selected_trial_id: Option<String>,
    #[serde(default)]
    pub selected_at_ms: Option<i64>,
    pub final_evaluation_id: Option<String>,
    pub target_met: bool,
    pub unmet_constraints: Vec<String>,
    pub stop_reason: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentTrialStatus {
    Scheduled,
    Validated,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentTrial {
    pub id: String,
    pub experiment_id: String,
    pub candidate_index: usize,
    pub idempotency_key: String,
    pub job_id: String,
    /// Registered transformed dataset, including the controller-only final holdout.
    pub dataset_snapshot_id: String,
    /// Worker view containing training and validation samples only.
    pub snapshot_id: String,
    pub artifact_id: Option<String>,
    pub status: ExperimentTrialStatus,
    pub validation_metrics: BTreeMap<String, f64>,
    pub training_time_ms: u64,
    pub reserved_training_time_ms: u64,
    pub reserved_artifact_bytes: u64,
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentDataset {
    pub snapshot_id: String,
    pub digest: String,
    pub training_snapshot_id: String,
    pub training_digest: String,
    pub preprocessing_manifest: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExperimentResult {
    pub experiment: Experiment,
    pub trials: Vec<ExperimentTrial>,
    pub datasets: Vec<ExperimentDataset>,
    pub best_artifact: Option<crate::ModelArtifact>,
    pub final_evaluation: Option<crate::EvaluationReport>,
    #[serde(default)]
    pub events: Vec<Value>,
}

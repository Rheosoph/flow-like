use crate::{ExperimentBudget, ExperimentGoals, ExperimentUsage, LabelSource, StreamKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningState {
    Collecting,
    Training,
    Shadow,
    Canary,
    Active,
    Paused,
    BudgetExhausted,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LearningPolicy {
    pub minimum_new_reviews: usize,
    pub minimum_audit_samples: usize,
    pub cooldown_ms: u64,
    pub review_batch_size: usize,
    pub review_pool_size: usize,
    pub uncertainty_weight: f64,
    pub disagreement_weight: f64,
    pub diversity_weight: f64,
    pub rare_class_quota: usize,
    pub drift_window: usize,
    pub embedding_shift_threshold: f64,
    pub maximum_observed_error: f64,
    pub minimum_improvement: f64,
    pub canary_fraction: f64,
    pub declared_slices: Vec<String>,
}
impl Default for LearningPolicy {
    fn default() -> Self {
        Self {
            minimum_new_reviews: 32,
            minimum_audit_samples: 20,
            cooldown_ms: 60_000,
            review_batch_size: 16,
            review_pool_size: 2048,
            uncertainty_weight: 1.,
            disagreement_weight: 1.,
            diversity_weight: 0.5,
            rare_class_quota: 1,
            drift_window: 64,
            embedding_shift_threshold: 1.,
            maximum_observed_error: 0.1,
            minimum_improvement: 0.,
            canary_fraction: 0.,
            declared_slices: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LearningBudget {
    pub maximum_cycles: usize,
    pub maximum_observations: usize,
    pub maximum_observation_bytes: u64,
    pub maximum_training_time_ms: u64,
    pub maximum_llm_tokens: u64,
    pub maximum_llm_cost_micros: u64,
    pub maximum_artifact_bytes: u64,
}
impl Default for LearningBudget {
    fn default() -> Self {
        Self {
            maximum_cycles: 100,
            maximum_observations: 100_000,
            maximum_observation_bytes: 64 * 1024 * 1024,
            maximum_training_time_ms: 36_000_000,
            maximum_llm_tokens: 1_000_000,
            maximum_llm_cost_micros: 100_000_000,
            maximum_artifact_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningProjectRequest {
    pub stream: StreamKey,
    pub spec: Value,
    pub source: Value,
    pub goals: ExperimentGoals,
    #[serde(default)]
    pub policy: LearningPolicy,
    #[serde(default)]
    pub budget: LearningBudget,
    pub deployment_id: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LearningUsage {
    #[serde(default)]
    pub observation_bytes: u64,
    pub cycles_started: usize,
    pub cycles_finished: usize,
    pub spent: ExperimentUsage,
    pub reserved: ExperimentUsage,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningProject {
    pub id: String,
    pub request: LearningProjectRequest,
    pub generation: i64,
    pub state: LearningState,
    pub paused_from: Option<LearningState>,
    pub usage: LearningUsage,
    pub active_cycle_id: Option<String>,
    pub champion_artifact_id: Option<String>,
    pub candidate_artifact_id: Option<String>,
    #[serde(default)]
    pub canary_started_at_ms: Option<i64>,
    pub review_watermark: i64,
    pub product_change_watermark: i64,
    pub last_cycle_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LearningPrediction {
    pub artifact_id: Option<String>,
    pub label: Option<String>,
    pub confidence: f64,
    #[serde(default)]
    pub probabilities: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LearningObservation {
    pub sample_id: String,
    pub group_id: String,
    pub captured_at_ms: i64,
    pub student: Option<LearningPrediction>,
    pub teacher: Option<LearningPrediction>,
    #[serde(default)]
    pub embedding: Vec<f64>,
    #[serde(default)]
    pub slices: BTreeMap<String, String>,
    #[serde(default)]
    pub sample: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LearningReview {
    pub sample_id: String,
    pub annotation: Value,
    pub source: LabelSource,
    pub reviewer: String,
    pub available_at_ms: i64,
    pub revision: u64,
    #[serde(default)]
    pub outcome: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningReviewItem {
    pub observation: LearningObservation,
    pub score: f64,
    pub reasons: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LearningErrorMetrics {
    pub samples: usize,
    pub errors: usize,
    pub accuracy: Option<f64>,
    pub confusion: BTreeMap<String, BTreeMap<String, usize>>,
    pub per_class_recall: BTreeMap<String, f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningErrorAnalysis {
    pub overall: LearningErrorMetrics,
    pub slices: BTreeMap<String, LearningErrorMetrics>,
    pub excluded_unreviewed: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningNextActions {
    pub project_id: String,
    pub state: LearningState,
    pub new_reviews: usize,
    pub ready_to_train: bool,
    pub reasons: Vec<String>,
    pub blocking: Vec<String>,
    pub embedding_shift: Option<f64>,
    pub observed_error: Option<f64>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningCycleState {
    Reserved,
    Attached,
    Settled,
    Aborted,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningCycle {
    pub id: String,
    pub project_id: String,
    pub key: String,
    pub state: LearningCycleState,
    pub budget: ExperimentBudget,
    pub experiment_id: Option<String>,
    pub artifact_id: Option<String>,
    pub evaluation_id: Option<String>,
    pub audit_sample_ids: Vec<String>,
    pub audit_group_ids: Vec<String>,
    #[serde(default)]
    pub canary_sample_ids: Vec<String>,
    pub reviewed_through: i64,
    #[serde(default)]
    pub observations_through: i64,
    #[serde(default)]
    pub reviewed_sample_ids: Vec<String>,
    #[serde(default)]
    pub reviewed_revisions: BTreeMap<String, u64>,
    pub product_change_through: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LearningDatasetExclusions {
    pub sealed_audit_sample_ids: Vec<String>,
    pub sealed_audit_group_ids: Vec<String>,
    pub previously_trained_sample_ids: Vec<String>,
    pub previously_trained_group_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LearningComparison {
    pub cycle_id: String,
    pub candidate_artifact_id: String,
    pub champion_artifact_id: Option<String>,
    pub candidate_evaluation_id: String,
    pub champion_evaluation_id: Option<String>,
    pub candidate_metrics: BTreeMap<String, f64>,
    pub champion_metrics: BTreeMap<String, f64>,
    pub eligible: bool,
    pub reasons: Vec<String>,
    pub evaluated_at_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearningRoute {
    Teacher,
    Champion {
        artifact_id: String,
    },
    Canary {
        artifact_id: String,
        champion_artifact_id: String,
    },
    Shadow {
        champion_artifact_id: Option<String>,
        candidate_artifact_id: String,
    },
}

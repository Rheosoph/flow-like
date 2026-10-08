//! Durable local training state. SQLite owns mutable coordination; immutable blobs hold data.
#[cfg(any(feature = "native", feature = "burn"))]
pub mod engines;
#[cfg(feature = "execution")]
mod repository;
#[cfg(feature = "execution")]
pub mod worker;
pub mod experiment;
pub use experiment::*;
pub mod learning;
pub use learning::*;
#[cfg(feature = "execution")]
mod learning_repository;
#[cfg(feature = "execution")]
mod experiment_repository;
#[cfg(all(feature = "native", feature = "burn"))]
pub mod auto_training;

#[cfg(feature = "execution")]
pub use repository::*;
#[cfg(feature = "native")]
mod evaluation;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "execution")]
    #[error("SQLite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid input: {0}")]
    Invalid(String),
    #[error("State conflict: {0}")]
    Conflict(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("The worker no longer owns a current lease")]
    LeaseLost,
    #[error("Shadow execution may write prediction telemetry only")]
    ShadowWrite,
    #[error("Training was cancelled")]
    Cancelled,
    #[error("Training backend: {0}")]
    Engine(String),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StreamKey {
    pub project_id: String,
    pub stream_id: String,
    pub inspection_version: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    Teacher,
    Reviewed,
    ObservedOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TrainingSample {
    pub id: String,
    pub annotation_revision: u64,
    pub group_id: String,
    pub captured_at_ms: i64,
    /// Labels cannot enter a snapshot before they were actually available.
    pub label_available_at_ms: i64,
    pub content_digest: String,
    pub source: LabelSource,
    pub accepted: bool,
    /// Asset/window reference, annotation, and teacher/prompt provenance.
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SampleReceipt {
    pub inserted: bool,
    pub accepted_sequence: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SplitPolicy {
    Group {
        train_fraction: f64,
        validation_fraction: f64,
        seed: u64,
    },
    /// Whole groups crossing a boundary or the embargo are excluded.
    Time {
        train_end_ms: i64,
        validation_end_ms: i64,
        embargo_ms: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DatasetSnapshot {
    pub id: String,
    pub digest: String,
    pub stream: StreamKey,
    pub as_of_ms: i64,
    pub cutoff_sequence: i64,
    pub policy: SplitPolicy,
    pub train: Vec<TrainingSample>,
    pub validation: Vec<TrainingSample>,
    pub test: Vec<TrainingSample>,
    pub excluded: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TrainingRequest {
    pub engine: String,
    pub recipe: Value,
    #[serde(default = "default_compute_request")]
    pub compute: Value,
}

fn default_compute_request() -> Value {
    serde_json::json!({})
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkerLimits {
    pub memory_budget_bytes: u64,
    pub maximum_artifact_bytes: u64,
    pub maximum_checkpoint_bytes: u64,
    pub maximum_duration_ms: u64,
    pub lease_duration_ms: i64,
}
impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            memory_budget_bytes: 2 * 1024 * 1024 * 1024,
            maximum_artifact_bytes: 1024 * 1024 * 1024,
            maximum_checkpoint_bytes: 2 * 1024 * 1024 * 1024,
            maximum_duration_ms: 60 * 60 * 1000,
            lease_duration_ms: 30_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    CancelRequested,
    Paused,
    Interrupted,
    Cancelled,
    Failed,
    Succeeded,
}
impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Failed | Self::Succeeded)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TrainingJob {
    pub id: String,
    pub stream: StreamKey,
    pub snapshot_id: String,
    pub request: TrainingRequest,
    pub status: JobStatus,
    pub generation: i64,
    pub lease_owner: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    pub checkpoint: Option<CheckpointRef>,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
    pub progress: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct JobLease {
    pub job_id: String,
    pub owner: String,
    pub generation: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ResourceRequest {
    pub key: String,
    pub estimated_bytes: u64,
    pub budget_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct BlobRef {
    pub sha256: String,
    pub bytes: u64,
    /// Repository-relative path; never an arbitrary filesystem path.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CheckpointRef {
    pub blob: BlobRef,
    pub step: u64,
    /// Engine version, optimizer/scheduler/RNG/sampler state contract.
    pub metadata: Value,
    pub request_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelArtifact {
    pub id: String,
    pub job_id: String,
    pub stream: StreamKey,
    pub snapshot_id: String,
    pub dataset_digest: String,
    pub blob: BlobRef,
    /// Native model manifest including architecture, labels and preprocessing.
    pub manifest: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PredictionRecord {
    pub sample_id: String,
    pub artifact_id: String,
    pub predicted_label: Option<String>,
    pub teacher_label: Option<String>,
    pub actual_label: Option<String>,
    pub actual_source: Option<LabelSource>,
    pub latency_ms: f64,
    pub failed: bool,
    pub recorded_at_ms: i64,
    pub details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct EvaluationReport {
    pub id: String,
    pub artifact_id: String,
    pub dataset_digest: String,
    pub evidence_digest: String,
    #[serde(default)]
    pub truth_digest: Option<String>,
    pub audited_samples: usize,
    pub teacher_samples: usize,
    pub metrics: BTreeMap<String, f64>,
    pub per_class_recall: BTreeMap<String, f64>,
    pub created_at_ms: i64,
    #[serde(default)]
    pub task: EvaluationTask,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluationTask {
    #[default]
    Classification,
    Regression,
    Anomaly {
        threshold: f64,
    },
    Detection {
        classes: usize,
        iou_threshold: f64,
    },
    Segmentation {
        classes: usize,
    },
    InstanceSegmentation {
        classes: usize,
        iou_threshold: f64,
    },
    Events {
        horizon_ms: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DetectionEvidence {
    pub class_id: u32,
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct InstanceEvidence {
    pub class_id: u32,
    pub confidence: f64,
    pub width: usize,
    pub height: usize,
    pub foreground: Vec<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidencePrediction {
    Anomaly {
        probability: f64,
    },
    Regression {
        values: Vec<f64>,
    },
    Detection {
        boxes: Vec<DetectionEvidence>,
    },
    Segmentation {
        width: usize,
        height: usize,
        classes: Vec<u32>,
    },
    Instances {
        instances: Vec<InstanceEvidence>,
    },
    Events {
        alarms_ms: Vec<i64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MetricBound {
    pub name: String,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MetricPromotionPolicy {
    pub minimum_audited_samples: usize,
    pub bounds: Vec<MetricBound>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PromotionPolicy {
    pub minimum_audited_samples: usize,
    pub minimum_accuracy: f64,
    pub minimum_class_recall: BTreeMap<String, f64>,
    pub maximum_mean_latency_ms: f64,
    pub maximum_failure_rate: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Deployment {
    pub id: String,
    pub stream: StreamKey,
    pub generation: i64,
    pub active_artifact_id: String,
    #[serde(default)]
    pub paused: bool,
    pub previous_artifact_id: Option<String>,
    pub evaluation_id: String,
    pub policy: Option<PromotionPolicy>,
    #[serde(default)]
    pub metric_policy: Option<MetricPromotionPolicy>,
    pub updated_at_ms: i64,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(feature = "execution")]
pub(crate) fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
#[cfg(feature = "execution")]
pub(crate) fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[cfg(feature = "execution")]
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

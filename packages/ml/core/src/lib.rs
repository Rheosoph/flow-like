use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

pub fn content_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TensorData {
    pub shape: Vec<usize>,
    pub values: Vec<f32>,
}

impl TensorData {
    pub fn validate(&self, max_elements: usize) -> Result<()> {
        require(
            !self.shape.is_empty() && self.shape.len() <= 8,
            "Tensor rank must be between 1 and 8",
        )?;
        let count = self.shape.iter().try_fold(1usize, |n, d| {
            require(*d > 0, "Tensor dimensions must be positive")?;
            n.checked_mul(*d)
                .ok_or_else(|| Error::Invalid("Tensor shape overflows usize".into()))
        })?;
        require(
            count <= max_elements,
            "Tensor exceeds the configured element limit",
        )?;
        require(
            count == self.values.len(),
            "Tensor shape does not match its values",
        )?;
        require(
            self.values.iter().all(|v| v.is_finite()),
            "Tensor contains non-finite values",
        )
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    ImageClassification,
    ObjectDetection,
    Segmentation,
    VisualAnomaly,
    SensorClassification,
    SensorRegression,
    SensorAnomaly,
    Fusion,
    SequenceForecast,
    SequenceAutoencoder,
    InstanceSegmentation,
    VisualSequenceClassification,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct InspectionSpec {
    pub id: String,
    pub description: String,
    pub task: TaskKind,
    pub labels: Vec<String>,
    pub input_shape: Vec<usize>,
    pub prediction_horizon_ms: Option<u64>,
    pub minimum_examples: usize,
    pub minimum_examples_per_class: usize,
}

impl InspectionSpec {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.id.trim().is_empty() && !self.description.trim().is_empty(),
            "Inspection id and description are required",
        )?;
        validate_labels(&self.labels)?;
        require(
            self.minimum_examples > 0,
            "Minimum examples must be positive",
        )?;
        require(
            !self.input_shape.is_empty() && self.input_shape.iter().all(|v| *v > 0),
            "Input shape must have positive dimensions",
        )?;
        if matches!(
            self.task,
            TaskKind::ImageClassification
                | TaskKind::SensorClassification
                | TaskKind::ObjectDetection
                | TaskKind::Segmentation
                | TaskKind::InstanceSegmentation
                | TaskKind::VisualSequenceClassification
        ) {
            require(!self.labels.is_empty(), "This task requires labels")?;
        }
        Ok(())
    }

    pub fn validate_sample(&self, sample: &Sample) -> Result<()> {
        self.validate()?;
        sample.validate(self.labels.len(), 64 * 1024 * 1024)?;
        require(
            sample.input.shape == self.input_shape,
            "Sample input shape differs from the inspection specification",
        )?;
        let compatible = match self.task {
            TaskKind::ImageClassification
            | TaskKind::SensorClassification
            | TaskKind::VisualSequenceClassification => {
                matches!(sample.annotation, Annotation::Class { .. })
            }
            TaskKind::Fusion => {
                if self.labels.is_empty() {
                    matches!(
                        sample.annotation,
                        Annotation::Scalar { .. } | Annotation::Values { .. }
                    )
                } else {
                    matches!(sample.annotation, Annotation::Class { .. })
                }
            }
            TaskKind::ObjectDetection => matches!(sample.annotation, Annotation::Boxes { .. }),
            TaskKind::Segmentation => matches!(sample.annotation, Annotation::Mask { .. }),
            TaskKind::InstanceSegmentation => {
                matches!(sample.annotation, Annotation::InstanceMasks { .. })
            }
            TaskKind::VisualAnomaly | TaskKind::SensorAnomaly | TaskKind::SequenceAutoencoder => {
                matches!(
                    sample.annotation,
                    Annotation::Anomaly { .. } | Annotation::Unlabeled
                )
            }
            TaskKind::SensorRegression => matches!(sample.annotation, Annotation::Scalar { .. }),
            TaskKind::SequenceForecast => matches!(
                sample.annotation,
                Annotation::Values { .. } | Annotation::Scalar { .. }
            ),
        };
        require(
            compatible,
            "Annotation type does not match the inspection task",
        )?;
        if self.task == TaskKind::SequenceForecast
            || self.prediction_horizon_ms.is_some_and(|h| h > 0)
        {
            let outcome = sample.outcome.as_ref().ok_or_else(|| Error::Invalid("Predictive targets require an observed outcome interval and availability timestamp".into()))?;
            require(
                outcome.target_start_ms > sample.window_end_ms,
                "Forecast target overlaps its feature window",
            )?;
            if let Some(horizon) = self.prediction_horizon_ms {
                let latest = sample.window_end_ms as i128 + horizon as i128;
                require(
                    outcome.target_end_ms as i128 <= latest,
                    "Outcome lies beyond the configured prediction horizon",
                )?;
            }
            require(
                matches!(
                    sample.provenance,
                    LabelProvenance::Measured { .. } | LabelProvenance::Reviewed { .. }
                ),
                "Future outcomes must be measured or reviewed rather than invented by a teacher",
            )?;
        }
        Ok(())
    }

    pub fn validate_sample_at(&self, sample: &Sample, observed_at_ms: i64) -> Result<()> {
        self.validate_sample(sample)?;
        require(
            sample.window_end_ms <= observed_at_ms,
            "The feature window has not finished yet",
        )?;
        if let LabelProvenance::Reviewed { reviewed_at_ms, .. } = sample.provenance {
            require(
                reviewed_at_ms <= observed_at_ms,
                "The reviewed label has not become available yet",
            )?;
        }
        if let Some(outcome) = &sample.outcome {
            require(
                outcome.available_at_ms <= observed_at_ms,
                "The target outcome has not become available yet",
            )?;
        }
        Ok(())
    }
}

pub fn validate_labels(labels: &[String]) -> Result<()> {
    let mut seen = HashSet::new();
    require(
        labels
            .iter()
            .all(|s| !s.trim().is_empty() && seen.insert(s)),
        "Labels must be nonempty and unique",
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BoundingBox {
    pub class_id: u32,
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

impl BoundingBox {
    pub fn validate(&self, classes: usize) -> Result<()> {
        require(
            (self.class_id as usize) < classes,
            "Bounding box class is outside the label schema",
        )?;
        require(
            [self.x_min, self.y_min, self.x_max, self.y_max]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "Bounding box coordinates must be finite and normalized",
        )?;
        require(
            self.x_min < self.x_max && self.y_min < self.y_max,
            "Bounding box must have positive area",
        )
    }

    pub fn horizontal_flip(&self) -> Self {
        Self {
            x_min: 1.0 - self.x_max,
            x_max: 1.0 - self.x_min,
            ..self.clone()
        }
    }

    pub fn iou(&self, other: &Self) -> f64 {
        let intersection = (self.x_max.min(other.x_max) - self.x_min.max(other.x_min)).max(0.0)
            * (self.y_max.min(other.y_max) - self.y_min.max(other.y_min)).max(0.0);
        let area = (self.x_max - self.x_min) * (self.y_max - self.y_min);
        let other_area = (other.x_max - other.x_min) * (other.y_max - other.y_min);
        let union = area + other_area - intersection;
        if union > 0.0 {
            (intersection / union) as f64
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Annotation {
    Unlabeled,
    Class {
        class_id: u32,
    },
    Scalar {
        value: f64,
    },
    Values {
        values: Vec<f32>,
    },
    Boxes {
        boxes: Vec<BoundingBox>,
    },
    Mask {
        width: usize,
        height: usize,
        classes: Vec<u32>,
    },
    Anomaly {
        is_anomaly: bool,
    },
    InstanceMasks {
        instances: Vec<InstanceMask>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct InstanceMask {
    pub instance_id: String,
    pub bounds: BoundingBox,
    pub width: usize,
    pub height: usize,
    pub foreground: Vec<bool>,
}

impl Annotation {
    pub fn validate(&self, label_count: usize, max_elements: usize) -> Result<()> {
        match self {
            Self::Unlabeled | Self::Anomaly { .. } => Ok(()),
            Self::Class { class_id } => require(
                (*class_id as usize) < label_count,
                "Class is outside the label schema",
            ),
            Self::Scalar { value } => require(value.is_finite(), "Scalar label must be finite"),
            Self::Values { values } => require(
                !values.is_empty()
                    && values.len() <= max_elements
                    && values.iter().all(|v| v.is_finite()),
                "Dense target must be nonempty, finite and within the element limit",
            ),
            Self::Boxes { boxes } => {
                require(boxes.len() <= max_elements, "Too many bounding boxes")?;
                boxes.iter().try_for_each(|b| b.validate(label_count))
            }
            Self::Mask {
                width,
                height,
                classes,
            } => {
                let area = width
                    .checked_mul(*height)
                    .ok_or_else(|| Error::Invalid("Mask shape overflows".into()))?;
                require(
                    *width > 0 && *height > 0 && area <= max_elements && area == classes.len(),
                    "Invalid mask shape or element limit",
                )?;
                require(
                    classes.iter().all(|c| (*c as usize) < label_count),
                    "Mask class is outside the label schema",
                )
            }
            Self::InstanceMasks { instances } => {
                let mut ids = HashSet::new();
                let mut total = 0usize;
                for instance in instances {
                    require(
                        !instance.instance_id.is_empty() && ids.insert(&instance.instance_id),
                        "Instance ids must be nonempty and unique",
                    )?;
                    instance.bounds.validate(label_count)?;
                    let area = instance
                        .width
                        .checked_mul(instance.height)
                        .ok_or_else(|| Error::Invalid("Instance mask shape overflows".into()))?;
                    total = total
                        .checked_add(area)
                        .ok_or_else(|| Error::Invalid("Instance mask size overflows".into()))?;
                    require(
                        instance.width > 0
                            && instance.height > 0
                            && area == instance.foreground.len()
                            && total <= max_elements,
                        "Invalid instance mask shape or element limit",
                    )?;
                }
                Ok(())
            }
        }
    }

    pub fn horizontal_flip(&self) -> Self {
        match self {
            Self::Boxes { boxes } => Self::Boxes {
                boxes: boxes.iter().map(BoundingBox::horizontal_flip).collect(),
            },
            Self::Mask {
                width,
                height,
                classes,
            } if *width > 0 => {
                let mut flipped = classes.clone();
                for row in flipped.chunks_mut(*width) {
                    row.reverse();
                }
                Self::Mask {
                    width: *width,
                    height: *height,
                    classes: flipped,
                }
            }
            Self::InstanceMasks { instances } => Self::InstanceMasks {
                instances: instances
                    .iter()
                    .map(|instance| {
                        let mut flipped = instance.clone();
                        flipped.bounds = instance.bounds.horizontal_flip();
                        if instance.width > 0 {
                            for row in flipped.foreground.chunks_mut(instance.width) {
                                row.reverse();
                            }
                        }
                        flipped
                    })
                    .collect(),
            },
            other => other.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabelProvenance {
    Unlabeled,
    Teacher {
        model: String,
        prompt_digest: String,
        confidence: Option<f64>,
    },
    Reviewed {
        reviewer: String,
        reviewed_at_ms: i64,
    },
    Measured {
        source: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Sample {
    pub id: String,
    pub group_id: String,
    pub stream_id: String,
    pub timestamp_ms: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub input: TensorData,
    pub annotation: Annotation,
    pub provenance: LabelProvenance,
    #[serde(default)]
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Outcome {
    pub available_at_ms: i64,
    pub target_start_ms: i64,
    pub target_end_ms: i64,
}

impl Sample {
    pub fn validate(&self, label_count: usize, max_elements: usize) -> Result<()> {
        require(
            !self.id.is_empty() && !self.group_id.is_empty() && !self.stream_id.is_empty(),
            "Sample, group and stream ids are required",
        )?;
        require(
            self.window_start_ms <= self.timestamp_ms && self.timestamp_ms <= self.window_end_ms,
            "Sample timestamp must be inside its window",
        )?;
        self.input.validate(max_elements)?;
        self.annotation.validate(label_count, max_elements)?;
        if let Some(outcome) = &self.outcome {
            require(
                outcome.target_start_ms <= outcome.target_end_ms
                    && outcome.available_at_ms >= outcome.target_end_ms,
                "Outcome availability must follow its target interval",
            )?;
        }
        if let LabelProvenance::Teacher {
            model,
            prompt_digest,
            confidence,
        } = &self.provenance
        {
            require(
                !model.is_empty() && !prompt_digest.is_empty(),
                "Teacher provenance requires model and prompt digest",
            )?;
            if let Some(c) = confidence {
                require(
                    c.is_finite() && (0.0..=1.0).contains(c),
                    "Teacher confidence must be in [0, 1]",
                )?;
            }
        }
        match &self.provenance {
            LabelProvenance::Reviewed {
                reviewer,
                reviewed_at_ms,
            } => {
                require(
                    !reviewer.trim().is_empty() && *reviewed_at_ms >= self.window_end_ms,
                    "A reviewed label needs a reviewer and a review time at or after its feature window ends",
                )?;
                if let Some(outcome) = &self.outcome {
                    require(
                        *reviewed_at_ms >= outcome.available_at_ms,
                        "An outcome must be available before its label can be reviewed",
                    )?;
                }
            }
            LabelProvenance::Measured { source } => require(
                !source.trim().is_empty(),
                "A measured label needs an observation source",
            )?,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DatasetSnapshot {
    pub format_version: u32,
    pub id: String,
    pub labels: Vec<String>,
    pub samples: Vec<Sample>,
    pub digest: String,
}

impl DatasetSnapshot {
    pub fn new(id: String, labels: Vec<String>, mut samples: Vec<Sample>) -> Result<Self> {
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        let mut snapshot = Self {
            format_version: 1,
            id,
            labels,
            samples,
            digest: String::new(),
        };
        snapshot.validate_structure(64 * 1024 * 1024)?;
        snapshot.digest = snapshot.compute_digest()?;
        Ok(snapshot)
    }

    pub fn compute_digest(&self) -> Result<String> {
        Ok(content_digest(&serde_json::to_vec(&(
            self.format_version,
            &self.labels,
            &self.samples,
        ))?))
    }

    fn validate_structure(&self, max_elements: usize) -> Result<()> {
        require(
            self.format_version == 1,
            "Unsupported dataset format version",
        )?;
        require(!self.id.is_empty(), "Dataset id is required")?;
        validate_labels(&self.labels)?;
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for sample in &self.samples {
            require(seen.insert(&sample.id), "Duplicate sample id")?;
            sample.validate(self.labels.len(), max_elements)?;
            let annotation_elements = match &sample.annotation {
                Annotation::Values { values } => values.len(),
                Annotation::Mask { classes, .. } => classes.len(),
                Annotation::InstanceMasks { instances } => instances
                    .iter()
                    .try_fold(0usize, |n, i| n.checked_add(i.foreground.len()))
                    .ok_or_else(|| Error::Invalid("Dataset annotation size overflows".into()))?,
                Annotation::Boxes { boxes } => boxes
                    .len()
                    .checked_mul(5)
                    .ok_or_else(|| Error::Invalid("Dataset box count overflows".into()))?,
                _ => 1,
            };
            total = total
                .checked_add(sample.input.values.len())
                .and_then(|n| n.checked_add(annotation_elements))
                .ok_or_else(|| Error::Invalid("Dataset element count overflows".into()))?;
            require(
                total <= max_elements,
                "Dataset exceeds the aggregate element limit",
            )?;
        }
        Ok(())
    }

    pub fn validate(&self, max_elements: usize) -> Result<()> {
        self.validate_structure(max_elements)?;
        require(
            self.digest == self.compute_digest()?,
            "Dataset content does not match its digest",
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComputeBackend {
    Cpu,
    Wgpu,
    Cuda,
    Rocm,
    #[default]
    Auto,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct ComputeConfig {
    pub backend: ComputeBackend,
    pub device_index: usize,
    pub allow_cpu_fallback: bool,
    pub memory_limit_bytes: u64,
}

impl Default for ComputeConfig {
    fn default() -> Self {
        Self {
            backend: ComputeBackend::Auto,
            device_index: 0,
            allow_cpu_fallback: true,
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

impl ComputeConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.memory_limit_bytes > 0,
            "Compute memory limit must be positive",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TrainingRecipe {
    pub id: String,
    pub architecture: String,
    pub task: TaskKind,
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub seed: u64,
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
}

impl TrainingRecipe {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.id.is_empty() && !self.architecture.is_empty(),
            "Recipe id and architecture are required",
        )?;
        require(
            self.epochs > 0 && self.batch_size > 0,
            "Epochs and batch size must be positive",
        )?;
        require(
            self.learning_rate.is_finite() && self.learning_rate > 0.0,
            "Learning rate must be finite and positive",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreprocessingStep {
    Standardize {
        mean: Vec<f64>,
        scale: Vec<f64>,
    },
    Resize {
        width: usize,
        height: usize,
    },
    Normalize {
        mean: Vec<f32>,
        std: Vec<f32>,
    },
    SensorWindow {
        samples: usize,
        stride: usize,
        sample_rate_hz: f64,
    },
    Features {
        names: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelArtifact {
    pub format_version: u32,
    pub id: String,
    pub task: TaskKind,
    pub architecture: String,
    pub architecture_version: u32,
    pub weights_uri: String,
    pub weights_sha256: String,
    pub dataset_digest: String,
    pub labels: Vec<String>,
    pub input_shape: Vec<usize>,
    pub preprocessing: Vec<PreprocessingStep>,
    pub metrics: BTreeMap<String, f64>,
    pub created_at_ms: i64,
}

impl ModelArtifact {
    pub fn validate(&self) -> Result<()> {
        require(
            self.format_version == 1 && self.architecture_version > 0,
            "Unsupported artifact version",
        )?;
        require(
            !self.id.is_empty() && !self.architecture.is_empty() && !self.weights_uri.is_empty(),
            "Artifact id, architecture and weights URI are required",
        )?;
        for digest in [&self.weights_sha256, &self.dataset_digest] {
            require(
                digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
                "Artifact digests must be SHA-256 hex strings",
            )?;
        }
        validate_labels(&self.labels)?;
        require(
            !self.input_shape.is_empty() && self.input_shape.iter().all(|v| *v > 0),
            "Artifact input shape is invalid",
        )?;
        require(
            self.metrics.values().all(|v| v.is_finite()),
            "Artifact metrics must be finite",
        )?;
        for step in &self.preprocessing {
            match step {
                PreprocessingStep::Standardize { mean, scale } => require(
                    !mean.is_empty()
                        && mean.len() == scale.len()
                        && mean.iter().all(|v| v.is_finite())
                        && scale.iter().all(|v| v.is_finite() && *v > 0.0),
                    "Invalid fitted standardization parameters",
                )?,
                PreprocessingStep::Resize { width, height } => require(
                    *width > 0 && *height > 0 && width.checked_mul(*height).is_some(),
                    "Invalid resize dimensions",
                )?,
                PreprocessingStep::Normalize { mean, std } => require(
                    !mean.is_empty()
                        && mean.len() == std.len()
                        && mean.iter().all(|v| v.is_finite())
                        && std.iter().all(|v| v.is_finite() && *v > 0.0),
                    "Invalid normalization parameters",
                )?,
                PreprocessingStep::SensorWindow {
                    samples,
                    stride,
                    sample_rate_hz,
                } => require(
                    *samples > 0
                        && *stride > 0
                        && sample_rate_hz.is_finite()
                        && *sample_rate_hz > 0.0,
                    "Invalid sensor window parameters",
                )?,
                PreprocessingStep::Features { names } => {
                    require(!names.is_empty(), "Feature names are required")?;
                    validate_labels(names)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_sample() -> Sample {
        Sample {
            id: "sample".into(),
            group_id: "lot".into(),
            stream_id: "press".into(),
            timestamp_ms: 15,
            window_start_ms: 10,
            window_end_ms: 20,
            input: TensorData {
                shape: vec![2],
                values: vec![1., 2.],
            },
            annotation: Annotation::Scalar { value: 3. },
            provenance: LabelProvenance::Measured {
                source: "press-sensor".into(),
            },
            outcome: None,
        }
    }

    fn contract_spec(task: TaskKind, labels: Vec<String>) -> InspectionSpec {
        InspectionSpec {
            id: "inspection".into(),
            description: "Inspect the recorded sample".into(),
            task,
            labels,
            input_shape: vec![2],
            prediction_horizon_ms: None,
            minimum_examples: 1,
            minimum_examples_per_class: 0,
        }
    }

    #[test]
    fn reviewed_provenance_requires_identity_and_a_completed_window() {
        let mut sample = contract_sample();
        for reviewer in ["", " \t\n"] {
            sample.provenance = LabelProvenance::Reviewed {
                reviewer: reviewer.into(),
                reviewed_at_ms: 20,
            };
            assert!(sample.validate(0, 10).is_err());
        }
        sample.provenance = LabelProvenance::Reviewed {
            reviewer: "operator-7".into(),
            reviewed_at_ms: 19,
        };
        assert!(sample.validate(0, 10).is_err());
        for reviewed_at_ms in [20, 21] {
            sample.provenance = LabelProvenance::Reviewed {
                reviewer: "operator-7".into(),
                reviewed_at_ms,
            };
            sample.validate(0, 10).unwrap();
        }
    }

    #[test]
    fn measured_provenance_requires_an_observation_source() {
        let mut sample = contract_sample();
        sample.validate(0, 10).unwrap();
        for source in ["", " \n\t"] {
            sample.provenance = LabelProvenance::Measured {
                source: source.into(),
            };
            assert!(sample.validate(0, 10).is_err());
        }
    }

    #[test]
    fn fusion_label_schema_selects_classification_or_regression() {
        let mut sample = contract_sample();
        let regression = contract_spec(TaskKind::Fusion, vec![]);
        regression.validate_sample(&sample).unwrap();
        sample.annotation = Annotation::Values {
            values: vec![3., 4.],
        };
        regression.validate_sample(&sample).unwrap();
        sample.annotation = Annotation::Class { class_id: 0 };
        assert!(regression.validate_sample(&sample).is_err());

        let classification = contract_spec(TaskKind::Fusion, vec!["good".into(), "defect".into()]);
        for class_id in [0, 1] {
            sample.annotation = Annotation::Class { class_id };
            classification.validate_sample(&sample).unwrap();
        }
        for annotation in [
            Annotation::Class { class_id: 2 },
            Annotation::Scalar { value: 3. },
            Annotation::Values {
                values: vec![3., 4.],
            },
        ] {
            sample.annotation = annotation;
            assert!(classification.validate_sample(&sample).is_err());
        }
    }

    #[test]
    fn class_and_localization_tasks_require_a_label_schema() {
        for task in [
            TaskKind::ImageClassification,
            TaskKind::SensorClassification,
            TaskKind::VisualSequenceClassification,
            TaskKind::ObjectDetection,
            TaskKind::Segmentation,
            TaskKind::InstanceSegmentation,
        ] {
            assert!(contract_spec(task, vec![]).validate().is_err(), "{task:?}");
            contract_spec(task, vec!["part".into()]).validate().unwrap();
        }
        for task in [
            TaskKind::Fusion,
            TaskKind::SensorRegression,
            TaskKind::SequenceForecast,
            TaskKind::VisualAnomaly,
            TaskKind::SensorAnomaly,
            TaskKind::SequenceAutoencoder,
        ] {
            contract_spec(task, vec![]).validate().unwrap();
        }
    }

    #[test]
    fn observed_time_excludes_unfinished_windows_and_future_reviews() {
        let spec = contract_spec(TaskKind::SensorRegression, vec![]);
        let mut sample = contract_sample();
        assert!(spec.validate_sample_at(&sample, 19).is_err());
        spec.validate_sample_at(&sample, 20).unwrap();
        sample.provenance = LabelProvenance::Reviewed {
            reviewer: "operator".into(),
            reviewed_at_ms: 25,
        };
        assert!(spec.validate_sample_at(&sample, 24).is_err());
        spec.validate_sample_at(&sample, 25).unwrap();
    }

    #[test]
    fn tensors_reject_shape_overflow_nan_and_limits() {
        assert!(
            TensorData {
                shape: vec![usize::MAX, 2],
                values: vec![]
            }
            .validate(100)
            .is_err()
        );
        assert!(
            TensorData {
                shape: vec![1],
                values: vec![f32::NAN]
            }
            .validate(100)
            .is_err()
        );
        assert!(
            TensorData {
                shape: vec![2],
                values: vec![0.0, 1.0]
            }
            .validate(1)
            .is_err()
        );
    }

    #[test]
    fn annotation_geometry_survives_flip() {
        let bbox = BoundingBox {
            class_id: 0,
            x_min: 0.1,
            y_min: 0.2,
            x_max: 0.3,
            y_max: 0.7,
        };
        let flipped = bbox.horizontal_flip();
        assert!((flipped.x_min - 0.7).abs() < 1e-6);
        assert!((flipped.x_max - 0.9).abs() < 1e-6);
        assert!((bbox.iou(&bbox) - 1.0).abs() < 1e-6);
        let mask = Annotation::Mask {
            width: 3,
            height: 2,
            classes: vec![0, 1, 2, 2, 1, 0],
        };
        assert_eq!(mask.horizontal_flip().horizontal_flip(), mask);
        assert!(bbox.validate(0).is_err());
    }

    #[test]
    fn snapshot_tampering_is_detected_and_order_is_canonical() {
        let sample = |id: &str| Sample {
            id: id.into(),
            group_id: "g".into(),
            stream_id: "s".into(),
            timestamp_ms: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            input: TensorData {
                shape: vec![1],
                values: vec![1.0],
            },
            annotation: Annotation::Class { class_id: 0 },
            provenance: LabelProvenance::Measured {
                source: "fixture".into(),
            },
            outcome: None,
        };
        let mut a = DatasetSnapshot::new(
            "a".into(),
            vec!["good".into()],
            vec![sample("b"), sample("a")],
        )
        .unwrap();
        let b = DatasetSnapshot::new(
            "b".into(),
            vec!["good".into()],
            vec![sample("a"), sample("b")],
        )
        .unwrap();
        assert_eq!(a.digest, b.digest);
        a.samples[0].input.values[0] = 2.0;
        assert!(a.validate(100).is_err());
    }

    #[test]
    fn forecast_labels_require_observed_disjoint_available_outcomes() {
        let spec = InspectionSpec {
            id: "future".into(),
            description: "Predict the next measurement".into(),
            task: TaskKind::SequenceForecast,
            labels: vec![],
            input_shape: vec![1],
            prediction_horizon_ms: Some(100),
            minimum_examples: 1,
            minimum_examples_per_class: 0,
        };
        let mut sample = Sample {
            id: "1".into(),
            group_id: "1".into(),
            stream_id: "sensor".into(),
            timestamp_ms: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            input: TensorData {
                shape: vec![1],
                values: vec![1.0],
            },
            annotation: Annotation::Values { values: vec![2.0] },
            provenance: LabelProvenance::Measured {
                source: "sensor".into(),
            },
            outcome: Some(Outcome {
                available_at_ms: 100,
                target_start_ms: 1,
                target_end_ms: 100,
            }),
        };
        spec.validate_sample_at(&sample, 100).unwrap();
        assert!(spec.validate_sample_at(&sample, 99).is_err());
        sample.provenance = LabelProvenance::Reviewed {
            reviewer: "operator".into(),
            reviewed_at_ms: 99,
        };
        assert!(spec.validate_sample_at(&sample, 100).is_err());
        sample.provenance = LabelProvenance::Reviewed {
            reviewer: "operator".into(),
            reviewed_at_ms: 100,
        };
        spec.validate_sample_at(&sample, 100).unwrap();
        sample.outcome.as_mut().unwrap().target_start_ms = 0;
        assert!(spec.validate_sample(&sample).is_err());
        sample.outcome.as_mut().unwrap().target_start_ms = 1;
        sample.provenance = LabelProvenance::Teacher {
            model: "teacher".into(),
            prompt_digest: "p".into(),
            confidence: None,
        };
        assert!(spec.validate_sample(&sample).is_err());
    }

    #[test]
    fn aggregate_dataset_limit_includes_targets() {
        let sample = Sample {
            id: "1".into(),
            group_id: "1".into(),
            stream_id: "sensor".into(),
            timestamp_ms: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            input: TensorData {
                shape: vec![2],
                values: vec![1.0, 2.0],
            },
            annotation: Annotation::Values {
                values: vec![2.0, 3.0],
            },
            provenance: LabelProvenance::Measured {
                source: "sensor".into(),
            },
            outcome: None,
        };
        let snapshot = DatasetSnapshot::new("test".into(), vec![], vec![sample]).unwrap();
        assert!(snapshot.validate(3).is_err());
        snapshot.validate(4).unwrap();
    }

    #[test]
    fn compute_defaults_to_auto_and_preserves_explicit_device_policy() {
        let automatic: ComputeConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(automatic, ComputeConfig::default());
        assert_eq!(ComputeBackend::default(), ComputeBackend::Auto);
        assert_eq!(automatic.backend, ComputeBackend::Auto);
        assert!(automatic.allow_cpu_fallback);
        automatic.validate().unwrap();

        let strict: ComputeConfig = serde_json::from_str(
            r#"{"backend":"cuda","device_index":2,"allow_cpu_fallback":false}"#,
        )
        .unwrap();
        assert_eq!(strict.backend, ComputeBackend::Cuda);
        assert_eq!(strict.device_index, 2);
        assert!(!strict.allow_cpu_fallback);
        assert_eq!(strict.memory_limit_bytes, automatic.memory_limit_bytes);
        assert_eq!(
            serde_json::from_value::<ComputeConfig>(serde_json::to_value(&strict).unwrap())
                .unwrap(),
            strict
        );
    }
}

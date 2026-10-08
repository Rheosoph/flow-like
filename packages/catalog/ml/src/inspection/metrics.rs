use super::operation;
use flow_like_ml_core::BoundingBox;
use flow_like_ml_native::{calibration::*, drift::*, evaluation::*};
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecisionThresholdRequest {
    pub score: f64,
    pub threshold: f64,
    pub higher_is_positive: bool,
}
fn decision_threshold(input: DecisionThresholdRequest) -> Result<bool> {
    flow_like_ml_core::require(
        input.score.is_finite() && input.threshold.is_finite(),
        "Decision score and threshold must be finite",
    )?;
    Ok(if input.higher_is_positive {
        input.score >= input.threshold
    } else {
        input.score <= input.threshold
    })
}
#[crate::register_node]
#[derive(Default)]
pub struct ApplyDecisionThresholdNode;
operation!(
    ApplyDecisionThresholdNode,
    "ml_apply_decision_threshold",
    "Apply Decision Threshold",
    "Compare a prediction score with an explicit inclusive threshold and direction",
    "threshold",
    "AI/ML/Inference",
    DecisionThresholdRequest,
    bool,
    decision_threshold
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfidenceRequest {
    pub probabilities: Vec<f64>,
    pub minimum_confidence: f64,
    pub maximum_entropy: f64,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfidenceDecision {
    pub class_id: usize,
    pub confidence: f64,
    pub entropy: f64,
    pub accepted: bool,
}
fn confidence(input: ConfidenceRequest) -> Result<ConfidenceDecision> {
    flow_like_ml_core::require(
        input.probabilities.len() >= 2
            && input
                .probabilities
                .iter()
                .all(|p| p.is_finite() && (0.0..=1.0).contains(p))
            && (input.probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-5,
        "Supply normalized class probabilities",
    )?;
    flow_like_ml_core::require(
        input.minimum_confidence.is_finite()
            && (0.0..=1.0).contains(&input.minimum_confidence)
            && input.maximum_entropy.is_finite()
            && input.maximum_entropy >= 0.0,
        "Confidence and entropy limits are invalid",
    )?;
    let (class_id, confidence) = input
        .probabilities
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let entropy = -input
        .probabilities
        .iter()
        .filter(|p| **p > 0.0)
        .map(|p| p * p.ln())
        .sum::<f64>();
    Ok(ConfidenceDecision {
        class_id,
        confidence,
        entropy,
        accepted: confidence >= input.minimum_confidence && entropy <= input.maximum_entropy,
    })
}
#[crate::register_node]
#[derive(Default)]
pub struct StudentConfidenceGateNode;
operation!(
    StudentConfidenceGateNode,
    "ml_student_confidence_gate",
    "Student Confidence Gate",
    "Check calibrated class confidence and entropy before accepting a prediction",
    "confidenceGate",
    "AI/ML/Inference",
    ConfidenceRequest,
    ConfidenceDecision,
    confidence
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClassMetricsRequest {
    pub actual: Vec<u32>,
    pub predicted: Vec<u32>,
    pub classes: usize,
}
fn classification(input: ClassMetricsRequest) -> Result<ClassificationMetrics> {
    Ok(classification_metrics(
        &input.actual,
        &input.predicted,
        input.classes,
    )?)
}
fn segmentation(input: ClassMetricsRequest) -> Result<SegmentationMetrics> {
    Ok(segmentation_metrics(
        &input.actual,
        &input.predicted,
        input.classes,
    )?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BinaryMetricsRequest {
    pub actual: Vec<bool>,
    pub scores: Vec<f64>,
}
fn binary(input: BinaryMetricsRequest) -> Result<BinaryCurve> {
    Ok(binary_curve(&input.actual, &input.scores)?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegressionMetricsRequest {
    pub actual: Vec<f64>,
    pub predicted: Vec<f64>,
}
fn regression(input: RegressionMetricsRequest) -> Result<RegressionMetrics> {
    Ok(regression_metrics(&input.actual, &input.predicted)?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DetectionMetricsRequest {
    pub actual: Vec<Vec<BoundingBox>>,
    pub predicted: Vec<Vec<ScoredBox>>,
    pub classes: usize,
    pub iou_threshold: f64,
}
fn detection(input: DetectionMetricsRequest) -> Result<DetectionMetrics> {
    Ok(detection_metrics(
        &input.actual,
        &input.predicted,
        input.classes,
        input.iou_threshold,
    )?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventMetricsRequest {
    pub events: Vec<EventInterval>,
    pub alarms_ms: Vec<i64>,
    pub horizon_ms: u64,
    pub observed_duration_ms: u64,
}
fn events(input: EventMetricsRequest) -> Result<EventMetrics> {
    Ok(event_metrics(
        &input.events,
        &input.alarms_ms,
        input.horizon_ms,
        input.observed_duration_ms,
    )?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum CalibrationRequest {
    Platt {
        scores: Vec<f64>,
        actual: Vec<bool>,
    },
    Isotonic {
        scores: Vec<f64>,
        actual: Vec<bool>,
    },
    Temperature {
        logits: Vec<Vec<f64>>,
        actual: Vec<u32>,
    },
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "model", rename_all = "snake_case")]
pub enum Calibrator {
    Platt(PlattCalibrator),
    Isotonic(IsotonicCalibrator),
    Temperature(TemperatureScaler),
}
fn calibrate(input: CalibrationRequest) -> Result<Calibrator> {
    Ok(match input {
        CalibrationRequest::Platt { scores, actual } => {
            Calibrator::Platt(PlattCalibrator::fit(&scores, &actual)?)
        }
        CalibrationRequest::Isotonic { scores, actual } => {
            Calibrator::Isotonic(IsotonicCalibrator::fit(&scores, &actual)?)
        }
        CalibrationRequest::Temperature { logits, actual } => {
            Calibrator::Temperature(TemperatureScaler::fit(&logits, &actual)?)
        }
    })
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyCalibrationRequest {
    pub calibrator: Calibrator,
    pub scores: Vec<f64>,
}
fn apply_calibration(input: ApplyCalibrationRequest) -> Result<Vec<f64>> {
    Ok(match input.calibrator {
        Calibrator::Platt(model) => input
            .scores
            .iter()
            .map(|s| model.predict(*s))
            .collect::<flow_like_ml_core::Result<_>>()?,
        Calibrator::Isotonic(model) => input
            .scores
            .iter()
            .map(|s| model.predict(*s))
            .collect::<flow_like_ml_core::Result<_>>()?,
        Calibrator::Temperature(model) => model.predict(&input.scores)?,
    })
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ThresholdRequest {
    pub actual: Vec<bool>,
    pub scores: Vec<f64>,
    pub constraints: ThresholdConstraints,
}
fn threshold(input: ThresholdRequest) -> Result<Option<ThresholdSelection>> {
    Ok(select_threshold(
        &input.actual,
        &input.scores,
        &input.constraints,
    )?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DistributionDriftRequest {
    pub reference: Vec<f64>,
    pub current: Vec<f64>,
    pub alpha: f64,
    pub bin_edges: Vec<f64>,
    pub psi_threshold: f64,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct DistributionDrift {
    pub ks: KsResult,
    pub population_stability_index: f64,
    pub psi_drift: bool,
}
fn distribution_drift(input: DistributionDriftRequest) -> Result<DistributionDrift> {
    flow_like_ml_core::require(
        input.psi_threshold.is_finite() && input.psi_threshold > 0.0,
        "PSI threshold must be positive",
    )?;
    let psi = population_stability_index(&input.reference, &input.current, &input.bin_edges)?;
    Ok(DistributionDrift {
        ks: ks_test(&input.reference, &input.current, input.alpha)?,
        population_stability_index: psi,
        psi_drift: psi > input.psi_threshold,
    })
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LossDriftRequest {
    pub state: Option<PageHinkley>,
    pub delta: f64,
    pub threshold: f64,
    pub minimum_samples: usize,
    pub values: Vec<f64>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct LossDrift {
    pub state: PageHinkley,
    pub drift: bool,
    pub first_detection_index: Option<usize>,
}
fn loss_drift(input: LossDriftRequest) -> Result<LossDrift> {
    let mut state = match input.state {
        Some(state) => state,
        None => PageHinkley::new(input.delta, input.threshold, input.minimum_samples)?,
    };
    let mut first = None;
    for (i, value) in input.values.into_iter().enumerate() {
        if state.update(value)? {
            first.get_or_insert(i);
        }
    }
    Ok(LossDrift {
        state,
        drift: first.is_some(),
        first_detection_index: first,
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct ClassificationMetricsNode;
operation!(
    ClassificationMetricsNode,
    "ml_classification_metrics",
    "Classification Metrics",
    "Measure per-class precision, recall and F1 with accuracy confidence bounds",
    "classificationMetrics",
    "AI/ML/Evaluation",
    ClassMetricsRequest,
    ClassificationMetrics,
    classification
);
#[crate::register_node]
#[derive(Default)]
pub struct BinaryCurveNode;
operation!(
    BinaryCurveNode,
    "ml_binary_curve",
    "Precision/Recall and ROC Curve",
    "Measure threshold curves, AUROC, average precision and calibration errors",
    "binaryCurve",
    "AI/ML/Evaluation",
    BinaryMetricsRequest,
    BinaryCurve,
    binary
);
#[crate::register_node]
#[derive(Default)]
pub struct RegressionMetricsNode;
operation!(
    RegressionMetricsNode,
    "ml_regression_metrics",
    "Regression Metrics",
    "Measure MAE, RMSE and R squared",
    "regressionMetrics",
    "AI/ML/Evaluation",
    RegressionMetricsRequest,
    RegressionMetrics,
    regression
);
#[crate::register_node]
#[derive(Default)]
pub struct DetectionMetricsNode;
operation!(
    DetectionMetricsNode,
    "ml_detection_metrics",
    "Detection Metrics",
    "Measure per-class 101-point AP at an explicit IoU threshold",
    "detectionMetrics",
    "AI/ML/Evaluation",
    DetectionMetricsRequest,
    DetectionMetrics,
    detection
);
#[crate::register_node]
#[derive(Default)]
pub struct SegmentationMetricsNode;
operation!(
    SegmentationMetricsNode,
    "ml_segmentation_metrics",
    "Segmentation Metrics",
    "Measure per-class IoU and Dice",
    "segmentationMetrics",
    "AI/ML/Evaluation",
    ClassMetricsRequest,
    SegmentationMetrics,
    segmentation
);
#[crate::register_node]
#[derive(Default)]
pub struct EventMetricsNode;
operation!(
    EventMetricsNode,
    "ml_event_metrics",
    "Predictive Event Metrics",
    "Match alarms to observed events and measure lead time and false alarms per hour",
    "eventMetrics",
    "AI/ML/Evaluation",
    EventMetricsRequest,
    EventMetrics,
    events
);
#[crate::register_node]
#[derive(Default)]
pub struct CalibrateProbabilitiesNode;
operation!(
    CalibrateProbabilitiesNode,
    "ml_calibrate_probabilities",
    "Fit Probability Calibration",
    "Fit Platt, isotonic or temperature calibration on held-out validation data",
    "fitCalibration",
    "AI/ML/Evaluation",
    CalibrationRequest,
    Calibrator,
    calibrate
);
#[crate::register_node]
#[derive(Default)]
pub struct ApplyCalibrationNode;
operation!(
    ApplyCalibrationNode,
    "ml_apply_calibration",
    "Apply Probability Calibration",
    "Apply a saved calibrator to scores or multiclass logits",
    "calibrate",
    "AI/ML/Inference",
    ApplyCalibrationRequest,
    Vec<f64>,
    apply_calibration
);
#[crate::register_node]
#[derive(Default)]
pub struct SelectThresholdNode;
operation!(
    SelectThresholdNode,
    "ml_select_threshold",
    "Select Decision Threshold",
    "Select a validation threshold satisfying recall, precision and false-positive constraints",
    "selectThreshold",
    "AI/ML/Evaluation",
    ThresholdRequest,
    Option<ThresholdSelection>,
    threshold
);
#[crate::register_node]
#[derive(Default)]
pub struct DistributionDriftNode;
operation!(
    DistributionDriftNode,
    "ml_distribution_drift",
    "Distribution Drift",
    "Compare reference and current distributions using KS and PSI",
    "distributionDrift",
    "AI/ML/Monitoring",
    DistributionDriftRequest,
    DistributionDrift,
    distribution_drift
);
#[crate::register_node]
#[derive(Default)]
pub struct LossDriftNode;
operation!(
    LossDriftNode,
    "ml_loss_drift",
    "Loss Drift",
    "Update a persisted Page-Hinkley detector for increasing prediction error",
    "lossDrift",
    "AI/ML/Monitoring",
    LossDriftRequest,
    LossDrift,
    loss_drift
);

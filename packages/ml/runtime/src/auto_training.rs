//! Bounded model search. Validation ranks candidates; the final holdout is read after selection.
use crate::{
    engines::LoadedArtifactPredictor,
    worker::{LocalRunLock, TrainingWorker},
    *,
};
use flow_like_ml_burn as burn;
use flow_like_ml_core::{
    Annotation, ComputeConfig, InspectionSpec, LabelProvenance, Sample, TaskKind, TensorData,
};
use flow_like_ml_native::evaluation as metrics;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Instant,
};

pub trait SearchPredictor {
    /// Input has a leading batch dimension. Return native scores or Burn-shaped predictions.
    fn predict(&mut self, input: &TensorData) -> Result<Value>;
}

pub trait SearchPredictorFactory: Send + Sync {
    fn load(
        &self,
        repository: &TrainingRepository,
        artifact_id: &str,
        compute: &ComputeConfig,
    ) -> Result<Box<dyn SearchPredictor>>;
}

#[derive(Default)]
pub struct NativeSearchPredictorFactory;
impl SearchPredictor for LoadedArtifactPredictor {
    fn predict(&mut self, input: &TensorData) -> Result<Value> {
        LoadedArtifactPredictor::predict(self, input)
    }
}
impl SearchPredictorFactory for NativeSearchPredictorFactory {
    fn load(
        &self,
        repository: &TrainingRepository,
        artifact_id: &str,
        compute: &ComputeConfig,
    ) -> Result<Box<dyn SearchPredictor>> {
        Ok(Box::new(LoadedArtifactPredictor::load(
            repository,
            artifact_id,
            compute,
        )?))
    }
}

pub fn default_goals(spec: &InspectionSpec) -> ExperimentGoals {
    let (task, primary, direction) = match spec.task {
        TaskKind::SensorRegression | TaskKind::SequenceForecast => (
            EvaluationTask::Regression,
            "rmse",
            MetricDirection::Minimize,
        ),
        TaskKind::Fusion if spec.labels.is_empty() => (
            EvaluationTask::Regression,
            "rmse",
            MetricDirection::Minimize,
        ),
        TaskKind::SensorAnomaly | TaskKind::VisualAnomaly | TaskKind::SequenceAutoencoder => (
            EvaluationTask::Anomaly { threshold: 0.5 },
            "auroc",
            MetricDirection::Maximize,
        ),
        TaskKind::ObjectDetection => (
            EvaluationTask::Detection {
                classes: spec.labels.len(),
                iou_threshold: 0.5,
            },
            "mean_average_precision",
            MetricDirection::Maximize,
        ),
        TaskKind::Segmentation => (
            EvaluationTask::Segmentation {
                classes: spec.labels.len(),
            },
            "mean_iou",
            MetricDirection::Maximize,
        ),
        TaskKind::InstanceSegmentation => (
            EvaluationTask::InstanceSegmentation {
                classes: spec.labels.len(),
                iou_threshold: 0.5,
            },
            "mask_mean_average_precision",
            MetricDirection::Maximize,
        ),
        _ => (
            EvaluationTask::Classification,
            "accuracy",
            MetricDirection::Maximize,
        ),
    };
    ExperimentGoals {
        task,
        primary_metric: primary.into(),
        direction,
        minimum_audited_samples: 1,
        bounds: Vec::new(),
    }
}

/// Validate search limits independently of whether the task has generated candidates.
pub fn validate_search(
    spec: &InspectionSpec,
    search: &AutoSearchConfig,
    compute: &ComputeConfig,
) -> Result<()> {
    spec.validate().map_err(engine_error)?;
    compute.validate().map_err(engine_error)?;
    if search.maximum_candidates == 0
        || search.maximum_candidates > 64
        || search.min_epochs == 0
        || search.max_epochs < search.min_epochs
        || search.max_epochs > 1000
        || search.batch_size == 0
        || search.batch_size > 128
        || search.output_features == 0
        || search.output_features > 4096
        || !(2..=8).contains(&search.reduction_factor)
    {
        return Err(invalid(
            "search requires 1..64 candidates, ordered epochs up to 1000, batch 1..128 and bounded output width",
        ));
    }
    Ok(())
}

/// Generate a reproducible, bounded set of actual supported training recipes.
pub fn generate_candidates(
    spec: &InspectionSpec,
    search: &AutoSearchConfig,
    compute: &ComputeConfig,
) -> Result<Vec<TrainingRequest>> {
    validate_search(spec, search, compute)?;
    let shape = &spec.input_shape;
    let classification = !spec.labels.is_empty()
        && !matches!(
            spec.task,
            TaskKind::SensorAnomaly | TaskKind::VisualAnomaly | TaskKind::SequenceAutoencoder
        );
    let objective = if classification {
        burn::Objective::Classification
    } else {
        burn::Objective::Regression
    };
    let outputs = if classification {
        spec.labels.len()
    } else {
        search.output_features
    };
    let mut recipes = Vec::new();
    let mut candidates = Vec::new();
    let compute = serde_json::to_value(compute)?;
    let native = |engine: &str, parameters: Value| TrainingRequest {
        engine: engine.into(),
        recipe: parameters,
        compute: compute.clone(),
    };
    match spec.task {
        TaskKind::SensorClassification
        | TaskKind::SensorRegression
        | TaskKind::SequenceForecast => {
            if (classification && outputs == 2) || (!classification && outputs == 1) {
                for depth in [2, 4, 6] {
                    candidates.push(native("histogram_gradient_boosting", serde_json::json!({"labels":spec.labels,"inspection_task":spec.task,"config":{
                        "objective":if classification {"binary_log_loss"} else {"squared_error"},
                        "estimators":64,"max_depth":depth,"max_bins":32,"min_leaf":2,"learning_rate":0.08,"l2":1.0
                    }})));
                }
            }
            match shape.as_slice() {
                [features] => {
                    for hidden in [16, 32, 64] {
                        recipes.push(burn::Recipe::Mlp {
                            input_features: *features,
                            hidden,
                            outputs,
                            objective,
                        });
                    }
                }
                [_, features] => {
                    for hidden in [16, 32] {
                        recipes.extend([
                            burn::Recipe::Lstm {
                                input_features: *features,
                                hidden,
                                outputs,
                                objective,
                            },
                            burn::Recipe::Gru {
                                input_features: *features,
                                hidden,
                                outputs,
                                objective,
                            },
                            burn::Recipe::Cnn1d {
                                input_features: *features,
                                hidden,
                                outputs,
                                objective,
                            },
                            burn::Recipe::Tcn {
                                input_features: *features,
                                hidden,
                                outputs,
                                objective,
                                levels: 3,
                            },
                        ]);
                    }
                }
                _ => {
                    return Err(invalid(
                        "sensor search expects feature vectors or [time, features] windows",
                    ));
                }
            }
        }
        TaskKind::SensorAnomaly | TaskKind::VisualAnomaly | TaskKind::SequenceAutoencoder => {
            for trees in [64, 128] {
                candidates.push(native("isolation_forest", serde_json::json!({"labels":spec.labels,"inspection_task":spec.task,"config":{"trees":trees,"sample_size":256,"seed":search.seed}})));
            }
            match shape.as_slice() {
                [features] => {
                    for hidden in [16, 32] {
                        recipes.push(burn::Recipe::DenseAutoencoder {
                            input_features: *features,
                            hidden,
                            latent: 8,
                        });
                    }
                }
                [_, features] if spec.task != TaskKind::VisualAnomaly => {
                    for hidden in [16, 32] {
                        recipes.extend([
                            burn::Recipe::LstmAutoencoder {
                                input_features: *features,
                                hidden,
                                latent: 8,
                            },
                            burn::Recipe::Conv1dAutoencoder {
                                input_features: *features,
                                hidden,
                                latent: 8,
                            },
                        ]);
                    }
                }
                [_, _] => {
                    candidates.push(native("patchcore", serde_json::json!({"labels":spec.labels,"inspection_task":spec.task,"coreset_size":128})));
                    candidates.push(native("padim", serde_json::json!({"labels":spec.labels,"inspection_task":spec.task,"regularization":0.01})));
                }
                _ => {}
            }
        }
        TaskKind::ImageClassification => {
            let [channels, _, _] = shape.as_slice() else {
                return Err(invalid("image search expects [channels,height,width]"));
            };
            recipes.extend([
                burn::Recipe::ResNet18 {
                    input_channels: *channels,
                    classes: outputs,
                    base_channels: 16,
                },
                burn::Recipe::ResNet18 {
                    input_channels: *channels,
                    classes: outputs,
                    base_channels: 32,
                },
                burn::Recipe::MobileNetV2 {
                    input_channels: *channels,
                    classes: outputs,
                    width_multiplier: 0.5,
                },
                burn::Recipe::EfficientNet {
                    input_channels: *channels,
                    classes: outputs,
                    width_multiplier: 0.5,
                },
            ]);
        }
        TaskKind::ObjectDetection => {
            let [channels, _, _] = shape.as_slice() else {
                return Err(invalid("detection search expects image tensors"));
            };
            for width in [0.25, 0.5] {
                recipes.push(burn::Recipe::YoloX {
                    input_channels: *channels,
                    classes: outputs,
                    width_multiplier: width,
                    depth_multiplier: 0.33,
                });
            }
        }
        TaskKind::Segmentation => {
            let [channels, _, _] = shape.as_slice() else {
                return Err(invalid("segmentation search expects image tensors"));
            };
            for channels_base in [8, 16, 32] {
                recipes.push(burn::Recipe::UNet {
                    input_channels: *channels,
                    classes: outputs,
                    base_channels: channels_base,
                    depth: 2,
                });
            }
        }
        TaskKind::InstanceSegmentation => {
            let [channels, _, _] = shape.as_slice() else {
                return Err(invalid("instance search expects image tensors"));
            };
            for channels_base in [8, 16] {
                recipes.push(burn::Recipe::MaskRcnn {
                    config: burn::MaskRcnnConfig {
                        input_channels: *channels,
                        classes: outputs,
                        base_channels: channels_base,
                        ..Default::default()
                    },
                });
            }
        }
        TaskKind::VisualSequenceClassification => {
            let [_, channels, _, _] = shape.as_slice() else {
                return Err(invalid(
                    "visual sequence search expects [time,channels,height,width]",
                ));
            };
            for hidden in [16, 32] {
                recipes.push(burn::Recipe::CnnLstm {
                    input_channels: *channels,
                    cnn_channels: 16,
                    hidden,
                    outputs,
                    objective,
                });
            }
        }
        TaskKind::Fusion => {
            return Err(invalid(
                "automatic fusion search needs an explicit recipe defining image dimensions and sensor features",
            ));
        }
    }
    let mut rng = SearchRng(search.seed);
    for recipe in recipes {
        let config = burn::TrainingConfig {
            recipe,
            backend: burn::BackendChoice::Auto,
            epochs: search.min_epochs,
            batch_size: search
                .batch_size
                .min(if spec.task == TaskKind::InstanceSegmentation {
                    32
                } else {
                    128
                }),
            learning_rate: if rng.next() % 2 == 0 { 0.003 } else { 0.01 },
            seed: search.seed,
            gradient_clip: 5.0,
        };
        if burn::validate_inspection_config(spec, &config).is_ok() {
            let family = digest(&serde_json::to_vec(&config)?);
            candidates.push(TrainingRequest {engine:"burn".into(),recipe:serde_json::json!({"config":config,"labels":spec.labels,"inspection_task":spec.task,"search":{"family":family,"rung":0,"min_epochs":search.min_epochs,"max_epochs":search.max_epochs,"reduction_factor":search.reduction_factor}}),compute:compute.clone()});
        }
    }
    for i in (1..candidates.len()).rev() {
        let j = rng.next() as usize % (i + 1);
        candidates.swap(i, j);
    }
    candidates.truncate(search.maximum_candidates);
    if candidates.is_empty() {
        return Err(invalid(
            "no supported candidate matches this shape and task",
        ));
    }
    Ok(candidates)
}

struct SearchRng(u64);
impl SearchRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
}

fn engine_error(error: impl std::fmt::Display) -> Error {
    Error::Engine(error.to_string())
}

struct Observation {
    sample: Sample,
    predicted_class: Option<u32>,
    evidence: Option<EvidencePrediction>,
    latency_ms: f64,
}

fn sealed_sample(
    row: &TrainingSample,
    spec: &InspectionSpec,
    as_of_ms: i64,
    audited: bool,
) -> Result<Sample> {
    let sample: Sample =
        serde_json::from_value(row.payload.get("sample").unwrap_or(&row.payload).clone())?;
    spec.validate_sample_at(&sample, as_of_ms)
        .map_err(engine_error)?;
    if !row.accepted
        || row.id != sample.id
        || row.group_id != sample.group_id
        || row.label_available_at_ms > as_of_ms
    {
        return Err(invalid(
            "snapshot contains an unavailable, unaccepted or mismatched sample",
        ));
    }
    if audited
        && !matches!(
            (&row.source, &sample.provenance),
            (LabelSource::Reviewed, LabelProvenance::Reviewed { .. })
                | (
                    LabelSource::ObservedOutcome,
                    LabelProvenance::Measured { .. }
                )
        )
    {
        return Err(invalid(
            "model selection and final evaluation require reviewed or measured labels",
        ));
    }
    Ok(sample)
}

fn predict_sample(predictor: &mut dyn SearchPredictor, sample: &Sample) -> Result<Value> {
    let mut input = sample.input.clone();
    input.shape.insert(0, 1);
    predictor.predict(&input)
}

fn floats(value: &Value) -> Result<Vec<f64>> {
    let values: Vec<f64> = serde_json::from_value(value.clone())?;
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return Err(invalid(
            "prediction must contain finite nonempty numeric values",
        ));
    }
    Ok(values)
}

fn scalar_score(raw: &Value) -> Result<f64> {
    for key in ["scores", "reconstruction_error"] {
        if raw.get(key).is_some_and(Value::is_array) {
            return Ok(floats(&raw[key])?[0]);
        }
    }
    Err(invalid(
        "anomaly model did not return anomaly scores or reconstruction errors",
    ))
}

fn decode_prediction(
    sample: Sample,
    raw: &Value,
    task: &EvaluationTask,
    labels: usize,
    anomaly_scale: f64,
    latency_ms: f64,
) -> Result<Observation> {
    let mut observation = Observation {
        sample,
        predicted_class: None,
        evidence: None,
        latency_ms,
    };
    observation.evidence = match task {
        EvaluationTask::Classification => {
            let class = if let Some(class) = raw
                .get("classes")
                .and_then(Value::as_array)
                .and_then(|v| v.first())
                .and_then(Value::as_u64)
            {
                class as u32
            } else if raw.get("scores").is_some() && labels == 2 {
                u32::from(floats(&raw["scores"])?[0] >= 0.5)
            } else {
                let values = floats(&raw["output"]["values"])?;
                if values.len() != labels {
                    return Err(invalid("class scores differ from the label order"));
                }
                values
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(i, _)| i as u32)
                    .ok_or_else(|| invalid("empty class prediction"))?
            };
            if class as usize >= labels {
                return Err(invalid("predicted class is outside the label schema"));
            }
            observation.predicted_class = Some(class);
            None
        }
        EvaluationTask::Regression => Some(EvidencePrediction::Regression {
            values: floats(raw.get("scores").unwrap_or(&raw["output"]["values"]))?,
        }),
        EvaluationTask::Anomaly { .. } => {
            let score = scalar_score(raw)?;
            if score < 0.0 || !anomaly_scale.is_finite() || anomaly_scale <= 0.0 {
                return Err(invalid("invalid anomaly score calibration"));
            }
            Some(EvidencePrediction::Anomaly {
                probability: score / (score + anomaly_scale),
            })
        }
        EvaluationTask::Detection { .. } => {
            let detections: Vec<Vec<burn::Detection>> =
                serde_json::from_value(raw["detections"].clone())?;
            if detections.len() != 1 {
                return Err(invalid(
                    "detection predictor returned a different batch size",
                ));
            }
            let boxes = detections
                .into_iter()
                .next()
                .unwrap_or_default()
                .into_iter()
                .map(|d| DetectionEvidence {
                    class_id: d.class_id as u32,
                    x_min: d.x_min,
                    y_min: d.y_min,
                    x_max: d.x_max,
                    y_max: d.y_max,
                    confidence: d.confidence as f64,
                })
                .collect();
            Some(EvidencePrediction::Detection { boxes })
        }
        EvaluationTask::Segmentation { classes } => {
            let shape: Vec<usize> = serde_json::from_value(raw["output"]["shape"].clone())?;
            let [1, channels, height, width] = shape.as_slice() else {
                return Err(invalid(
                    "segmentation output must be [1,classes,height,width]",
                ));
            };
            if channels != classes {
                return Err(invalid("segmentation output class count differs"));
            }
            let values = floats(&raw["output"]["values"])?;
            let area = height
                .checked_mul(*width)
                .ok_or_else(|| invalid("segmentation area overflow"))?;
            if values.len() != area * channels {
                return Err(invalid("segmentation output length differs"));
            }
            let found = (0..area)
                .map(|pixel| {
                    (0..*channels)
                        .max_by(|a, b| {
                            values[a * area + pixel].total_cmp(&values[b * area + pixel])
                        })
                        .unwrap_or(0) as u32
                })
                .collect();
            Some(EvidencePrediction::Segmentation {
                width: *width,
                height: *height,
                classes: found,
            })
        }
        EvaluationTask::InstanceSegmentation { .. } => {
            let predictions: Vec<Vec<burn::InstanceDetection>> =
                serde_json::from_value(raw["instances"].clone())?;
            if predictions.len() != 1 {
                return Err(invalid(
                    "instance predictor returned a different batch size",
                ));
            }
            let mut instances = Vec::new();
            for instance in predictions.into_iter().next().unwrap_or_default() {
                let [height, width] = instance.mask.shape.as_slice() else {
                    return Err(invalid("instance mask must be an image grid"));
                };
                instances.push(InstanceEvidence {
                    class_id: instance.detection.class_id as u32,
                    confidence: instance.detection.confidence as f64,
                    width: *width,
                    height: *height,
                    foreground: instance.mask.values.into_iter().map(|v| v >= 0.5).collect(),
                });
            }
            Some(EvidencePrediction::Instances { instances })
        }
        EvaluationTask::Events { .. } => {
            return Err(invalid(
                "automatic event-interval scoring needs an explicit alarm policy; use classification or regression search",
            ));
        }
    };
    Ok(observation)
}

/// Convert an actual model output into the same evidence format used by final audits.
pub fn prediction_record_from_output(
    row: &TrainingSample,
    spec: &InspectionSpec,
    task: &EvaluationTask,
    artifact_id: &str,
    raw: &Value,
    anomaly_scale: f64,
    latency_ms: f64,
) -> Result<PredictionRecord> {
    let sample = sealed_sample(row, spec, now_ms(), true)?;
    let observation = decode_prediction(
        sample,
        raw,
        task,
        spec.labels.len(),
        anomaly_scale,
        latency_ms,
    )?;
    let actual_label = if let Annotation::Class { class_id } = observation.sample.annotation {
        Some(
            spec.labels
                .get(class_id as usize)
                .ok_or_else(|| invalid("audit class outside label order"))?
                .clone(),
        )
    } else {
        None
    };
    Ok(PredictionRecord {
        sample_id: row.id.clone(),
        artifact_id: artifact_id.into(),
        predicted_label: observation
            .predicted_class
            .map(|class| spec.labels[class as usize].clone()),
        teacher_label: None,
        actual_label,
        actual_source: Some(row.source),
        latency_ms,
        failed: false,
        recorded_at_ms: now_ms(),
        details: serde_json::json!({"prediction":observation.evidence,"partition":"learning_audit"}),
    })
}

fn measure(
    observations: &[Observation],
    task: &EvaluationTask,
    labels: &[String],
) -> Result<BTreeMap<String, f64>> {
    let classes = labels.len();
    if observations.is_empty() {
        return Err(invalid("held-out partition is empty"));
    }
    let mut values = BTreeMap::from([
        ("audited_samples".into(), observations.len() as f64),
        (
            "mean_latency_ms".into(),
            observations.iter().map(|v| v.latency_ms).sum::<f64>() / observations.len() as f64,
        ),
        ("failure_rate".into(), 0.0),
    ]);
    match task {
        EvaluationTask::Classification => {
            let actual = observations
                .iter()
                .map(|v| match v.sample.annotation {
                    Annotation::Class { class_id } => Ok(class_id),
                    _ => Err(invalid(
                        "classification validation requires class annotations",
                    )),
                })
                .collect::<Result<Vec<_>>>()?;
            let predicted = observations
                .iter()
                .map(|v| {
                    v.predicted_class
                        .ok_or_else(|| invalid("missing predicted class"))
                })
                .collect::<Result<Vec<_>>>()?;
            let result = metrics::classification_metrics(&actual, &predicted, classes)
                .map_err(engine_error)?;
            let ordinal =
                metrics::ordinal_metrics(&actual, &predicted, classes).map_err(engine_error)?;
            values.extend([
                ("mean_rank_error".into(), ordinal.mean_rank_error),
                ("macro_rank_error".into(), ordinal.macro_rank_error),
                ("quadratic_kappa".into(), ordinal.quadratic_kappa),
            ]);
            values.extend([
                ("accuracy".into(), result.accuracy),
                (
                    "accuracy_wilson_lower_95".into(),
                    result.accuracy_interval_95.lower,
                ),
                ("macro_f1".into(), result.macro_f1),
                ("balanced_accuracy".into(), result.balanced_accuracy),
            ]);
            for class in result.per_class {
                let label = &labels[class.class_id as usize];
                values.insert(format!("support/{label}"), class.support as f64);
                values.insert(
                    format!("class/{}/support", class.class_id),
                    class.support as f64,
                );
                if let Some(recall) = class.recall {
                    values.insert(format!("recall/{label}"), recall);
                    values.insert(format!("class/{}/recall", class.class_id), recall);
                }
            }
        }
        EvaluationTask::Regression => {
            let mut actual = Vec::new();
            let mut predicted = Vec::new();
            for observation in observations {
                let truth = match &observation.sample.annotation {
                    Annotation::Scalar { value } => vec![*value],
                    Annotation::Values { values } => values.iter().map(|v| *v as f64).collect(),
                    _ => return Err(invalid("regression validation requires numeric targets")),
                };
                let Some(EvidencePrediction::Regression { values: found }) = &observation.evidence
                else {
                    return Err(invalid("missing regression evidence"));
                };
                if truth.len() != found.len() {
                    return Err(invalid("regression output width differs from target"));
                }
                actual.extend(truth);
                predicted.extend(found);
            }
            let result = metrics::regression_metrics(&actual, &predicted).map_err(engine_error)?;
            values.insert("rmse".into(), result.rmse);
            values.insert("mae".into(), result.mae);
            if let Some(r2) = result.r2 {
                values.insert("r2".into(), r2);
            }
        }
        EvaluationTask::Anomaly { threshold } => {
            let mut actual = Vec::new();
            let mut scores = Vec::new();
            for observation in observations {
                let Annotation::Anomaly { is_anomaly } = observation.sample.annotation else {
                    return Err(invalid(
                        "anomaly validation needs reviewed normal/anomaly labels",
                    ));
                };
                let Some(EvidencePrediction::Anomaly { probability }) = observation.evidence else {
                    return Err(invalid("missing anomaly evidence"));
                };
                actual.push(is_anomaly);
                scores.push(probability);
            }
            let curve = metrics::binary_curve(&actual, &scores).map_err(engine_error)?;
            if let Some(auc) = curve.roc_auc {
                values.insert("auroc".into(), auc);
            }
            if let Some(ap) = curve.average_precision {
                values.insert("average_precision".into(), ap);
            }
            let truth: Vec<_> = actual.iter().map(|v| u32::from(*v)).collect();
            let predicted: Vec<_> = scores.iter().map(|v| u32::from(v >= threshold)).collect();
            let result =
                metrics::classification_metrics(&truth, &predicted, 2).map_err(engine_error)?;
            values.insert("accuracy".into(), result.accuracy);
            values.insert("brier_score".into(), curve.brier_score);
            if let Some(recall) = result.per_class[1].recall {
                values.insert("recall".into(), recall);
            }
            if result.per_class[0].support > 0 {
                values.insert(
                    "false_positive_rate".into(),
                    result.per_class[1].false_positive as f64 / result.per_class[0].support as f64,
                );
            }
        }
        EvaluationTask::Detection {
            classes,
            iou_threshold,
        } => {
            let mut actual = Vec::new();
            let mut predicted = Vec::new();
            for observation in observations {
                let Annotation::Boxes { boxes } = &observation.sample.annotation else {
                    return Err(invalid("detection validation needs box annotations"));
                };
                let Some(EvidencePrediction::Detection { boxes: found }) = &observation.evidence
                else {
                    return Err(invalid("missing detection evidence"));
                };
                actual.push(boxes.clone());
                predicted.push(
                    found
                        .iter()
                        .map(|b| metrics::ScoredBox {
                            bounds: flow_like_ml_core::BoundingBox {
                                class_id: b.class_id,
                                x_min: b.x_min,
                                y_min: b.y_min,
                                x_max: b.x_max,
                                y_max: b.y_max,
                            },
                            confidence: b.confidence,
                        })
                        .collect(),
                );
            }
            let result = metrics::detection_metrics(&actual, &predicted, *classes, *iou_threshold)
                .map_err(engine_error)?;
            if let Some(ap) = result.mean_average_precision {
                values.insert("mean_average_precision".into(), ap);
            }
        }
        EvaluationTask::Segmentation { classes } => {
            let mut actual = Vec::new();
            let mut predicted = Vec::new();
            for observation in observations {
                let Annotation::Mask {
                    width,
                    height,
                    classes: truth,
                } = &observation.sample.annotation
                else {
                    return Err(invalid("segmentation validation needs masks"));
                };
                let Some(EvidencePrediction::Segmentation {
                    width: pw,
                    height: ph,
                    classes: found,
                }) = &observation.evidence
                else {
                    return Err(invalid("missing segmentation evidence"));
                };
                if width != pw || height != ph || truth.len() != found.len() {
                    return Err(invalid("segmentation masks do not align"));
                }
                actual.extend(truth);
                predicted.extend(found);
            }
            let result = metrics::segmentation_metrics(&actual, &predicted, *classes)
                .map_err(engine_error)?;
            values.insert("mean_iou".into(), result.mean_iou);
            values.insert("mean_dice".into(), result.mean_dice);
        }
        EvaluationTask::InstanceSegmentation {
            classes,
            iou_threshold,
        } => {
            let mut actual = Vec::new();
            let mut predicted = Vec::new();
            for observation in observations {
                let Annotation::InstanceMasks { instances } = &observation.sample.annotation else {
                    return Err(invalid("instance validation needs masks"));
                };
                let Some(EvidencePrediction::Instances { instances: found }) =
                    &observation.evidence
                else {
                    return Err(invalid("missing instance evidence"));
                };
                actual.push(instances.clone());
                predicted.push(found.clone());
            }
            crate::evaluation::instance_metrics(
                &actual,
                &predicted,
                *classes,
                *iou_threshold,
                &mut values,
                &mut BTreeMap::new(),
            )?;
        }
        EvaluationTask::Events { .. } => {
            return Err(invalid("event intervals require an explicit alarm policy"));
        }
    }
    Ok(values)
}

pub struct AutoTrainingController {
    repository: TrainingRepository,
    worker: TrainingWorker,
    predictors: Arc<dyn SearchPredictorFactory>,
}
impl AutoTrainingController {
    pub fn new(repository: TrainingRepository, worker: TrainingWorker) -> Self {
        Self {
            repository,
            worker,
            predictors: Arc::new(NativeSearchPredictorFactory),
        }
    }
    pub fn with_predictor_factory(mut self, predictors: Arc<dyn SearchPredictorFactory>) -> Self {
        self.predictors = predictors;
        self
    }

    /// Execute at most one candidate. Reopening the repository resumes a scheduled job or
    /// validates its already-published artifact without retraining it.
    pub fn run_next(&self, experiment_id: &str) -> Result<Experiment> {
        let _lock =
            match LocalRunLock::acquire(&self.repository, &format!("experiment:{experiment_id}")) {
                Ok(lock) => lock,
                Err(Error::Conflict(_)) => return self.repository.get_experiment(experiment_id),
                Err(error) => return Err(error),
            };
        let experiment = self.repository.get_experiment(experiment_id)?;
        if experiment.status != ExperimentStatus::Running || experiment.selected_trial_id.is_some()
        {
            return Ok(experiment);
        }
        let limits = self.worker.limits();
        let allowed = &experiment.request.budget.worker_limits;
        if limits.memory_budget_bytes > allowed.memory_budget_bytes
            || limits.maximum_duration_ms > allowed.maximum_duration_ms
            || limits.maximum_artifact_bytes > allowed.maximum_artifact_bytes
            || limits.maximum_checkpoint_bytes > allowed.maximum_checkpoint_bytes
        {
            return Err(invalid(
                "worker limits exceed the experiment's reserved resource limits",
            ));
        }
        let trials = self.repository.list_experiment_trials(experiment_id)?;
        let trial = if let Some(trial) = trials
            .iter()
            .find(|t| t.status == ExperimentTrialStatus::Scheduled)
        {
            trial.clone()
        } else {
            let index = (0..experiment.request.candidates.len())
                .find(|i| !trials.iter().any(|t| t.candidate_index == *i));
            if experiment.usage.submitted_trials >= experiment.request.budget.maximum_trials {
                return self.repository.stop_experiment_budget(
                    experiment_id,
                    "trial_budget_exhausted",
                    now_ms(),
                );
            }
            let submitted = if let Some(index) = index {
                self.repository
                    .submit_experiment_trial(
                        experiment_id,
                        index,
                        &format!("candidate-{index}"),
                        now_ms(),
                    )
                    .map(Some)
            } else {
                self.next_halving_trial(&experiment, &trials)
            };
            match submitted {
                Ok(Some(trial)) => trial,
                Ok(None) => return Ok(experiment),
                Err(Error::Invalid(message)) if message.contains("budget") => {
                    return self.repository.stop_experiment_budget(
                        experiment_id,
                        &message,
                        now_ms(),
                    );
                }
                Err(error) => return Err(error),
            }
        };
        let recovering_completed =
            self.repository.get_job(&trial.job_id)?.status == JobStatus::Succeeded;
        let started = Instant::now();
        let result = (|| -> Result<(ModelArtifact, BTreeMap<String, f64>)> {
            self.check_cancel(experiment_id)?;
            let job = self.repository.get_job(&trial.job_id)?;
            if job.status == JobStatus::Running
                && job
                    .lease_expires_at_ms
                    .is_none_or(|deadline| deadline > now_ms())
            {
                return Err(Error::Conflict(
                    "candidate is already owned by a training worker".into(),
                ));
            }
            let dataset = self
                .repository
                .get_experiment_search_snapshot(&trial.id, snapshot_limit(&experiment))?;
            if !dataset.test.is_empty() {
                return Err(invalid(
                    "candidate training snapshot exposes the final holdout",
                ));
            }
            let mut spec: InspectionSpec = serde_json::from_value(experiment.request.spec.clone())?;
            let first: Sample = serde_json::from_value(
                dataset
                    .train
                    .first()
                    .ok_or_else(|| invalid("empty training partition"))?
                    .payload
                    .get("sample")
                    .unwrap_or(&dataset.train[0].payload)
                    .clone(),
            )?;
            spec.input_shape = first.input.shape;
            validate_task(&spec, &experiment.request.goals.task)?;
            validate_recipe(&spec, &job.request)?;
            if dataset.validation.len() < experiment.request.goals.minimum_audited_samples {
                return Err(invalid(
                    "validation partition has fewer samples than the audited minimum",
                ));
            }
            for sample in &dataset.validation {
                sealed_sample(sample, &spec, dataset.as_of_ms, true)?;
            }
            let artifact = if job.status == JobStatus::Succeeded {
                self.repository.get_artifact(
                    job.artifact_id
                        .as_deref()
                        .ok_or_else(|| invalid("succeeded candidate has no artifact"))?,
                )?
            } else if matches!(
                job.status,
                JobStatus::Queued | JobStatus::Paused | JobStatus::Interrupted | JobStatus::Running
            ) {
                self.worker.run(&job.id)?
            } else {
                return Err(Error::Engine(format!(
                    "candidate job is {:?}: {}",
                    job.status,
                    job.error.unwrap_or_default()
                )));
            };
            let compute = inference_compute(&job.request.compute, &experiment)?;
            let mut predictor = self
                .predictors
                .load(&self.repository, &artifact.id, &compute)?;
            let scale = if matches!(
                experiment.request.goals.task,
                EvaluationTask::Anomaly { .. }
            ) {
                self.anomaly_scale(experiment_id, &mut *predictor, &dataset, &spec)?
            } else {
                1.0
            };
            let observations = self.observe(
                experiment_id,
                &mut *predictor,
                &dataset.validation,
                &spec,
                dataset.as_of_ms,
                &experiment.request.goals.task,
                scale,
            )?;
            let mut metrics = measure(&observations, &experiment.request.goals.task, &spec.labels)?;
            metrics.insert("artifact_bytes".into(), artifact.blob.bytes as f64);
            if !metrics.contains_key(&experiment.request.goals.primary_metric) {
                return Err(invalid(
                    "primary metric is undefined for the validation labels or task",
                ));
            }
            if matches!(
                experiment.request.goals.task,
                EvaluationTask::Anomaly { .. }
            ) {
                metrics.insert("anomaly_scale".into(), scale);
            }
            Ok((artifact, metrics))
        })();
        let elapsed = if recovering_completed {
            trial.reserved_training_time_ms
        } else {
            started.elapsed().as_millis().min(u64::MAX as u128) as u64
        };
        match result {
            Ok((_, metrics)) => {
                self.repository.record_experiment_trial_validation(
                    &trial.id,
                    &metrics,
                    elapsed,
                    now_ms(),
                )?;
            }
            Err(Error::Conflict(_) | Error::LeaseLost) => {
                return self.repository.get_experiment(experiment_id);
            }
            Err(error) => {
                self.repository.fail_experiment_trial(
                    &trial.id,
                    &error.to_string(),
                    elapsed,
                    now_ms(),
                )?;
            }
        }
        let updated = self.repository.get_experiment(experiment_id)?;
        if updated.status == ExperimentStatus::Running && wall_budget_exhausted(&updated) {
            return self.repository.stop_experiment_budget(
                experiment_id,
                "wall_time_budget_exhausted",
                now_ms(),
            );
        }
        Ok(updated)
    }

    /// Run the fixed candidate list, then evaluate one frozen winner. Hosts can call run_next
    /// instead to inspect validation results and append additional bounded candidates.
    pub fn run(&self, experiment_id: &str) -> Result<Experiment> {
        loop {
            let before = self.repository.get_experiment(experiment_id)?;
            if matches!(
                before.status,
                ExperimentStatus::Completed
                    | ExperimentStatus::Cancelled
                    | ExperimentStatus::Failed
                    | ExperimentStatus::CancelRequested
            ) {
                return Ok(before);
            }
            if before.selected_trial_id.is_some() {
                return self.finalize(experiment_id);
            }
            let after = self.run_next(experiment_id)?;
            if after.generation == before.generation {
                if self
                    .repository
                    .list_experiment_trials(experiment_id)?
                    .iter()
                    .any(|t| t.status == ExperimentTrialStatus::Scheduled)
                {
                    return Ok(after);
                }
                return self.finalize(experiment_id);
            }
        }
    }

    /// Freeze the validation winner before opening its final test partition. Repeated calls
    /// reuse durable prediction evidence and the final evaluation, never select another winner.
    pub fn finalize(&self, experiment_id: &str) -> Result<Experiment> {
        let _lock =
            match LocalRunLock::acquire(&self.repository, &format!("experiment:{experiment_id}")) {
                Ok(lock) => lock,
                Err(Error::Conflict(_)) => return self.repository.get_experiment(experiment_id),
                Err(error) => return Err(error),
            };
        let experiment = self.repository.get_experiment(experiment_id)?;
        if experiment.final_evaluation_id.is_some()
            || matches!(
                experiment.status,
                ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
            )
        {
            return Ok(experiment);
        }
        if experiment.best_trial_id.is_none() {
            return self.repository.finalize_experiment(
                experiment_id,
                None,
                "no_candidate_met_validation_requirements",
                now_ms(),
            );
        }
        let experiment = self
            .repository
            .select_experiment_winner(experiment_id, now_ms())?;
        if wall_budget_exhausted(&experiment) {
            return self.repository.finalize_experiment(
                experiment_id,
                None,
                "wall_time_budget_exhausted",
                now_ms(),
            );
        }
        let trial = self.repository.get_experiment_trial(
            experiment
                .selected_trial_id
                .as_deref()
                .ok_or_else(|| invalid("winner was not frozen"))?,
        )?;
        let artifact_id = trial
            .artifact_id
            .as_deref()
            .ok_or_else(|| invalid("selected trial has no model artifact"))?;
        let dataset = self
            .repository
            .get_snapshot_limited(&trial.dataset_snapshot_id, snapshot_limit(&experiment))?;
        let mut spec: InspectionSpec = serde_json::from_value(experiment.request.spec.clone())?;
        let first: Sample = serde_json::from_value(
            dataset
                .train
                .first()
                .ok_or_else(|| invalid("empty training partition"))?
                .payload
                .get("sample")
                .unwrap_or(&dataset.train[0].payload)
                .clone(),
        )?;
        spec.input_shape = first.input.shape;
        validate_task(&spec, &experiment.request.goals.task)?;
        if dataset.test.len() < experiment.request.goals.minimum_audited_samples {
            return Err(invalid(
                "final test partition has fewer samples than the audited minimum",
            ));
        }
        let job = self.repository.get_job(&trial.job_id)?;
        let compute = inference_compute(&job.request.compute, &experiment)?;
        let mut predictor = self
            .predictors
            .load(&self.repository, artifact_id, &compute)?;
        let recorded = self.repository.predictions(artifact_id)?;
        let scale = trial
            .validation_metrics
            .get("anomaly_scale")
            .copied()
            .unwrap_or(1.0);
        for row in &dataset.test {
            if wall_budget_exhausted(&experiment) {
                return self.repository.finalize_experiment(
                    experiment_id,
                    None,
                    "wall_time_budget_exhausted",
                    now_ms(),
                );
            }
            self.check_cancel(experiment_id)?;
            if recorded.iter().any(|p| p.sample_id == row.id) {
                continue;
            }
            let sample = sealed_sample(row, &spec, dataset.as_of_ms, true)?;
            let started = Instant::now();
            let raw = predict_sample(&mut *predictor, &sample)?;
            let observation = decode_prediction(
                sample,
                &raw,
                &experiment.request.goals.task,
                spec.labels.len(),
                scale,
                started.elapsed().as_secs_f64() * 1000.0,
            )?;
            let actual_label = if let Annotation::Class { class_id } = observation.sample.annotation
            {
                let expected = spec
                    .labels
                    .get(class_id as usize)
                    .ok_or_else(|| invalid("test class outside label order"))?;
                if row.payload.get("label").and_then(Value::as_str) != Some(expected.as_str()) {
                    return Err(invalid(
                        "test ledger label differs from the inspection class order",
                    ));
                }
                Some(expected.clone())
            } else {
                None
            };
            self.repository.record_prediction(&PredictionRecord {
                sample_id:row.id.clone(),artifact_id:artifact_id.into(),predicted_label:observation.predicted_class.map(|class|spec.labels[class as usize].clone()),
                teacher_label:None,actual_label,actual_source:Some(row.source),latency_ms:observation.latency_ms,failed:false,recorded_at_ms:now_ms(),
                details:serde_json::json!({"prediction":observation.evidence,"experiment_id":experiment_id,"partition":"final_test"}),
            })?;
        }
        let evaluation = self.repository.evaluate_task(
            artifact_id,
            experiment.request.goals.task.clone(),
            now_ms(),
        )?;
        self.repository.finalize_experiment(
            experiment_id,
            Some(&evaluation.id),
            experiment
                .stop_reason
                .as_deref()
                .unwrap_or("candidate_search_completed"),
            now_ms(),
        )
    }

    fn check_cancel(&self, experiment_id: &str) -> Result<()> {
        let experiment = self.repository.get_experiment(experiment_id)?;
        if matches!(
            experiment.status,
            ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
        ) {
            Err(Error::Cancelled)
        } else if wall_budget_exhausted(&experiment) {
            Err(Error::Engine(
                "experiment wall time budget exhausted".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn next_halving_trial(
        &self,
        experiment: &Experiment,
        trials: &[ExperimentTrial],
    ) -> Result<Option<ExperimentTrial>> {
        let mut rungs = Vec::new();
        for trial in trials {
            let request = &self.repository.get_job(&trial.job_id)?.request;
            if request.engine != "burn" {
                continue;
            }
            let Some(search) = request.recipe.get("search") else {
                continue;
            };
            let factor = search["reduction_factor"]
                .as_u64()
                .ok_or_else(|| invalid("invalid halving reduction factor"))?
                as usize;
            if !(2..=8).contains(&factor) {
                return Err(invalid("halving reduction factor must be 2..8"));
            }
            let mut config = request.recipe["config"].clone();
            config
                .as_object_mut()
                .ok_or_else(|| invalid("missing trial config"))?
                .remove("epochs");
            let family = digest(&serde_json::to_vec(&serde_json::json!({
                "config": config,
                "compute": request.compute,
                "dataset": trial.dataset_snapshot_id,
                "maximum": search["max_epochs"],
                "factor": factor,
            }))?);
            rungs.push(HalvingCandidate {
                trial: trial.clone(),
                rung: search["rung"].as_u64().unwrap_or(0),
                factor,
                epochs: request.recipe["config"]["epochs"]
                    .as_u64()
                    .ok_or_else(|| invalid("missing trial epochs"))?
                    as usize,
                maximum: search["max_epochs"]
                    .as_u64()
                    .ok_or_else(|| invalid("missing maximum epochs"))?
                    as usize,
                parent: search["parent_trial_id"].as_str().map(str::to_owned),
                family,
            });
        }
        let Some(highest) = rungs.iter().map(|v| v.rung).max() else {
            return Ok(None);
        };
        // Finish every selected survivor of the previous rung before pruning the current one.
        for rung in [highest.checked_sub(1), Some(highest)]
            .into_iter()
            .flatten()
        {
            let mut parents = rungs
                .iter()
                .filter(|v| v.rung == rung && v.trial.status == ExperimentTrialStatus::Validated)
                .collect::<Vec<_>>();
            if parents.is_empty() {
                continue;
            }
            parents.sort_by(|a, b| rank_candidates(&a.trial, &b.trial, &experiment.request.goals));
            let mut families = HashSet::new();
            parents.retain(|parent| families.insert(parent.family.as_str()));
            let survivors = parents.len().div_ceil(parents[0].factor);
            for parent in parents.into_iter().take(survivors) {
                if parent.epochs >= parent.maximum
                    || rungs.iter().any(|child| {
                        child.parent.as_deref() == Some(parent.trial.id.as_str())
                            || (child.rung == parent.rung + 1 && child.family == parent.family)
                    })
                {
                    continue;
                }
                let epochs = parent
                    .epochs
                    .saturating_mul(parent.factor)
                    .min(parent.maximum);
                return self
                    .repository
                    .continue_experiment_trial(&parent.trial.id, epochs, now_ms())
                    .map(Some);
            }
        }
        Ok(None)
    }

    fn anomaly_scale(
        &self,
        id: &str,
        predictor: &mut dyn SearchPredictor,
        dataset: &DatasetSnapshot,
        spec: &InspectionSpec,
    ) -> Result<f64> {
        let mut scores = Vec::new();
        for row in &dataset.train {
            self.check_cancel(id)?;
            let sample = sealed_sample(row, spec, dataset.as_of_ms, false)?;
            if matches!(sample.annotation, Annotation::Anomaly { is_anomaly: false }) {
                scores.push(scalar_score(&predict_sample(predictor, &sample)?)?);
                if scores.len() == 256 {
                    break;
                }
            }
        }
        if scores.is_empty() {
            return Err(invalid(
                "anomaly calibration requires explicitly normal training examples",
            ));
        }
        scores.sort_by(f64::total_cmp);
        Ok(scores[((scores.len() - 1) as f64 * 0.95).floor() as usize].max(1e-8))
    }

    fn observe(
        &self,
        id: &str,
        predictor: &mut dyn SearchPredictor,
        rows: &[TrainingSample],
        spec: &InspectionSpec,
        at_ms: i64,
        task: &EvaluationTask,
        scale: f64,
    ) -> Result<Vec<Observation>> {
        let mut observations = Vec::with_capacity(rows.len());
        for row in rows {
            self.check_cancel(id)?;
            let sample = sealed_sample(row, spec, at_ms, true)?;
            let started = Instant::now();
            let raw = predict_sample(predictor, &sample)?;
            let mut observation = decode_prediction(
                sample,
                &raw,
                task,
                spec.labels.len(),
                scale,
                started.elapsed().as_secs_f64() * 1000.0,
            )?;
            observation.sample.input.values = Vec::new();
            observations.push(observation);
        }
        Ok(observations)
    }
}

fn wall_budget_exhausted(experiment: &Experiment) -> bool {
    now_ms() as i128 - experiment.created_at_ms as i128
        > experiment.request.budget.maximum_wall_time_ms as i128
}

struct HalvingCandidate {
    trial: ExperimentTrial,
    rung: u64,
    factor: usize,
    epochs: usize,
    maximum: usize,
    parent: Option<String>,
    family: String,
}

fn rank_candidates(
    a: &ExperimentTrial,
    b: &ExperimentTrial,
    goals: &ExperimentGoals,
) -> std::cmp::Ordering {
    let eligible = |trial: &ExperimentTrial| {
        trial
            .validation_metrics
            .get("audited_samples")
            .is_some_and(|samples| *samples >= goals.minimum_audited_samples as f64)
            && goals.bounds.iter().all(|bound| {
                trial
                    .validation_metrics
                    .get(&bound.name)
                    .is_some_and(|value| {
                        value.is_finite()
                            && bound.minimum.is_none_or(|minimum| *value >= minimum)
                            && bound.maximum.is_none_or(|maximum| *value <= maximum)
                    })
            })
    };
    eligible(b)
        .cmp(&eligible(a))
        .then_with(|| {
            let a = a
                .validation_metrics
                .get(&goals.primary_metric)
                .copied()
                .unwrap_or(f64::NAN);
            let b = b
                .validation_metrics
                .get(&goals.primary_metric)
                .copied()
                .unwrap_or(f64::NAN);
            match goals.direction {
                MetricDirection::Maximize => b.total_cmp(&a),
                MetricDirection::Minimize => a.total_cmp(&b),
            }
        })
        .then(a.candidate_index.cmp(&b.candidate_index))
}

fn inference_compute(request: &Value, experiment: &Experiment) -> Result<ComputeConfig> {
    let mut compute: ComputeConfig = serde_json::from_value(request.clone())?;
    compute.memory_limit_bytes = compute
        .memory_limit_bytes
        .min(experiment.request.budget.worker_limits.memory_budget_bytes);
    compute.validate().map_err(engine_error)?;
    Ok(compute)
}

fn snapshot_limit(experiment: &Experiment) -> u64 {
    experiment
        .request
        .budget
        .maximum_dataset_bytes
        .min(experiment.request.budget.worker_limits.memory_budget_bytes / 8)
}

fn validate_task(spec: &InspectionSpec, task: &EvaluationTask) -> Result<()> {
    let compatible = match (&default_goals(spec).task, task) {
        (EvaluationTask::Classification, EvaluationTask::Classification)
        | (EvaluationTask::Regression, EvaluationTask::Regression)
        | (EvaluationTask::Anomaly { .. }, EvaluationTask::Anomaly { .. }) => true,
        (
            EvaluationTask::Detection { classes: a, .. },
            EvaluationTask::Detection { classes: b, .. },
        )
        | (
            EvaluationTask::Segmentation { classes: a },
            EvaluationTask::Segmentation { classes: b },
        )
        | (
            EvaluationTask::InstanceSegmentation { classes: a, .. },
            EvaluationTask::InstanceSegmentation { classes: b, .. },
        ) => a == b,
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(invalid("evaluation task differs from the inspection task"))
    }
}

fn validate_recipe(spec: &InspectionSpec, request: &TrainingRequest) -> Result<()> {
    if request.engine == "burn" {
        let config: burn::TrainingConfig =
            serde_json::from_value(request.recipe["config"].clone())?;
        burn::validate_inspection_config(spec, &config).map_err(engine_error)?;
    }
    let labels: Vec<String> = serde_json::from_value(
        request
            .recipe
            .get("labels")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    if labels != spec.labels {
        return Err(invalid("candidate label order differs from the inspection"));
    }
    if request.recipe.get("inspection_task").is_some() {
        let task: TaskKind = serde_json::from_value(request.recipe["inspection_task"].clone())?;
        if task != spec.task {
            return Err(invalid("candidate task differs from the experiment"));
        }
    }
    Ok(())
}

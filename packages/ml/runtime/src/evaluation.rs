use crate::repository::{evaluation_truth_digest, validate_prediction_truth};
use crate::*;
use flow_like_ml_core::{Annotation, BoundingBox, LabelProvenance, Sample};
use flow_like_ml_native::evaluation as metrics;
use rusqlite::{TransactionBehavior, params};

impl TrainingRepository {
    /// Task metrics are computed from recorded predictions and accepted audit annotations.
    /// Caller-supplied metric numbers are never registered as promotion evidence.
    pub fn evaluate_task(
        &self,
        artifact_id: &str,
        task: EvaluationTask,
        at_ms: i64,
    ) -> Result<EvaluationReport> {
        self.evaluate_task_cohort(artifact_id, task, None, at_ms)
    }

    pub(crate) fn evaluate_task_cohort(
        &self,
        artifact_id: &str,
        task: EvaluationTask,
        cohort: Option<&std::collections::BTreeSet<String>>,
        at_ms: i64,
    ) -> Result<EvaluationReport> {
        self.writable()?;
        if task == EvaluationTask::Classification {
            return self.evaluate_classification_cohort(artifact_id, cohort, at_ms);
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let artifact: ModelArtifact = serde_json::from_str(&tx.query_row(
            "SELECT body FROM artifacts WHERE id=?",
            [artifact_id],
            |r| r.get::<_, String>(0),
        )?)?;
        let snapshot: DatasetSnapshot = serde_json::from_str(&tx.query_row(
            "SELECT body FROM snapshots WHERE id=?",
            [&artifact.snapshot_id],
            |r| r.get::<_, String>(0),
        )?)?;
        let mut query =
            tx.prepare("SELECT body FROM predictions WHERE artifact_id=? ORDER BY sample_id")?;
        let mut predictions: Vec<PredictionRecord> = query
            .query_map([artifact_id], |r| r.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<_>>()?;
        drop(query);
        if let Some(cohort) = cohort {
            predictions.retain(|p| cohort.contains(&p.sample_id));
            if predictions.len() != cohort.len() {
                return Err(invalid(
                    "model predictions do not cover the complete audit cohort",
                ));
            }
        }
        if predictions
            .iter()
            .any(|prediction| prediction.recorded_at_ms > at_ms)
        {
            return Err(invalid(
                "prediction evidence was not available at evaluation time",
            ));
        }
        let consumed_groups: std::collections::HashSet<_> = snapshot
            .train
            .iter()
            .chain(&snapshot.validation)
            .map(|sample| sample.group_id.as_str())
            .collect();
        let classes = match task {
            EvaluationTask::Detection { classes, .. }
            | EvaluationTask::Segmentation { classes }
            | EvaluationTask::InstanceSegmentation { classes, .. } => classes,
            EvaluationTask::Events { .. } | EvaluationTask::Anomaly { .. } => 2,
            _ => 0,
        };
        let mut audited = Vec::new();
        for prediction in &predictions {
            if !matches!(
                prediction.actual_source,
                Some(LabelSource::Reviewed | LabelSource::ObservedOutcome)
            ) {
                continue;
            }
            let recorded = validate_prediction_truth(&tx, &artifact.stream, prediction)?;
            if consumed_groups.contains(recorded.group_id.as_str()) {
                return Err(invalid("audited sample shares a training/validation group"));
            }
            if recorded.label_available_at_ms > at_ms {
                return Err(invalid("recorded outcome was not yet available"));
            }
            let sample: Sample = serde_json::from_value(
                recorded
                    .payload
                    .get("sample")
                    .unwrap_or(&recorded.payload)
                    .clone(),
            )?;
            sample
                .validate(classes, 64 * 1024 * 1024)
                .map_err(|e| invalid(e.to_string()))?;
            if sample.id != recorded.id
                || sample.group_id != recorded.group_id
                || sample.stream_id != artifact.stream.stream_id
            {
                return Err(invalid(
                    "evaluation payload identity differs from artifact stream",
                ));
            }
            if !matches!(
                sample.provenance,
                LabelProvenance::Reviewed { .. } | LabelProvenance::Measured { .. }
            ) {
                return Err(invalid(
                    "audited metrics require reviewed or measured annotations",
                ));
            }
            let evidence: EvidencePrediction = serde_json::from_value(
                prediction
                    .details
                    .get("prediction")
                    .unwrap_or(&prediction.details)
                    .clone(),
            )?;
            audited.push((sample, evidence, prediction));
        }
        if audited.is_empty() {
            return Err(invalid("no independently audited prediction evidence"));
        }
        let mut values = BTreeMap::new();
        values.insert("artifact_bytes".into(), artifact.blob.bytes as f64);
        let mut per_class_recall = BTreeMap::new();
        values.insert(
            "failure_rate".into(),
            audited.iter().filter(|(_, _, p)| p.failed).count() as f64 / audited.len() as f64,
        );
        values.insert(
            "mean_latency_ms".into(),
            audited.iter().map(|(_, _, p)| p.latency_ms).sum::<f64>() / audited.len() as f64,
        );
        match &task {
            EvaluationTask::Anomaly { threshold } => {
                if !threshold.is_finite() || !(0.0..=1.0).contains(threshold) {
                    return Err(invalid("anomaly threshold must be a probability in [0,1]"));
                }
                let mut actual = Vec::with_capacity(audited.len());
                let mut probabilities = Vec::with_capacity(audited.len());
                for (sample, evidence, record) in &audited {
                    if record.failed {
                        return Err(invalid("anomaly evidence contains failed predictions"));
                    }
                    let Annotation::Anomaly { is_anomaly } = sample.annotation else {
                        return Err(invalid(
                            "anomaly truth requires boolean anomaly annotations",
                        ));
                    };
                    let EvidencePrediction::Anomaly { probability } = evidence else {
                        return Err(invalid(
                            "prediction evidence is not a calibrated anomaly probability",
                        ));
                    };
                    actual.push(is_anomaly);
                    probabilities.push(*probability);
                }
                let curve =
                    metrics::binary_curve(&actual, &probabilities).map_err(engine_invalid)?;
                let truth: Vec<_> = actual.iter().map(|value| u32::from(*value)).collect();
                let predicted: Vec<_> = probabilities
                    .iter()
                    .map(|value| u32::from(value >= threshold))
                    .collect();
                let classification = metrics::classification_metrics(&truth, &predicted, 2)
                    .map_err(engine_invalid)?;
                values.insert("accuracy".into(), classification.accuracy);
                values.insert(
                    "accuracy_wilson_lower_95".into(),
                    classification.accuracy_interval_95.lower,
                );
                values.insert("brier_score".into(), curve.brier_score);
                values.insert("log_loss".into(), curve.log_loss);
                if let Some(value) = curve.roc_auc {
                    values.insert("auroc".into(), value);
                }
                if let Some(value) = curve.average_precision {
                    values.insert("average_precision".into(), value);
                }
                let anomaly = &classification.per_class[1];
                if let Some(value) = anomaly.precision {
                    values.insert("precision".into(), value);
                }
                if let Some(value) = anomaly.recall {
                    values.insert("recall".into(), value);
                }
                let normal = &classification.per_class[0];
                if normal.support > 0 {
                    values.insert(
                        "false_positive_rate".into(),
                        anomaly.false_positive as f64 / normal.support as f64,
                    );
                }
                for (name, class) in [("normal", normal), ("anomaly", anomaly)] {
                    values.insert(format!("support/{name}"), class.support as f64);
                    if let Some(value) = class.recall {
                        per_class_recall.insert(name.into(), value);
                    }
                }
            }
            EvaluationTask::Regression => {
                let mut actual = Vec::new();
                let mut predicted = Vec::new();
                for (sample, evidence, record) in &audited {
                    if record.failed {
                        return Err(invalid("regression evidence contains failed predictions"));
                    }
                    let target = match &sample.annotation {
                        Annotation::Scalar { value } => vec![*value],
                        Annotation::Values { values } => values.iter().map(|v| *v as f64).collect(),
                        _ => {
                            return Err(invalid(
                                "regression truth requires scalar/dense annotations",
                            ));
                        }
                    };
                    let EvidencePrediction::Regression { values: prediction } = evidence else {
                        return Err(invalid("prediction evidence is not regression"));
                    };
                    if target.len() != prediction.len() {
                        return Err(invalid(
                            "regression target and prediction dimensions differ",
                        ));
                    }
                    actual.extend(target);
                    predicted.extend(prediction);
                }
                let result = metrics::regression_metrics(&actual, &predicted)
                    .map_err(|e| invalid(e.to_string()))?;
                values.insert("mae".into(), result.mae);
                values.insert("rmse".into(), result.rmse);
                if let Some(r2) = result.r2 {
                    values.insert("r2".into(), r2);
                }
            }
            EvaluationTask::Detection {
                classes,
                iou_threshold,
            } => {
                let mut actual = Vec::new();
                let mut predicted = Vec::new();
                for (sample, evidence, record) in &audited {
                    let Annotation::Boxes { boxes } = &sample.annotation else {
                        return Err(invalid("detection truth requires box annotations"));
                    };
                    let EvidencePrediction::Detection { boxes: found } = evidence else {
                        return Err(invalid("prediction evidence is not detection"));
                    };
                    actual.push(boxes.clone());
                    predicted.push(if record.failed {
                        Vec::new()
                    } else {
                        found
                            .iter()
                            .map(|box_| metrics::ScoredBox {
                                bounds: BoundingBox {
                                    class_id: box_.class_id,
                                    x_min: box_.x_min,
                                    y_min: box_.y_min,
                                    x_max: box_.x_max,
                                    y_max: box_.y_max,
                                },
                                confidence: box_.confidence,
                            })
                            .collect()
                    });
                }
                let result =
                    metrics::detection_metrics(&actual, &predicted, *classes, *iou_threshold)
                        .map_err(|e| invalid(e.to_string()))?;
                if let Some(ap) = result.mean_average_precision {
                    values.insert("mean_average_precision".into(), ap);
                }
                for class in result.per_class {
                    values.insert(format!("support/{}", class.class_id), class.actual as f64);
                    if let Some(value) = class.recall {
                        values.insert(format!("recall/{}", class.class_id), value);
                        per_class_recall.insert(class.class_id.to_string(), value);
                    }
                    if let Some(value) = class.precision {
                        values.insert(format!("precision/{}", class.class_id), value);
                    }
                    if let Some(value) = class.average_precision {
                        values.insert(format!("average_precision/{}", class.class_id), value);
                    }
                }
            }
            EvaluationTask::Segmentation { classes } => {
                let mut actual = Vec::new();
                let mut predicted = Vec::new();
                for (sample, evidence, record) in &audited {
                    if record.failed {
                        return Err(invalid("segmentation evidence contains failed predictions"));
                    }
                    let Annotation::Mask {
                        width,
                        height,
                        classes: truth,
                    } = &sample.annotation
                    else {
                        return Err(invalid("segmentation truth requires dense masks"));
                    };
                    let EvidencePrediction::Segmentation {
                        width: pw,
                        height: ph,
                        classes: found,
                    } = evidence
                    else {
                        return Err(invalid("prediction evidence is not segmentation"));
                    };
                    if width != pw || height != ph || truth.len() != found.len() {
                        return Err(invalid(
                            "segmentation prediction dimensions differ from truth",
                        ));
                    }
                    actual.extend(truth);
                    predicted.extend(found);
                }
                let result = metrics::segmentation_metrics(&actual, &predicted, *classes)
                    .map_err(|e| invalid(e.to_string()))?;
                values.insert("mean_iou".into(), result.mean_iou);
                values.insert("mean_dice".into(), result.mean_dice);
                for (class, value) in result.per_class_iou.into_iter().enumerate() {
                    if let Some(value) = value {
                        values.insert(format!("iou/{class}"), value);
                    }
                }
                for (class, value) in result.per_class_dice.into_iter().enumerate() {
                    if let Some(value) = value {
                        values.insert(format!("dice/{class}"), value);
                    }
                }
            }
            EvaluationTask::Events { horizon_ms } => {
                let mut events = std::collections::BTreeSet::new();
                let mut alarms = std::collections::BTreeSet::new();
                let mut coverage = Vec::new();
                for (sample, evidence, record) in &audited {
                    let positive = match sample.annotation {
                        Annotation::Class { class_id } => class_id == 1,
                        Annotation::Anomaly { is_anomaly } => is_anomaly,
                        _ => {
                            return Err(invalid(
                                "event truth requires binary class or anomaly annotation",
                            ));
                        }
                    };
                    if positive {
                        let outcome = sample.outcome.as_ref().ok_or_else(|| {
                            invalid("positive events require an observed outcome interval")
                        })?;
                        if outcome.available_at_ms > at_ms {
                            return Err(invalid("event outcome was unavailable when evaluated"));
                        }
                        events.insert((outcome.target_start_ms, outcome.target_end_ms));
                    }
                    let EvidencePrediction::Events { alarms_ms } = evidence else {
                        return Err(invalid("prediction evidence is not events"));
                    };
                    if !record.failed {
                        for alarm in alarms_ms {
                            if *alarm < sample.window_start_ms || *alarm > sample.window_end_ms {
                                return Err(invalid(
                                    "alarm lies outside its observed sensor window",
                                ));
                            }
                            alarms.insert(*alarm);
                        }
                    }
                    coverage.push((sample.window_start_ms, sample.window_end_ms));
                }
                let observed = coverage_duration(coverage)?;
                let events = events
                    .into_iter()
                    .map(|(start_ms, end_ms)| metrics::EventInterval { start_ms, end_ms })
                    .collect::<Vec<_>>();
                let result = metrics::event_metrics(
                    &events,
                    &alarms.into_iter().collect::<Vec<_>>(),
                    *horizon_ms,
                    observed,
                )
                .map_err(|e| invalid(e.to_string()))?;
                values.insert("events".into(), result.events as f64);
                values.insert("detected_events".into(), result.detected as f64);
                values.insert("missed_events".into(), result.missed as f64);
                values.insert("false_alarms_per_hour".into(), result.false_alarms_per_hour);
                values.insert("observed_duration_ms".into(), observed as f64);
                if let Some(recall) = result.recall {
                    values.insert("event_recall".into(), recall);
                }
                if let Some(lead) = result.mean_lead_time_ms {
                    values.insert("mean_lead_time_ms".into(), lead);
                }
            }
            EvaluationTask::InstanceSegmentation {
                classes,
                iou_threshold,
            } => {
                let mut actual = Vec::with_capacity(audited.len());
                let mut predicted = Vec::with_capacity(audited.len());
                for (sample, evidence, record) in &audited {
                    let Annotation::InstanceMasks { instances } = &sample.annotation else {
                        return Err(invalid(
                            "instance segmentation truth requires reviewed instance masks",
                        ));
                    };
                    let EvidencePrediction::Instances { instances: found } = evidence else {
                        return Err(invalid("prediction evidence is not instance masks"));
                    };
                    let shape = &sample.input.shape;
                    if shape.len() < 2 {
                        return Err(invalid("instance segmentation requires image dimensions"));
                    }
                    let (height, width) = (shape[shape.len() - 2], shape[shape.len() - 1]);
                    for instance in instances {
                        if instance.width != width
                            || instance.height != height
                            || !instance.foreground.iter().any(|value| *value)
                        {
                            return Err(invalid(
                                "reviewed instance masks must be nonempty full-image grids",
                            ));
                        }
                    }
                    let mut elements = 0usize;
                    for instance in found {
                        elements = elements
                            .checked_add(instance.foreground.len())
                            .ok_or_else(|| invalid("instance mask size overflows"))?;
                        if instance.width != width
                            || instance.height != height
                            || width.checked_mul(height) != Some(instance.foreground.len())
                            || elements > 64 * 1024 * 1024
                            || instance.class_id as usize >= *classes
                            || !instance.confidence.is_finite()
                            || !(0.0..=1.0).contains(&instance.confidence)
                            || !instance.foreground.iter().any(|value| *value)
                        {
                            return Err(invalid(
                                "predicted instance masks require valid classes, probabilities and nonempty full-image grids",
                            ));
                        }
                    }
                    actual.push(instances.clone());
                    predicted.push(if record.failed {
                        Vec::new()
                    } else {
                        found.clone()
                    });
                }
                instance_metrics(
                    &actual,
                    &predicted,
                    *classes,
                    *iou_threshold,
                    &mut values,
                    &mut per_class_recall,
                )?;
            }
            EvaluationTask::Classification => unreachable!(),
        }
        if values.values().any(|value| !value.is_finite()) {
            return Err(invalid("metric computation produced a non-finite value"));
        }
        let report = EvaluationReport {
            id: id(),
            artifact_id: artifact.id,
            dataset_digest: artifact.dataset_digest,
            evidence_digest: digest(&serde_json::to_vec(&predictions)?),
            truth_digest: Some(evaluation_truth_digest(
                &tx,
                &artifact.stream,
                &predictions,
            )?),
            audited_samples: audited.len(),
            teacher_samples: predictions
                .iter()
                .filter(|p| p.teacher_label.is_some())
                .count(),
            metrics: values,
            per_class_recall,
            created_at_ms: at_ms,
            task,
        };
        tx.execute(
            "INSERT INTO evaluations(id,artifact_id,body) VALUES (?,?,?)",
            params![
                report.id,
                report.artifact_id,
                serde_json::to_string(&report)?
            ],
        )?;
        tx.commit()?;
        Ok(report)
    }
}

fn engine_invalid(error: impl std::fmt::Display) -> Error {
    invalid(error.to_string())
}

pub(crate) fn instance_metrics(
    actual: &[Vec<flow_like_ml_core::InstanceMask>],
    predicted: &[Vec<InstanceEvidence>],
    classes: usize,
    threshold: f64,
    values: &mut BTreeMap<String, f64>,
    recalls: &mut BTreeMap<String, f64>,
) -> Result<()> {
    if !(1..=4096).contains(&classes) || !threshold.is_finite() || threshold <= 0. || threshold > 1.
    {
        return Err(invalid(
            "instance metrics require 1..4096 classes and IoU in (0,1]",
        ));
    }
    let mut aps = Vec::new();
    let mut total_iou = 0.;
    let mut total_truth = 0usize;
    for class in 0..classes as u32 {
        let count = actual
            .iter()
            .flatten()
            .filter(|instance| instance.bounds.class_id == class)
            .count();
        let mut candidates: Vec<_> = predicted
            .iter()
            .enumerate()
            .flat_map(|(image, instances)| {
                instances
                    .iter()
                    .filter(move |instance| instance.class_id == class)
                    .map(move |instance| (image, instance))
            })
            .collect();
        candidates.sort_by(|a, b| b.1.confidence.total_cmp(&a.1.confidence));
        let mut used: Vec<_> = actual
            .iter()
            .map(|instances| vec![false; instances.len()])
            .collect();
        let mut matched = 0usize;
        let mut matched_iou = 0.;
        let mut curve = Vec::with_capacity(candidates.len());
        for (rank, (image, instance)) in candidates.iter().enumerate() {
            let best = actual[*image]
                .iter()
                .enumerate()
                .filter(|(index, truth)| truth.bounds.class_id == class && !used[*image][*index])
                .map(|(index, truth)| {
                    let mut intersection = 0usize;
                    let mut union = 0usize;
                    for (a, b) in truth.foreground.iter().zip(&instance.foreground) {
                        intersection += usize::from(*a && *b);
                        union += usize::from(*a || *b);
                    }
                    (index, intersection as f64 / union as f64)
                })
                .filter(|(_, iou)| *iou >= threshold)
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((index, iou)) = best {
                used[*image][index] = true;
                matched += 1;
                matched_iou += iou;
            }
            curve.push((
                if count > 0 {
                    matched as f64 / count as f64
                } else {
                    0.
                },
                matched as f64 / (rank + 1) as f64,
            ));
        }
        values.insert(format!("support/{class}"), count as f64);
        values.insert(format!("predicted/{class}"), candidates.len() as f64);
        if !candidates.is_empty() {
            values.insert(
                format!("precision/{class}"),
                matched as f64 / candidates.len() as f64,
            );
        }
        if count > 0 {
            let recall = matched as f64 / count as f64;
            values.insert(format!("recall/{class}"), recall);
            recalls.insert(class.to_string(), recall);
            let ap = (0..=100)
                .map(|level| {
                    curve
                        .iter()
                        .filter(|(recall, _)| *recall >= level as f64 / 100.)
                        .map(|(_, precision)| *precision)
                        .fold(0., f64::max)
                })
                .sum::<f64>()
                / 101.;
            values.insert(format!("mask_average_precision/{class}"), ap);
            // Missed audited objects contribute zero to the mean mask IoU.
            values.insert(format!("mask_iou/{class}"), matched_iou / count as f64);
            aps.push(ap);
            total_iou += matched_iou;
            total_truth += count;
        }
    }
    if !aps.is_empty() {
        values.insert(
            "mask_mean_average_precision".into(),
            aps.iter().sum::<f64>() / aps.len() as f64,
        );
        values.insert("mask_mean_iou".into(), total_iou / total_truth as f64);
    }
    Ok(())
}

fn coverage_duration(mut windows: Vec<(i64, i64)>) -> Result<u64> {
    windows.sort();
    let mut duration = 0i128;
    let mut current = None;
    for (start, end) in windows {
        if end <= start {
            continue;
        }
        match current {
            None => current = Some((start, end)),
            Some((left, right)) if start <= right => current = Some((left, right.max(end))),
            Some((left, right)) => {
                duration += right as i128 - left as i128;
                current = Some((start, end));
            }
        }
    }
    if let Some((left, right)) = current {
        duration += right as i128 - left as i128;
    }
    if duration <= 0 || duration > u64::MAX as i128 {
        return Err(invalid(
            "event metrics need positive observed sensor-window coverage",
        ));
    }
    Ok(duration as u64)
}

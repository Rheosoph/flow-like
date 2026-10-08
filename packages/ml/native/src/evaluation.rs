use flow_like_ml_core::{BoundingBox, Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalMetrics {
    pub mean_rank_error: f64,
    pub macro_rank_error: f64,
    pub quadratic_kappa: f64,
}

/// Rank distances use the caller's fixed class order. Absent actual classes do not
/// contribute to the macro average; perfect constant predictions have kappa one.
pub fn ordinal_metrics(
    actual: &[u32],
    predicted: &[u32],
    classes: usize,
) -> Result<OrdinalMetrics> {
    require(
        !actual.is_empty() && actual.len() == predicted.len() && (1..=4096).contains(&classes),
        "Ordinal metrics require aligned labels and a bounded class order",
    )?;
    let mut actual_count = vec![0usize; classes];
    let mut predicted_count = vec![0usize; classes];
    let mut class_error = vec![0.0; classes];
    let mut absolute = 0.0;
    let mut squared = 0.0;
    for (&a, &p) in actual.iter().zip(predicted) {
        require(
            (a as usize) < classes && (p as usize) < classes,
            "Ordinal class is outside the fixed order",
        )?;
        let error = a.abs_diff(p) as f64;
        actual_count[a as usize] += 1;
        predicted_count[p as usize] += 1;
        class_error[a as usize] += error;
        absolute += error;
        squared += error * error;
    }
    let samples = actual.len() as f64;
    let moments = |counts: &[usize]| {
        counts
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(sum, squares), (rank, count)| {
                (
                    sum + rank as f64 * *count as f64,
                    squares + (rank as f64).powi(2) * *count as f64,
                )
            })
    };
    let (a_sum, a_squares) = moments(&actual_count);
    let (p_sum, p_squares) = moments(&predicted_count);
    let expected = (a_squares + p_squares - 2.0 * a_sum * p_sum / samples).max(0.0);
    let present = actual_count.iter().filter(|n| **n > 0).count();
    let macro_error = actual_count
        .iter()
        .zip(class_error)
        .filter(|(n, _)| **n > 0)
        .map(|(n, error)| error / *n as f64)
        .sum::<f64>()
        / present as f64;
    Ok(OrdinalMetrics {
        mean_rank_error: absolute / samples,
        macro_rank_error: macro_error,
        quadratic_kappa: if expected == 0.0 {
            1.0
        } else {
            1.0 - squared / expected
        },
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Interval {
    pub lower: f64,
    pub upper: f64,
}

pub fn wilson_interval(successes: u64, trials: u64, z: f64) -> Result<Option<Interval>> {
    require(
        successes <= trials && z.is_finite() && z > 0.0,
        "Invalid binomial count or confidence multiplier",
    )?;
    if trials == 0 {
        return Ok(None);
    }
    let n = trials as f64;
    let p = successes as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    Ok(Some(Interval {
        lower: (center - half).max(0.0),
        upper: (center + half).min(1.0),
    }))
}
fn ratio(n: u64, d: u64) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClassMetrics {
    pub class_id: u32,
    pub support: u64,
    pub true_positive: u64,
    pub false_positive: u64,
    pub false_negative: u64,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub f1: Option<f64>,
    pub recall_interval_95: Option<Interval>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClassificationMetrics {
    pub samples: u64,
    pub confusion: Vec<Vec<u64>>,
    pub accuracy: f64,
    pub accuracy_interval_95: Interval,
    pub macro_f1: f64,
    pub weighted_f1: f64,
    pub balanced_accuracy: f64,
    pub per_class: Vec<ClassMetrics>,
}

pub fn classification_metrics(
    actual: &[u32],
    predicted: &[u32],
    classes: usize,
) -> Result<ClassificationMetrics> {
    require(
        !actual.is_empty() && actual.len() == predicted.len() && (1..=4096).contains(&classes),
        "Classification needs aligned nonempty labels and 1..4096 classes",
    )?;
    require(
        actual
            .iter()
            .chain(predicted)
            .all(|v| (*v as usize) < classes),
        "Class id is outside the label schema",
    )?;
    let mut confusion = vec![vec![0u64; classes]; classes];
    for (a, p) in actual.iter().zip(predicted) {
        confusion[*a as usize][*p as usize] += 1;
    }
    let mut per_class = Vec::with_capacity(classes);
    let mut correct = 0;
    let mut supported = 0;
    let mut recall_total = 0.0;
    for class in 0..classes {
        let tp = confusion[class][class];
        let support = confusion[class].iter().sum::<u64>();
        let fp = confusion.iter().map(|row| row[class]).sum::<u64>() - tp;
        let fn_ = support - tp;
        correct += tp;
        let recall = ratio(tp, support);
        if let Some(r) = recall {
            recall_total += r;
            supported += 1;
        }
        per_class.push(ClassMetrics {
            class_id: class as u32,
            support,
            true_positive: tp,
            false_positive: fp,
            false_negative: fn_,
            precision: ratio(tp, tp + fp),
            recall,
            f1: ratio(2 * tp, 2 * tp + fp + fn_),
            recall_interval_95: wilson_interval(tp, support, 1.959963984540054)?,
        });
    }
    let count = actual.len() as u64;
    Ok(ClassificationMetrics {
        samples: count,
        confusion,
        accuracy: correct as f64 / count as f64,
        accuracy_interval_95: wilson_interval(correct, count, 1.959963984540054)?.unwrap(),
        macro_f1: per_class.iter().map(|c| c.f1.unwrap_or(0.0)).sum::<f64>() / classes as f64,
        weighted_f1: per_class
            .iter()
            .map(|c| c.f1.unwrap_or(0.0) * c.support as f64)
            .sum::<f64>()
            / count as f64,
        balanced_accuracy: recall_total / supported as f64,
        per_class,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CurvePoint {
    pub threshold: Option<f64>,
    pub precision: f64,
    pub recall: f64,
    pub false_positive_rate: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct BinaryCurve {
    pub roc_auc: Option<f64>,
    pub average_precision: Option<f64>,
    pub log_loss: f64,
    pub brier_score: f64,
    pub points: Vec<CurvePoint>,
}

pub fn binary_curve(actual: &[bool], scores: &[f64]) -> Result<BinaryCurve> {
    require(
        !actual.is_empty() && actual.len() == scores.len(),
        "Binary metrics need aligned nonempty labels and scores",
    )?;
    require(
        scores
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "Binary probabilities must be in [0,1]",
    )?;
    let positives = actual.iter().filter(|v| **v).count();
    let negatives = actual.len() - positives;
    let mut order: Vec<_> = (0..scores.len()).collect();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    let (mut tp, mut fp) = (0usize, 0usize);
    let mut points = vec![CurvePoint {
        threshold: None,
        precision: 1.0,
        recall: 0.0,
        false_positive_rate: 0.0,
    }];
    let mut index = 0;
    let (mut auc, mut ap, mut previous_tpr, mut previous_fpr) = (0.0, 0.0, 0.0, 0.0);
    while index < order.len() {
        let score = scores[order[index]];
        while index < order.len() && scores[order[index]] == score {
            if actual[order[index]] {
                tp += 1
            } else {
                fp += 1
            }
            index += 1;
        }
        let recall = if positives > 0 {
            tp as f64 / positives as f64
        } else {
            0.0
        };
        let fpr = if negatives > 0 {
            fp as f64 / negatives as f64
        } else {
            0.0
        };
        let precision = tp as f64 / (tp + fp) as f64;
        auc += (fpr - previous_fpr) * (recall + previous_tpr) / 2.0;
        ap += (recall - previous_tpr) * precision;
        previous_tpr = recall;
        previous_fpr = fpr;
        points.push(CurvePoint {
            threshold: Some(score),
            precision,
            recall,
            false_positive_rate: fpr,
        });
    }
    let count = actual.len() as f64;
    let log_loss = actual
        .iter()
        .zip(scores)
        .map(|(a, s)| {
            let p = s.clamp(1e-15, 1.0 - 1e-15);
            if *a { -p.ln() } else { -(1.0 - p).ln() }
        })
        .sum::<f64>()
        / count;
    let brier_score = actual
        .iter()
        .zip(scores)
        .map(|(a, s)| (f64::from(*a) - s).powi(2))
        .sum::<f64>()
        / count;
    Ok(BinaryCurve {
        roc_auc: (positives > 0 && negatives > 0).then_some(auc),
        average_precision: (positives > 0).then_some(ap),
        log_loss,
        brier_score,
        points,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RegressionMetrics {
    pub samples: usize,
    pub mae: f64,
    pub rmse: f64,
    pub r2: Option<f64>,
}
pub fn regression_metrics(actual: &[f64], predicted: &[f64]) -> Result<RegressionMetrics> {
    require(
        !actual.is_empty()
            && actual.len() == predicted.len()
            && actual.iter().chain(predicted).all(|v| v.is_finite()),
        "Regression metrics need aligned nonempty finite values",
    )?;
    let n = actual.len() as f64;
    let mean = actual.iter().sum::<f64>() / n;
    let sse = actual
        .iter()
        .zip(predicted)
        .map(|(a, p)| (a - p).powi(2))
        .sum::<f64>();
    let sst = actual.iter().map(|a| (a - mean).powi(2)).sum::<f64>();
    Ok(RegressionMetrics {
        samples: actual.len(),
        mae: actual
            .iter()
            .zip(predicted)
            .map(|(a, p)| (a - p).abs())
            .sum::<f64>()
            / n,
        rmse: (sse / n).sqrt(),
        r2: (sst > 0.0).then_some(1.0 - sse / sst),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScoredBox {
    pub bounds: BoundingBox,
    pub confidence: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DetectionClassMetrics {
    pub class_id: u32,
    pub actual: usize,
    pub predicted: usize,
    pub matched: usize,
    pub average_precision: Option<f64>,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DetectionMetrics {
    pub iou_threshold: f64,
    pub mean_average_precision: Option<f64>,
    pub per_class: Vec<DetectionClassMetrics>,
}
pub fn detection_metrics(
    actual: &[Vec<BoundingBox>],
    predicted: &[Vec<ScoredBox>],
    classes: usize,
    iou_threshold: f64,
) -> Result<DetectionMetrics> {
    require(
        !actual.is_empty()
            && actual.len() == predicted.len()
            && classes > 0
            && iou_threshold.is_finite()
            && iou_threshold > 0.0
            && iou_threshold <= 1.0,
        "Detection needs aligned images, classes and IoU in (0,1]",
    )?;
    for bounds in actual.iter().flatten() {
        bounds.validate(classes)?;
    }
    for scored in predicted.iter().flatten() {
        scored.bounds.validate(classes)?;
        require(
            scored.confidence.is_finite() && (0.0..=1.0).contains(&scored.confidence),
            "Detection confidence must be in [0,1]",
        )?;
    }
    let mut per_class = vec![];
    for class in 0..classes as u32 {
        let count = actual
            .iter()
            .flatten()
            .filter(|b| b.class_id == class)
            .count();
        let mut candidates: Vec<_> = predicted
            .iter()
            .enumerate()
            .flat_map(|(image, boxes)| {
                boxes
                    .iter()
                    .filter(move |b| b.bounds.class_id == class)
                    .map(move |b| (image, b))
            })
            .collect();
        candidates.sort_by(|a, b| b.1.confidence.total_cmp(&a.1.confidence));
        let mut used: Vec<_> = actual
            .iter()
            .map(|boxes| vec![false; boxes.len()])
            .collect();
        let mut matched = 0;
        let mut precision_recall = vec![];
        for (rank, (image, detected)) in candidates.iter().enumerate() {
            let best = actual[*image]
                .iter()
                .enumerate()
                .filter(|(i, b)| b.class_id == class && !used[*image][*i])
                .map(|(i, b)| (i, b.iou(&detected.bounds)))
                .filter(|(_, iou)| *iou >= iou_threshold)
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((index, _)) = best {
                used[*image][index] = true;
                matched += 1;
            }
            precision_recall.push((
                if count > 0 {
                    matched as f64 / count as f64
                } else {
                    0.0
                },
                matched as f64 / (rank + 1) as f64,
            ));
        }
        let ap = (count > 0).then(|| {
            (0..=100)
                .map(|r| {
                    precision_recall
                        .iter()
                        .filter(|(recall, _)| *recall >= r as f64 / 100.0)
                        .map(|(_, p)| *p)
                        .fold(0.0, f64::max)
                })
                .sum::<f64>()
                / 101.0
        });
        per_class.push(DetectionClassMetrics {
            class_id: class,
            actual: count,
            predicted: candidates.len(),
            matched,
            average_precision: ap,
            precision: ratio(matched as u64, candidates.len() as u64),
            recall: ratio(matched as u64, count as u64),
        });
    }
    let values: Vec<_> = per_class
        .iter()
        .filter_map(|c| c.average_precision)
        .collect();
    Ok(DetectionMetrics {
        iou_threshold,
        mean_average_precision: (!values.is_empty())
            .then(|| values.iter().sum::<f64>() / values.len() as f64),
        per_class,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SegmentationMetrics {
    pub per_class_iou: Vec<Option<f64>>,
    pub per_class_dice: Vec<Option<f64>>,
    pub mean_iou: f64,
    pub mean_dice: f64,
}
pub fn segmentation_metrics(
    actual: &[u32],
    predicted: &[u32],
    classes: usize,
) -> Result<SegmentationMetrics> {
    let classification = classification_metrics(actual, predicted, classes)?;
    let per_class_iou: Vec<_> = classification
        .per_class
        .iter()
        .map(|c| {
            ratio(
                c.true_positive,
                c.true_positive + c.false_positive + c.false_negative,
            )
        })
        .collect();
    let per_class_dice: Vec<_> = classification.per_class.iter().map(|c| c.f1).collect();
    let average = |values: &Vec<Option<f64>>| {
        values.iter().flatten().sum::<f64>() / values.iter().flatten().count() as f64
    };
    Ok(SegmentationMetrics {
        mean_iou: average(&per_class_iou),
        mean_dice: average(&per_class_dice),
        per_class_iou,
        per_class_dice,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventInterval {
    pub start_ms: i64,
    pub end_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventMetrics {
    pub events: usize,
    pub detected: usize,
    pub missed: usize,
    pub false_alarms: usize,
    pub recall: Option<f64>,
    pub false_alarms_per_hour: f64,
    pub mean_lead_time_ms: Option<f64>,
}
pub fn event_metrics(
    events: &[EventInterval],
    alarms: &[i64],
    horizon_ms: u64,
    observed_duration_ms: u64,
) -> Result<EventMetrics> {
    require(
        observed_duration_ms > 0 && events.iter().all(|e| e.start_ms <= e.end_ms),
        "Events need valid intervals and positive observation duration",
    )?;
    let mut order: Vec<_> = (0..events.len()).collect();
    order.sort_by_key(|i| events[*i].start_ms);
    let mut alarms = alarms.to_vec();
    alarms.sort_unstable();
    let mut used = vec![false; events.len()];
    let mut detected = 0;
    let mut lead = 0.0;
    let mut false_alarms = 0;
    for alarm in alarms {
        let event = order.iter().copied().find(|i| {
            !used[*i]
                && alarm as i128 >= events[*i].start_ms as i128 - horizon_ms as i128
                && alarm <= events[*i].end_ms
        });
        if let Some(i) = event {
            used[i] = true;
            detected += 1;
            lead += (events[i].start_ms as i128 - alarm as i128) as f64;
        } else {
            false_alarms += 1;
        }
    }
    Ok(EventMetrics {
        events: events.len(),
        detected,
        missed: events.len() - detected,
        false_alarms,
        recall: ratio(detected as u64, events.len() as u64),
        false_alarms_per_hour: false_alarms as f64 * 3_600_000.0 / observed_duration_ms as f64,
        mean_lead_time_ms: (detected > 0).then_some(lead / detected as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinal_metrics_respect_distance_and_imbalance() {
        let result = ordinal_metrics(&[0, 0, 0, 2], &[0, 1, 2, 0], 3).unwrap();
        assert_eq!(result.mean_rank_error, 1.25);
        assert_eq!(result.macro_rank_error, 1.5);
        assert!((result.quadratic_kappa + 0.5).abs() < 1e-12);
        assert_eq!(
            ordinal_metrics(&[1, 1], &[1, 1], 3)
                .unwrap()
                .quadratic_kappa,
            1.0
        );
        assert!(ordinal_metrics(&[3], &[1], 3).is_err());
    }
    #[test]
    fn classification_reports_true_macro_f1_and_rare_class_recall() {
        let m = classification_metrics(&[0, 0, 0, 1], &[0, 0, 0, 0], 2).unwrap();
        assert_eq!(m.accuracy, 0.75);
        assert_eq!(m.per_class[1].recall, Some(0.0));
        assert!((m.macro_f1 - 3.0 / 7.0).abs() < 1e-12);
        assert!((m.weighted_f1 - 9.0 / 14.0).abs() < 1e-12);
        assert!(m.accuracy_interval_95.lower < 0.75 && m.accuracy_interval_95.upper > 0.75);
    }
    #[test]
    fn roc_ties_and_pr_average_precision_match_known_values() {
        let m = binary_curve(&[false, false, true, true], &[0.1, 0.4, 0.35, 0.8]).unwrap();
        assert!((m.roc_auc.unwrap() - 0.75).abs() < 1e-12);
        assert!((m.average_precision.unwrap() - 5.0 / 6.0).abs() < 1e-12);
        assert_eq!(
            binary_curve(&[true, false], &[0.5, 0.5]).unwrap().roc_auc,
            Some(0.5)
        );
    }
    #[test]
    fn detection_matching_does_not_count_duplicate_boxes_twice() {
        let bbox = BoundingBox {
            class_id: 0,
            x_min: 0.0,
            y_min: 0.0,
            x_max: 1.0,
            y_max: 1.0,
        };
        let m = detection_metrics(
            &[vec![bbox.clone()]],
            &[vec![
                ScoredBox {
                    bounds: bbox.clone(),
                    confidence: 0.9,
                },
                ScoredBox {
                    bounds: bbox,
                    confidence: 0.8,
                },
            ]],
            1,
            0.5,
        )
        .unwrap();
        assert_eq!(m.per_class[0].matched, 1);
        assert_eq!(m.per_class[0].precision, Some(0.5));
        assert_eq!(m.mean_average_precision, Some(1.0));
        let segmentation = segmentation_metrics(&[0, 0, 1, 1], &[0, 1, 1, 1], 2).unwrap();
        assert_eq!(segmentation.per_class_iou, vec![Some(0.5), Some(2.0 / 3.0)]);
    }
    #[test]
    fn event_metrics_count_duplicate_alarms_and_lead_time() {
        let m = event_metrics(
            &[
                EventInterval {
                    start_ms: 100,
                    end_ms: 110,
                },
                EventInterval {
                    start_ms: 300,
                    end_ms: 310,
                },
            ],
            &[80, 90, 500],
            30,
            3_600_000,
        )
        .unwrap();
        assert_eq!(m.detected, 1);
        assert_eq!(m.false_alarms, 2);
        assert_eq!(m.mean_lead_time_ms, Some(20.0));
        assert_eq!(m.false_alarms_per_hour, 2.0);
    }
}

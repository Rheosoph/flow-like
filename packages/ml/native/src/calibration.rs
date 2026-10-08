use flow_like_ml_core::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn validate_scores(scores: &[f64], actual: &[bool]) -> Result<()> {
    require(
        scores.len() == actual.len() && !scores.is_empty() && scores.iter().all(|s| s.is_finite()),
        "Calibration needs aligned finite scores and labels",
    )
}
fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct IsotonicCalibrator {
    pub thresholds: Vec<f64>,
    pub probabilities: Vec<f64>,
}
impl IsotonicCalibrator {
    pub fn fit(scores: &[f64], actual: &[bool]) -> Result<Self> {
        validate_scores(scores, actual)?;
        let mut order: Vec<_> = (0..scores.len()).collect();
        order.sort_by(|a, b| scores[*a].total_cmp(&scores[*b]));
        let mut blocks: Vec<(f64, f64, usize)> = vec![];
        let mut i = 0;
        while i < order.len() {
            let threshold = scores[order[i]];
            let (mut sum, mut count) = (0.0, 0usize);
            while i < order.len() && scores[order[i]] == threshold {
                sum += f64::from(actual[order[i]]);
                count += 1;
                i += 1;
            }
            blocks.push((threshold, sum, count));
            while blocks.len() >= 2 {
                let n = blocks.len();
                if blocks[n - 2].1 / blocks[n - 2].2 as f64
                    <= blocks[n - 1].1 / blocks[n - 1].2 as f64
                {
                    break;
                }
                let right = blocks.pop().unwrap();
                let left = blocks.pop().unwrap();
                blocks.push((right.0, left.1 + right.1, left.2 + right.2));
            }
        }
        Ok(Self {
            thresholds: blocks.iter().map(|b| b.0).collect(),
            probabilities: blocks.iter().map(|b| b.1 / b.2 as f64).collect(),
        })
    }
    pub fn predict(&self, score: f64) -> Result<f64> {
        require(
            score.is_finite()
                && !self.thresholds.is_empty()
                && self.thresholds.len() == self.probabilities.len()
                && self.thresholds.windows(2).all(|w| w[0] < w[1])
                && self.probabilities.windows(2).all(|w| w[0] <= w[1])
                && self
                    .probabilities
                    .iter()
                    .all(|p| p.is_finite() && (0.0..=1.0).contains(p)),
            "Invalid isotonic calibration state or score",
        )?;
        let index = self
            .thresholds
            .partition_point(|t| *t < score)
            .min(self.probabilities.len() - 1);
        Ok(self.probabilities[index])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlattCalibrator {
    pub slope: f64,
    pub intercept: f64,
}
impl PlattCalibrator {
    pub fn fit(scores: &[f64], actual: &[bool]) -> Result<Self> {
        validate_scores(scores, actual)?;
        let positive = actual.iter().filter(|v| **v).count();
        let negative = actual.len() - positive;
        require(
            positive > 0 && negative > 0,
            "Platt calibration requires both classes",
        )?;
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;
        let scale = (scores.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / scores.len() as f64)
            .sqrt()
            .max(1e-12);
        let x: Vec<_> = scores.iter().map(|s| (s - mean) / scale).collect();
        let y: Vec<_> = actual
            .iter()
            .map(|label| {
                if *label {
                    (positive + 1) as f64 / (positive + 2) as f64
                } else {
                    1.0 / (negative + 2) as f64
                }
            })
            .collect();
        let objective = |a: f64, b: f64| {
            x.iter()
                .zip(&y)
                .map(|(x, y)| {
                    let z = a * x + b;
                    z.max(0.0) + (-z.abs()).exp().ln_1p() - y * z
                })
                .sum::<f64>()
                + 0.5e-6 * a * a
        };
        let (mut a, mut b) = (0.0, ((positive + 1) as f64 / (negative + 1) as f64).ln());
        for _ in 0..100 {
            let (mut ga, mut gb, mut haa, mut hab, mut hbb) = (1e-6 * a, 0.0, 1e-6, 0.0, 1e-10);
            for (x, y) in x.iter().zip(&y) {
                let p = sigmoid(a * x + b);
                let h = (p * (1.0 - p)).max(1e-12);
                ga += (p - y) * x;
                gb += p - y;
                haa += h * x * x;
                hab += h * x;
                hbb += h;
            }
            let determinant = haa * hbb - hab * hab;
            require(
                determinant.is_finite() && determinant > 0.0,
                "Calibration Hessian is singular",
            )?;
            let da = (hbb * ga - hab * gb) / determinant;
            let db = (haa * gb - hab * ga) / determinant;
            if da.abs() + db.abs() < 1e-9 {
                break;
            }
            let before = objective(a, b);
            let mut step = 1.0;
            while step > 1e-8 && objective(a - step * da, b - step * db) > before {
                step *= 0.5;
            }
            a -= step * da;
            b -= step * db;
        }
        let slope = a / scale;
        Ok(Self {
            slope,
            intercept: b - slope * mean,
        })
    }
    pub fn predict(&self, score: f64) -> Result<f64> {
        require(
            score.is_finite() && self.slope.is_finite() && self.intercept.is_finite(),
            "Calibration inputs must be finite",
        )?;
        Ok(sigmoid(self.slope * score + self.intercept))
    }
}

fn softmax(logits: &[f64], temperature: f64) -> Vec<f64> {
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut values: Vec<_> = logits
        .iter()
        .map(|v| ((v - max) / temperature).exp())
        .collect();
    let sum = values.iter().sum::<f64>();
    for value in &mut values {
        *value /= sum;
    }
    values
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TemperatureScaler {
    pub temperature: f64,
}
impl TemperatureScaler {
    pub fn fit(logits: &[Vec<f64>], actual: &[u32]) -> Result<Self> {
        let width = crate::validate_matrix(logits)?;
        require(
            width >= 2
                && actual.len() == logits.len()
                && actual.iter().all(|v| (*v as usize) < width),
            "Temperature scaling needs aligned multiclass logits and labels",
        )?;
        let objective = |log_t: f64| {
            logits
                .iter()
                .zip(actual)
                .map(|(row, target)| {
                    let t = log_t.exp();
                    let max = row.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let lse = row.iter().map(|v| ((v - max) / t).exp()).sum::<f64>().ln();
                    lse + (max - row[*target as usize]) / t
                })
                .sum::<f64>()
        };
        let (mut left, mut right) = (-6.0, 6.0);
        let golden = (5.0f64.sqrt() - 1.0) / 2.0;
        for _ in 0..90 {
            let a = right - golden * (right - left);
            let b = left + golden * (right - left);
            if objective(a) < objective(b) {
                right = b
            } else {
                left = a
            }
        }
        Ok(Self {
            temperature: ((left + right) / 2.0).exp(),
        })
    }
    pub fn predict(&self, logits: &[f64]) -> Result<Vec<f64>> {
        require(
            logits.len() >= 2
                && logits.iter().all(|v| v.is_finite())
                && self.temperature.is_finite()
                && self.temperature > 0.0,
            "Temperature or logits are invalid",
        )?;
        Ok(softmax(logits, self.temperature))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ThresholdConstraints {
    pub minimum_recall: f64,
    pub minimum_precision: f64,
    pub maximum_false_positive_rate: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ThresholdSelection {
    pub threshold: f64,
    pub recall: f64,
    pub precision: f64,
    pub false_positive_rate: f64,
    pub true_positive: usize,
    pub false_positive: usize,
}
pub fn select_threshold(
    actual: &[bool],
    scores: &[f64],
    constraints: &ThresholdConstraints,
) -> Result<Option<ThresholdSelection>> {
    validate_scores(scores, actual)?;
    require(
        scores.iter().all(|s| (0.0..=1.0).contains(s)),
        "Threshold probabilities must be in [0,1]",
    )?;
    require(
        [
            constraints.minimum_recall,
            constraints.minimum_precision,
            constraints.maximum_false_positive_rate,
        ]
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        "Threshold constraints must be in [0,1]",
    )?;
    let positives = actual.iter().filter(|v| **v).count();
    let negatives = actual.len() - positives;
    require(
        positives > 0 && negatives > 0,
        "Threshold selection requires positives and negatives",
    )?;
    let mut candidates = scores.to_vec();
    candidates.push(1.0);
    candidates.sort_by(|a, b| b.total_cmp(a));
    candidates.dedup();
    for threshold in candidates {
        let (mut tp, mut fp) = (0usize, 0usize);
        for (a, s) in actual.iter().zip(scores) {
            if *s >= threshold {
                if *a { tp += 1 } else { fp += 1 }
            }
        }
        let recall = tp as f64 / positives as f64;
        let precision = if tp + fp > 0 {
            tp as f64 / (tp + fp) as f64
        } else {
            0.0
        };
        let fpr = fp as f64 / negatives as f64;
        if recall >= constraints.minimum_recall
            && precision >= constraints.minimum_precision
            && fpr <= constraints.maximum_false_positive_rate
        {
            return Ok(Some(ThresholdSelection {
                threshold,
                recall,
                precision,
                false_positive_rate: fpr,
                true_positive: tp,
                false_positive: fp,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isotonic_pools_violations_and_equal_scores() {
        let model =
            IsotonicCalibrator::fit(&[0.1, 0.2, 0.3, 0.4], &[false, true, false, true]).unwrap();
        assert_eq!(model.probabilities, vec![0.0, 0.5, 1.0]);
        assert_eq!(model.predict(0.25).unwrap(), 0.5);
        let equal = IsotonicCalibrator::fit(&[0.5, 0.5], &[true, false]).unwrap();
        assert_eq!(equal.predict(0.5).unwrap(), 0.5);
    }
    #[test]
    fn platt_handles_large_margins_and_calibrates_direction() {
        let model = PlattCalibrator::fit(
            &[-1000.0, -900.0, 900.0, 1000.0],
            &[false, false, true, true],
        )
        .unwrap();
        assert!(model.predict(-1000.0).unwrap() < 0.3);
        assert!(model.predict(1000.0).unwrap() > 0.7);
    }
    #[test]
    fn temperature_reduces_overconfidence() {
        let logits = vec![vec![8.0, 0.0]; 4];
        let model = TemperatureScaler::fit(&logits, &[0, 0, 0, 1]).unwrap();
        let p = model.predict(&[8.0, 0.0]).unwrap();
        assert!((p[0] - 0.75).abs() < 1e-5);
    }
    #[test]
    fn threshold_honors_recall_and_false_positive_limits() {
        let labels = [false, false, true, true];
        let scores = [0.1, 0.4, 0.35, 0.8];
        let c = ThresholdConstraints {
            minimum_recall: 1.0,
            minimum_precision: 0.0,
            maximum_false_positive_rate: 0.5,
        };
        assert_eq!(
            select_threshold(&labels, &scores, &c)
                .unwrap()
                .unwrap()
                .threshold,
            0.35
        );
        assert!(
            select_threshold(
                &labels,
                &scores,
                &ThresholdConstraints {
                    maximum_false_positive_rate: 0.0,
                    ..c
                }
            )
            .unwrap()
            .is_none()
        );
    }
}

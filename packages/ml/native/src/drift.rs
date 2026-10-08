use flow_like_ml_core::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct KsResult {
    pub statistic: f64,
    pub critical_value: f64,
    pub drift: bool,
    pub reference_samples: usize,
    pub current_samples: usize,
}
pub fn ks_test(reference: &[f64], current: &[f64], alpha: f64) -> Result<KsResult> {
    require(
        !reference.is_empty()
            && !current.is_empty()
            && reference.iter().chain(current).all(|v| v.is_finite()),
        "KS comparison needs nonempty finite samples",
    )?;
    require(
        alpha.is_finite() && alpha > 0.0 && alpha < 1.0,
        "KS alpha must be in (0,1)",
    )?;
    let mut a = reference.to_vec();
    let mut b = current.to_vec();
    a.sort_by(f64::total_cmp);
    b.sort_by(f64::total_cmp);
    let (mut i, mut j, mut statistic) = (0usize, 0usize, 0.0f64);
    while i < a.len() || j < b.len() {
        let next = if i == a.len() {
            b[j]
        } else if j == b.len() {
            a[i]
        } else {
            a[i].min(b[j])
        };
        while i < a.len() && a[i] <= next {
            i += 1;
        }
        while j < b.len() && b[j] <= next {
            j += 1;
        }
        statistic = statistic.max((i as f64 / a.len() as f64 - j as f64 / b.len() as f64).abs());
    }
    // Two-sample asymptotic bound; tied/discrete distributions make this conservative.
    let critical_value = (-0.5 * (alpha / 2.0).ln()).sqrt()
        * ((a.len() + b.len()) as f64 / (a.len() as f64 * b.len() as f64)).sqrt();
    Ok(KsResult {
        statistic,
        critical_value,
        drift: statistic > critical_value,
        reference_samples: a.len(),
        current_samples: b.len(),
    })
}

pub fn population_stability_index(
    reference: &[f64],
    current: &[f64],
    edges: &[f64],
) -> Result<f64> {
    require(
        !reference.is_empty()
            && !current.is_empty()
            && reference
                .iter()
                .chain(current)
                .chain(edges)
                .all(|v| v.is_finite())
            && edges.windows(2).all(|w| w[0] < w[1]),
        "PSI needs finite samples and increasing bin edges",
    )?;
    let count = |values: &[f64]| {
        let mut bins = vec![0.5; edges.len() + 1];
        for value in values {
            bins[edges.partition_point(|edge| *value > *edge)] += 1.0;
        }
        let total = values.len() as f64 + 0.5 * bins.len() as f64;
        bins.into_iter().map(|b| b / total).collect::<Vec<_>>()
    };
    let a = count(reference);
    let b = count(current);
    Ok(a.iter().zip(b).map(|(a, b)| (b - a) * (b / a).ln()).sum())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PageHinkley {
    pub delta: f64,
    pub threshold: f64,
    pub minimum_samples: usize,
    count: usize,
    mean: f64,
    cumulative: f64,
    minimum: f64,
}
impl PageHinkley {
    pub fn new(delta: f64, threshold: f64, minimum_samples: usize) -> Result<Self> {
        require(
            delta.is_finite()
                && delta >= 0.0
                && threshold.is_finite()
                && threshold > 0.0
                && minimum_samples > 0,
            "Invalid Page-Hinkley parameters",
        )?;
        Ok(Self {
            delta,
            threshold,
            minimum_samples,
            count: 0,
            mean: 0.0,
            cumulative: 0.0,
            minimum: 0.0,
        })
    }
    /// Detects increases in a monitored loss or error rate; reset after accepting a new baseline.
    pub fn update(&mut self, value: f64) -> Result<bool> {
        require(value.is_finite(), "Drift observation must be finite")?;
        self.count += 1;
        self.mean += (value - self.mean) / self.count as f64;
        self.cumulative += value - self.mean - self.delta;
        self.minimum = self.minimum.min(self.cumulative);
        Ok(self.count >= self.minimum_samples && self.cumulative - self.minimum > self.threshold)
    }
    pub fn reset(&mut self) {
        self.count = 0;
        self.mean = 0.0;
        self.cumulative = 0.0;
        self.minimum = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drift_statistics_identify_shift_and_handle_ties() {
        let a = vec![0.0; 100];
        let b = vec![1.0; 100];
        assert_eq!(ks_test(&a, &a, 0.05).unwrap().statistic, 0.0);
        assert!(ks_test(&a, &b, 0.05).unwrap().drift);
        assert_eq!(population_stability_index(&a, &a, &[0.5]).unwrap(), 0.0);
        assert!(population_stability_index(&a, &b, &[0.5]).unwrap() > 1.0);
    }
    #[test]
    fn page_hinkley_detects_loss_increase_and_resets() {
        let mut detector = PageHinkley::new(0.01, 5.0, 20).unwrap();
        for _ in 0..100 {
            assert!(!detector.update(0.1).unwrap());
        }
        assert!((0..20).any(|_| detector.update(1.0).unwrap()));
        detector.reset();
        assert!(!detector.update(0.1).unwrap());
    }
}

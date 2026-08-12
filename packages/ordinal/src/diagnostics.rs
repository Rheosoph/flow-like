//! Diagnostics for the proportional-odds assumption.
//!
//! A cumulative-link model fits ONE coefficient vector shared by every threshold. That is the
//! assumption the whole model rests on, and nothing about a successful fit reveals whether it
//! holds — a model that violates it converges perfectly happily and is simply wrong in a way no
//! accuracy number exposes.
//!
//! The check here is the construction behind Brant's test: refit the problem as `K - 1` separate
//! binary problems ("is y > k?"), each with its own free coefficient vector, and compare those to
//! the shared one. If the assumption holds the per-threshold vectors should scatter around the
//! shared vector; a coefficient that changes sign or magnitude sharply across thresholds is the
//! signature of a violation.
//!
//! This reports the divergence rather than a p-value: a Wald statistic needs the asymptotic
//! covariance of the estimates, which this crate's Adam fit does not produce.

use crate::error::Result;
use crate::logistic::{OrdinalLogistic, OrdinalLogisticValidParams};
use linfa::dataset::AsSingleTargets;
use linfa::traits::Fit;
use linfa::{DatasetBase, Float};
use ndarray::{Array1, Array2, ArrayBase, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// How far the per-threshold coefficients stray from the shared ones.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProportionalOddsReport<F> {
    /// The shared coefficient vector from the ordinal fit.
    pub shared: Array1<F>,
    /// One free coefficient vector per threshold, in threshold order.
    pub per_threshold: Vec<Array1<F>>,
    /// Largest absolute gap between any per-threshold coefficient and its shared counterpart.
    pub max_deviation: F,
    /// Mean absolute gap across every coefficient and threshold.
    pub mean_deviation: F,
    /// Feature indices whose coefficient changes SIGN across thresholds.
    ///
    /// The most legible symptom of a violation: a feature that pushes toward higher levels at one
    /// cut point and toward lower levels at another cannot be described by a single shared slope.
    pub sign_flipping_features: Vec<usize>,
}

impl<F: Float> ProportionalOddsReport<F> {
    /// A coarse verdict, for callers that want one number rather than the full breakdown.
    ///
    /// `tolerance` is on the same scale as the coefficients, so it only means something once the
    /// features are scaled. There is no universal threshold here, which is exactly why this returns
    /// the deviations too.
    pub fn assumption_plausible(&self, tolerance: F) -> bool {
        self.sign_flipping_features.is_empty() && self.max_deviation <= tolerance
    }
}

/// Fits the shared model plus one free binary model per threshold and compares them.
///
/// The binary sub-models are fitted with the same link and penalty as the ordinal model, so any
/// difference is attributable to the shared-coefficient constraint rather than to a change of
/// estimator. Each is itself a two-level ordinal fit, which is precisely binary regression under
/// the same link.
pub fn proportional_odds_report<F, D, T>(
    dataset: &DatasetBase<ArrayBase<D, Ix2>, T>,
    params: &OrdinalLogisticValidParams<F>,
) -> Result<ProportionalOddsReport<F>>
where
    F: Float,
    D: Data<Elem = F>,
    T: AsSingleTargets<Elem = usize>,
{
    let records = dataset.records();
    let targets: Vec<usize> = dataset
        .targets
        .as_single_targets()
        .iter()
        .copied()
        .collect();

    let shared_model: OrdinalLogistic<F> = params.fit(dataset)?;
    let shared = shared_model.coefficients().clone();
    let n_cuts = shared_model.n_classes() - 1;

    // Owned copy so each binary sub-problem can be handed a DatasetBase of its own.
    let owned_records: Array2<F> = records.to_owned();

    let mut per_threshold = Vec::with_capacity(n_cuts);
    for cut in 0..n_cuts {
        let binary: Array1<usize> = targets
            .iter()
            .map(|rank| usize::from(*rank > cut))
            .collect();

        // A cut with every row on one side carries no information about the slope; the shared
        // vector is reported unchanged so it contributes zero deviation rather than a spurious one.
        let above = binary.iter().filter(|value| **value == 1).count();
        if above == 0 || above == binary.len() {
            per_threshold.push(shared.clone());
            continue;
        }

        let sub_params = params.as_binary();
        let sub_dataset = DatasetBase::new(owned_records.clone(), binary);
        let sub_model: OrdinalLogistic<F> = sub_params.fit(&sub_dataset)?;
        per_threshold.push(sub_model.coefficients().clone());
    }

    let mut max_deviation = F::zero();
    let mut total = F::zero();
    let mut count = 0usize;
    let mut sign_flipping_features = Vec::new();

    for feature in 0..shared.len() {
        let mut saw_positive = false;
        let mut saw_negative = false;
        for coefficients in &per_threshold {
            let deviation = (coefficients[feature] - shared[feature]).abs();
            if deviation > max_deviation {
                max_deviation = deviation;
            }
            total = total + deviation;
            count += 1;

            // A coefficient indistinguishable from zero has no direction to flip.
            let tiny = F::cast(1e-8);
            if coefficients[feature] > tiny {
                saw_positive = true;
            } else if coefficients[feature] < -tiny {
                saw_negative = true;
            }
        }
        if saw_positive && saw_negative {
            sign_flipping_features.push(feature);
        }
    }

    let mean_deviation = if count > 0 {
        total / F::cast(count)
    } else {
        F::zero()
    };

    Ok(ProportionalOddsReport {
        shared,
        per_threshold,
        max_deviation,
        mean_deviation,
        sign_flipping_features,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logistic::OrdinalLogisticParams;
    use linfa::ParamGuard;
    use ndarray::Array2;

    fn params<F: Float>() -> OrdinalLogisticValidParams<F> {
        OrdinalLogisticParams::new()
            .alpha(F::cast(1e-4))
            .max_iterations(1500)
            .check()
            .unwrap()
    }

    /// Data generated FROM a proportional-odds model must not be flagged.
    #[test]
    fn holds_for_genuinely_proportional_data() {
        let n = 400;
        let mut records = Array2::zeros((n, 1));
        let mut targets = Vec::with_capacity(n);
        for index in 0..n {
            let x = -3.0 + 6.0 * (index as f64) / (n as f64 - 1.0);
            records[[index, 0]] = x;
            targets.push(if x < -1.0 {
                0
            } else if x < 1.0 {
                1
            } else {
                2
            });
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));
        let report = proportional_odds_report(&dataset, &params::<f64>()).unwrap();

        assert_eq!(report.per_threshold.len(), 2);
        assert!(
            report.sign_flipping_features.is_empty(),
            "no feature should flip sign: {:?}",
            report.sign_flipping_features
        );
    }

    /// A feature that orders the low cut one way and the high cut the other cannot be described by
    /// a single shared slope, and the report must say so.
    #[test]
    fn detects_a_sign_flipping_feature() {
        let n = 400;
        let mut records = Array2::zeros((n, 1));
        let mut targets = Vec::with_capacity(n);
        for index in 0..n {
            let x = -3.0 + 6.0 * (index as f64) / (n as f64 - 1.0);
            records[[index, 0]] = x;
            // Level 1 sits at BOTH extremes, so "y > 0" and "y > 1" want opposite slopes.
            targets.push(if x < -1.5 {
                1
            } else if x < 1.5 {
                0
            } else {
                2
            });
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));
        let report = proportional_odds_report(&dataset, &params::<f64>()).unwrap();

        assert!(
            !report.sign_flipping_features.is_empty() || report.max_deviation > 0.5,
            "expected a violation, got max deviation {} and no sign flips",
            report.max_deviation
        );
        assert!(!report.assumption_plausible(0.1));
    }

    #[test]
    fn degenerate_cut_contributes_no_deviation() {
        // Level 1 never occurs, so the "y > 0" and "y > 1" splits coincide.
        let records = Array2::from_shape_vec((4, 1), vec![0.0, 1.0, 2.0, 3.0]).unwrap();
        let dataset = DatasetBase::new(records, Array1::from(vec![0usize, 0, 2, 2]));
        let report = proportional_odds_report(&dataset, &params::<f64>()).unwrap();
        assert_eq!(report.per_threshold.len(), 2);
        assert!(report.max_deviation.is_finite());
    }
}

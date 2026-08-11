//! Ordinal ridge regression: regress the level *rank* on the features, then cut the resulting
//! score into levels.
//!
//! This is the cheap counterpart to the proportional-odds model. It has a closed-form solve rather
//! than an iterative one, which makes it the practical choice when there are many levels or many
//! features, and it degrades gracefully when the proportional-odds assumption does not hold.
//!
//! Treating ranks as numbers is only defensible because the target is ordered — the whole point of
//! an ordinal model. The cut points are *learned* from the training score distribution rather than
//! obtained by rounding, so an uneven level distribution does not systematically starve the rare
//! levels the way naive rounding does.
//!
//! # Example
//!
//! ```no_run
//! use linfa::prelude::*;
//! use flow_like_ordinal::OrdinalRidge;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dataset: linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>> = todo!();
//! let model = OrdinalRidge::params().alpha(1.0).fit(&dataset)?;
//! let predicted = model.predict(&dataset);
//! # Ok(())
//! # }
//! ```

use crate::error::{OrdinalError, Result};
use linfa::dataset::AsSingleTargets;
use linfa::traits::{Fit, PredictInplace};
use linfa::{DatasetBase, Float, ParamGuard};
use ndarray::{Array1, Array2, ArrayBase, ArrayView1, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Checked hyperparameters for [`OrdinalRidge`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalRidgeValidParams<F> {
    alpha: F,
    n_levels: Option<usize>,
}

impl<F: Float> OrdinalRidgeValidParams<F> {
    pub fn alpha(&self) -> F {
        self.alpha
    }

    /// Declared level count, or `None` to infer it from the observed targets.
    pub fn n_levels(&self) -> Option<usize> {
        self.n_levels
    }
}

/// Unchecked hyperparameters for [`OrdinalRidge`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalRidgeParams<F>(OrdinalRidgeValidParams<F>);

impl<F: Float> Default for OrdinalRidgeParams<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Float> OrdinalRidgeParams<F> {
    pub fn new() -> Self {
        Self(OrdinalRidgeValidParams {
            alpha: F::one(),
            n_levels: None,
        })
    }

    /// L2 penalty. Must be strictly positive: it is what keeps the normal equations invertible.
    pub fn alpha(mut self, alpha: F) -> Self {
        self.0.alpha = alpha;
        self
    }

    /// Declares the number of ordered levels.
    ///
    /// Use this when a level may legitimately be absent from the training sample but must still
    /// occupy a position in the ordering. Left unset, the level count is inferred as `max rank + 1`.
    pub fn n_levels(mut self, n_levels: usize) -> Self {
        self.0.n_levels = Some(n_levels);
        self
    }
}

impl<F: Float> ParamGuard for OrdinalRidgeParams<F> {
    type Checked = OrdinalRidgeValidParams<F>;
    type Error = OrdinalError;

    fn check_ref(&self) -> Result<&Self::Checked> {
        if !self.0.alpha.is_finite() || self.0.alpha <= F::zero() {
            return Err(OrdinalError::InvalidParameter {
                name: "alpha",
                reason:
                    "must be finite and strictly positive to keep the normal equations solvable"
                        .to_string(),
            });
        }
        if let Some(levels) = self.0.n_levels
            && levels < 2
        {
            return Err(OrdinalError::TooFewClasses { found: levels });
        }
        Ok(&self.0)
    }

    fn check(self) -> Result<Self::Checked> {
        self.check_ref()?;
        Ok(self.0)
    }
}

/// A fitted ordinal ridge model.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalRidge<F> {
    coefficients: Array1<F>,
    intercept: F,
    /// `K - 1` increasing cut points on the predicted-score scale.
    thresholds: Array1<F>,
    n_classes: usize,
}

impl<F: Float> OrdinalRidge<F> {
    /// Starts a builder for the hyperparameters.
    pub fn params() -> OrdinalRidgeParams<F> {
        OrdinalRidgeParams::new()
    }

    /// One coefficient per feature. A positive entry pushes samples toward HIGHER levels.
    pub fn coefficients(&self) -> &Array1<F> {
        &self.coefficients
    }

    pub fn intercept(&self) -> F {
        self.intercept
    }

    pub fn thresholds(&self) -> &Array1<F> {
        &self.thresholds
    }

    pub fn n_classes(&self) -> usize {
        self.n_classes
    }

    pub fn n_features(&self) -> usize {
        self.coefficients.len()
    }

    /// The continuous latent score for one sample, before it is cut into a level.
    pub fn score(&self, row: &ArrayView1<'_, F>) -> Result<F> {
        if row.len() != self.coefficients.len() {
            return Err(OrdinalError::FeatureWidthMismatch {
                expected: self.coefficients.len(),
                found: row.len(),
            });
        }
        Ok(row.dot(&self.coefficients) + self.intercept)
    }

    /// Level for one sample: the number of learned cut points the score exceeds.
    fn level_of(&self, score: F) -> usize {
        self.thresholds
            .iter()
            .filter(|threshold| score >= **threshold)
            .count()
            .min(self.n_classes - 1)
    }
}

impl<F: Float, D: Data<Elem = F>, T: AsSingleTargets<Elem = usize>>
    Fit<ArrayBase<D, Ix2>, T, OrdinalError> for OrdinalRidgeValidParams<F>
{
    type Object = OrdinalRidge<F>;

    /// Fits the model on ranks in `0..n_levels`, ordered lowest to highest by the caller.
    fn fit(&self, dataset: &DatasetBase<ArrayBase<D, Ix2>, T>) -> Result<Self::Object> {
        let records = dataset.records();
        let targets: Vec<usize> = dataset
            .targets
            .as_single_targets()
            .iter()
            .copied()
            .collect();

        let (n_samples, n_features) = records.dim();
        if n_samples == 0 || n_features == 0 {
            return Err(OrdinalError::EmptyTrainingSet);
        }
        if targets.len() != n_samples {
            return Err(OrdinalError::LengthMismatch {
                records: n_samples,
                targets: targets.len(),
            });
        }

        let observed = targets.iter().copied().max().map_or(0, |max| max + 1);
        let n_classes = self.n_levels.unwrap_or(observed);
        if n_classes < 2 {
            return Err(OrdinalError::TooFewClasses { found: n_classes });
        }
        if let Some(&bad) = targets.iter().find(|rank| **rank >= n_classes) {
            return Err(OrdinalError::RankOutOfRange {
                rank: bad,
                n_classes,
            });
        }
        if records.iter().any(|value| !value.is_finite()) {
            return Err(OrdinalError::NonFinite {
                context: "the feature matrix",
            });
        }

        // Centering removes the intercept from the penalized solve, so the bias term is never
        // shrunk toward zero by the penalty.
        let mut feature_means = Array1::<F>::zeros(n_features);
        for row in records.rows() {
            feature_means = feature_means + row.to_owned();
        }
        feature_means = feature_means / F::cast(n_samples);

        let ranks: Array1<F> = targets.iter().map(|rank| F::cast(*rank)).collect();
        let rank_mean = ranks.iter().fold(F::zero(), |acc, v| acc + *v) / F::cast(n_samples);

        let mut centered = Array2::<F>::zeros((n_samples, n_features));
        for (index, row) in records.rows().into_iter().enumerate() {
            let mut target = centered.row_mut(index);
            target.assign(&row);
            target -= &feature_means;
        }
        let centered_ranks = &ranks - rank_mean;

        // Normal equations: (X'X + alpha I) beta = X'y, positive definite for alpha > 0.
        let mut gram = centered.t().dot(&centered);
        for index in 0..n_features {
            gram[[index, index]] = gram[[index, index]] + self.alpha;
        }
        let rhs = centered.t().dot(&centered_ranks);
        let coefficients = cholesky_solve(gram, rhs)?;

        let intercept = rank_mean - feature_means.dot(&coefficients);

        let scores: Array1<F> = centered.dot(&coefficients) + rank_mean;
        let thresholds = learn_thresholds(&scores, &targets, n_classes);

        if coefficients.iter().any(|v| !v.is_finite())
            || !intercept.is_finite()
            || thresholds.iter().any(|v| !v.is_finite())
        {
            return Err(OrdinalError::NonFinite {
                context: "the fitted parameters",
            });
        }

        Ok(OrdinalRidge {
            coefficients,
            intercept,
            thresholds,
            n_classes,
        })
    }
}

impl<F: Float, D: Data<Elem = F>> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>>
    for OrdinalRidge<F>
{
    fn predict_inplace(&self, x: &ArrayBase<D, Ix2>, y: &mut Array1<usize>) {
        assert_eq!(
            x.nrows(),
            y.len(),
            "The number of data points must match the number of output targets."
        );
        assert_eq!(
            x.ncols(),
            self.coefficients.len(),
            "The number of features must match the number the model was fitted on."
        );

        for (index, row) in x.rows().into_iter().enumerate() {
            let score = row.dot(&self.coefficients) + self.intercept;
            y[index] = self.level_of(score);
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// Places `K - 1` cut points so the training scores split into the observed level proportions.
///
/// Rounding the score instead would push every level toward the middle of the range whenever the
/// levels are unevenly represented, which is the usual failure of "just round the regression".
fn learn_thresholds<F: Float>(
    scores: &Array1<F>,
    targets: &[usize],
    n_classes: usize,
) -> Array1<F> {
    let mut sorted: Vec<F> = scores.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut counts = vec![0usize; n_classes];
    for &rank in targets {
        counts[rank] += 1;
    }

    let n = sorted.len();
    let half = F::cast(0.5);
    let mut thresholds = Array1::zeros(n_classes - 1);
    let mut cumulative = 0usize;
    for level in 0..n_classes - 1 {
        cumulative += counts[level];
        // Cut between the two sorted scores straddling the class boundary.
        let cut = if cumulative == 0 {
            sorted[0] - F::one()
        } else if cumulative >= n {
            sorted[n - 1] + F::one()
        } else {
            half * (sorted[cumulative - 1] + sorted[cumulative])
        };
        thresholds[level] = cut;
    }

    // Ties in the score distribution can make two cuts coincide; nudge them apart so every level
    // keeps a non-empty interval and prediction stays monotone.
    let nudge = F::cast(1e-9);
    for level in 1..thresholds.len() {
        if thresholds[level] <= thresholds[level - 1] {
            thresholds[level] = thresholds[level - 1] + nudge;
        }
    }
    thresholds
}

/// Solves `A x = b` for a symmetric positive-definite `A` by Cholesky decomposition.
///
/// Hand-rolled so the crate needs no LAPACK backend. `A` is consumed and overwritten with its
/// lower triangular factor.
fn cholesky_solve<F: Float>(mut a: Array2<F>, b: Array1<F>) -> Result<Array1<F>> {
    let n = a.nrows();
    debug_assert_eq!(n, a.ncols());

    for column in 0..n {
        let mut diagonal = a[[column, column]];
        for k in 0..column {
            diagonal = diagonal - a[[column, k]] * a[[column, k]];
        }
        if !diagonal.is_finite() || diagonal <= F::zero() {
            return Err(OrdinalError::NotPositiveDefinite);
        }
        let diagonal = diagonal.sqrt();
        a[[column, column]] = diagonal;

        for row in column + 1..n {
            let mut value = a[[row, column]];
            for k in 0..column {
                value = value - a[[row, k]] * a[[column, k]];
            }
            a[[row, column]] = value / diagonal;
        }
    }

    // Forward substitution: L y = b.
    let mut y = Array1::<F>::zeros(n);
    for row in 0..n {
        let mut value = b[row];
        for k in 0..row {
            value = value - a[[row, k]] * y[k];
        }
        y[row] = value / a[[row, row]];
    }

    // Back substitution: L' x = y.
    let mut x = Array1::<F>::zeros(n);
    for row in (0..n).rev() {
        let mut value = y[row];
        for k in row + 1..n {
            value = value - a[[k, row]] * x[k];
        }
        x[row] = value / a[[row, row]];
    }

    if x.iter().any(|value| !value.is_finite()) {
        return Err(OrdinalError::NonFinite {
            context: "the ridge solve",
        });
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use linfa::traits::Predict;
    use ndarray::array;

    #[test]
    fn cholesky_solves_a_known_system() {
        // A = [[4, 1], [1, 3]], b = [1, 2]  =>  x = [1/11, 7/11]
        let a = array![[4.0f64, 1.0], [1.0, 3.0]];
        let b = array![1.0, 2.0];
        let x = cholesky_solve(a, b).unwrap();
        assert!((x[0] - 1.0 / 11.0).abs() < 1e-12, "{x:?}");
        assert!((x[1] - 7.0 / 11.0).abs() < 1e-12, "{x:?}");
    }

    #[test]
    fn cholesky_rejects_indefinite_input() {
        let a = array![[1.0f64, 2.0], [2.0, 1.0]];
        let b = array![1.0, 1.0];
        assert!(matches!(
            cholesky_solve(a, b),
            Err(OrdinalError::NotPositiveDefinite)
        ));
    }

    #[test]
    fn recovers_monotone_levels() {
        let n = 300;
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

        let model = OrdinalRidge::params().alpha(1e-3).fit(&dataset).unwrap();

        assert!(model.coefficients()[0] > 0.0);
        assert!(model.thresholds()[0] < model.thresholds()[1]);

        let predictions = model.predict(&dataset);
        let correct = predictions
            .iter()
            .zip(dataset.targets.iter())
            .filter(|(a, b)| a == b)
            .count();
        assert!(
            correct as f64 / n as f64 > 0.9,
            "training accuracy {}",
            correct as f64 / n as f64
        );
    }

    /// Learned cuts must keep the rare level reachable; naive rounding is what loses it.
    #[test]
    fn imbalanced_levels_stay_reachable() {
        let n = 200;
        let mut records = Array2::zeros((n, 1));
        let mut targets = Vec::with_capacity(n);
        for index in 0..n {
            let x = index as f64 / n as f64;
            records[[index, 0]] = x;
            // Only the top 5% belong to level 2.
            targets.push(if x < 0.90 {
                0
            } else if x < 0.95 {
                1
            } else {
                2
            });
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));

        let model = OrdinalRidge::params().alpha(1e-4).fit(&dataset).unwrap();
        let predictions = model.predict(&dataset);
        assert!(
            predictions.iter().any(|rank| *rank == 2),
            "the rare top level was never predicted"
        );
        assert!(predictions.iter().any(|rank| *rank == 1));
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            OrdinalRidge::<f64>::params().alpha(0.0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            OrdinalRidge::<f64>::params().n_levels(1).check(),
            Err(OrdinalError::TooFewClasses { .. })
        ));
    }

    #[test]
    fn fits_at_f32() {
        let mut records = Array2::<f32>::zeros((60, 1));
        let mut targets = Vec::with_capacity(60);
        for index in 0..60 {
            let x = -3.0 + 6.0 * (index as f32) / 59.0;
            records[[index, 0]] = x;
            targets.push(if x < 0.0 { 0usize } else { 1 });
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));
        let model = OrdinalRidge::<f32>::params()
            .alpha(1e-3)
            .fit(&dataset)
            .unwrap();
        assert!(model.coefficients()[0] > 0.0);
    }
}

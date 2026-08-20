//! Adjacent-category ordinal regression: coefficients that describe ONE step up the ordering.
//!
//! The model compares each level with the one immediately below it:
//!
//! ```text
//! log( P(y = k+1 | x) / P(y = k | x) ) = theta_k + x . beta,   k = 0 .. K-2
//! ```
//!
//! # Why this instead of the cumulative model
//!
//! [`crate::logistic::OrdinalLogistic`] is a *cumulative* model: it contrasts "at or below cut `k`"
//! against "above cut `k`", so a coefficient there is the log odds ratio of the whole lower group
//! versus the whole upper group. This model contrasts two neighbouring levels only, so a
//! coefficient here is the log odds of landing on level `k+1` rather than level `k`.
//!
//! The two are genuinely different quantities and are constantly confused:
//!
//! | | cumulative (proportional odds) | adjacent category |
//! |---|---|---|
//! | contrast | `y > k` vs `y <= k` | `y = k+1` vs `y = k` |
//! | `exp(beta_j)` means | odds ratio for exceeding *any* cut | odds of stepping up *one* level |
//! | thresholds | must be increasing (they are cut points) | unconstrained (they are level contrasts) |
//! | typical magnitude | larger, it pools levels | smaller, it is a local comparison |
//!
//! Reporting an adjacent-category coefficient as if it were a proportional-odds one overstates
//! nothing in sign but understates the implied effect across the full range, because a shared `beta`
//! here accumulates over every step: moving from the bottom level to the top costs `(K-1)` steps,
//! so the *total* log-odds effect is `(K-1) * x . beta`. Read the wrong table and the same fitted
//! number means a different thing.
//!
//! Pick this model when "what does one more unit of `x` do to my rating?" is the actual question —
//! product ratings, severity grades, Likert responses — and the cumulative one when you care about
//! passing a threshold ("does this case escalate beyond level 2?").
//!
//! # How it is fitted
//!
//! This is a *reparameterized multinomial*, not a decomposition into binary problems: every
//! parameter is fitted jointly on the full sample by penalized maximum likelihood. Summing the
//! defining equation over the steps below `k` gives the level log-probabilities
//!
//! ```text
//! log P(y = k | x) = const + (theta_0 + ... + theta_{k-1}) + k * (x . beta)
//! ```
//!
//! which is linear in `k * (x . beta)`. That linear-in-`k` slope is what keeps the model ordinal
//! under one shared coefficient vector: the level distribution has a monotone likelihood ratio in
//! `x . beta`, so raising the score can only move the mode up the ordering, never sideways.
//!
//! The normalizer is taken with a log-sum-exp over the `K` unnormalized log-probabilities. It has
//! to be: those scores carry a factor of `k`, so a score of 40 with six levels already means
//! `exp(200)`, and the naive exponentiate-then-divide overflows to `NaN` long before the fit is
//! anywhere near unreasonable.
//!
//! Optimized with Adam on the penalized negative log-likelihood. The `L2` penalty applies to `beta`
//! only — penalizing the level contrasts would pull adjacent levels toward equal frequency, which
//! is a statement about the data nobody asked for. Features should be scaled.
//!
//! # Example
//!
//! ```no_run
//! use linfa::prelude::*;
//! use flow_like_ordinal::adjacent_category::AdjacentCategory;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dataset: linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>> = todo!();
//! let model = AdjacentCategory::params().alpha(1.0).fit(&dataset)?;
//! let levels = model.predict(&dataset);
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

/// Checked hyperparameters for [`AdjacentCategory`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AdjacentCategoryValidParams<F> {
    alpha: F,
    max_iterations: usize,
    tolerance: F,
    learning_rate: F,
    n_levels: Option<usize>,
}

impl<F: Float> AdjacentCategoryValidParams<F> {
    pub fn alpha(&self) -> F {
        self.alpha
    }

    pub fn max_iterations(&self) -> usize {
        self.max_iterations
    }

    pub fn tolerance(&self) -> F {
        self.tolerance
    }

    pub fn learning_rate(&self) -> F {
        self.learning_rate
    }

    /// Declared level count, or `None` to infer it from the observed targets.
    pub fn n_levels(&self) -> Option<usize> {
        self.n_levels
    }
}

/// Unchecked hyperparameters for [`AdjacentCategory`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AdjacentCategoryParams<F>(AdjacentCategoryValidParams<F>);

impl<F: Float> Default for AdjacentCategoryParams<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Float> AdjacentCategoryParams<F> {
    pub fn new() -> Self {
        Self(AdjacentCategoryValidParams {
            alpha: F::one(),
            max_iterations: 500,
            tolerance: F::cast(1e-7),
            learning_rate: F::cast(0.1),
            n_levels: None,
        })
    }

    /// L2 penalty on the coefficients. The level contrasts are never penalized.
    pub fn alpha(mut self, alpha: F) -> Self {
        self.0.alpha = alpha;
        self
    }

    /// Maximum optimizer iterations.
    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.0.max_iterations = max_iterations;
        self
    }

    /// Relative change in the objective below which fitting stops.
    pub fn tolerance(mut self, tolerance: F) -> Self {
        self.0.tolerance = tolerance;
        self
    }

    /// Adam step size.
    pub fn learning_rate(mut self, learning_rate: F) -> Self {
        self.0.learning_rate = learning_rate;
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

impl<F: Float> ParamGuard for AdjacentCategoryParams<F> {
    type Checked = AdjacentCategoryValidParams<F>;
    type Error = OrdinalError;

    fn check_ref(&self) -> Result<&Self::Checked> {
        if !self.0.alpha.is_finite() || self.0.alpha < F::zero() {
            return Err(OrdinalError::InvalidParameter {
                name: "alpha",
                reason: "must be finite and non-negative".to_string(),
            });
        }
        if self.0.max_iterations == 0 {
            return Err(OrdinalError::InvalidParameter {
                name: "max_iterations",
                reason: "must be at least 1".to_string(),
            });
        }
        if !self.0.tolerance.is_finite() || self.0.tolerance < F::zero() {
            return Err(OrdinalError::InvalidParameter {
                name: "tolerance",
                reason: "must be finite and non-negative".to_string(),
            });
        }
        if !self.0.learning_rate.is_finite() || self.0.learning_rate <= F::zero() {
            return Err(OrdinalError::InvalidParameter {
                name: "learning_rate",
                reason: "must be finite and positive".to_string(),
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

/// A fitted adjacent-category model.
///
/// The interpretation of every number in here is local: [`AdjacentCategory::coefficients`] and
/// [`AdjacentCategory::thresholds`] describe the step from one level to the next one up, NOT the
/// cumulative "at or below this cut" contrast a proportional-odds model reports. See the module
/// documentation for the comparison.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AdjacentCategory<F> {
    coefficients: Array1<F>,
    /// `K - 1` level contrasts, `theta_k` comparing level `k + 1` against level `k`.
    thresholds: Array1<F>,
    n_classes: usize,
    iterations: usize,
    converged: bool,
}

impl<F: Float> AdjacentCategory<F> {
    /// Starts a builder for the hyperparameters.
    pub fn params() -> AdjacentCategoryParams<F> {
        AdjacentCategoryParams::new()
    }

    /// One coefficient per feature, shared by every adjacent pair of levels.
    ///
    /// `exp(coefficients[j])` is the factor by which one unit of feature `j` multiplies the odds of
    /// level `k + 1` over level `k` — the same factor at every `k`. That is the adjacent-category
    /// analogue of the proportional-odds assumption, and it is what makes a single number
    /// summarize the feature. It is *not* a cumulative odds ratio: do not compare it to a
    /// coefficient printed by [`crate::logistic::OrdinalLogistic`] without converting.
    ///
    /// A positive entry pushes samples toward HIGHER levels.
    pub fn coefficients(&self) -> &Array1<F> {
        &self.coefficients
    }

    /// The `K - 1` fitted level contrasts.
    ///
    /// `thresholds[k]` is the log odds of level `k + 1` against level `k` at `x . beta = 0`. Unlike
    /// the cut points of a cumulative model these need NOT increase — they are `K - 1` free
    /// intercepts, one per adjacent pair, and a dip simply means that level is rarer than its
    /// neighbour below. Reading them as ordered cut points is a misinterpretation.
    pub fn thresholds(&self) -> &Array1<F> {
        &self.thresholds
    }

    pub fn n_classes(&self) -> usize {
        self.n_classes
    }

    pub fn n_features(&self) -> usize {
        self.coefficients.len()
    }

    pub fn iterations(&self) -> usize {
        self.iterations
    }

    /// False when the optimizer hit `max_iterations` before the objective settled.
    pub fn converged(&self) -> bool {
        self.converged
    }

    /// The shared linear predictor `x . beta`.
    ///
    /// Adding it to `thresholds()[k]` gives the log odds of level `k + 1` against level `k` for
    /// this sample; it is not itself a latent score to be cut into levels the way
    /// [`crate::ridge::OrdinalRidge`] uses one.
    pub fn score(&self, row: &ArrayView1<'_, F>) -> Result<F> {
        if row.len() != self.coefficients.len() {
            return Err(OrdinalError::FeatureWidthMismatch {
                expected: self.coefficients.len(),
                found: row.len(),
            });
        }
        Ok(row.dot(&self.coefficients))
    }

    /// Log odds of level `lower_level + 1` against level `lower_level` for this sample.
    ///
    /// This is the quantity the model is defined by, exposed directly because it is the one users
    /// actually want to quote: "for this customer, the odds of a 4-star rating rather than a 3-star
    /// one are `exp(...)`".
    pub fn adjacent_log_odds(&self, row: &ArrayView1<'_, F>, lower_level: usize) -> Result<F> {
        if lower_level + 1 >= self.n_classes {
            return Err(OrdinalError::InvalidParameter {
                name: "lower_level",
                reason: format!(
                    "must be below the top level {} so a level above it exists, got {lower_level}",
                    self.n_classes - 1
                ),
            });
        }
        Ok(self.thresholds[lower_level] + self.score(row)?)
    }

    /// The level contrasts in the flat form the scoring helpers work on.
    ///
    /// Copied rather than borrowed through `as_slice`: that returns `None` for a non-contiguous
    /// array, and any fallback for it would silently score the wrong number of levels.
    fn contrasts(&self) -> Vec<F> {
        self.thresholds.iter().copied().collect()
    }

    /// Shared body of the probability accessors, so the guards live in exactly one place.
    fn probabilities_with(&self, row: &ArrayView1<'_, F>, contrasts: &[F]) -> Result<Vec<F>> {
        let eta = self.score(row)?;
        if !eta.is_finite() {
            return Err(OrdinalError::NonFinite {
                context: "the linear predictor",
            });
        }
        Ok(level_probabilities(eta, contrasts, self.n_classes))
    }

    /// Per-level probabilities for one sample, in level order. Always sums to 1.
    pub fn predict_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        self.probabilities_with(row, &self.contrasts())
    }

    /// Per-level probabilities for every row, as an `n_samples x n_classes` matrix.
    pub fn predict_probabilities_batch<D: Data<Elem = F>>(
        &self,
        records: &ArrayBase<D, Ix2>,
    ) -> Result<Array2<F>> {
        let contrasts = self.contrasts();
        let mut out = Array2::zeros((records.nrows(), self.n_classes));
        for (index, row) in records.rows().into_iter().enumerate() {
            let probabilities = self.probabilities_with(&row, &contrasts)?;
            for (level, value) in probabilities.into_iter().enumerate() {
                out[[index, level]] = value;
            }
        }
        Ok(out)
    }

    /// Probability-weighted mean level.
    ///
    /// Meaningful precisely because the target is ordered: rounding it optimizes rank error, where
    /// the modal prediction from [`PredictInplace`] optimizes exact-match accuracy.
    pub fn predict_expected_level(&self, row: &ArrayView1<'_, F>) -> Result<F> {
        let probabilities = self.predict_probabilities(row)?;
        Ok(probabilities
            .iter()
            .enumerate()
            .map(|(level, p)| F::cast(level) * *p)
            .fold(F::zero(), |acc, v| acc + v))
    }
}

impl<F: Float, D: Data<Elem = F>, T: AsSingleTargets<Elem = usize>>
    Fit<ArrayBase<D, Ix2>, T, OrdinalError> for AdjacentCategoryValidParams<F>
{
    type Object = AdjacentCategory<F>;

    /// Fits the model on ranks in `0..n_levels`, ordered lowest to highest by the caller.
    ///
    /// The ordering of the levels is the caller's contract: rank 0 must be the lowest level. This
    /// crate cannot verify it, and getting it wrong trains a model that is confidently backwards.
    fn fit(&self, dataset: &DatasetBase<ArrayBase<D, Ix2>, T>) -> Result<Self::Object> {
        let records = dataset.records();
        let targets = dataset.targets.as_single_targets();
        let targets: Vec<usize> = targets.iter().copied().collect();

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

        let n_cuts = n_classes - 1;
        let mut beta = Array1::<F>::zeros(n_features);
        let mut thresholds = initial_thresholds::<F>(&targets, n_classes);

        // Adam moments over the concatenated parameter vector [beta, thresholds]. Both blocks are
        // free — unlike the cumulative model there is no ordering constraint to parameterize away,
        // because adjacent contrasts are allowed to move in either direction.
        let n_params = n_features + n_cuts;
        let mut m = vec![F::zero(); n_params];
        let mut v = vec![F::zero(); n_params];
        let beta1 = F::cast(0.9);
        let beta2 = F::cast(0.999);
        let eps = F::cast(1e-8);

        let mut previous = F::infinity();
        let mut iterations = 0;
        let mut converged = false;

        for step in 1..=self.max_iterations {
            iterations = step;
            let (objective, grad_beta, grad_theta) = objective_and_gradient(
                records,
                &targets,
                &beta,
                &thresholds,
                n_classes,
                self.alpha,
            )?;

            if !objective.is_finite() {
                return Err(OrdinalError::NonFinite {
                    context: "the log-likelihood",
                });
            }

            // Relative test, so the stopping rule does not depend on the sample count. Skipped on
            // the first step: there is no previous objective to compare against yet, and the
            // infinite sentinel would satisfy any relative tolerance.
            if step > 1
                && (previous - objective).abs() <= self.tolerance * (previous.abs() + F::one())
            {
                converged = true;
                break;
            }
            previous = objective;

            let bias_correction1 = F::one() - beta1.powi(step as i32);
            let bias_correction2 = F::one() - beta2.powi(step as i32);

            for index in 0..n_params {
                let gradient = if index < n_features {
                    grad_beta[index]
                } else {
                    grad_theta[index - n_features]
                };
                m[index] = beta1 * m[index] + (F::one() - beta1) * gradient;
                v[index] = beta2 * v[index] + (F::one() - beta2) * gradient * gradient;
                let m_hat = m[index] / bias_correction1;
                let v_hat = v[index] / bias_correction2;
                let update = self.learning_rate * m_hat / (v_hat.sqrt() + eps);
                if index < n_features {
                    beta[index] -= update;
                } else {
                    thresholds[index - n_features] -= update;
                }
            }
        }

        if beta
            .iter()
            .chain(thresholds.iter())
            .any(|value| !value.is_finite())
        {
            return Err(OrdinalError::NonFinite {
                context: "the fitted parameters",
            });
        }

        Ok(AdjacentCategory {
            coefficients: beta,
            thresholds: Array1::from(thresholds),
            n_classes,
            iterations,
            converged,
        })
    }
}

impl<F: Float, D: Data<Elem = F>> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>>
    for AdjacentCategory<F>
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

        let contrasts = self.contrasts();
        for (index, row) in x.rows().into_iter().enumerate() {
            let eta = row.dot(&self.coefficients);
            y[index] = argmax(&level_probabilities(eta, &contrasts, self.n_classes));
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// Seeds the level contrasts at the MLE for `beta = 0`, which is the log ratio of neighbouring
/// level counts. Starting there rather than at zero saves a large number of iterations on skewed
/// level distributions, where the contrasts have to travel several units before the fit even
/// resembles the data.
fn initial_thresholds<F: Float>(targets: &[usize], n_classes: usize) -> Vec<F> {
    let mut counts = vec![F::zero(); n_classes];
    for &rank in targets {
        counts[rank] += F::one();
    }

    // Jeffreys-style pseudo-count. A declared-but-unobserved level would otherwise start at a log
    // ratio of minus infinity, and every later step would inherit the NaN.
    let prior = F::cast(0.5);
    counts
        .windows(2)
        .map(|pair| ((pair[1] + prior) / (pair[0] + prior)).ln())
        .collect()
}

/// Unnormalized log-probabilities of the `K` levels: `z_k = sum_{j<k} theta_j + k * eta`.
///
/// Summing the defining adjacent-category equation up from level 0 is what produces the `k * eta`
/// term, and that growing multiple of the score is exactly why these must stay in log space.
fn level_log_scores<F: Float>(eta: F, thresholds: &[F], n_classes: usize) -> Vec<F> {
    debug_assert_eq!(thresholds.len() + 1, n_classes);

    let mut scores = Vec::with_capacity(n_classes);
    scores.push(F::zero());
    let mut offset = F::zero();
    for (index, theta) in thresholds.iter().enumerate() {
        offset += *theta;
        scores.push(offset + F::cast(index + 1) * eta);
    }
    scores
}

/// `ln(sum(exp(values)))`, shifted by the maximum so no term can overflow.
fn log_sum_exp<F: Float>(values: &[F]) -> F {
    let max = values.iter().fold(
        F::neg_infinity(),
        |acc, value| {
            if *value > acc { *value } else { acc }
        },
    );
    if !max.is_finite() {
        return max;
    }
    let total = values
        .iter()
        .fold(F::zero(), |acc, value| acc + (*value - max).exp());
    max + total.ln()
}

/// Per-level probabilities from the shared score and the level contrasts.
///
/// A non-finite score can only arrive from a caller's feature row (fitted parameters are checked),
/// and there is no sensible distribution for it; a uniform one keeps prediction total and
/// deterministic instead of propagating NaN into an `argmax`. [`AdjacentCategory::predict_probabilities`]
/// reports it as an error rather than hiding it.
fn level_probabilities<F: Float>(eta: F, thresholds: &[F], n_classes: usize) -> Vec<F> {
    if !eta.is_finite() {
        return vec![F::one() / F::cast(n_classes); n_classes];
    }

    let scores = level_log_scores(eta, thresholds, n_classes);
    let normalizer = log_sum_exp(&scores);
    let mut probabilities: Vec<F> = scores
        .iter()
        .map(|score| (*score - normalizer).exp())
        .collect();

    // The largest entry contributes exactly `exp(0) = 1`, so the total is at least one and the
    // division is always safe. Normalizing anyway costs nothing and absorbs the rounding.
    let total = probabilities
        .iter()
        .fold(F::zero(), |acc, value| acc + *value);
    if total > F::zero() {
        for value in probabilities.iter_mut() {
            *value /= total;
        }
    }
    probabilities
}

/// Index of the largest entry, resolving ties toward the LOWER level so predictions are
/// deterministic regardless of floating-point noise.
fn argmax<F: Float>(values: &[F]) -> usize {
    let mut best = 0;
    for (index, value) in values.iter().enumerate().skip(1) {
        if *value > values[best] {
            best = index;
        }
    }
    best
}

/// Penalized negative log-likelihood and its gradient with respect to `beta` and the level
/// contrasts.
///
/// Both gradient blocks are moment conditions, which is the cleanest way to see whether the
/// derivation is right: the `beta` block matches the expected level against the observed one, and
/// contrast `theta_j` matches the predicted probability of landing above level `j` against whether
/// the sample actually did.
fn objective_and_gradient<F: Float, D: Data<Elem = F>>(
    records: &ArrayBase<D, Ix2>,
    targets: &[usize],
    beta: &Array1<F>,
    thresholds: &[F],
    n_classes: usize,
    alpha: F,
) -> Result<(F, Array1<F>, Vec<F>)> {
    let n_cuts = n_classes - 1;
    let mut negative_log_likelihood = F::zero();
    let mut grad_beta = Array1::<F>::zeros(beta.len());
    let mut grad_theta = vec![F::zero(); n_cuts];

    for (row, &rank) in records.rows().into_iter().zip(targets.iter()) {
        let eta = row.dot(beta);
        let scores = level_log_scores(eta, thresholds, n_classes);
        let observed_score = match scores.get(rank) {
            Some(value) => *value,
            None => return Err(OrdinalError::RankOutOfRange { rank, n_classes }),
        };

        // `lse - z_y` is `-ln P(y)` computed without ever forming a ratio, so it stays finite even
        // when the observed level's probability underflows to zero in linear space.
        let normalizer = log_sum_exp(&scores);
        negative_log_likelihood = negative_log_likelihood + normalizer - observed_score;

        let probabilities: Vec<F> = scores
            .iter()
            .map(|score| (*score - normalizer).exp())
            .collect();

        // d/d eta: level `k`'s score carries the factor `k`, so differentiating the normalizer
        // gives the expected level and differentiating the observed score gives the observed one.
        let expected_level = probabilities
            .iter()
            .enumerate()
            .fold(F::zero(), |acc, (level, p)| acc + F::cast(level) * *p);
        let d_eta = expected_level - F::cast(rank);
        for (index, feature) in row.iter().enumerate() {
            grad_beta[index] += d_eta * *feature;
        }

        // theta_j enters the score of every level above `j`, so its derivative is the upper-tail
        // probability minus the indicator of actually being up there.
        let mut tail = F::zero();
        for cut in (0..n_cuts).rev() {
            tail += probabilities[cut + 1];
            let indicator = if rank > cut { F::one() } else { F::zero() };
            grad_theta[cut] = grad_theta[cut] + tail - indicator;
        }
    }

    // L2 on the coefficients only. Penalizing the contrasts would shrink the level distribution
    // toward uniform, which is an assertion about the data rather than about model complexity.
    let half = F::cast(0.5);
    let penalty = beta.iter().fold(F::zero(), |acc, b| acc + *b * *b);
    negative_log_likelihood += half * alpha * penalty;
    for (index, value) in beta.iter().enumerate() {
        grad_beta[index] += alpha * *value;
    }

    if !negative_log_likelihood.is_finite()
        || grad_beta.iter().any(|g| !g.is_finite())
        || grad_theta.iter().any(|g| !g.is_finite())
    {
        return Err(OrdinalError::NonFinite {
            context: "the gradient",
        });
    }

    Ok((negative_log_likelihood, grad_beta, grad_theta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use linfa::traits::Predict;
    use ndarray::array;

    /// One informative feature, levels cut at fixed points.
    fn synthetic(n: usize) -> DatasetBase<Array2<f64>, Array1<usize>> {
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
        DatasetBase::new(records, Array1::from(targets))
    }

    /// A model with known parameters, so the probability construction can be checked against the
    /// model definition without any dependence on the optimizer.
    fn fixed_model() -> AdjacentCategory<f64> {
        AdjacentCategory {
            coefficients: array![0.8, -0.35],
            thresholds: array![-0.4, 0.9],
            n_classes: 3,
            iterations: 0,
            converged: true,
        }
    }

    /// The defining identity of the model. If the log-sum-exp reparameterization is wrong in any
    /// way — a missing `k`, an off-by-one in the cumulative offsets — this is what catches it.
    #[test]
    fn probabilities_realize_the_adjacent_category_definition() {
        let model = fixed_model();
        for point in [[-2.0, 1.0], [0.0, 0.0], [1.5, -2.0], [3.0, 3.0]] {
            let row = array![point[0], point[1]];
            let view = row.view();
            let probabilities = model.predict_probabilities(&view).unwrap();
            let eta = model.score(&view).unwrap();

            for level in 0..model.n_classes() - 1 {
                assert!(probabilities[level] > 0.0, "{probabilities:?}");
                let observed = (probabilities[level + 1] / probabilities[level]).ln();
                let expected = model.thresholds()[level] + eta;
                assert!(
                    (observed - expected).abs() < 1e-9,
                    "at {point:?} level {level}: probability ratio gave {observed}, parameters say {expected}"
                );
                assert!((model.adjacent_log_odds(&view, level).unwrap() - expected).abs() < 1e-12);
            }
        }
    }

    /// The gradient is the one place a wrong derivation is silent: the optimizer still converges,
    /// just to the wrong parameters. Central differences are the only real check.
    #[test]
    fn analytic_gradient_matches_finite_differences() {
        let dataset = synthetic(40);
        let targets: Vec<usize> = dataset.targets.iter().copied().collect();
        let beta = array![0.35];
        let thresholds = vec![-0.4, 0.2];
        let alpha = 0.7;
        let step = 1e-6;

        let (_, grad_beta, grad_theta) =
            objective_and_gradient(dataset.records(), &targets, &beta, &thresholds, 3, alpha)
                .unwrap();

        let objective_at = |b: &Array1<f64>, t: &[f64]| {
            objective_and_gradient(dataset.records(), &targets, b, t, 3, alpha)
                .unwrap()
                .0
        };

        let mut shifted = beta.clone();
        shifted[0] += step;
        let up = objective_at(&shifted, &thresholds);
        shifted[0] -= 2.0 * step;
        let down = objective_at(&shifted, &thresholds);
        let numeric = (up - down) / (2.0 * step);
        assert!(
            (numeric - grad_beta[0]).abs() < 1e-5,
            "beta gradient: analytic {} vs numeric {numeric}",
            grad_beta[0]
        );

        for index in 0..thresholds.len() {
            let mut shifted_thresholds = thresholds.clone();
            shifted_thresholds[index] += step;
            let up = objective_at(&beta, &shifted_thresholds);
            shifted_thresholds[index] -= 2.0 * step;
            let down = objective_at(&beta, &shifted_thresholds);
            let numeric = (up - down) / (2.0 * step);
            assert!(
                (numeric - grad_theta[index]).abs() < 1e-5,
                "theta[{index}] gradient: analytic {} vs numeric {numeric}",
                grad_theta[index]
            );
        }
    }

    /// The likelihood the optimizer sees must be the likelihood the probabilities describe. The two
    /// are computed by different routes — log space versus normalized space — so agreement pins
    /// down the bookkeeping in both.
    #[test]
    fn objective_matches_the_probability_path() {
        let dataset = synthetic(50);
        let targets: Vec<usize> = dataset.targets.iter().copied().collect();
        let beta = array![0.6];
        let thresholds = vec![-0.2, 0.7];

        let (objective, _, _) =
            objective_and_gradient(dataset.records(), &targets, &beta, &thresholds, 3, 0.0)
                .unwrap();

        let mut manual = 0.0;
        for (row, &rank) in dataset.records().rows().into_iter().zip(targets.iter()) {
            let probabilities = level_probabilities(row.dot(&beta), &thresholds, 3);
            manual -= probabilities[rank].ln();
        }
        assert!(
            (objective - manual).abs() < 1e-9,
            "objective {objective} vs probability path {manual}"
        );
    }

    #[test]
    fn recovers_monotone_structure() {
        let dataset = synthetic(300);
        let model = AdjacentCategory::params()
            .alpha(1e-4)
            .max_iterations(2000)
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.n_classes(), 3);
        assert_eq!(model.thresholds().len(), 2);
        assert!(
            model.coefficients()[0] > 0.0,
            "coefficient {} should be positive",
            model.coefficients()[0]
        );

        let predictions = model.predict(&dataset);
        let correct = predictions
            .iter()
            .zip(dataset.targets.iter())
            .filter(|(a, b)| a == b)
            .count();
        assert!(
            correct as f64 / dataset.targets.len() as f64 > 0.9,
            "training accuracy was {}",
            correct as f64 / dataset.targets.len() as f64
        );
    }

    /// Raising the score can only move probability mass up the ordering — the property that makes
    /// this an ordinal model rather than a plain multinomial one.
    #[test]
    fn expected_level_increases_with_the_score() {
        let dataset = synthetic(200);
        let model = AdjacentCategory::params()
            .alpha(1e-3)
            .max_iterations(1000)
            .fit(&dataset)
            .unwrap();

        let low = model.predict_expected_level(&array![-3.0].view()).unwrap();
        let mid = model.predict_expected_level(&array![0.0].view()).unwrap();
        let high = model.predict_expected_level(&array![3.0].view()).unwrap();
        assert!(low < mid && mid < high, "{low} {mid} {high}");
    }

    #[test]
    fn probabilities_are_a_distribution() {
        let dataset = synthetic(120);
        let model = AdjacentCategory::params().fit(&dataset).unwrap();

        let batch = model
            .predict_probabilities_batch(dataset.records())
            .unwrap();
        assert_eq!(batch.dim(), (120, 3));
        for row in batch.rows() {
            let total: f64 = row.iter().sum();
            assert!(
                (total - 1.0).abs() < 1e-9,
                "probabilities summed to {total}"
            );
            assert!(row.iter().all(|p| *p >= 0.0));
        }
    }

    /// With a feature column that carries no information the fit reduces to the empirical level
    /// distribution, which is the one case where the answer is known in closed form.
    #[test]
    fn uninformative_feature_reproduces_the_level_frequencies() {
        let counts = [100usize, 60, 40];
        let total: usize = counts.iter().sum();
        let mut targets = Vec::with_capacity(total);
        for (level, count) in counts.iter().enumerate() {
            targets.extend(std::iter::repeat_n(level, *count));
        }
        let dataset = DatasetBase::new(Array2::<f64>::zeros((total, 1)), Array1::from(targets));

        let model = AdjacentCategory::params()
            .alpha(1e-6)
            .learning_rate(0.02)
            .tolerance(0.0)
            .max_iterations(4000)
            .fit(&dataset)
            .unwrap();

        let probabilities = model.predict_probabilities(&array![0.0].view()).unwrap();
        for (level, count) in counts.iter().enumerate() {
            let empirical = *count as f64 / total as f64;
            assert!(
                (probabilities[level] - empirical).abs() < 0.03,
                "level {level}: fitted {} vs empirical {empirical}",
                probabilities[level]
            );
        }
    }

    /// Scores multiply by the level index, so a naive exponentiation overflows well before the
    /// parameters look unreasonable. Eight levels at a score of 1000 would need `exp(7000)`.
    #[test]
    fn extreme_scores_stay_finite() {
        let model = AdjacentCategory {
            coefficients: array![50.0f64],
            thresholds: Array1::zeros(7),
            n_classes: 8,
            iterations: 0,
            converged: true,
        };

        for (x, expected) in [(20.0, 7usize), (-20.0, 0)] {
            let row = array![x];
            let probabilities = model.predict_probabilities(&row.view()).unwrap();
            assert!(
                probabilities.iter().all(|p| p.is_finite()),
                "{probabilities:?}"
            );
            let total: f64 = probabilities.iter().sum();
            assert!((total - 1.0).abs() < 1e-9, "summed to {total}");
            assert_eq!(argmax(&probabilities), expected);
        }
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            AdjacentCategory::<f64>::params().alpha(-1.0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            AdjacentCategory::<f64>::params().max_iterations(0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            AdjacentCategory::<f64>::params().learning_rate(0.0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            AdjacentCategory::<f64>::params().tolerance(-1e-3).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            AdjacentCategory::<f64>::params().n_levels(1).check(),
            Err(OrdinalError::TooFewClasses { .. })
        ));
    }

    #[test]
    fn rejects_degenerate_training_sets() {
        let empty = DatasetBase::new(Array2::<f64>::zeros((0, 2)), Array1::<usize>::zeros(0));
        assert!(matches!(
            AdjacentCategory::params().fit(&empty),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let no_features = DatasetBase::new(Array2::<f64>::zeros((3, 0)), array![0usize, 1, 1]);
        assert!(matches!(
            AdjacentCategory::params().fit(&no_features),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let mismatched = DatasetBase::new(array![[1.0], [2.0], [3.0]], array![0usize, 1]);
        assert!(matches!(
            AdjacentCategory::params().fit(&mismatched),
            Err(OrdinalError::LengthMismatch { .. })
        ));

        let single_level = DatasetBase::new(array![[1.0], [2.0]], array![0usize, 0]);
        assert!(matches!(
            AdjacentCategory::params().fit(&single_level),
            Err(OrdinalError::TooFewClasses { .. })
        ));

        let out_of_range = DatasetBase::new(array![[1.0], [2.0]], array![0usize, 5]);
        assert!(matches!(
            AdjacentCategory::params().n_levels(2).fit(&out_of_range),
            Err(OrdinalError::RankOutOfRange { .. })
        ));

        let non_finite = DatasetBase::new(array![[1.0], [f64::NAN]], array![0usize, 1]);
        assert!(matches!(
            AdjacentCategory::params().fit(&non_finite),
            Err(OrdinalError::NonFinite { .. })
        ));
    }

    #[test]
    fn rejects_bad_prediction_input() {
        let model = fixed_model();
        assert!(matches!(
            model.predict_probabilities(&array![1.0].view()),
            Err(OrdinalError::FeatureWidthMismatch {
                expected: 2,
                found: 1
            })
        ));
        assert!(matches!(
            model.predict_probabilities(&array![f64::INFINITY, 0.0].view()),
            Err(OrdinalError::NonFinite { .. })
        ));
        assert!(matches!(
            model.adjacent_log_odds(&array![0.0, 0.0].view(), 2),
            Err(OrdinalError::InvalidParameter { .. })
        ));
    }

    /// A declared level absent from the sample must still occupy its position in the ordering, and
    /// the pseudo-count is what keeps its contrast finite.
    #[test]
    fn declared_levels_survive_absence_from_the_sample() {
        let records = array![[0.0], [1.0], [2.0], [3.0]];
        let dataset = DatasetBase::new(records, array![0usize, 0, 2, 2]);
        let model = AdjacentCategory::params()
            .n_levels(3)
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.n_classes(), 3);
        assert_eq!(model.thresholds().len(), 2);
        assert!(model.thresholds().iter().all(|t: &f64| t.is_finite()));

        let probabilities = model.predict_probabilities(&array![1.5].view()).unwrap();
        assert!(probabilities.iter().all(|p: &f64| p.is_finite()));
    }

    #[test]
    fn non_convergence_is_reported() {
        let dataset = synthetic(60);
        let model = AdjacentCategory::params()
            .max_iterations(1)
            .fit(&dataset)
            .unwrap();
        assert_eq!(model.iterations(), 1);
        assert!(!model.converged());
    }

    /// The estimator must work at f32 too, since it is generic over `linfa::Float`.
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
        let model = AdjacentCategory::<f32>::params().fit(&dataset).unwrap();

        assert!(model.coefficients()[0] > 0.0);
        let probabilities = model.predict_probabilities(&array![2.0f32].view()).unwrap();
        let total: f32 = probabilities.iter().sum();
        assert!((total - 1.0).abs() < 1e-5);
    }
}

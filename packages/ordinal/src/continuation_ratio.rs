//! Continuation-ratio model: an ordered target read as a *sequential* process.
//!
//! Every other estimator in this crate asks a question about the whole scale at once — "how far up
//! is this sample?". The continuation-ratio model asks a different one, level by level: *given that
//! the sample got as far as level `k`, does it stop there?* Its native quantity is
//!
//! ```text
//! h_k(x) = P(y = k | y >= k) = G(theta_k + x . beta_k)
//! ```
//!
//! which is a *conditional* stopping probability, not a cumulative one. That conditioning is the
//! whole point and makes this a genuinely different family from the cumulative-link model in
//! [`crate::logistic`]: it describes a progression through the levels that can halt at each step,
//! which is the right story for "how far did this escalate before it stopped" — an incident that
//! reached severity 3 had to pass through 1 and 2 first, and the interesting question at each stage
//! is whether it went further.
//!
//! # Why the sub-models are fitted on shrinking subsets
//!
//! Sub-model `k` is fitted only on the rows that *reached* level `k`, i.e. `y >= k`, with the binary
//! target `y == k`. This is what separates it from [`crate::frank_hall`], whose `K - 1` binary
//! problems are each fitted on *every* row. Conditioning the fit on having reached the level is what
//! makes the resulting `h_k` a conditional probability, and it is what lets the level probabilities
//! be recovered exactly by the chain rule rather than by differencing two independent fits that
//! nothing forces to agree.
//!
//! # The complementary log-log special case
//!
//! With [`Link::CLogLog`] this model *is* the discrete-time proportional-hazards (grouped survival)
//! model of Prentice & Gloeckler (1978). `h_k` is then the discrete hazard at step `k`, the survival
//! function factorizes as `P(y > k | x) = exp(-sum_{j <= k} exp(theta_j + x . beta_j))`, and a shared
//! coefficient vector multiplies every hazard by the same `exp(x . beta)` — proportional hazards,
//! exactly. If the target is "how long / how far until something stopped", pairing this family with
//! `CLogLog` is not a coincidence but the same model written twice.
//!
//! Agresti, A. (2010), *Analysis of Ordinal Categorical Data*, chapter 4 (continuation-ratio logits).

use crate::error::{OrdinalError, Result};
use crate::link::Link;
use crate::math::min_probability;
use linfa::dataset::AsSingleTargets;
use linfa::traits::{Fit, PredictInplace};
use linfa::{DatasetBase, Float, ParamGuard};
use ndarray::{Array1, Array2, ArrayBase, ArrayView1, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Checked hyperparameters for [`ContinuationRatio`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContinuationRatioValidParams<F> {
    alpha: F,
    max_iterations: usize,
    tolerance: F,
    learning_rate: F,
    n_levels: Option<usize>,
    link: Link,
}

impl<F: Float> ContinuationRatioValidParams<F> {
    /// L2 penalty applied to each sub-model's coefficients.
    pub fn alpha(&self) -> F {
        self.alpha
    }

    /// Maximum optimizer iterations *per sub-model*.
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

    /// The CDF each conditional stopping probability is read through.
    pub fn link(&self) -> Link {
        self.link
    }
}

/// Unchecked hyperparameters for [`ContinuationRatio`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContinuationRatioParams<F>(ContinuationRatioValidParams<F>);

impl<F: Float> Default for ContinuationRatioParams<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Float> ContinuationRatioParams<F> {
    pub fn new() -> Self {
        Self(ContinuationRatioValidParams {
            alpha: F::one(),
            max_iterations: 500,
            tolerance: F::cast(1e-7),
            learning_rate: F::cast(0.1),
            n_levels: None,
            link: Link::Logit,
        })
    }

    /// The CDF mapping each sub-model's linear score to a conditional stopping probability.
    ///
    /// [`Link::CLogLog`] turns the model into the discrete-time proportional-hazards model; see the
    /// module documentation.
    pub fn link(mut self, link: Link) -> Self {
        self.0.link = link;
        self
    }

    /// L2 penalty on each sub-model's coefficients. Intercepts are never penalized.
    ///
    /// The penalty is an absolute quantity added to a *summed* log-likelihood, so a fixed `alpha`
    /// shrinks the high levels harder than the low ones — their conditioning subsets hold fewer
    /// rows. That is the intended behaviour here rather than an accident: the high sub-models are
    /// exactly the ones with too little data to be trusted unshrunk.
    pub fn alpha(mut self, alpha: F) -> Self {
        self.0.alpha = alpha;
        self
    }

    /// Maximum optimizer iterations per sub-model.
    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.0.max_iterations = max_iterations;
        self
    }

    /// Relative change in a sub-model's objective below which its fit stops.
    pub fn tolerance(mut self, tolerance: F) -> Self {
        self.0.tolerance = tolerance;
        self
    }

    /// Adam step size.
    pub fn learning_rate(mut self, learning_rate: F) -> Self {
        self.0.learning_rate = learning_rate;
        self
    }

    /// Declares the number of ordered levels. Left unset, it is inferred as `max rank + 1`.
    ///
    /// Unlike the cumulative-link estimators, a declared level that never occurs in the training
    /// sample cannot be carried here: its sub-model would have no positive rows. See
    /// [`ContinuationRatio`] for what is rejected and why.
    pub fn n_levels(mut self, n_levels: usize) -> Self {
        self.0.n_levels = Some(n_levels);
        self
    }
}

impl<F: Float> ParamGuard for ContinuationRatioParams<F> {
    type Checked = ContinuationRatioValidParams<F>;
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

/// A fitted continuation-ratio model: `K - 1` conditional stopping sub-models.
///
/// Sub-model `k` carries its own intercept `theta_k` *and* its own coefficient vector `beta_k`, so
/// nothing here assumes proportional odds. The price is `(K - 1) * (n_features + 1)` parameters
/// instead of `n_features + K - 1`.
///
/// # The high levels are the least reliable, by construction
///
/// Sub-model `k` sees only the rows with `y >= k`, so the training subsets shrink monotonically as
/// `k` grows — [`Self::subset_sizes`] reports exactly how far. On a realistic ordered target, where
/// most mass sits at the low levels, the top sub-model may be fitted on a handful of rows, and its
/// intercept and coefficients are correspondingly noisy. This asymmetry is inherent to the family,
/// not a defect of this implementation: conditioning on having reached a level is what gives the
/// model its sequential reading, and there is no more data about the rare far end than there is.
/// Read [`Self::subset_sizes`] before trusting the top thresholds, and raise [`alpha`] when they
/// are thin.
///
/// # Probabilities
///
/// Unlike the threshold losses in [`crate::logistic`], this family always has a probability model:
/// the sub-models are fitted by maximum likelihood, and the chain rule
///
/// ```text
/// P(y = 0)     = h_0
/// P(y = k)     = h_k * prod_{j < k} (1 - h_j)
/// P(y = K - 1) =       prod_{j < K - 1} (1 - h_j)
/// ```
///
/// telescopes to exactly 1 — no renormalization is applied, because none is needed. See
/// [`Self::predict_probabilities`].
///
/// # Example
///
/// ```no_run
/// use linfa::prelude::*;
/// use flow_like_ordinal::Link;
/// use flow_like_ordinal::continuation_ratio::ContinuationRatio;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let dataset: linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>> = todo!();
/// // CLogLog makes this the discrete-time proportional-hazards model.
/// let model = ContinuationRatio::params()
///     .link(Link::CLogLog)
///     .alpha(1.0)
///     .fit(&dataset)?;
///
/// let levels = model.predict(&dataset);
/// let first = dataset.records().row(0);
/// let probabilities = model.predict_probabilities(&first)?;
/// let hazards = model.conditional_probabilities(&first)?;
/// # Ok(())
/// # }
/// ```
///
/// [`alpha`]: ContinuationRatioParams::alpha
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContinuationRatio<F> {
    /// Row `k` is sub-model `k`'s coefficient vector, `K - 1` rows by `n_features` columns.
    coefficients: Array2<F>,
    /// Sub-model `k`'s intercept.
    intercepts: Array1<F>,
    n_classes: usize,
    link: Link,
    iterations: Vec<usize>,
    converged: Vec<bool>,
    subset_sizes: Vec<usize>,
}

impl<F: Float> ContinuationRatio<F> {
    /// Starts a builder for the hyperparameters.
    pub fn params() -> ContinuationRatioParams<F> {
        ContinuationRatioParams::new()
    }

    /// One coefficient row per sub-model, lowest level first.
    ///
    /// A positive entry in row `k` raises `P(y = k | y >= k)`, i.e. it makes the progression more
    /// likely to STOP at level `k`. On a target where the feature pushes samples upward, these
    /// entries are therefore typically negative — the opposite sign to
    /// [`crate::ridge::OrdinalRidge::coefficients`], which scores the level itself.
    pub fn coefficients(&self) -> &Array2<F> {
        &self.coefficients
    }

    /// Sub-model intercepts, lowest level first.
    pub fn intercepts(&self) -> &Array1<F> {
        &self.intercepts
    }

    pub fn n_classes(&self) -> usize {
        self.n_classes
    }

    pub fn n_features(&self) -> usize {
        self.coefficients.ncols()
    }

    /// The CDF the conditional stopping probabilities are read through.
    pub fn link(&self) -> Link {
        self.link
    }

    /// Optimizer iterations spent on each sub-model, lowest level first.
    pub fn iterations(&self) -> &[usize] {
        &self.iterations
    }

    /// True only when *every* sub-model's objective settled before `max_iterations`.
    ///
    /// A single stubborn sub-model — usually the top one, fitted on the fewest rows — is enough to
    /// make this false. [`Self::converged_per_level`] says which.
    pub fn converged(&self) -> bool {
        self.converged.iter().all(|flag| *flag)
    }

    /// Per-sub-model convergence, lowest level first.
    pub fn converged_per_level(&self) -> &[bool] {
        &self.converged
    }

    /// How many training rows each sub-model saw: `subset_sizes()[k]` counts the rows with `y >= k`.
    ///
    /// Non-increasing by construction. This is the honest measure of how much evidence stands behind
    /// each level's parameters, and the reason the high levels deserve scepticism.
    pub fn subset_sizes(&self) -> &[usize] {
        &self.subset_sizes
    }

    /// The conditional stopping probabilities `h_k = P(y = k | y >= k)` for one sample, `K - 1` of
    /// them in level order.
    ///
    /// These are the model's native quantity — under [`Link::CLogLog`] they are the discrete hazards
    /// of the equivalent survival model. They are NOT a distribution and do not sum to anything in
    /// particular; use [`Self::predict_probabilities`] for that.
    pub fn conditional_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        self.check_width(row.len())?;
        Ok(self.conditional_unchecked(row))
    }

    /// Per-level probabilities for one sample, in level order. Sums to 1.
    ///
    /// Reconstructed from the conditional stopping probabilities by the chain rule: reaching level
    /// `k` at all means not having stopped at any earlier one, so `P(y = k)` is `h_k` times the
    /// running product of the `1 - h_j` below it, and the top level is whatever survives every step.
    /// The sum telescopes to exactly 1, so — unlike the cumulative-link model, whose level
    /// probabilities are differences that can be driven slightly negative by numerically
    /// indistinguishable thresholds — nothing is clamped or renormalized here.
    pub fn predict_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        self.check_width(row.len())?;
        Ok(self.probabilities_unchecked(row))
    }

    /// Per-level probabilities for every row, as an `n_samples x n_classes` matrix.
    pub fn predict_probabilities_batch<D: Data<Elem = F>>(
        &self,
        records: &ArrayBase<D, Ix2>,
    ) -> Result<Array2<F>> {
        self.check_width(records.ncols())?;
        // Width taken from the sub-models rather than from `n_classes`, for the same reason
        // [`Self::conditional_unchecked`] is: the two can only disagree on a hand-built or
        // deserialized model, and a mismatch must not become an out-of-bounds write.
        let mut out = Array2::zeros((records.nrows(), self.intercepts.len() + 1));
        for (index, row) in records.rows().into_iter().enumerate() {
            for (level, value) in self.probabilities_unchecked(&row).into_iter().enumerate() {
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
            .fold(F::zero(), |acc, value| acc + value))
    }

    fn check_width(&self, found: usize) -> Result<()> {
        if found != self.n_features() {
            return Err(OrdinalError::FeatureWidthMismatch {
                expected: self.n_features(),
                found,
            });
        }
        Ok(())
    }

    /// Conditional stopping probabilities without the width check.
    ///
    /// Driven by the intercept vector rather than by `n_classes`, so a model rebuilt from
    /// deserialized parts whose `n_classes` disagrees with its sub-model count cannot index out of
    /// bounds — it simply reports the levels it actually has.
    fn conditional_unchecked(&self, row: &ArrayView1<'_, F>) -> Vec<F> {
        self.intercepts
            .iter()
            .enumerate()
            .map(|(level, intercept)| {
                self.link
                    .cdf(row.dot(&self.coefficients.row(level)) + *intercept)
            })
            .collect()
    }

    fn probabilities_unchecked(&self, row: &ArrayView1<'_, F>) -> Vec<F> {
        let conditional = self.conditional_unchecked(row);
        let mut probabilities = Vec::with_capacity(conditional.len() + 1);
        // `reached` is P(y >= k): the sample survived every earlier stopping opportunity.
        let mut reached = F::one();
        for stop in conditional {
            probabilities.push(reached * stop);
            reached *= F::one() - stop;
        }
        probabilities.push(reached);
        probabilities
    }
}

impl<F: Float, D: Data<Elem = F>, T: AsSingleTargets<Elem = usize>>
    Fit<ArrayBase<D, Ix2>, T, OrdinalError> for ContinuationRatioValidParams<F>
{
    type Object = ContinuationRatio<F>;

    /// Fits `K - 1` conditional sub-models on ranks in `0..n_levels`, ordered lowest to highest.
    ///
    /// The ordering of the levels is the caller's contract: rank 0 must be the lowest level. This
    /// crate cannot verify it, and getting it wrong trains a model that is confidently backwards.
    ///
    /// Every level's conditioning subset is checked for fittability *before* any sub-model is
    /// fitted, so an unusable level fails immediately rather than after `K - 2` optimizations.
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

        let mut counts = vec![0usize; n_classes];
        for rank in &targets {
            counts[*rank] += 1;
        }
        check_levels_are_fittable(&counts)?;

        let n_cuts = n_classes - 1;
        let mut coefficients = Array2::<F>::zeros((n_cuts, n_features));
        let mut intercepts = Array1::<F>::zeros(n_cuts);
        let mut iterations = Vec::with_capacity(n_cuts);
        let mut converged = Vec::with_capacity(n_cuts);
        let mut subset_sizes = Vec::with_capacity(n_cuts);

        // Rows that reached the current level. Starts as everything — every sample reaches level 0 —
        // and is filtered down in step with the levels, which is the shrinking-subset structure that
        // defines this family.
        let mut reached: Vec<usize> = (0..n_samples).collect();
        for level in 0..n_cuts {
            let stops: Vec<bool> = reached.iter().map(|row| targets[*row] == level).collect();
            subset_sizes.push(reached.len());

            let fitted = fit_sub_model(records, &reached, &stops, self)?;
            coefficients.row_mut(level).assign(&fitted.coefficients);
            intercepts[level] = fitted.intercept;
            iterations.push(fitted.iterations);
            converged.push(fitted.converged);

            reached.retain(|row| targets[*row] > level);
        }

        Ok(ContinuationRatio {
            coefficients,
            intercepts,
            n_classes,
            link: self.link,
            iterations,
            converged,
            subset_sizes,
        })
    }
}

impl<F: Float, D: Data<Elem = F>> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>>
    for ContinuationRatio<F>
{
    fn predict_inplace(&self, x: &ArrayBase<D, Ix2>, y: &mut Array1<usize>) {
        assert_eq!(
            x.nrows(),
            y.len(),
            "The number of data points must match the number of output targets."
        );
        assert_eq!(
            x.ncols(),
            self.n_features(),
            "The number of features must match the number the model was fitted on."
        );

        for (index, row) in x.rows().into_iter().enumerate() {
            y[index] = argmax(&self.probabilities_unchecked(&row));
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// One fitted conditional sub-model.
struct SubModel<F> {
    coefficients: Array1<F>,
    intercept: F,
    iterations: usize,
    converged: bool,
}

/// Rejects every level whose conditioning subset cannot support a binary fit, naming it.
///
/// Sub-model `k` is fitted on the rows with `y >= k` against the target `y == k`, so it is unusable
/// when that subset is empty, holds no positives (level `k` never occurs), or holds nothing but
/// positives (nothing ever went past level `k`). Failing is the only safe answer: skipping a level
/// would renumber every level above it and silently change what the fitted model means, and
/// substituting a constant probability would invent a sub-model the caller never asked for.
///
/// The practical consequence is that this family is stricter than [`crate::frank_hall`]: *every*
/// declared level must occur in the training data, including the middle ones, which a Frank & Hall
/// decomposition tolerates being absent.
fn check_levels_are_fittable(counts: &[usize]) -> Result<()> {
    let n_levels = counts.len();
    let mut reached: usize = counts.iter().sum();

    for (level, stops) in counts.iter().take(n_levels.saturating_sub(1)).enumerate() {
        if reached == 0 {
            return Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                reason: format!(
                    "no training sample reaches level {level}, so the sub-model for \
                     `y = {level} | y >= {level}` has no rows to fit; all {n_levels} declared \
                     levels must occur in the training data"
                ),
            });
        }
        if *stops == 0 {
            return Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                reason: format!(
                    "every training sample that reaches level {level} continues past it, so the \
                     sub-model for `y = {level} | y >= {level}` has only negatives and cannot be \
                     fitted; level {level} must occur in the training data"
                ),
            });
        }
        if *stops == reached {
            let next = level + 1;
            return Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                reason: format!(
                    "every training sample that reaches level {level} stops there, so the \
                     sub-model for `y = {level} | y >= {level}` has only positives and cannot be \
                     fitted; some sample must reach level {next} or higher"
                ),
            });
        }
        reached -= *stops;
    }
    Ok(())
}

/// Penalized maximum-likelihood fit of one conditional sub-model, by Adam.
///
/// Adam rather than Newton because the Hessian would be an `n_features + 1` square solve per step
/// and this crate carries no LAPACK backend; and because Adam's per-parameter scaling keeps the fit
/// moving on the small, badly conditioned subsets the high levels produce, where a raw
/// gradient-descent step size that works for level 0 diverges.
fn fit_sub_model<F: Float, D: Data<Elem = F>>(
    records: &ArrayBase<D, Ix2>,
    rows: &[usize],
    stops: &[bool],
    params: &ContinuationRatioValidParams<F>,
) -> Result<SubModel<F>> {
    let n_features = records.ncols();
    let mut coefficients = Array1::<F>::zeros(n_features);
    let mut intercept = initial_intercept(stops, params.link);

    // Adam moments over the concatenated parameter vector [beta, theta].
    let n_params = n_features + 1;
    let mut m = vec![F::zero(); n_params];
    let mut v = vec![F::zero(); n_params];
    let beta1 = F::cast(0.9);
    let beta2 = F::cast(0.999);
    let eps = F::cast(1e-8);

    let mut previous = F::infinity();
    let mut iterations = 0;
    let mut converged = false;

    for step in 1..=params.max_iterations {
        iterations = step;
        let (objective, grad_coefficients, grad_intercept) = objective_and_gradient(
            records,
            rows,
            stops,
            &coefficients,
            intercept,
            params.alpha,
            params.link,
        );

        if !objective.is_finite() {
            return Err(OrdinalError::NonFinite {
                context: "a continuation-ratio sub-model's log-likelihood",
            });
        }

        // Relative test, so the stopping rule does not depend on the subset size — which is exactly
        // what differs between sub-models here. Skipped on the first step: there is no previous
        // objective yet, and the infinite sentinel would satisfy any relative tolerance.
        if step > 1
            && (previous - objective).abs() <= params.tolerance * (previous.abs() + F::one())
        {
            converged = true;
            break;
        }
        previous = objective;

        let bias_correction1 = F::one() - beta1.powi(step as i32);
        let bias_correction2 = F::one() - beta2.powi(step as i32);

        for index in 0..n_params {
            let gradient = if index < n_features {
                grad_coefficients[index]
            } else {
                grad_intercept
            };
            m[index] = beta1 * m[index] + (F::one() - beta1) * gradient;
            v[index] = beta2 * v[index] + (F::one() - beta2) * gradient * gradient;
            let m_hat = m[index] / bias_correction1;
            let v_hat = v[index] / bias_correction2;
            let update = params.learning_rate * m_hat / (v_hat.sqrt() + eps);
            if index < n_features {
                coefficients[index] -= update;
            } else {
                intercept -= update;
            }
        }
    }

    if coefficients.iter().any(|value| !value.is_finite()) || !intercept.is_finite() {
        return Err(OrdinalError::NonFinite {
            context: "the fitted sub-model parameters",
        });
    }

    Ok(SubModel {
        coefficients,
        intercept,
        iterations,
        converged,
    })
}

/// Seeds the intercept at the MLE for `beta = 0`: the link-transformed share of the subset that
/// stops at this level.
///
/// Starting there rather than at zero matters more here than in a cumulative-link fit, because the
/// stopping share varies wildly across levels — near zero at the bottom of a heavy-tailed target,
/// near one at the top — and a zero start leaves the high sub-models many iterations from anywhere
/// sensible.
fn initial_intercept<F: Float>(stops: &[bool], link: Link) -> F {
    let total = F::cast(stops.len().max(1));
    let positives = F::cast(stops.iter().filter(|stop| **stop).count());
    // Clamped away from 0 and 1 so a lopsided subset cannot make the inverse CDF infinite.
    let floor = F::cast(1e-6);
    let proportion = (positives / total).min(F::one() - floor).max(floor);
    link.inverse_cdf(proportion)
}

/// Penalized negative log-likelihood of one sub-model and its gradient.
///
/// The sub-model is an ordinary binary regression under `link` on the subset `rows`, with
/// `stops[i]` the binary target for `rows[i]`. Returns `(objective, d/d beta, d/d theta)`.
///
/// The chain rule through the link is `d(-ln L_i)/d eta = g(eta) * (h - z) / (h * (1 - h))`, which
/// collapses to the familiar `h - z` for [`Link::Logit`] — where `g = h(1 - h)` — and stays correct
/// for the links where it does not.
fn objective_and_gradient<F: Float, D: Data<Elem = F>>(
    records: &ArrayBase<D, Ix2>,
    rows: &[usize],
    stops: &[bool],
    coefficients: &Array1<F>,
    intercept: F,
    alpha: F,
    link: Link,
) -> (F, Array1<F>, F) {
    let one = F::one();
    // `min_probability` alone is smaller than the spacing of floats near 1 at `f32`, where
    // `1 - floor` would round straight back to 1 and `ln(1 - h)` would be `-inf` for a saturated
    // link. Widening it to the type's epsilon keeps the clamp effective at both precisions and
    // leaves the `f64` value untouched.
    let floor = min_probability::<F>().max(F::epsilon());

    let mut objective = F::zero();
    let mut grad_coefficients = Array1::<F>::zeros(coefficients.len());
    let mut grad_intercept = F::zero();

    for (&index, &stops_here) in rows.iter().zip(stops.iter()) {
        let row = records.row(index);
        let eta = row.dot(coefficients) + intercept;
        // Floored on both sides: a saturating link can return exactly 0 or 1, and the logarithm of
        // either is what turns a badly scaled feature column into a NaN gradient.
        let probability = link.cdf(eta).min(one - floor).max(floor);

        let target = if stops_here { one } else { F::zero() };
        let log_likelihood = if stops_here {
            probability.ln()
        } else {
            (one - probability).ln()
        };
        objective -= log_likelihood;

        let variance = (probability * (one - probability)).max(floor);
        let d_eta = link.pdf(eta) * (probability - target) / variance;
        grad_intercept += d_eta;
        for (feature, value) in row.iter().enumerate() {
            grad_coefficients[feature] += d_eta * *value;
        }
    }

    // L2 on the coefficients only. Penalizing the intercept would bias the stopping rate itself,
    // which on the small high-level subsets is the one quantity the data does pin down.
    objective += F::cast(0.5) * alpha * coefficients.dot(coefficients);
    for (feature, value) in coefficients.iter().enumerate() {
        grad_coefficients[feature] += alpha * *value;
    }

    (objective, grad_coefficients, grad_intercept)
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

#[cfg(test)]
mod tests {
    use super::*;
    use linfa::traits::Predict;
    use ndarray::array;

    const LINKS: [Link; 4] = [Link::Logit, Link::Probit, Link::CLogLog, Link::Cauchit];

    /// Four ordered levels as separated clusters on a single feature, `per_level` rows each.
    ///
    /// The level rises with `x`, so the progression genuinely escalates, and every level is
    /// populated — which this family, unlike Frank & Hall, requires. The clusters sit two units
    /// apart with a spread of 0.4, so any separating boundary inside a gap classifies exactly and
    /// the assertions do not depend on the optimizer stopping at one particular point.
    fn clustered(per_level: usize) -> DatasetBase<Array2<f64>, Array1<usize>> {
        let n = 4 * per_level;
        let mut records = Array2::zeros((n, 1));
        let mut targets = Vec::with_capacity(n);
        for level in 0..4 {
            for index in 0..per_level {
                let offset = index as f64 / (per_level as f64 - 1.0) - 0.5;
                records[[level * per_level + index, 0]] = -3.0 + 2.0 * level as f64 + 0.4 * offset;
                targets.push(level);
            }
        }
        DatasetBase::new(records, Array1::from(targets))
    }

    /// A model with known parameters, so the reconstruction is checked against arithmetic done by
    /// hand rather than against another run of the same optimizer.
    fn handmade(
        intercepts: Array1<f64>,
        slopes: Array1<f64>,
        link: Link,
    ) -> ContinuationRatio<f64> {
        let n_cuts = intercepts.len();
        let mut coefficients = Array2::zeros((n_cuts, 1));
        for (level, slope) in slopes.iter().enumerate() {
            coefficients[[level, 0]] = *slope;
        }
        ContinuationRatio {
            coefficients,
            intercepts,
            n_classes: n_cuts + 1,
            link,
            iterations: vec![1; n_cuts],
            converged: vec![true; n_cuts],
            subset_sizes: vec![0; n_cuts],
        }
    }

    /// The chain rule must produce a distribution — the property the whole reconstruction rests on.
    /// The tolerance is deliberately tight: the sum telescopes to exactly 1, so anything beyond
    /// rounding noise means the recursion is wrong.
    #[test]
    fn probabilities_are_a_distribution() {
        for link in LINKS {
            let model = handmade(array![-0.7, 0.4, 1.3], array![0.9, -0.5, 0.2], link);
            for x in [-4.0, -1.0, 0.0, 0.6, 3.5] {
                let row = array![x];
                let probabilities = model.predict_probabilities(&row.view()).unwrap();

                assert_eq!(probabilities.len(), 4);
                for p in &probabilities {
                    assert!(
                        (0.0..=1.0).contains(p),
                        "{link:?} at {x}: {probabilities:?}"
                    );
                }
                let total: f64 = probabilities.iter().sum();
                assert!(
                    (total - 1.0).abs() < 1e-12,
                    "{link:?} at {x}: sums to {total} ({probabilities:?})"
                );
            }
        }
    }

    /// The chain rule spelled out against values computable by hand: with every intercept at zero
    /// and no slope, a logit model has `h_k = 0.5` everywhere, so the level probabilities must halve
    /// and the two lowest-probability entries — the top level and the one below it — must tie.
    #[test]
    fn the_chain_rule_conditions_on_having_reached_the_level() {
        let model = handmade(array![0.0, 0.0, 0.0], array![0.0, 0.0, 0.0], Link::Logit);
        let row = array![0.0];

        let conditional = model.conditional_probabilities(&row.view()).unwrap();
        assert_eq!(conditional.len(), 3);
        for h in &conditional {
            assert!((h - 0.5).abs() < 1e-12, "{conditional:?}");
        }

        let probabilities = model.predict_probabilities(&row.view()).unwrap();
        for (level, want) in [0.5, 0.25, 0.125, 0.125].iter().enumerate() {
            assert!(
                (probabilities[level] - want).abs() < 1e-12,
                "level {level}: {probabilities:?}"
            );
        }

        // And the general form, recomputed independently from the conditionals.
        let mut survival = 1.0;
        for (level, h) in conditional.iter().enumerate() {
            assert!(
                (probabilities[level] - survival * h).abs() < 1e-12,
                "level {level}: {probabilities:?}"
            );
            survival *= 1.0 - h;
        }
        assert!((probabilities[3] - survival).abs() < 1e-12);
    }

    /// The documented CLogLog identity: this is the discrete-time proportional-hazards model, so a
    /// shared coefficient scales the whole cumulative hazard and the survival curve is the baseline
    /// raised to `exp(x . beta)`. Breaks if the conditioning or the chain rule is wrong.
    #[test]
    fn cloglog_is_the_discrete_time_proportional_hazards_model() {
        let slope = 0.6;
        let model = handmade(
            array![-1.1, -0.3, 0.4],
            array![slope, slope, slope],
            Link::CLogLog,
        );

        let baseline = model.predict_probabilities(&array![0.0].view()).unwrap();
        for x in [-2.0, -0.5, 0.75, 2.0] {
            let shifted = model.predict_probabilities(&array![x].view()).unwrap();
            let scale = (slope * x).exp();

            // P(y > k), accumulated from both distributions and compared through the identity.
            let mut base_survival = 1.0;
            let mut shifted_survival = 1.0;
            for level in 0..3 {
                base_survival -= baseline[level];
                shifted_survival -= shifted[level];
                let expected = base_survival.powf(scale);
                assert!(
                    (shifted_survival - expected).abs() < 1e-9,
                    "x = {x}, level {level}: {shifted_survival} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn recovers_a_monotone_ordinal_problem() {
        let dataset = clustered(60);
        let model = ContinuationRatio::params()
            .alpha(1e-3)
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.n_classes(), 4);
        assert_eq!(model.coefficients().nrows(), 3);
        assert_eq!(model.n_features(), 1);
        assert_eq!(model.intercepts().len(), 3);

        // A higher `x` makes the progression LESS likely to stop at any given level, so every
        // sub-model's coefficient is negative. A positive one would mean the ordering was learned
        // backwards — the opposite sign convention to a model that scores the level itself.
        for level in 0..3 {
            assert!(
                model.coefficients()[[level, 0]] < 0.0,
                "sub-model {level}: {:?}",
                model.coefficients()
            );
        }

        let predicted = model.predict(&dataset);
        let correct = predicted
            .iter()
            .zip(dataset.targets.iter())
            .filter(|(a, b)| a == b)
            .count();
        assert!(
            correct as f64 / 240.0 > 0.9,
            "training accuracy {}",
            correct as f64 / 240.0
        );
    }

    /// The defining structural property: sub-model `k` sees only the rows that reached level `k`.
    /// If the subsets did not shrink this would be a Frank & Hall decomposition instead, and the
    /// conditional probabilities would not be conditional on anything.
    #[test]
    fn the_conditioning_subsets_shrink() {
        let dataset = clustered(60);
        let model = ContinuationRatio::params()
            .alpha(1e-3)
            .fit(&dataset)
            .unwrap();

        let sizes = model.subset_sizes();
        assert_eq!(sizes.len(), 3);
        for (level, size) in sizes.iter().enumerate() {
            let expected = dataset
                .targets
                .iter()
                .filter(|rank| **rank >= level)
                .count();
            assert_eq!(*size, expected, "level {level}");
        }
        assert_eq!(sizes, [240, 180, 120]);
    }

    #[test]
    fn analytic_gradient_matches_finite_differences() {
        let dataset = clustered(5);
        let rows: Vec<usize> = (0..dataset.targets.len()).collect();
        let stops: Vec<bool> = dataset.targets.iter().map(|rank| *rank == 1).collect();
        let coefficients = array![0.35];
        let intercept = -0.2;
        let alpha = 0.7;
        let step = 1e-5;

        for link in LINKS {
            let (_, grad_coefficients, grad_intercept) = objective_and_gradient(
                dataset.records(),
                &rows,
                &stops,
                &coefficients,
                intercept,
                alpha,
                link,
            );

            let objective_at = |b: &Array1<f64>, t: f64| {
                objective_and_gradient(dataset.records(), &rows, &stops, b, t, alpha, link).0
            };

            let mut shifted = coefficients.clone();
            shifted[0] += step;
            let up = objective_at(&shifted, intercept);
            shifted[0] -= 2.0 * step;
            let down = objective_at(&shifted, intercept);
            let numeric = (up - down) / (2.0 * step);
            assert!(
                (numeric - grad_coefficients[0]).abs() < 1e-3 * (1.0 + grad_coefficients[0].abs()),
                "{link:?} coefficient gradient: analytic {} vs numeric {numeric}",
                grad_coefficients[0]
            );

            let up = objective_at(&coefficients, intercept + step);
            let down = objective_at(&coefficients, intercept - step);
            let numeric = (up - down) / (2.0 * step);
            assert!(
                (numeric - grad_intercept).abs() < 1e-3 * (1.0 + grad_intercept.abs()),
                "{link:?} intercept gradient: analytic {grad_intercept} vs numeric {numeric}"
            );
        }
    }

    #[test]
    fn every_link_fits_and_predicts() {
        let dataset = clustered(40);
        for link in LINKS {
            let model = ContinuationRatio::params()
                .link(link)
                .alpha(1e-3)
                .fit(&dataset)
                .unwrap();

            assert_eq!(model.link(), link);
            assert_eq!(model.iterations().len(), 3);
            assert_eq!(model.converged_per_level().len(), 3);

            let probabilities = model
                .predict_probabilities_batch(dataset.records())
                .unwrap();
            assert_eq!(probabilities.dim(), (160, 4));
            for index in 0..160 {
                let total: f64 = probabilities.row(index).iter().sum();
                assert!((total - 1.0).abs() < 1e-12, "{link:?} row {index}: {total}");
            }

            let expected = model
                .predict_expected_level(&dataset.records().row(0))
                .unwrap();
            assert!((0.0..=3.0).contains(&expected), "{link:?}: {expected}");
        }
    }

    /// A level whose conditioning subset cannot support a binary fit must fail loudly and name
    /// itself. Skipping it silently would renumber every level above it.
    #[test]
    fn rejects_a_level_whose_subset_cannot_be_fitted() {
        // Level 1 never occurs: its sub-model sees rows but no positives.
        let missing_middle = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0], [3.0]],
            Array1::from(vec![0usize, 0, 2, 2]),
        );
        let error: OrdinalError = ContinuationRatio::params()
            .fit(&missing_middle)
            .unwrap_err();
        assert!(matches!(error, OrdinalError::InvalidParameter { .. }));
        assert!(error.to_string().contains("level 1"), "{error}");

        // Level 0 never occurs, so the very first sub-model has no positives.
        let missing_bottom = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0]],
            Array1::from(vec![1usize, 1, 2]),
        );
        let error: OrdinalError = ContinuationRatio::params()
            .fit(&missing_bottom)
            .unwrap_err();
        assert!(matches!(error, OrdinalError::InvalidParameter { .. }));
        assert!(error.to_string().contains("level 0"), "{error}");

        // A declared top level the data never reaches: nothing continues past level 2, so that
        // sub-model is all positives.
        let short = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0]],
            Array1::from(vec![0usize, 1, 2]),
        );
        let error: OrdinalError = ContinuationRatio::params()
            .n_levels(4)
            .fit(&short)
            .unwrap_err();
        assert!(matches!(error, OrdinalError::InvalidParameter { .. }));
        assert!(error.to_string().contains("level 2"), "{error}");
    }

    #[test]
    fn rejects_malformed_training_data() {
        let empty = DatasetBase::new(
            Array2::<f64>::zeros((0, 1)),
            Array1::from(Vec::<usize>::new()),
        );
        assert!(matches!(
            ContinuationRatio::params().fit(&empty),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let no_features = DatasetBase::new(
            Array2::<f64>::zeros((3, 0)),
            Array1::from(vec![0usize, 1, 1]),
        );
        assert!(matches!(
            ContinuationRatio::params().fit(&no_features),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let mismatched =
            DatasetBase::new(Array2::<f64>::zeros((3, 1)), Array1::from(vec![0usize, 1]));
        assert!(matches!(
            ContinuationRatio::params().fit(&mismatched),
            Err(OrdinalError::LengthMismatch {
                records: 3,
                targets: 2
            })
        ));

        let single_level = DatasetBase::new(array![[0.0f64], [1.0]], Array1::from(vec![0usize, 0]));
        assert!(matches!(
            ContinuationRatio::params().fit(&single_level),
            Err(OrdinalError::TooFewClasses { found: 1 })
        ));

        let out_of_range = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0]],
            Array1::from(vec![0usize, 1, 2]),
        );
        assert!(matches!(
            ContinuationRatio::params().n_levels(2).fit(&out_of_range),
            Err(OrdinalError::RankOutOfRange {
                rank: 2,
                n_classes: 2
            })
        ));

        let not_finite = DatasetBase::new(
            array![[f64::NAN], [1.0], [2.0]],
            Array1::from(vec![0usize, 1, 1]),
        );
        assert!(matches!(
            ContinuationRatio::params().fit(&not_finite),
            Err(OrdinalError::NonFinite { .. })
        ));
    }

    #[test]
    fn prediction_rejects_the_wrong_feature_width() {
        let model = handmade(array![0.0, 0.0], array![0.0, 0.0], Link::Logit);
        let wide = array![0.0f64, 1.0];

        assert!(matches!(
            model.predict_probabilities(&wide.view()),
            Err(OrdinalError::FeatureWidthMismatch {
                expected: 1,
                found: 2
            })
        ));
        assert!(matches!(
            model.conditional_probabilities(&wide.view()),
            Err(OrdinalError::FeatureWidthMismatch { .. })
        ));
        assert!(matches!(
            model.predict_expected_level(&wide.view()),
            Err(OrdinalError::FeatureWidthMismatch { .. })
        ));
        assert!(matches!(
            model.predict_probabilities_batch(&Array2::<f64>::zeros((2, 5))),
            Err(OrdinalError::FeatureWidthMismatch { .. })
        ));
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            ContinuationRatio::<f64>::params().alpha(-1.0).check(),
            Err(OrdinalError::InvalidParameter { name: "alpha", .. })
        ));
        assert!(matches!(
            ContinuationRatio::<f64>::params().max_iterations(0).check(),
            Err(OrdinalError::InvalidParameter {
                name: "max_iterations",
                ..
            })
        ));
        assert!(matches!(
            ContinuationRatio::<f64>::params().tolerance(-1e-3).check(),
            Err(OrdinalError::InvalidParameter {
                name: "tolerance",
                ..
            })
        ));
        assert!(matches!(
            ContinuationRatio::<f64>::params()
                .learning_rate(0.0)
                .check(),
            Err(OrdinalError::InvalidParameter {
                name: "learning_rate",
                ..
            })
        ));
        assert!(matches!(
            ContinuationRatio::<f64>::params().n_levels(1).check(),
            Err(OrdinalError::TooFewClasses { found: 1 })
        ));

        let checked = ContinuationRatio::<f64>::params()
            .alpha(0.5)
            .link(Link::Probit)
            .n_levels(3)
            .check()
            .unwrap();
        assert_eq!(checked.alpha(), 0.5);
        assert_eq!(checked.link(), Link::Probit);
        assert_eq!(checked.n_levels(), Some(3));
        assert_eq!(checked.max_iterations(), 500);
        assert!(checked.tolerance() > 0.0);
        assert!(checked.learning_rate() > 0.0);
    }

    #[test]
    fn fits_at_f32() {
        let mut records = Array2::<f32>::zeros((60, 1));
        let mut targets = Vec::with_capacity(60);
        for index in 0..60 {
            let level = index / 20;
            records[[index, 0]] = -2.0 + 2.0 * level as f32 + 0.4 * (index % 20) as f32 / 19.0;
            targets.push(level);
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));

        let model = ContinuationRatio::<f32>::params()
            .alpha(1e-3)
            .fit(&dataset)
            .unwrap();
        assert_eq!(model.n_classes(), 3);
        assert_eq!(model.subset_sizes(), [60, 40]);

        let probabilities = model
            .predict_probabilities(&dataset.records().row(0))
            .unwrap();
        let total: f32 = probabilities.iter().sum();
        assert!((total - 1.0).abs() < 1e-5, "{total}");
    }
}

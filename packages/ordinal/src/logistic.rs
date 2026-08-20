//! Threshold-based ordinal regression.
//!
//! The model shares ONE coefficient vector across all levels and separates them with `K - 1`
//! ordered thresholds:
//!
//! ```text
//! P(y <= k | x) = G(theta_k - x . beta),  k = 0 .. K-2
//! P(y  = k | x) = P(y <= k) - P(y <= k-1)
//! ```
//!
//! That single shared `beta` is exactly what makes it *ordinal* rather than multinomial: moving
//! along `x . beta` can only push probability mass monotonically up or down the level ordering, so
//! the model can never conclude that "high" is likely while "medium" is not.
//!
//! Four axes of choice, which between them cover most of the ordinal literature:
//!
//! - [`Link`] picks the CDF `G`. With [`Link::Logit`] and the defaults this is the classical
//!   proportional-odds model; [`Link::Probit`] gives ordered probit, and so on.
//! - [`OrdinalLoss`] picks what is optimized. The default fits the likelihood above; the two
//!   threshold losses instead penalize misplaced cut points directly, which abandons the
//!   proportional-odds assumption in exchange for robustness when it does not hold.
//! - [`Margin`] shapes that penalty. [`Margin::Hinge`] with [`OrdinalLoss::AllThreshold`] is
//!   support vector ordinal regression (SVOR-IMC) rather than a logistic model.
//! - `free_features` on the params relaxes the shared coefficient for chosen features, giving the
//!   partial proportional odds model — and, when every feature is freed, the generalized ordinal
//!   model. Watch [`OrdinalLogistic::crossing_rate`] when you do: unconstrained per-threshold
//!   slopes can make the cumulative curves cross, which is no longer a probability model.
//!
//! Fitted by Adam on the penalized objective. The thresholds are optimized through a softplus
//! parameterization, `theta_k = theta_{k-1} + softplus(a_k)`, which makes the ordering constraint
//! structural — no projection step, and the constraint can never be violated by a large step.
//! Features should be scaled.
//!
//! # Example
//!
//! ```no_run
//! use linfa::prelude::*;
//! use flow_like_ordinal::OrdinalLogistic;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dataset: linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>> = todo!();
//! let model = OrdinalLogistic::params().alpha(1.0).max_iterations(500).fit(&dataset)?;
//! let predicted = model.predict(&dataset);
//! # Ok(())
//! # }
//! ```

use crate::error::{OrdinalError, Result};
use crate::link::Link;
use crate::math::{min_probability, sigmoid, softplus, softplus_inv};
use linfa::dataset::AsSingleTargets;
use linfa::traits::{Fit, PredictInplace};
use linfa::{DatasetBase, Float, ParamGuard};
use ndarray::{Array1, Array2, ArrayBase, ArrayView1, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// How the ordered thresholds are fitted.
///
/// [`OrdinalLoss::CumulativeLink`] models the level probabilities directly and is the only variant
/// that yields calibrated probabilities. The two threshold losses instead penalize each cut point
/// that falls on the wrong side of the observation, which drops the proportional-odds assumption
/// and is often the more robust choice when that assumption does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum OrdinalLoss {
    /// Maximum likelihood on `P(y = k) = G(theta_k - eta) - G(theta_{k-1} - eta)`.
    ///
    /// The classical cumulative-link model; with [`Link::Logit`] this is proportional odds.
    #[default]
    CumulativeLink,
    /// Every threshold is penalized for being on the wrong side of the observation.
    ///
    /// Rennie & Srebro's all-threshold loss. Penalizing distant thresholds too makes the fit
    /// aware of *how far* wrong a cut point is, which is what usually makes it beat the immediate
    /// variant on real data.
    AllThreshold,
    /// Only the two thresholds bracketing the observed level are penalized.
    ///
    /// Cheaper and more local than [`OrdinalLoss::AllThreshold`], but blind to how badly the
    /// non-adjacent cut points are placed.
    ImmediateThreshold,
}

impl OrdinalLoss {
    /// True for the losses that carry no probability model, so per-level probabilities are
    /// unavailable rather than merely uncalibrated.
    pub fn is_threshold_loss(&self) -> bool {
        matches!(
            self,
            OrdinalLoss::AllThreshold | OrdinalLoss::ImmediateThreshold
        )
    }
}

/// Penalty applied to a threshold that sits on the wrong side of an observation.
///
/// Only the threshold losses consult this — [`OrdinalLoss::CumulativeLink`] optimizes a likelihood
/// and has no margin. The choice is what separates a logistic threshold model from a support
/// vector one: [`Margin::Hinge`] with [`OrdinalLoss::AllThreshold`] IS support vector ordinal
/// regression with implicit constraints (Chu & Keerthi's SVOR-IMC), and with
/// [`OrdinalLoss::ImmediateThreshold`] it is the explicit-constraint variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Margin {
    /// `softplus(u)`. Smooth everywhere and penalizes even well-placed thresholds a little.
    #[default]
    Logistic,
    /// `max(0, 1 + u)`. Zero once the threshold clears the margin, so only the support vectors —
    /// the observations near a cut point — influence the fit.
    Hinge,
    /// `max(0, 1 + u)^2`. Like [`Margin::Hinge`] but differentiable at the kink, which makes the
    /// gradient better behaved at the cost of punishing distant violations far harder.
    SquaredHinge,
}

impl Margin {
    /// Penalty at margin score `u`, which is negative when the threshold is correctly placed.
    fn loss<F: Float>(&self, u: F) -> F {
        match self {
            Margin::Logistic => softplus(u),
            Margin::Hinge => (F::one() + u).max(F::zero()),
            Margin::SquaredHinge => {
                let slack = (F::one() + u).max(F::zero());
                slack * slack
            }
        }
    }

    /// `d(loss)/du`. At the hinge kink this is the subgradient 0, which Adam handles fine.
    fn derivative<F: Float>(&self, u: F) -> F {
        match self {
            Margin::Logistic => sigmoid(u),
            Margin::Hinge => {
                if u > -F::one() {
                    F::one()
                } else {
                    F::zero()
                }
            }
            Margin::SquaredHinge => F::cast(2.0) * (F::one() + u).max(F::zero()),
        }
    }
}

/// Checked hyperparameters for [`OrdinalLogistic`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalLogisticValidParams<F> {
    alpha: F,
    max_iterations: usize,
    tolerance: F,
    learning_rate: F,
    n_levels: Option<usize>,
    link: Link,
    loss: OrdinalLoss,
    margin: Margin,
    free_features: Vec<usize>,
}

impl<F: Float> OrdinalLogisticValidParams<F> {
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

    pub fn link(&self) -> Link {
        self.link
    }

    pub fn loss(&self) -> OrdinalLoss {
        self.loss
    }

    pub fn margin(&self) -> Margin {
        self.margin
    }

    /// Feature indices given their own coefficient at every threshold.
    pub fn free_features(&self) -> &[usize] {
        &self.free_features
    }

    /// The same settings pinned to the two-level case.
    ///
    /// Used by the proportional-odds diagnostic, which refits each threshold as its own binary
    /// problem and must hold every other setting fixed for the comparison to mean anything.
    pub(crate) fn as_binary(&self) -> Self {
        let mut copy = self.clone();
        copy.n_levels = Some(2);
        copy
    }
}

/// Unchecked hyperparameters for [`OrdinalLogistic`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalLogisticParams<F>(OrdinalLogisticValidParams<F>);

impl<F: Float> Default for OrdinalLogisticParams<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Float> OrdinalLogisticParams<F> {
    pub fn new() -> Self {
        Self(OrdinalLogisticValidParams {
            alpha: F::one(),
            max_iterations: 500,
            tolerance: F::cast(1e-7),
            learning_rate: F::cast(0.1),
            n_levels: None,
            link: Link::Logit,
            loss: OrdinalLoss::CumulativeLink,
            margin: Margin::Logistic,
            free_features: Vec::new(),
        })
    }

    /// Cumulative link function. Applies to [`OrdinalLoss::CumulativeLink`] only — the threshold
    /// losses use a logistic margin and ignore this.
    pub fn link(mut self, link: Link) -> Self {
        self.0.link = link;
        self
    }

    /// Which loss the thresholds are fitted with.
    pub fn loss(mut self, loss: OrdinalLoss) -> Self {
        self.0.loss = loss;
        self
    }

    /// Penalty shape for the threshold losses. Ignored by [`OrdinalLoss::CumulativeLink`].
    ///
    /// [`Margin::Hinge`] turns the threshold losses into support vector ordinal regression.
    pub fn margin(mut self, margin: Margin) -> Self {
        self.0.margin = margin;
        self
    }

    /// Features that get their OWN coefficient at every threshold instead of one shared across all
    /// of them — the partial proportional odds model.
    ///
    /// The shared coefficient is what makes the standard model *proportional*: one slope describes
    /// every cut point. That is an assumption, and [`crate::proportional_odds_report`] exists to
    /// test it. When it fails for a particular feature, freeing just that feature keeps the
    /// parsimony of the shared model everywhere else.
    ///
    /// Freeing EVERY feature gives the generalized ordinal model. Be aware that unconstrained
    /// per-threshold slopes let the cumulative curves cross, which implies negative category
    /// probabilities; the fitted model reports how often that happened on the training data through
    /// [`OrdinalLogistic::crossing_rate`].
    pub fn free_features(mut self, features: &[usize]) -> Self {
        let mut owned: Vec<usize> = features.to_vec();
        owned.sort_unstable();
        owned.dedup();
        self.0.free_features = owned;
        self
    }

    /// L2 penalty on the coefficients. Thresholds are never penalized.
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

impl<F: Float> ParamGuard for OrdinalLogisticParams<F> {
    type Checked = OrdinalLogisticValidParams<F>;
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

/// A fitted proportional-odds model.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalLogistic<F> {
    coefficients: Array1<F>,
    /// Feature indices carrying a per-threshold coefficient; entries in `coefficients` for these
    /// are zero, because the whole slope lives in `free_coefficients`.
    free_features: Vec<usize>,
    /// `(K - 1) x free_features.len()` per-threshold slopes.
    free_coefficients: Array2<F>,
    /// `K - 1` strictly increasing cut points on the latent scale.
    thresholds: Array1<F>,
    n_classes: usize,
    iterations: usize,
    converged: bool,
    link: Link,
    loss: OrdinalLoss,
    margin: Margin,
    /// Share of training rows whose cumulative curves crossed. Always 0 without free features.
    crossing_rate: F,
}

impl<F: Float> OrdinalLogistic<F> {
    /// Starts a builder for the hyperparameters.
    pub fn params() -> OrdinalLogisticParams<F> {
        OrdinalLogisticParams::new()
    }

    /// One coefficient per feature. A positive entry pushes samples toward HIGHER levels.
    pub fn coefficients(&self) -> &Array1<F> {
        &self.coefficients
    }

    /// The `K - 1` fitted cut points, strictly increasing.
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

    /// The cumulative link this model was fitted with.
    pub fn link(&self) -> Link {
        self.link
    }

    /// The loss this model was fitted with.
    pub fn loss(&self) -> OrdinalLoss {
        self.loss
    }

    /// The margin this model was fitted with. Meaningful only for the threshold losses.
    pub fn margin(&self) -> Margin {
        self.margin
    }

    /// Feature indices that were given a per-threshold coefficient.
    pub fn free_features(&self) -> &[usize] {
        &self.free_features
    }

    /// The effective coefficient of every feature at every threshold, `(K - 1) x n_features`.
    ///
    /// Shared features repeat down the column; free features vary. This is the matrix to inspect
    /// when checking whether freeing a feature actually bought anything.
    pub fn effective_coefficients(&self) -> Array2<F> {
        let n_cuts = self.n_classes - 1;
        let mut out = Array2::zeros((n_cuts, self.coefficients.len()));
        for cut in 0..n_cuts {
            for (feature, value) in self.coefficients.iter().enumerate() {
                out[[cut, feature]] = *value;
            }
            for (slot, feature) in self.free_features.iter().enumerate() {
                out[[cut, *feature]] = self.free_coefficients[[cut, slot]];
            }
        }
        out
    }

    /// Share of training rows on which the cumulative curves crossed.
    ///
    /// Only ever non-zero with free features: unconstrained per-threshold slopes can make
    /// `P(y <= k)` exceed `P(y <= k+1)`, which is not a probability model at all. Prediction
    /// clamps and renormalizes so nothing downstream sees a negative number, but a non-trivial rate
    /// here means the generalized fit is not trustworthy and fewer features should be freed.
    pub fn crossing_rate(&self) -> F {
        self.crossing_rate
    }

    /// Latent score at one threshold. Identical across thresholds unless features were freed.
    fn score_at(&self, row: &ArrayView1<'_, F>, cut: usize) -> F {
        let mut score = row.dot(&self.coefficients);
        for (slot, feature) in self.free_features.iter().enumerate() {
            score += row[*feature] * self.free_coefficients[[cut, slot]];
        }
        score
    }

    /// Cumulative probabilities `P(y <= k)` at each of the `K - 1` cuts.
    fn cumulative(&self, row: &ArrayView1<'_, F>) -> Vec<F> {
        (0..self.n_classes - 1)
            .map(|cut| {
                self.link
                    .cdf(self.thresholds[cut] - self.score_at(row, cut))
            })
            .collect()
    }

    /// Per-level probabilities for one sample, in level order. Always sums to 1.
    ///
    /// Only available under [`OrdinalLoss::CumulativeLink`]. The threshold losses optimize cut-point
    /// placement rather than a likelihood, so they define no probability model; returning a
    /// plausible-looking number from them would be inventing calibration that was never fitted.
    pub fn predict_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        if self.loss.is_threshold_loss() {
            return Err(OrdinalError::InvalidParameter {
                name: "loss",
                reason: format!(
                    "{:?} fits threshold placement, not a likelihood, so it yields no per-level probabilities. Refit with OrdinalLoss::CumulativeLink if you need them.",
                    self.loss
                ),
            });
        }
        if row.len() != self.coefficients.len() {
            return Err(OrdinalError::FeatureWidthMismatch {
                expected: self.coefficients.len(),
                found: row.len(),
            });
        }
        Ok(probabilities_from_cumulative(
            &self.cumulative(row),
            self.n_classes,
        ))
    }

    /// Level under a threshold loss: the number of cut points the score clears.
    ///
    /// Evaluated per cut, since a partial-proportional-odds fit gives each cut its own score.
    fn level_from_thresholds(&self, row: &ArrayView1<'_, F>) -> usize {
        (0..self.n_classes - 1)
            .filter(|cut| self.score_at(row, *cut) > self.thresholds[*cut])
            .count()
            .min(self.n_classes - 1)
    }

    /// Per-level probabilities for every row, as an `n_samples x n_classes` matrix.
    pub fn predict_probabilities_batch<D: Data<Elem = F>>(
        &self,
        records: &ArrayBase<D, Ix2>,
    ) -> Result<Array2<F>> {
        let mut out = Array2::zeros((records.nrows(), self.n_classes));
        for (index, row) in records.rows().into_iter().enumerate() {
            let probabilities = self.predict_probabilities(&row)?;
            for (level, value) in probabilities.into_iter().enumerate() {
                out[[index, level]] = value;
            }
        }
        Ok(out)
    }

    /// Probability-weighted mean level.
    ///
    /// Meaningful precisely because the target is ordered, in a way it never is for unordered
    /// classes: rounding it optimizes rank error, where the modal prediction from
    /// [`PredictInplace`] optimizes exact-match accuracy.
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
    Fit<ArrayBase<D, Ix2>, T, OrdinalError> for OrdinalLogisticValidParams<F>
{
    type Object = OrdinalLogistic<F>;

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

        if let Some(&bad) = self.free_features.iter().find(|f| **f >= n_features) {
            return Err(OrdinalError::InvalidParameter {
                name: "free_features",
                reason: format!(
                    "feature index {bad} is outside the {n_features} columns of the training matrix"
                ),
            });
        }

        let n_cuts = n_classes - 1;
        let n_free = self.free_features.len();
        let mut beta = Array1::<F>::zeros(n_features);
        let mut gamma = Array2::<F>::zeros((n_cuts, n_free));
        let mut raw = initial_raw_thresholds::<F>(&targets, n_classes, self.link);

        // Adam moments over the concatenated parameter vector [beta, gamma, raw].
        let gamma_offset = n_features;
        let raw_offset = gamma_offset + n_cuts * n_free;
        let n_params = raw_offset + n_cuts;
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
            let (objective, grad_beta, grad_gamma, grad_raw) = objective_and_gradient(
                records,
                &targets,
                &beta,
                &gamma,
                &self.free_features,
                &raw,
                n_classes,
                self.alpha,
                self.link,
                self.loss,
                self.margin,
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
                let gradient = if index < gamma_offset {
                    grad_beta[index]
                } else if index < raw_offset {
                    let flat = index - gamma_offset;
                    grad_gamma[[flat / n_free.max(1), flat % n_free.max(1)]]
                } else {
                    grad_raw[index - raw_offset]
                };
                m[index] = beta1 * m[index] + (F::one() - beta1) * gradient;
                v[index] = beta2 * v[index] + (F::one() - beta2) * gradient * gradient;
                let m_hat = m[index] / bias_correction1;
                let v_hat = v[index] / bias_correction2;
                let update = self.learning_rate * m_hat / (v_hat.sqrt() + eps);
                if index < gamma_offset {
                    beta[index] -= update;
                } else if index < raw_offset {
                    let flat = index - gamma_offset;
                    let cell = [flat / n_free.max(1), flat % n_free.max(1)];
                    gamma[cell] -= update;
                } else {
                    raw[index - raw_offset] -= update;
                }
            }
        }

        let thresholds = thresholds_from_raw(&raw);
        if beta
            .iter()
            .chain(gamma.iter())
            .chain(thresholds.iter())
            .any(|v| !v.is_finite())
        {
            return Err(OrdinalError::NonFinite {
                context: "the fitted parameters",
            });
        }

        let mut model = OrdinalLogistic {
            coefficients: beta,
            free_features: self.free_features.clone(),
            free_coefficients: gamma,
            thresholds,
            n_classes,
            iterations,
            converged,
            link: self.link,
            loss: self.loss,
            margin: self.margin,
            crossing_rate: F::zero(),
        };

        // Measured, not assumed: per-threshold slopes can make the cumulative curves cross, and the
        // rate is the only honest signal of how far the generalized fit has drifted from a real
        // probability model. Skipped entirely when nothing was freed, where crossing is impossible.
        if !model.free_features.is_empty() {
            let crossings = records
                .rows()
                .into_iter()
                .filter(|row| cumulative_crosses(&model.cumulative(row)))
                .count();
            model.crossing_rate = F::cast(crossings) / F::cast(n_samples);
        }

        Ok(model)
    }
}

impl<F: Float, D: Data<Elem = F>> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>>
    for OrdinalLogistic<F>
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
            // Under a threshold loss there is no level distribution to take a mode of; the fitted
            // cut points ARE the decision rule, so the level is read off directly.
            y[index] = if self.loss.is_threshold_loss() {
                self.level_from_thresholds(&row)
            } else {
                argmax(&probabilities_from_cumulative(
                    &self.cumulative(&row),
                    self.n_classes,
                ))
            };
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// Turns the unconstrained parameters into strictly increasing thresholds.
fn thresholds_from_raw<F: Float>(raw: &[F]) -> Array1<F> {
    let mut thresholds = Array1::zeros(raw.len());
    let mut current = F::zero();
    for (index, value) in raw.iter().enumerate() {
        current = if index == 0 {
            *value
        } else {
            current + softplus(*value)
        };
        thresholds[index] = current;
    }
    thresholds
}

/// Seeds the thresholds at the MLE for `beta = 0`, which is the logit of the empirical cumulative
/// class frequencies. Starting there rather than at zero saves a large number of iterations and
/// keeps badly imbalanced level counts from stalling the optimizer.
fn initial_raw_thresholds<F: Float>(targets: &[usize], n_classes: usize, link: Link) -> Vec<F> {
    let n_cuts = n_classes - 1;
    let total = F::cast(targets.len());

    let mut counts = vec![F::zero(); n_classes];
    for &rank in targets {
        counts[rank] += F::one();
    }

    let floor = F::cast(1e-6);
    let mut cumulative = F::zero();
    let mut thresholds: Vec<F> = Vec::with_capacity(n_cuts);
    for count in counts.iter().take(n_cuts) {
        cumulative += *count;
        // Clamped away from 0 and 1 so an unobserved level cannot make the logit infinite.
        let proportion = (cumulative / total).min(F::one() - floor).max(floor);
        thresholds.push(link.inverse_cdf(proportion));
    }

    // Enforce a strict gap before inverting, so `softplus_inv` never sees a non-positive delta.
    let min_gap = F::cast(1e-3);
    for index in 1..thresholds.len() {
        if thresholds[index] <= thresholds[index - 1] + min_gap {
            thresholds[index] = thresholds[index - 1] + min_gap;
        }
    }

    let mut raw = Vec::with_capacity(n_cuts);
    for (index, value) in thresholds.iter().enumerate() {
        if index == 0 {
            raw.push(*value);
        } else {
            raw.push(softplus_inv(*value - thresholds[index - 1]));
        }
    }
    raw
}

/// Per-level probabilities from the cumulative probabilities at each cut.
///
/// `cumulative[k]` is `P(y <= k)`. With free features these are NOT guaranteed to increase in `k`
/// (the curves can cross), so each difference is clamped at zero and the result renormalized —
/// otherwise a generalized fit would emit negative probabilities. `OrdinalLogistic::crossing_rate`
/// reports how often that clamping was needed rather than letting it pass unnoticed.
fn probabilities_from_cumulative<F: Float>(cumulative: &[F], n_classes: usize) -> Vec<F> {
    let mut probabilities = Vec::with_capacity(n_classes);
    let mut previous = F::zero();
    for value in cumulative {
        probabilities.push((*value - previous).max(F::zero()));
        previous = *value;
    }
    probabilities.push((F::one() - previous).max(F::zero()));

    let total = probabilities.iter().fold(F::zero(), |acc, v| acc + *v);
    if total > F::zero() {
        for value in probabilities.iter_mut() {
            *value /= total;
        }
    } else {
        let uniform = F::one() / F::cast(n_classes);
        probabilities.iter_mut().for_each(|value| *value = uniform);
    }
    probabilities
}

/// True when the cumulative probabilities do not increase across the cuts.
fn cumulative_crosses<F: Float>(cumulative: &[F]) -> bool {
    cumulative.windows(2).any(|pair| pair[1] < pair[0])
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

/// Objective value, gradient in the shared coefficients, gradient in the per-threshold
/// coefficients, and gradient in the raw threshold parameters.
type ObjectiveGradient<F> = (F, Array1<F>, Array2<F>, Vec<F>);

/// Penalized objective and its gradient with respect to the shared coefficients, the
/// per-threshold coefficients, and the raw threshold parameters.
///
/// The latent score is evaluated PER CUT: `eta_k = x . beta + sum_f x[free_f] * gamma[k, f]`. With
/// no free features every `eta_k` is identical and this reduces exactly to the shared model.
#[allow(clippy::too_many_arguments)]
fn objective_and_gradient<F: Float, D: Data<Elem = F>>(
    records: &ArrayBase<D, Ix2>,
    targets: &[usize],
    beta: &Array1<F>,
    gamma: &Array2<F>,
    free_features: &[usize],
    raw: &[F],
    n_classes: usize,
    alpha: F,
    link: Link,
    loss: OrdinalLoss,
    margin: Margin,
) -> Result<ObjectiveGradient<F>> {
    let n_cuts = n_classes - 1;
    let thresholds = thresholds_from_raw(raw);

    let mut objective = F::zero();
    let mut grad_beta = Array1::<F>::zeros(beta.len());
    let mut grad_gamma = Array2::<F>::zeros(gamma.raw_dim());
    let mut grad_theta = vec![F::zero(); n_cuts];

    for (row, &rank) in records.rows().into_iter().zip(targets.iter()) {
        let shared = row.dot(beta);
        let score_at = |cut: usize| -> F {
            let mut value = shared;
            for (slot, feature) in free_features.iter().enumerate() {
                value += row[*feature] * gamma[[cut, slot]];
            }
            value
        };

        match loss {
            OrdinalLoss::CumulativeLink => {
                // The cuts adjacent to this sample's level. The out-of-range ends behave as the
                // constants 0 and 1, whose densities vanish.
                let upper = if rank < n_cuts {
                    let z = thresholds[rank] - score_at(rank);
                    Some((rank, link.cdf(z), link.pdf(z)))
                } else {
                    None
                };
                let lower = if rank >= 1 {
                    let z = thresholds[rank - 1] - score_at(rank - 1);
                    Some((rank - 1, link.cdf(z), link.pdf(z)))
                } else {
                    None
                };

                let upper_s = upper.map_or(F::one(), |(_, s, _)| s);
                let lower_s = lower.map_or(F::zero(), |(_, s, _)| s);
                let probability = (upper_s - lower_s).max(min_probability::<F>());
                objective -= probability.ln();

                // d(-ln P)/d eta at each adjacent cut.
                let d_upper = upper.map_or(F::zero(), |(_, _, g)| g / probability);
                let d_lower = lower.map_or(F::zero(), |(_, _, g)| -g / probability);

                if let Some((cut, _, g)) = upper {
                    grad_theta[cut] -= g / probability;
                }
                if let Some((cut, _, g)) = lower {
                    grad_theta[cut] += g / probability;
                }

                // The shared coefficients feed BOTH adjacent scores, so they take the sum.
                let combined = d_upper + d_lower;
                for (index, feature) in row.iter().enumerate() {
                    grad_beta[index] += combined * *feature;
                }
                if let Some((cut, _, _)) = upper {
                    for (slot, feature) in free_features.iter().enumerate() {
                        grad_gamma[[cut, slot]] += d_upper * row[*feature];
                    }
                }
                if let Some((cut, _, _)) = lower {
                    for (slot, feature) in free_features.iter().enumerate() {
                        grad_gamma[[cut, slot]] += d_lower * row[*feature];
                    }
                }
            }
            OrdinalLoss::AllThreshold | OrdinalLoss::ImmediateThreshold => {
                // Each cut pays `margin(s_k * (theta_k - eta_k))`, with `s_k` +1 when the
                // observation sits above the cut and -1 when below. The immediate variant restricts
                // this to the two cuts bracketing the level; the upper bound is `rank + 1`, not
                // `rank`, or the lowest level would train on an empty range.
                let (first, last) = match loss {
                    OrdinalLoss::ImmediateThreshold => {
                        (rank.saturating_sub(1), (rank + 1).min(n_cuts))
                    }
                    _ => (0, n_cuts),
                };

                for cut in first..last {
                    let sign = if rank > cut { F::one() } else { -F::one() };
                    let score = sign * (thresholds[cut] - score_at(cut));
                    objective += margin.loss(score);

                    let slope = margin.derivative(score);
                    grad_theta[cut] += sign * slope;

                    let d_eta = -sign * slope;
                    for (index, feature) in row.iter().enumerate() {
                        grad_beta[index] += d_eta * *feature;
                    }
                    for (slot, feature) in free_features.iter().enumerate() {
                        grad_gamma[[cut, slot]] += d_eta * row[*feature];
                    }
                }
            }
        }
    }

    // L2 on the slopes only, shared and per-threshold alike. Penalizing thresholds would drag the
    // cut points toward each other and quietly collapse adjacent levels.
    let half = F::cast(0.5);
    let shared_penalty = beta.iter().fold(F::zero(), |acc, b| acc + *b * *b);
    let free_penalty = gamma.iter().fold(F::zero(), |acc, g| acc + *g * *g);
    objective += half * alpha * (shared_penalty + free_penalty);
    for (index, value) in beta.iter().enumerate() {
        grad_beta[index] += alpha * *value;
    }
    for (index, value) in gamma.indexed_iter() {
        grad_gamma[index] += alpha * *value;
    }

    // A freed feature's slope lives entirely in `gamma`; its shared entry is pinned at zero so the
    // two cannot both move and make the split arbitrary.
    for feature in free_features {
        grad_beta[*feature] = F::zero();
    }

    // Map d/d theta onto d/d raw. theta_0 shifts every cut; each later raw entry shifts its own cut
    // and all cuts above it, scaled by the softplus derivative.
    let mut grad_raw = vec![F::zero(); n_cuts];
    let mut suffix = F::zero();
    for index in (0..n_cuts).rev() {
        suffix += grad_theta[index];
        grad_raw[index] = if index == 0 {
            suffix
        } else {
            sigmoid(raw[index]) * suffix
        };
    }

    if !objective.is_finite()
        || grad_beta.iter().any(|g| !g.is_finite())
        || grad_gamma.iter().any(|g| !g.is_finite())
        || grad_raw.iter().any(|g| !g.is_finite())
    {
        return Err(OrdinalError::NonFinite {
            context: "the gradient",
        });
    }

    Ok((objective, grad_beta, grad_gamma, grad_raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use linfa::traits::Predict;
    use ndarray::array;

    /// Builds a clean ordinal problem: one informative feature, levels cut at fixed points.
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

    #[test]
    fn recovers_monotone_structure() {
        let dataset = synthetic(300);
        let model = OrdinalLogistic::params()
            .alpha(1e-4)
            .max_iterations(2000)
            .fit(&dataset)
            .unwrap();

        // The single feature increases with the level, so its coefficient must be positive.
        assert!(
            model.coefficients()[0] > 0.0,
            "coefficient {} should be positive",
            model.coefficients()[0]
        );
        assert!(model.thresholds()[0] < model.thresholds()[1]);
        assert_eq!(model.n_classes(), 3);

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

    #[test]
    fn probabilities_are_a_distribution() {
        let dataset = synthetic(120);
        let model = OrdinalLogistic::params().fit(&dataset).unwrap();

        for row in dataset.records().rows() {
            let probabilities = model.predict_probabilities(&row).unwrap();
            assert_eq!(probabilities.len(), 3);
            let total: f64 = probabilities.iter().sum();
            assert!(
                (total - 1.0).abs() < 1e-9,
                "probabilities summed to {total}"
            );
            assert!(probabilities.iter().all(|p| *p >= 0.0));
        }
    }

    /// The defining property of a proportional-odds model: raising the latent score can only move
    /// probability mass monotonically up the ordering, never skip a level.
    #[test]
    fn cumulative_probabilities_are_monotone_in_the_score() {
        let dataset = synthetic(200);
        let model = OrdinalLogistic::params().fit(&dataset).unwrap();

        let low = model.predict_expected_level(&array![-3.0].view()).unwrap();
        let mid = model.predict_expected_level(&array![0.0].view()).unwrap();
        let high = model.predict_expected_level(&array![3.0].view()).unwrap();
        assert!(low < mid && mid < high, "{low} {mid} {high}");
    }

    /// The gradient is the one place a link, loss or margin mistake would be silent — a wrong
    /// derivative still converges, just to the wrong parameters. Every combination is checked
    /// against finite differences, including a partial-proportional-odds fit where the shared and
    /// per-threshold blocks must BOTH be right.
    #[test]
    fn analytic_gradient_matches_finite_differences() {
        let dataset = synthetic(40);
        let targets: Vec<usize> = dataset.targets.iter().copied().collect();
        let alpha = 0.7;
        let step = 1e-6;

        for free in [vec![], vec![0usize]] {
            let n_free = free.len();
            let beta = if n_free == 0 {
                array![0.35]
            } else {
                array![0.0]
            };
            let gamma = Array2::from_shape_vec((2, n_free), vec![0.2; 2 * n_free]).unwrap();
            let raw = vec![-0.4, 0.2];

            for link in [Link::Logit, Link::Probit, Link::CLogLog, Link::Cauchit] {
                for (loss, margin) in [
                    (OrdinalLoss::CumulativeLink, Margin::Logistic),
                    (OrdinalLoss::AllThreshold, Margin::Logistic),
                    (OrdinalLoss::ImmediateThreshold, Margin::Logistic),
                    // Plain Hinge is deliberately excluded: its derivative is a step, so a central
                    // difference straddling the kink disagrees with the subgradient by design.
                    (OrdinalLoss::AllThreshold, Margin::SquaredHinge),
                    (OrdinalLoss::ImmediateThreshold, Margin::SquaredHinge),
                ] {
                    let at = |b: &Array1<f64>, g: &Array2<f64>, r: &[f64]| {
                        objective_and_gradient(
                            dataset.records(),
                            &targets,
                            b,
                            g,
                            &free,
                            r,
                            3,
                            alpha,
                            link,
                            loss,
                            margin,
                        )
                        .unwrap()
                    };
                    let (_, grad_beta, grad_gamma, grad_raw) = at(&beta, &gamma, &raw);
                    let label = format!("{link:?}/{loss:?}/{margin:?}/free={free:?}");

                    if n_free == 0 {
                        let mut shifted = beta.clone();
                        shifted[0] += step;
                        let up = at(&shifted, &gamma, &raw).0;
                        shifted[0] -= 2.0 * step;
                        let down = at(&shifted, &gamma, &raw).0;
                        let numeric = (up - down) / (2.0 * step);
                        assert!(
                            (numeric - grad_beta[0]).abs() < 1e-4,
                            "{label} beta: analytic {} vs numeric {numeric}",
                            grad_beta[0]
                        );
                    }

                    for cut in 0..2 {
                        for slot in 0..n_free {
                            let mut shifted = gamma.clone();
                            shifted[[cut, slot]] += step;
                            let up = at(&beta, &shifted, &raw).0;
                            shifted[[cut, slot]] -= 2.0 * step;
                            let down = at(&beta, &shifted, &raw).0;
                            let numeric = (up - down) / (2.0 * step);
                            assert!(
                                (numeric - grad_gamma[[cut, slot]]).abs() < 1e-4,
                                "{label} gamma[{cut},{slot}]: analytic {} vs numeric {numeric}",
                                grad_gamma[[cut, slot]]
                            );
                        }
                    }

                    for index in 0..raw.len() {
                        let mut shifted = raw.clone();
                        shifted[index] += step;
                        let up = at(&beta, &gamma, &shifted).0;
                        shifted[index] -= 2.0 * step;
                        let down = at(&beta, &gamma, &shifted).0;
                        let numeric = (up - down) / (2.0 * step);
                        assert!(
                            (numeric - grad_raw[index]).abs() < 1e-4,
                            "{label} raw[{index}]: analytic {} vs numeric {numeric}",
                            grad_raw[index]
                        );
                    }
                }
            }
        }
    }

    /// Freeing no features must reproduce the shared model exactly — the generalized machinery has
    /// to be a strict superset, not a different estimator that happens to look similar.
    #[test]
    fn empty_free_set_reproduces_the_shared_fit() {
        let dataset = synthetic(200);
        let shared = OrdinalLogistic::params().alpha(1e-3).fit(&dataset).unwrap();
        let explicit = OrdinalLogistic::params()
            .alpha(1e-3)
            .free_features(&[])
            .fit(&dataset)
            .unwrap();
        assert_eq!(shared.coefficients(), explicit.coefficients());
        assert_eq!(shared.thresholds(), explicit.thresholds());
        assert_eq!(explicit.crossing_rate(), 0.0);
    }

    /// Partial proportional odds: a freed feature gets its own slope per threshold, and the shared
    /// entry for it is pinned at zero so the split is unambiguous.
    #[test]
    fn freed_features_get_per_threshold_slopes() {
        let dataset = synthetic(300);
        let model = OrdinalLogistic::params()
            .alpha(1e-4)
            .max_iterations(1500)
            .free_features(&[0])
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.free_features(), &[0]);
        assert_eq!(
            model.coefficients()[0],
            0.0,
            "shared entry must stay pinned"
        );

        let effective = model.effective_coefficients();
        assert_eq!(effective.dim(), (2, 1));
        assert!(effective[[0, 0]] > 0.0, "{effective:?}");
        assert!(effective[[1, 0]] > 0.0, "{effective:?}");
        assert!((0.0..=1.0).contains(&model.crossing_rate()));
    }

    /// Hinge margins turn the threshold losses into support vector ordinal regression; they must
    /// still recover the ordering.
    #[test]
    fn hinge_margins_recover_the_ordering() {
        let dataset = synthetic(300);
        for margin in [Margin::Hinge, Margin::SquaredHinge] {
            for loss in [OrdinalLoss::AllThreshold, OrdinalLoss::ImmediateThreshold] {
                let model = OrdinalLogistic::params()
                    .alpha(1e-4)
                    .max_iterations(2000)
                    .loss(loss)
                    .margin(margin)
                    .fit(&dataset)
                    .unwrap();
                assert!(
                    model.coefficients()[0] > 0.0,
                    "{margin:?}/{loss:?} learned the wrong direction"
                );
                let predictions = model.predict(&dataset);
                let correct = predictions
                    .iter()
                    .zip(dataset.targets.iter())
                    .filter(|(a, b)| a == b)
                    .count();
                let accuracy = correct as f64 / dataset.targets.len() as f64;
                assert!(accuracy > 0.8, "{margin:?}/{loss:?} accuracy {accuracy}");
            }
        }
    }

    #[test]
    fn free_feature_index_is_validated() {
        let dataset = synthetic(20);
        assert!(matches!(
            OrdinalLogistic::params().free_features(&[7]).fit(&dataset),
            Err(OrdinalError::InvalidParameter { .. })
        ));
    }

    /// Every link and loss must actually learn the direction of the ordering, not merely run.
    #[test]
    fn every_link_and_loss_recovers_the_ordering() {
        let dataset = synthetic(300);
        for link in [Link::Logit, Link::Probit, Link::CLogLog, Link::Cauchit] {
            for loss in [
                OrdinalLoss::CumulativeLink,
                OrdinalLoss::AllThreshold,
                OrdinalLoss::ImmediateThreshold,
            ] {
                let model = OrdinalLogistic::params()
                    .alpha(1e-4)
                    .max_iterations(2000)
                    .link(link)
                    .loss(loss)
                    .fit(&dataset)
                    .unwrap();

                assert!(
                    model.coefficients()[0] > 0.0,
                    "{link:?}/{loss:?} learned the wrong direction: {}",
                    model.coefficients()[0]
                );
                assert!(
                    model.thresholds()[0] < model.thresholds()[1],
                    "{link:?}/{loss:?} thresholds out of order"
                );

                let predictions = model.predict(&dataset);
                let correct = predictions
                    .iter()
                    .zip(dataset.targets.iter())
                    .filter(|(a, b)| a == b)
                    .count();
                let accuracy = correct as f64 / dataset.targets.len() as f64;
                assert!(
                    accuracy > 0.8,
                    "{link:?}/{loss:?} training accuracy was only {accuracy}"
                );
            }
        }
    }

    /// The threshold losses fit cut points, not a likelihood, so they must refuse to hand back
    /// probabilities rather than inventing calibration that was never fitted.
    #[test]
    fn threshold_losses_have_no_probabilities() {
        let dataset = synthetic(80);
        for loss in [OrdinalLoss::AllThreshold, OrdinalLoss::ImmediateThreshold] {
            let model = OrdinalLogistic::params().loss(loss).fit(&dataset).unwrap();
            assert!(matches!(
                model.predict_probabilities(&array![0.0].view()),
                Err(OrdinalError::InvalidParameter { .. })
            ));
        }

        let cumulative = OrdinalLogistic::params().fit(&dataset).unwrap();
        assert!(
            cumulative
                .predict_probabilities(&array![0.0].view())
                .is_ok()
        );
    }

    #[test]
    fn thresholds_stay_ordered_for_any_raw_values() {
        for raw in [
            vec![-5.0f64, -30.0, -30.0],
            vec![0.0, 40.0, -40.0],
            vec![1.0, 0.0, 0.0],
        ] {
            let thresholds = thresholds_from_raw(&raw);
            for index in 1..thresholds.len() {
                assert!(
                    thresholds[index] >= thresholds[index - 1],
                    "thresholds not ordered: {thresholds:?}"
                );
            }
        }
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            OrdinalLogistic::<f64>::params().alpha(-1.0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            OrdinalLogistic::<f64>::params().max_iterations(0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            OrdinalLogistic::<f64>::params().learning_rate(0.0).check(),
            Err(OrdinalError::InvalidParameter { .. })
        ));
        assert!(matches!(
            OrdinalLogistic::<f64>::params().n_levels(1).check(),
            Err(OrdinalError::TooFewClasses { .. })
        ));
    }

    #[test]
    fn rejects_bad_targets() {
        let records = array![[1.0], [2.0]];
        let single_level = DatasetBase::new(records.clone(), array![0usize, 0]);
        assert!(matches!(
            OrdinalLogistic::params().fit(&single_level),
            Err(OrdinalError::TooFewClasses { .. })
        ));

        let out_of_range = DatasetBase::new(records, array![0usize, 5]);
        assert!(matches!(
            OrdinalLogistic::params().n_levels(2).fit(&out_of_range),
            Err(OrdinalError::RankOutOfRange { .. })
        ));
    }

    /// A declared level absent from the sample must still occupy its position in the ordering.
    #[test]
    fn declared_levels_survive_absence_from_the_sample() {
        let records = array![[0.0], [1.0], [2.0], [3.0]];
        let dataset = DatasetBase::new(records, array![0usize, 0, 2, 2]);
        let model = OrdinalLogistic::params().n_levels(3).fit(&dataset).unwrap();
        assert_eq!(model.n_classes(), 3);
        assert_eq!(model.thresholds().len(), 2);
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
        let model = OrdinalLogistic::<f32>::params().fit(&dataset).unwrap();
        assert!(model.coefficients()[0] > 0.0);
    }
}

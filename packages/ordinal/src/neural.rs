//! Rank-consistent neural ordinal heads: CORAL and CORN on a small MLP backbone.
//!
//! Every other estimator in this crate is linear in the features except [`crate::frank_hall`], and
//! that one is a wrapper whose `K - 1` independently fitted binary models agree on nothing and
//! therefore yield no calibrated probabilities. This module fills the remaining gap: non-linear AND
//! probabilistic AND rank-consistent at the same time.
//!
//! # Rank consistency is the point
//!
//! Both heads answer the same `K - 1` questions — *is `y > k`?* — and both are built so that
//!
//! ```text
//! P(y > 0 | x) >= P(y > 1 | x) >= ... >= P(y > K-2 | x)
//! ```
//!
//! can never be violated, for ANY parameter values, fitted or not. That is what separates them from
//! a Frank & Hall decomposition, where the `K - 1` answers can contradict each other ("more likely
//! above 3 than above 2") and have to be patched up at prediction time. Here the level
//! probabilities are simply the differences of that non-increasing sequence, so they are
//! non-negative and sum to exactly 1 without renormalization.
//!
//! # Be honest about what the backbone buys
//!
//! With a ZERO-hidden-layer backbone:
//!
//! - [`OrdinalHead::Coral`] reduces *exactly* to [`crate::logistic::OrdinalLogistic`] with
//!   [`crate::logistic::OrdinalLoss::AllThreshold`] and [`crate::logistic::Margin::Logistic`] — the
//!   objectives are the same function of the same parameters, with `b_k = -theta_k`.
//! - [`OrdinalHead::Corn`] reduces to [`crate::continuation_ratio::ContinuationRatio`] under
//!   [`crate::link::Link::Logit`]: the `K - 1` output units are then independent linear models
//!   fitted on the same shrinking subsets, differing only in modelling `P(y > k | y >= k)` where
//!   the continuation-ratio model writes `P(y = k | y >= k) = 1 - that`.
//!
//! The hidden layers are the entire contribution. A user whose problem is linear in the features
//! should prefer those two estimators: they are simpler, better tested, have a convex objective,
//! and expose interpretable coefficients that a network cannot. Reach for this module when the
//! decision boundary is genuinely not monotone in the features — see the module tests for a target
//! defined by `|x|`, which no linear ordinal model can represent at all.
//!
//! # The two heads
//!
//! **CORAL** (Cao, Mirjalili & Raschka 2020) puts a SINGLE scalar `g(x)` behind every task and lets
//! the tasks differ only by a bias:
//!
//! ```text
//! P(y > k | x) = sigmoid(g(x) + b_k)
//! ```
//!
//! Rank monotonicity holds iff `b_0 >= b_1 >= ... >= b_{K-2}`. The paper argues that the optimum of
//! its loss has them ordered; this implementation does better and makes it structural, reusing the
//! trick the rest of the crate uses for thresholds:
//!
//! ```text
//! b_0 = a_0,   b_k = b_{k-1} - softplus(a_k)
//! ```
//!
//! Since `softplus` is non-negative everywhere, the biases are non-increasing for any real `a`, so
//! the cumulative curves cannot cross — not at initialization, not mid-optimization, and not after
//! a step so large it overshoots. That is a strictly stronger guarantee than the paper's, which
//! holds only at the optimum. The loss is the standard all-threshold binary cross-entropy summed
//! over every task and every row, with task `k`'s target `[y > k]`.
//!
//! **CORN** (Shi, Cao & Raschka 2021) is conditional instead. The backbone emits `K - 1` outputs
//! and task `k` models
//!
//! ```text
//! P(y > k | y > k-1)
//! ```
//!
//! trained ONLY on the rows that got that far, i.e. `y >= k` — a subset that shrinks as `k` grows.
//! The unconditional probabilities come back through the chain rule
//!
//! ```text
//! P(y > k | x) = prod_{j <= k} P(y > j | y > j-1)
//! ```
//!
//! which is non-increasing in `k` because every factor lies in `(0, 1)`. CORN buys away CORAL's
//! rigid "one score, biases only" structure — each task gets its own weights on the shared
//! representation — at the cost of training the high tasks on progressively less data, exactly as
//! in [`crate::continuation_ratio`]. Prefer CORN when the levels are not separated by a single
//! monotone quantity; prefer CORAL when they are, or when data is thin at the top.
//!
//! # Backbone and training
//!
//! A plain MLP in pure ndarray: `input -> hidden_1 -> ... -> hidden_L -> head`, with
//! [`Activation::Relu`] or [`Activation::Tanh`] between layers and nothing on the head. He/Glorot
//! uniform initialization is drawn from a seeded splitmix64 written inline, so a fit is exactly
//! reproducible and the crate still depends on nothing but linfa core and ndarray.
//!
//! Adam over every parameter block, with the L2 penalty on WEIGHTS only. Biases are never
//! penalized, and neither are CORAL's ordering parameters — shrinking those would drag the task
//! biases together and quietly collapse adjacent levels, the same reason
//! [`crate::logistic`] never penalizes its thresholds.
//!
//! Features should be scaled. Nothing here is convex, so the fit depends on the seed; that is why
//! the seed is a first-class parameter rather than a hidden constant.
//!
//! Cao, W., Mirjalili, V. & Raschka, S. (2020), *Rank consistent ordinal regression for neural
//! networks with application to age estimation*.
//! Shi, X., Cao, W. & Raschka, S. (2021), *Deep Neural Networks for Rank-Consistent Ordinal
//! Regression Based On Conditional Probabilities*.
//!
//! # Example
//!
//! ```no_run
//! use linfa::prelude::*;
//! use flow_like_ordinal::neural::{Activation, OrdinalHead, OrdinalNeural};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dataset: linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>> = todo!();
//! let model = OrdinalNeural::params()
//!     .head(OrdinalHead::Corn)
//!     .hidden_layers(&[32, 16])
//!     .activation(Activation::Relu)
//!     .alpha(1e-3)
//!     .max_iterations(2000)
//!     .fit(&dataset)?;
//!
//! let levels = model.predict(&dataset);
//! let first = dataset.records().row(0);
//! let probabilities = model.predict_probabilities(&first)?;
//! # Ok(())
//! # }
//! ```

use crate::error::{OrdinalError, Result};
use crate::math::{logit, sigmoid, softplus, softplus_inv};
use linfa::dataset::AsSingleTargets;
use linfa::traits::{Fit, PredictInplace};
use linfa::{DatasetBase, Float, ParamGuard};
use ndarray::{Array1, Array2, ArrayBase, ArrayView1, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Non-linearity applied between the backbone's layers. The head itself is always linear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Activation {
    /// `max(0, z)`. The default: cheap, and its piecewise-linear folds are what let a small network
    /// represent a boundary like `|x|` that no linear ordinal model can.
    #[default]
    Relu,
    /// `tanh(z)`. Smooth everywhere, which matters when the gradient itself is under test — a
    /// central difference across ReLU's kink disagrees with the analytic derivative by design.
    Tanh,
}

impl Activation {
    fn apply<F: Float>(&self, z: &Array1<F>) -> Array1<F> {
        match self {
            Activation::Relu => z.mapv(|value| value.max(F::zero())),
            Activation::Tanh => z.mapv(|value| value.tanh()),
        }
    }

    /// Elementwise derivative at the PRE-activation values.
    ///
    /// At exactly zero ReLU has no derivative; the subgradient 0 is used, which Adam handles fine
    /// and which is the same convention as the hinge margins in [`crate::logistic`].
    fn derivative<F: Float>(&self, z: &Array1<F>) -> Array1<F> {
        match self {
            Activation::Relu => z.mapv(|value| {
                if value > F::zero() {
                    F::one()
                } else {
                    F::zero()
                }
            }),
            Activation::Tanh => z.mapv(|value| {
                let t = value.tanh();
                F::one() - t * t
            }),
        }
    }
}

/// Which rank-consistent head sits on top of the backbone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum OrdinalHead {
    /// One shared scalar score plus `K - 1` ordered biases. See the module documentation.
    ///
    /// Every task sees every row, so a declared level that never occurs in the training sample is
    /// still fittable — the same tolerance the cumulative-link model has.
    #[default]
    Coral,
    /// `K - 1` conditional tasks, each trained on the rows that reached its level.
    ///
    /// More flexible than [`OrdinalHead::Coral`], but the high tasks are fitted on the fewest rows
    /// and a level nothing reaches is rejected outright rather than fitted on nothing.
    Corn,
}

impl OrdinalHead {
    /// How many outputs the backbone must produce for this head.
    fn output_width(&self, n_classes: usize) -> usize {
        match self {
            OrdinalHead::Coral => 1,
            OrdinalHead::Corn => n_classes - 1,
        }
    }
}

/// Checked hyperparameters for [`OrdinalNeural`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalNeuralValidParams<F> {
    alpha: F,
    max_iterations: usize,
    tolerance: F,
    learning_rate: F,
    hidden_layers: Vec<usize>,
    activation: Activation,
    head: OrdinalHead,
    seed: u64,
    n_levels: Option<usize>,
}

impl<F: Float> OrdinalNeuralValidParams<F> {
    /// L2 penalty on the backbone's weight matrices only.
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

    /// Hidden layer widths, input side first. Empty means no hidden layer at all.
    pub fn hidden_layers(&self) -> &[usize] {
        &self.hidden_layers
    }

    pub fn activation(&self) -> Activation {
        self.activation
    }

    pub fn head(&self) -> OrdinalHead {
        self.head
    }

    /// Seed for the weight initialization, which is the only source of randomness in a fit.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Declared level count, or `None` to infer it from the observed targets.
    pub fn n_levels(&self) -> Option<usize> {
        self.n_levels
    }
}

/// Unchecked hyperparameters for [`OrdinalNeural`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalNeuralParams<F>(OrdinalNeuralValidParams<F>);

impl<F: Float> Default for OrdinalNeuralParams<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Float> OrdinalNeuralParams<F> {
    pub fn new() -> Self {
        Self(OrdinalNeuralValidParams {
            alpha: F::one(),
            max_iterations: 500,
            tolerance: F::cast(1e-7),
            // Lower than the linear estimators' 0.1: a step that is fine for a single coefficient
            // vector routinely overshoots once a hidden layer compounds it.
            learning_rate: F::cast(0.05),
            // A hidden layer by default, because a network with none is a worse-tested copy of
            // `OrdinalLogistic` and nobody should reach for this module to get one.
            hidden_layers: vec![16],
            activation: Activation::Relu,
            head: OrdinalHead::Coral,
            seed: 0x5EED_1234_ABCD_0001,
            n_levels: None,
        })
    }

    /// Hidden layer widths, input side first. Pass an empty slice for no hidden layer.
    ///
    /// Every width must be at least 1; a zero-width layer would disconnect the head from the input
    /// entirely and fit a constant, which is never what the caller meant.
    pub fn hidden_layers(mut self, sizes: &[usize]) -> Self {
        self.0.hidden_layers = sizes.to_vec();
        self
    }

    /// Non-linearity between the layers. Ignored when there are no hidden layers.
    pub fn activation(mut self, activation: Activation) -> Self {
        self.0.activation = activation;
        self
    }

    /// Which rank-consistent head to fit. See the module documentation for how to choose.
    pub fn head(mut self, head: OrdinalHead) -> Self {
        self.0.head = head;
        self
    }

    /// Seed for the weight initialization.
    ///
    /// The objective is not convex, so the seed genuinely changes the fit. It is a parameter rather
    /// than a hidden constant precisely so that a disappointing fit can be retried reproducibly.
    pub fn seed(mut self, seed: u64) -> Self {
        self.0.seed = seed;
        self
    }

    /// L2 penalty on the weight matrices.
    ///
    /// Biases and CORAL's ordering parameters are never penalized: shrinking the ordering
    /// parameters would pull the task biases together and collapse adjacent levels, which is a
    /// change to the model rather than to its variance.
    pub fn alpha(mut self, alpha: F) -> Self {
        self.0.alpha = alpha;
        self
    }

    /// Maximum optimizer iterations. Each one is a full pass over the training set.
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

    /// Declares the number of ordered levels. Left unset, it is inferred as `max rank + 1`.
    ///
    /// [`OrdinalHead::Coral`] carries a declared level that never occurs in the sample;
    /// [`OrdinalHead::Corn`] rejects one that nothing even reaches, because its task would then
    /// have no rows to fit.
    pub fn n_levels(mut self, n_levels: usize) -> Self {
        self.0.n_levels = Some(n_levels);
        self
    }
}

impl<F: Float> ParamGuard for OrdinalNeuralParams<F> {
    type Checked = OrdinalNeuralValidParams<F>;
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
        if let Some(position) = self.0.hidden_layers.iter().position(|width| *width == 0) {
            return Err(OrdinalError::InvalidParameter {
                name: "hidden_layers",
                reason: format!(
                    "hidden layer {position} has width 0, which would disconnect the head from the \
                     features and fit a constant; pass an empty slice for no hidden layer at all"
                ),
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

/// A fitted rank-consistent neural ordinal model.
///
/// # Probabilities
///
/// Unlike the threshold losses in [`crate::logistic`], this family always carries a probability
/// model. [`Self::cumulative_probabilities`] gives `P(y > k)`, non-increasing in `k` by
/// construction, and [`Self::predict_probabilities`] differences it into a distribution that sums
/// to 1 with no clamping or renormalization needed.
///
/// # How a level is predicted
///
/// [`PredictInplace`] counts the tasks whose `P(y > k)` exceeds one half — the rule both papers
/// use, and the only one that stays faithful to a rank-consistent head. Because the sequence is
/// non-increasing that count is unambiguous. It is NOT the argmax of the level distribution and can
/// differ from it near a boundary; use [`Self::predict_probabilities`] and take the argmax yourself
/// if exact-match accuracy is what you are optimizing.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OrdinalNeural<F> {
    backbone: Backbone<F>,
    head: OrdinalHead,
    /// CORAL's unconstrained ordering parameters, `K - 1` of them. Empty under CORN, which keeps
    /// its per-task parameters in the backbone's output layer instead.
    head_params: Array1<F>,
    n_classes: usize,
    iterations: usize,
    converged: bool,
}

impl<F: Float> OrdinalNeural<F> {
    /// Starts a builder for the hyperparameters.
    pub fn params() -> OrdinalNeuralParams<F> {
        OrdinalNeuralParams::new()
    }

    pub fn head(&self) -> OrdinalHead {
        self.head
    }

    pub fn activation(&self) -> Activation {
        self.backbone.activation
    }

    pub fn n_classes(&self) -> usize {
        self.n_classes
    }

    pub fn n_features(&self) -> usize {
        self.backbone.n_features()
    }

    pub fn iterations(&self) -> usize {
        self.iterations
    }

    /// False when the optimizer hit `max_iterations` before the objective settled.
    pub fn converged(&self) -> bool {
        self.converged
    }

    /// Hidden layer widths as fitted, input side first. Empty when the backbone is a plain linear
    /// map, in which case this model is a worse-tested copy of a simpler estimator — see the module
    /// documentation.
    pub fn hidden_layers(&self) -> Vec<usize> {
        let n_layers = self.backbone.weights.len();
        self.backbone
            .weights
            .iter()
            .take(n_layers - 1)
            .map(|matrix| matrix.nrows())
            .collect()
    }

    /// Weight matrices, input side first. Layer `l` is `out_l x in_l`.
    pub fn weights(&self) -> &[Array2<F>] {
        &self.backbone.weights
    }

    /// Bias vectors, input side first.
    pub fn biases(&self) -> &[Array1<F>] {
        &self.backbone.biases
    }

    /// CORAL's `K - 1` task biases, non-increasing by construction. `None` under CORN.
    ///
    /// These are the negated equivalent of [`crate::logistic::OrdinalLogistic::thresholds`]: with a
    /// zero-hidden-layer backbone, `b_k = -theta_k` recovers that model exactly.
    pub fn task_biases(&self) -> Option<Array1<F>> {
        match self.head {
            OrdinalHead::Coral => Some(coral_biases(&self.head_params)),
            OrdinalHead::Corn => None,
        }
    }

    /// CORAL's single latent score `g(x)`, the one number every task shares.
    ///
    /// CORN has no such quantity — each of its tasks has its own output — so this fails there
    /// rather than returning one of the `K - 1` outputs and pretending it is the score.
    pub fn latent_score(&self, row: &ArrayView1<'_, F>) -> Result<F> {
        if self.head != OrdinalHead::Coral {
            return Err(OrdinalError::InvalidParameter {
                name: "head",
                reason: "only OrdinalHead::Coral has a single shared latent score; CORN fits one \
                         output per task, so use conditional_probabilities instead"
                    .to_string(),
            });
        }
        self.check_width(row)?;
        Ok(self.backbone.forward(row).output[0])
    }

    /// CORN's native quantities: the `K - 1` conditional continuation probabilities
    /// `P(y > k | y > k-1)`.
    ///
    /// These are NOT a distribution and do not sum to anything in particular; multiplying the first
    /// `k + 1` of them is what gives `P(y > k)`. CORAL has no conditional parameterization, so this
    /// fails there instead of manufacturing ratios the fit never optimized.
    pub fn conditional_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        if self.head != OrdinalHead::Corn {
            return Err(OrdinalError::InvalidParameter {
                name: "head",
                reason: "only OrdinalHead::Corn models conditional continuation probabilities; \
                         CORAL fits one shared score, so use latent_score or \
                         cumulative_probabilities instead"
                    .to_string(),
            });
        }
        self.check_width(row)?;
        Ok(self
            .backbone
            .forward(row)
            .output
            .iter()
            .map(|z| sigmoid(*z))
            .collect())
    }

    /// `P(y > k)` for each of the `K - 1` tasks, in level order.
    ///
    /// Non-increasing for ANY parameter values, which is the guarantee this whole module exists to
    /// provide. Nothing is clamped or sorted to make that true.
    pub fn cumulative_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        self.check_width(row)?;
        Ok(self.cumulative_unchecked(row))
    }

    /// Per-level probabilities for one sample, in level order. Always sums to 1.
    pub fn predict_probabilities(&self, row: &ArrayView1<'_, F>) -> Result<Vec<F>> {
        self.check_width(row)?;
        Ok(level_probabilities(&self.cumulative_unchecked(row)))
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
    /// Meaningful precisely because the target is ordered: rounding it optimizes rank error, where
    /// the threshold count from [`PredictInplace`] optimizes per-task correctness.
    pub fn predict_expected_level(&self, row: &ArrayView1<'_, F>) -> Result<F> {
        let probabilities = self.predict_probabilities(row)?;
        Ok(probabilities
            .iter()
            .enumerate()
            .map(|(level, p)| F::cast(level) * *p)
            .fold(F::zero(), |acc, value| acc + value))
    }

    fn check_width(&self, row: &ArrayView1<'_, F>) -> Result<()> {
        let expected = self.n_features();
        if row.len() != expected {
            return Err(OrdinalError::FeatureWidthMismatch {
                expected,
                found: row.len(),
            });
        }
        Ok(())
    }

    /// `P(y > k)` without the width check, for the prediction paths that already validated it.
    fn cumulative_unchecked(&self, row: &ArrayView1<'_, F>) -> Vec<F> {
        let output = self.backbone.forward(row).output;
        match self.head {
            OrdinalHead::Coral => {
                // One score, biases only: monotone because the biases are.
                let score = output[0];
                coral_biases(&self.head_params)
                    .iter()
                    .map(|bias| sigmoid(score + *bias))
                    .collect()
            }
            OrdinalHead::Corn => {
                // Chain rule: monotone because every factor is a sigmoid, hence in (0, 1).
                let mut running = F::one();
                output
                    .iter()
                    .map(|z| {
                        running *= sigmoid(*z);
                        running
                    })
                    .collect()
            }
        }
    }

    /// The level implied by a non-increasing `P(y > k)` sequence: how many tasks clear one half.
    fn level_from_cumulative(&self, above: &[F]) -> usize {
        let half = F::cast(0.5);
        above
            .iter()
            .filter(|value| **value > half)
            .count()
            .min(self.n_classes - 1)
    }
}

impl<F: Float, D: Data<Elem = F>, T: AsSingleTargets<Elem = usize>>
    Fit<ArrayBase<D, Ix2>, T, OrdinalError> for OrdinalNeuralValidParams<F>
{
    type Object = OrdinalNeural<F>;

    /// Fits the model on ranks in `0..n_levels`, ordered lowest to highest by the caller.
    ///
    /// The ordering of the levels is the caller's contract: rank 0 must be the lowest level. This
    /// crate cannot verify it, and getting it wrong trains a model that is confidently backwards.
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
        if self.head == OrdinalHead::Corn {
            check_corn_levels_are_reachable(&targets, n_classes)?;
        }

        let mut sizes = Vec::with_capacity(self.hidden_layers.len() + 2);
        sizes.push(n_features);
        sizes.extend(self.hidden_layers.iter().copied());
        sizes.push(self.head.output_width(n_classes));

        let mut backbone = Backbone::new(&sizes, self.activation, self.seed);
        let mut head_params = match self.head {
            OrdinalHead::Coral => initial_head_params(&targets, n_classes),
            OrdinalHead::Corn => {
                // CORN keeps its per-task parameters in the output layer, so the marginal seed goes
                // straight onto that layer's biases.
                let last = backbone.biases.len() - 1;
                backbone.biases[last] = initial_corn_biases(&targets, n_classes);
                Array1::zeros(0)
            }
        };

        // Adam moments, one flat buffer per parameter block. Kept as separate vectors rather than
        // one struct per layer so two blocks of the same layer can be borrowed at once.
        let mut weight_m: Vec<Vec<F>> = backbone
            .weights
            .iter()
            .map(|matrix| vec![F::zero(); matrix.len()])
            .collect();
        let mut weight_v = weight_m.clone();
        let mut bias_m: Vec<Vec<F>> = backbone
            .biases
            .iter()
            .map(|bias| vec![F::zero(); bias.len()])
            .collect();
        let mut bias_v = bias_m.clone();
        let mut head_m = vec![F::zero(); head_params.len()];
        let mut head_v = head_m.clone();

        let beta1 = F::cast(0.9);
        let beta2 = F::cast(0.999);

        let mut previous = F::infinity();
        let mut iterations = 0;
        let mut converged = false;

        for step in 1..=self.max_iterations {
            iterations = step;
            let (objective, grad_weights, grad_biases, grad_head) = objective_and_gradient(
                records,
                &targets,
                &backbone,
                &head_params,
                self.head,
                n_classes,
                self.alpha,
            )?;

            if !objective.is_finite() {
                return Err(OrdinalError::NonFinite {
                    context: "the network loss",
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

            let correction1 = F::one() - beta1.powi(step as i32);
            let correction2 = F::one() - beta2.powi(step as i32);

            for layer in 0..backbone.weights.len() {
                adam_update(
                    backbone.weights[layer].iter_mut(),
                    grad_weights[layer].iter(),
                    &mut weight_m[layer],
                    &mut weight_v[layer],
                    self.learning_rate,
                    correction1,
                    correction2,
                );
                adam_update(
                    backbone.biases[layer].iter_mut(),
                    grad_biases[layer].iter(),
                    &mut bias_m[layer],
                    &mut bias_v[layer],
                    self.learning_rate,
                    correction1,
                    correction2,
                );
            }
            adam_update(
                head_params.iter_mut(),
                grad_head.iter(),
                &mut head_m,
                &mut head_v,
                self.learning_rate,
                correction1,
                correction2,
            );
        }

        let finite = backbone
            .weights
            .iter()
            .flatten()
            .chain(backbone.biases.iter().flatten())
            .chain(head_params.iter())
            .all(|value| value.is_finite());
        if !finite {
            return Err(OrdinalError::NonFinite {
                context: "the fitted parameters",
            });
        }

        Ok(OrdinalNeural {
            backbone,
            head: self.head,
            head_params,
            n_classes,
            iterations,
            converged,
        })
    }
}

impl<F: Float, D: Data<Elem = F>> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>>
    for OrdinalNeural<F>
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
            y[index] = self.level_from_cumulative(&self.cumulative_unchecked(&row));
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// The MLP behind either head.
///
/// Kept separate from the fitted model so the optimizer and the finite-difference tests can perturb
/// its parameter blocks directly, which is the only way to check a hand-derived backward pass.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
struct Backbone<F> {
    /// Layer `l` maps `sizes[l]` inputs to `sizes[l + 1]` outputs, stored `out x in`.
    weights: Vec<Array2<F>>,
    biases: Vec<Array1<F>>,
    activation: Activation,
}

/// Everything the backward pass needs from one forward pass.
struct Forward<F> {
    /// `inputs[l]` is what layer `l` consumed; `inputs[0]` is the sample itself.
    inputs: Vec<Array1<F>>,
    /// `pre[l]` is layer `l`'s output BEFORE the activation, which is where its derivative is read.
    pre: Vec<Array1<F>>,
    output: Array1<F>,
}

impl<F: Float> Backbone<F> {
    /// `sizes` is `[n_features, hidden.., output_width]`, so it always holds at least two entries.
    fn new(sizes: &[usize], activation: Activation, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed);
        let n_layers = sizes.len() - 1;
        let mut weights = Vec::with_capacity(n_layers);
        let mut biases = Vec::with_capacity(n_layers);

        for layer in 0..n_layers {
            let fan_in = sizes[layer];
            let fan_out = sizes[layer + 1];
            // He for a layer a ReLU follows, Glorot for a tanh layer and for the linear head. The
            // head is always Glorot regardless of `activation`, since nothing rectifies its output.
            let limit = if layer + 1 < n_layers && activation == Activation::Relu {
                F::cast(6.0 / fan_in as f64).sqrt()
            } else {
                F::cast(6.0 / (fan_in + fan_out) as f64).sqrt()
            };

            let mut matrix = Array2::<F>::zeros((fan_out, fan_in));
            for value in matrix.iter_mut() {
                *value = rng.symmetric(limit);
            }
            weights.push(matrix);
            // Zero biases: symmetry is already broken by the weights, and a non-zero start would
            // only fight the marginal seeding applied to the output layer.
            biases.push(Array1::<F>::zeros(fan_out));
        }

        Self {
            weights,
            biases,
            activation,
        }
    }

    fn n_features(&self) -> usize {
        self.weights[0].ncols()
    }

    fn forward(&self, x: &ArrayView1<'_, F>) -> Forward<F> {
        let n_layers = self.weights.len();
        let mut inputs = Vec::with_capacity(n_layers);
        let mut pre = Vec::with_capacity(n_layers);
        let mut current = x.to_owned();

        for layer in 0..n_layers {
            let z = self.weights[layer].dot(&current) + &self.biases[layer];
            inputs.push(current);
            // The head is linear: no activation on the last layer.
            current = if layer + 1 < n_layers {
                self.activation.apply(&z)
            } else {
                z.clone()
            };
            pre.push(z);
        }

        Forward {
            inputs,
            pre,
            output: current,
        }
    }

    /// Backpropagates `delta = dL/d(output)` for one row, accumulating into the gradient buffers.
    fn accumulate(
        &self,
        cache: &Forward<F>,
        mut delta: Array1<F>,
        grad_weights: &mut [Array2<F>],
        grad_biases: &mut [Array1<F>],
    ) {
        for layer in (0..self.weights.len()).rev() {
            let input = &cache.inputs[layer];
            for (out_index, d) in delta.iter().enumerate() {
                for (in_index, value) in input.iter().enumerate() {
                    grad_weights[layer][[out_index, in_index]] += *d * *value;
                }
                grad_biases[layer][out_index] += *d;
            }
            if layer > 0 {
                // dL/d(input of layer) = W' delta, then through the activation below it.
                delta = self.weights[layer].t().dot(&delta)
                    * self.activation.derivative(&cache.pre[layer - 1]);
            }
        }
    }

    /// Zeroed gradient buffers shaped like this backbone.
    fn zeros_like(&self) -> (Vec<Array2<F>>, Vec<Array1<F>>) {
        (
            self.weights
                .iter()
                .map(|matrix| Array2::zeros(matrix.raw_dim()))
                .collect(),
            self.biases
                .iter()
                .map(|bias| Array1::zeros(bias.raw_dim()))
                .collect(),
        )
    }
}

/// Penalized objective and its gradient with respect to every parameter block.
///
/// Both heads sum a binary cross-entropy over their tasks, written as `softplus(z) - t * z` rather
/// than through logs of probabilities. That form is exact, needs no probability floor, and stays
/// finite for any logit, which matters here because a network's outputs are unbounded in a way a
/// penalized linear score is not. Its derivative in `z` is `sigmoid(z) - t`.
fn objective_and_gradient<F: Float, D: Data<Elem = F>>(
    records: &ArrayBase<D, Ix2>,
    targets: &[usize],
    backbone: &Backbone<F>,
    head_params: &Array1<F>,
    head: OrdinalHead,
    n_classes: usize,
    alpha: F,
) -> Result<(F, Vec<Array2<F>>, Vec<Array1<F>>, Array1<F>)> {
    let n_cuts = n_classes - 1;
    let (mut grad_weights, mut grad_biases) = backbone.zeros_like();
    let task_biases = coral_biases(head_params);
    let mut grad_task_biases = vec![F::zero(); task_biases.len()];
    let mut objective = F::zero();

    for (row, &rank) in records.rows().into_iter().zip(targets.iter()) {
        let cache = backbone.forward(&row);
        let mut delta = Array1::<F>::zeros(cache.output.len());

        match head {
            OrdinalHead::Coral => {
                // One score feeds every task, so its gradient is the SUM of the task residuals.
                let score = cache.output[0];
                let mut d_score = F::zero();
                for cut in 0..n_cuts {
                    let z = score + task_biases[cut];
                    let target = if rank > cut { F::one() } else { F::zero() };
                    objective = objective + softplus(z) - target * z;

                    let residual = sigmoid(z) - target;
                    d_score += residual;
                    grad_task_biases[cut] += residual;
                }
                delta[0] = d_score;
            }
            OrdinalHead::Corn => {
                // Task `k` is conditional on having reached level `k`, so this row trains only the
                // tasks up to its own rank. The tasks above it are simply absent from its loss.
                let last = rank.min(n_cuts - 1);
                for cut in 0..=last {
                    let z = cache.output[cut];
                    let target = if rank > cut { F::one() } else { F::zero() };
                    objective = objective + softplus(z) - target * z;
                    delta[cut] = sigmoid(z) - target;
                }
            }
        }

        backbone.accumulate(&cache, delta, &mut grad_weights, &mut grad_biases);
    }

    // Map dL/db onto the unconstrained ordering parameters. `a_0` shifts EVERY task bias, so it
    // takes the whole sum; `a_k` for k >= 1 shifts its own bias and all the ones after it, scaled by
    // softplus'(a_k) = sigmoid(a_k) and negated because the gap is subtracted.
    let mut grad_head = Array1::<F>::zeros(head_params.len());
    let mut suffix = F::zero();
    for cut in (0..grad_task_biases.len()).rev() {
        suffix += grad_task_biases[cut];
        grad_head[cut] = if cut == 0 {
            suffix
        } else {
            -sigmoid(head_params[cut]) * suffix
        };
    }

    // L2 on the weight matrices only. Penalizing the biases would pull the task thresholds toward
    // each other and collapse adjacent levels, which changes the model rather than its variance.
    let half = F::cast(0.5);
    let mut penalty = F::zero();
    for (layer, matrix) in backbone.weights.iter().enumerate() {
        for (index, value) in matrix.indexed_iter() {
            penalty += *value * *value;
            grad_weights[layer][index] += alpha * *value;
        }
    }
    objective += half * alpha * penalty;

    let finite = grad_weights
        .iter()
        .flatten()
        .chain(grad_biases.iter().flatten())
        .chain(grad_head.iter())
        .all(|value| value.is_finite());
    if !objective.is_finite() || !finite {
        return Err(OrdinalError::NonFinite {
            context: "the gradient",
        });
    }

    Ok((objective, grad_weights, grad_biases, grad_head))
}

/// One Adam step over a parameter block and its gradient.
///
/// Written over iterators so the same routine drives a weight matrix, a bias vector and the head's
/// ordering parameters without three copies of the update rule.
fn adam_update<'a, F: Float + 'a>(
    parameters: impl IntoIterator<Item = &'a mut F>,
    gradients: impl IntoIterator<Item = &'a F>,
    first_moment: &mut [F],
    second_moment: &mut [F],
    learning_rate: F,
    correction1: F,
    correction2: F,
) {
    let beta1 = F::cast(0.9);
    let beta2 = F::cast(0.999);
    let eps = F::cast(1e-8);

    for (index, (parameter, gradient)) in parameters.into_iter().zip(gradients).enumerate() {
        first_moment[index] = beta1 * first_moment[index] + (F::one() - beta1) * *gradient;
        second_moment[index] =
            beta2 * second_moment[index] + (F::one() - beta2) * *gradient * *gradient;
        let m_hat = first_moment[index] / correction1;
        let v_hat = second_moment[index] / correction2;
        *parameter -= learning_rate * m_hat / (v_hat.sqrt() + eps);
    }
}

/// CORAL's `K - 1` task biases from the unconstrained head parameters.
///
/// `b_0 = a_0`, `b_k = b_{k-1} - softplus(a_k)`. Since `softplus` is non-negative everywhere the
/// sequence is non-increasing for ANY real `a`, which is what makes rank monotonicity structural
/// rather than something the fit has to discover. The original paper only argues that the optimum
/// has them ordered; this holds at every point of the optimization, including after an overshoot.
fn coral_biases<F: Float>(raw: &Array1<F>) -> Array1<F> {
    let mut biases = Array1::zeros(raw.len());
    let mut current = F::zero();
    for (index, value) in raw.iter().enumerate() {
        current = if index == 0 {
            *value
        } else {
            current - softplus(*value)
        };
        biases[index] = current;
    }
    biases
}

/// Level probabilities from the non-increasing `P(y > k)` sequence.
///
/// `P(y = 0) = 1 - P(y > 0)`, `P(y = k) = P(y > k-1) - P(y > k)`, `P(y = K-1) = P(y > K-2)`. The
/// sum telescopes to exactly 1, so nothing is renormalized. The floor at zero is a guard against
/// floating-point noise only — every difference is non-negative by construction.
fn level_probabilities<F: Float>(above: &[F]) -> Vec<F> {
    let mut probabilities = Vec::with_capacity(above.len() + 1);
    let mut previous = F::one();
    for value in above {
        probabilities.push((previous - *value).max(F::zero()));
        previous = *value;
    }
    probabilities.push(previous.max(F::zero()));
    probabilities
}

/// Seeds CORAL's ordering parameters at the marginal log-odds of `P(y > k)`.
///
/// That is the exact optimum when the backbone contributes nothing, so starting there rather than
/// at zero saves a large number of iterations and keeps a badly imbalanced target from stalling the
/// optimizer — the same reasoning as the threshold seeding in [`crate::logistic`].
fn initial_head_params<F: Float>(targets: &[usize], n_classes: usize) -> Array1<F> {
    let n_cuts = n_classes - 1;
    let total = F::cast(targets.len());
    let floor = F::cast(1e-6);
    let min_gap = F::cast(1e-3);

    let mut biases: Vec<F> = Vec::with_capacity(n_cuts);
    for cut in 0..n_cuts {
        let above = F::cast(targets.iter().filter(|rank| **rank > cut).count());
        // Clamped away from 0 and 1 so a level absent from the sample cannot make the log-odds
        // infinite.
        let proportion = (above / total).min(F::one() - floor).max(floor);
        biases.push(logit(proportion));
    }

    // Enforce a strict gap before inverting, so `softplus_inv` never sees a non-positive delta.
    for index in 1..biases.len() {
        if biases[index] >= biases[index - 1] - min_gap {
            biases[index] = biases[index - 1] - min_gap;
        }
    }

    let mut raw = Array1::zeros(n_cuts);
    for (index, value) in biases.iter().enumerate() {
        raw[index] = if index == 0 {
            *value
        } else {
            softplus_inv(biases[index - 1] - *value)
        };
    }
    raw
}

/// Seeds CORN's output-layer biases at the log-odds of each observed conditional continuation rate.
///
/// Each task's own subset supplies its seed, which matters more here than for CORAL: the high tasks
/// see the fewest rows and would otherwise spend their whole budget walking away from zero.
fn initial_corn_biases<F: Float>(targets: &[usize], n_classes: usize) -> Array1<F> {
    let n_cuts = n_classes - 1;
    let floor = F::cast(1e-6);
    let mut biases = Array1::zeros(n_cuts);

    for cut in 0..n_cuts {
        let reached = targets.iter().filter(|rank| **rank >= cut).count();
        let continued = targets.iter().filter(|rank| **rank > cut).count();
        // `reached` cannot be zero: `check_corn_levels_are_reachable` rejected that case first.
        let proportion = (F::cast(continued) / F::cast(reached))
            .min(F::one() - floor)
            .max(floor);
        biases[cut] = logit(proportion);
    }
    biases
}

/// Rejects a declared level that no training sample reaches, which CORN cannot fit.
///
/// CORN task `k` trains only on the rows with `y >= k`. If nothing reaches level `k` that subset is
/// EMPTY, so the task contributes no gradient at all and its output drifts to whatever the penalty
/// leaves behind — a silent constant one half dressed up as a fitted probability. Failing is the
/// only honest answer. CORAL has no such failure mode: all of its tasks see every row.
fn check_corn_levels_are_reachable(targets: &[usize], n_classes: usize) -> Result<()> {
    for cut in 0..n_classes - 1 {
        if !targets.iter().any(|rank| *rank >= cut) {
            let previous = cut as i64 - 1;
            return Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                reason: format!(
                    "no training sample reaches level {cut}, so CORN's task for \
                     `P(y > {cut} | y > {previous})` has no rows to fit; drop the declared level or \
                     switch to OrdinalHead::Coral, whose tasks all see every row"
                ),
            });
        }
    }
    Ok(())
}

/// splitmix64: a deterministic PRNG in ten lines.
///
/// Written inline rather than pulling in `rand`/`rand_xoshiro` because the only randomness this
/// crate needs is weight initialization, and the whole crate's value proposition is that it depends
/// on linfa core and ndarray alone. splitmix64 passes BigCrush, needs no seeding ceremony (any u64
/// works, including zero) and is exactly reproducible across platforms, which is all a weight
/// initializer asks for.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform on `[-limit, limit)`.
    fn symmetric<F: Float>(&mut self, limit: F) -> F {
        // The top 53 bits fill an f64 mantissa exactly, so the draw is uniform at the full
        // representable resolution and never rounds to 1.
        let unit = F::cast((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64);
        (F::cast(2.0) * unit - F::one()) * limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logistic::{Margin, OrdinalLogistic, OrdinalLoss};
    use linfa::traits::Predict;
    use ndarray::array;

    /// A clean ordinal problem: one informative feature, levels cut at fixed points.
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

    /// The same three levels, but ordered by `|x|` instead of `x`.
    ///
    /// No linear ordinal model can represent this: the level is not monotone in the feature, so a
    /// single latent score `w * x + c` can only ever cut the line into three contiguous intervals
    /// and must sacrifice one of the two arms.
    fn folded(n: usize) -> DatasetBase<Array2<f64>, Array1<usize>> {
        let mut records = Array2::zeros((n, 1));
        let mut targets = Vec::with_capacity(n);
        for index in 0..n {
            let x = -3.0 + 6.0 * (index as f64) / (n as f64 - 1.0);
            records[[index, 0]] = x;
            targets.push(if x.abs() > 2.0 {
                0
            } else if x.abs() > 1.0 {
                1
            } else {
                2
            });
        }
        DatasetBase::new(records, Array1::from(targets))
    }

    fn accuracy(
        model: &OrdinalNeural<f64>,
        dataset: &DatasetBase<Array2<f64>, Array1<usize>>,
    ) -> f64 {
        let predictions = model.predict(dataset);
        let correct = predictions
            .iter()
            .zip(dataset.targets.iter())
            .filter(|(a, b)| a == b)
            .count();
        correct as f64 / dataset.targets.len() as f64
    }

    /// Small, spread-out weights and LARGE hidden biases.
    ///
    /// The large biases are the whole point: they keep every hidden pre-activation far from zero on
    /// the test inputs. ReLU's derivative is a step there, so a central difference straddling the
    /// kink disagrees with the analytic gradient by construction rather than because the gradient is
    /// wrong, and a test that hit the kink would be measuring the wrong thing.
    fn fixture_backbone(
        hidden: &[usize],
        out_width: usize,
        activation: Activation,
    ) -> Backbone<f64> {
        let mut sizes = vec![1usize];
        sizes.extend_from_slice(hidden);
        sizes.push(out_width);

        let mut backbone = Backbone::<f64>::new(&sizes, activation, 7);
        let n_layers = backbone.weights.len();
        for (layer, (matrix, bias)) in backbone
            .weights
            .iter_mut()
            .zip(backbone.biases.iter_mut())
            .enumerate()
        {
            // Range [-0.07, 0.13], never exactly zero, so no gradient is trivially null.
            for (index, value) in matrix.iter_mut().enumerate() {
                *value = 0.05 * ((index % 5) as f64 - 2.0) + 0.03;
            }
            if layer + 1 < n_layers {
                for (index, value) in bias.iter_mut().enumerate() {
                    *value = if index % 2 == 0 { 2.5 } else { -2.5 };
                }
            }
        }
        backbone
    }

    /// Distance from ReLU's kink over the whole dataset: the smallest hidden pre-activation seen.
    fn min_abs_hidden_pre_activation(backbone: &Backbone<f64>, records: &Array2<f64>) -> f64 {
        let hidden_layers = backbone.weights.len().saturating_sub(1);
        let mut smallest = f64::INFINITY;
        for row in records.rows() {
            let cache = backbone.forward(&row);
            for layer in 0..hidden_layers {
                for value in cache.pre[layer].iter() {
                    smallest = smallest.min(value.abs());
                }
            }
        }
        smallest
    }

    /// Backprop is where this silently goes wrong: a mis-derived gradient still converges, just to
    /// the wrong model. Every parameter block — every weight matrix, every bias vector and CORAL's
    /// ordering parameters — is checked against a central difference, for both heads, both
    /// activations, and zero, one and two hidden layers.
    #[test]
    fn analytic_gradient_matches_finite_differences() {
        let dataset = synthetic(30);
        let targets: Vec<usize> = dataset.targets.iter().copied().collect();
        let n_classes = 3;
        let alpha = 0.6;
        let step = 1e-6;

        for head in [OrdinalHead::Coral, OrdinalHead::Corn] {
            let out_width = head.output_width(n_classes);
            for activation in [Activation::Tanh, Activation::Relu] {
                for hidden in [vec![], vec![3], vec![3, 2]] {
                    let backbone = fixture_backbone(&hidden, out_width, activation);
                    let head_params: Array1<f64> = match head {
                        OrdinalHead::Coral => array![0.3, -0.2],
                        OrdinalHead::Corn => Array1::zeros(0),
                    };
                    let label = format!("{head:?}/{activation:?}/hidden={hidden:?}");

                    assert!(
                        min_abs_hidden_pre_activation(&backbone, dataset.records()) > 1e-2,
                        "{label}: the fixture landed on ReLU's kink, where a central difference \
                         cannot agree with any derivative"
                    );

                    let at = |b: &Backbone<f64>, h: &Array1<f64>| {
                        objective_and_gradient(
                            dataset.records(),
                            &targets,
                            b,
                            h,
                            head,
                            n_classes,
                            alpha,
                        )
                        .unwrap()
                    };
                    let (_, grad_weights, grad_biases, grad_head) = at(&backbone, &head_params);

                    for layer in 0..backbone.weights.len() {
                        let (rows, columns) = backbone.weights[layer].dim();
                        for row in 0..rows {
                            for column in 0..columns {
                                let mut shifted = backbone.clone();
                                shifted.weights[layer][[row, column]] += step;
                                let up = at(&shifted, &head_params).0;
                                shifted.weights[layer][[row, column]] -= 2.0 * step;
                                let down = at(&shifted, &head_params).0;
                                let numeric = (up - down) / (2.0 * step);
                                let analytic = grad_weights[layer][[row, column]];
                                assert!(
                                    (numeric - analytic).abs() < 1e-4,
                                    "{label} weights[{layer}][{row},{column}]: analytic {analytic} \
                                     vs numeric {numeric}"
                                );
                            }
                        }

                        for index in 0..backbone.biases[layer].len() {
                            let mut shifted = backbone.clone();
                            shifted.biases[layer][index] += step;
                            let up = at(&shifted, &head_params).0;
                            shifted.biases[layer][index] -= 2.0 * step;
                            let down = at(&shifted, &head_params).0;
                            let numeric = (up - down) / (2.0 * step);
                            let analytic = grad_biases[layer][index];
                            assert!(
                                (numeric - analytic).abs() < 1e-4,
                                "{label} biases[{layer}][{index}]: analytic {analytic} vs numeric \
                                 {numeric}"
                            );
                        }
                    }

                    for index in 0..head_params.len() {
                        let mut shifted = head_params.clone();
                        shifted[index] += step;
                        let up = at(&backbone, &shifted).0;
                        shifted[index] -= 2.0 * step;
                        let down = at(&backbone, &shifted).0;
                        let numeric = (up - down) / (2.0 * step);
                        let analytic = grad_head[index];
                        assert!(
                            (numeric - analytic).abs() < 1e-4,
                            "{label} head[{index}]: analytic {analytic} vs numeric {numeric}"
                        );
                    }
                }
            }
        }
    }

    /// Random, never-fitted parameters. This is what proves the parameterization is structural
    /// rather than a property the optimizer happens to find: if monotonicity only held after a fit,
    /// it would be a fact about the data, not about the model.
    #[test]
    fn rank_consistency_holds_for_random_parameters() {
        let n_classes = 5;
        for seed in [1u64, 2, 3, 17, 99, 12345] {
            for head in [OrdinalHead::Coral, OrdinalHead::Corn] {
                let out_width = head.output_width(n_classes);
                let backbone = random_backbone(&[3, 6, out_width], Activation::Tanh, seed);
                let head_params: Array1<f64> = match head {
                    OrdinalHead::Coral => {
                        let mut rng = SplitMix64::new(seed ^ 0xABCD);
                        (0..n_classes - 1).map(|_| rng.symmetric(4.0)).collect()
                    }
                    OrdinalHead::Corn => Array1::zeros(0),
                };
                let model = OrdinalNeural {
                    backbone,
                    head,
                    head_params,
                    n_classes,
                    iterations: 0,
                    converged: false,
                };

                let mut rng = SplitMix64::new(seed.wrapping_mul(7919));
                for _ in 0..25 {
                    let row: Array1<f64> = (0..3).map(|_| rng.symmetric(5.0)).collect();
                    let above = model.cumulative_probabilities(&row.view()).unwrap();
                    assert_eq!(above.len(), n_classes - 1);
                    for cut in 1..above.len() {
                        assert!(
                            above[cut] <= above[cut - 1],
                            "{head:?}/seed {seed}: P(y > k) increased: {above:?}"
                        );
                    }

                    let probabilities = model.predict_probabilities(&row.view()).unwrap();
                    assert_eq!(probabilities.len(), n_classes);
                    assert!(probabilities.iter().all(|p| *p >= 0.0), "{probabilities:?}");
                    let total: f64 = probabilities.iter().sum();
                    assert!(
                        (total - 1.0).abs() < 1e-9,
                        "{head:?}/seed {seed}: probabilities summed to {total}"
                    );
                }
            }
        }
    }

    /// Genuinely random, large-magnitude parameters — the initializer's Glorot limits are far too
    /// small to stress monotonicity.
    fn random_backbone(sizes: &[usize], activation: Activation, seed: u64) -> Backbone<f64> {
        let mut backbone = Backbone::<f64>::new(sizes, activation, seed);
        let mut rng = SplitMix64::new(seed.wrapping_mul(31).wrapping_add(7));
        for (matrix, bias) in backbone.weights.iter_mut().zip(backbone.biases.iter_mut()) {
            for value in matrix.iter_mut() {
                *value = rng.symmetric(3.0);
            }
            for value in bias.iter_mut() {
                *value = rng.symmetric(3.0);
            }
        }
        backbone
    }

    /// The reduction claimed in the module docs, tested rather than asserted.
    ///
    /// A zero-hidden-layer CORAL head optimizes the same function of the same parameters as the
    /// all-threshold logistic model, with `b_k = -theta_k`. If the two disagree on the data or on
    /// where they place their cut points, the docs are lying about what the backbone contributes.
    #[test]
    fn zero_hidden_coral_reduces_to_the_all_threshold_logistic() {
        let dataset = synthetic(150);
        // A penalty large enough to pin the optimum at a finite scale: the data are separable, so
        // with a vanishing penalty both objectives keep drifting outward and the comparison would
        // be measuring how far each optimizer happened to walk rather than where it converged.
        let coral = OrdinalNeural::params()
            .head(OrdinalHead::Coral)
            .hidden_layers(&[])
            .alpha(1e-2)
            .learning_rate(0.05)
            .max_iterations(6000)
            .tolerance(0.0)
            .fit(&dataset)
            .unwrap();
        let logistic = OrdinalLogistic::params()
            .loss(OrdinalLoss::AllThreshold)
            .margin(Margin::Logistic)
            .alpha(1e-2)
            .learning_rate(0.05)
            .max_iterations(6000)
            .tolerance(0.0)
            .fit(&dataset)
            .unwrap();

        let neural_levels = coral.predict(&dataset);
        let linear_levels = logistic.predict(&dataset);
        let agree = neural_levels
            .iter()
            .zip(linear_levels.iter())
            .filter(|(a, b)| a == b)
            .count();
        let agreement = agree as f64 / dataset.targets.len() as f64;
        assert!(
            agreement > 0.95,
            "the reduction is claimed in the module docs but the two models agree on only \
             {agreement} of the rows"
        );

        // Same geometry, not merely the same answers: both place their cut points at the same
        // feature values. CORAL's decision point for task k solves g(x) + b_k = 0.
        let slope = coral.weights()[0][[0, 0]];
        let intercept = coral.biases()[0][0];
        let task_biases = coral.task_biases().unwrap();
        for cut in 0..2 {
            let neural_cut = -(intercept + task_biases[cut]) / slope;
            let linear_cut = logistic.thresholds()[cut] / logistic.coefficients()[0];
            assert!(
                (neural_cut - linear_cut).abs() < 0.3,
                "cut {cut}: neural at {neural_cut}, linear at {linear_cut}"
            );
        }
        assert!(
            task_biases[1] < task_biases[0],
            "CORAL's task biases must decrease: {task_biases:?}"
        );
    }

    /// Both heads must actually learn a monotone ordinal problem, not merely run on one.
    #[test]
    fn recovers_a_monotone_ordinal_problem() {
        let dataset = synthetic(150);
        for head in [OrdinalHead::Coral, OrdinalHead::Corn] {
            let model = OrdinalNeural::params()
                .head(head)
                .hidden_layers(&[8])
                .activation(Activation::Tanh)
                .alpha(1e-4)
                .learning_rate(0.05)
                .max_iterations(1500)
                .fit(&dataset)
                .unwrap();

            let score = accuracy(&model, &dataset);
            assert!(score > 0.85, "{head:?} training accuracy was only {score}");
            assert_eq!(model.n_classes(), 3);
            assert_eq!(model.hidden_layers(), vec![8]);

            let low = model.predict_expected_level(&array![-3.0].view()).unwrap();
            let mid = model.predict_expected_level(&array![0.0].view()).unwrap();
            let high = model.predict_expected_level(&array![3.0].view()).unwrap();
            assert!(low < mid && mid < high, "{head:?}: {low} {mid} {high}");
        }
    }

    /// The hidden layers are the entire contribution over the linear estimators, so a boundary that
    /// is not monotone in the feature is the test that says whether they earn their keep.
    ///
    /// The objective is not convex, so a fit can land in a poor basin; trying another seed is the
    /// documented remedy, and the `seed` parameter is what makes that reproducible.
    #[test]
    fn hidden_layers_reach_a_non_linear_boundary() {
        let dataset = folded(120);

        let fit = |hidden: &[usize], seed: u64| {
            OrdinalNeural::params()
                .head(OrdinalHead::Coral)
                .hidden_layers(hidden)
                .activation(Activation::Relu)
                .alpha(1e-4)
                .learning_rate(0.05)
                .max_iterations(2500)
                .tolerance(1e-9)
                .seed(seed)
                .fit(&dataset)
                .unwrap()
        };

        let deep = [1u64, 2]
            .iter()
            .map(|seed| accuracy(&fit(&[16], *seed), &dataset))
            .fold(0.0f64, f64::max);
        let shallow = [1u64, 2]
            .iter()
            .map(|seed| accuracy(&fit(&[], *seed), &dataset))
            .fold(0.0f64, f64::max);

        assert!(deep > 0.8, "the hidden layer only reached {deep}");
        assert!(
            deep > shallow + 0.1,
            "a hidden layer bought nothing: {deep} against {shallow} for the linear backbone"
        );
    }

    /// The seed is the only randomness in a fit, so it must fully determine it.
    #[test]
    fn the_seed_determines_the_fit() {
        let dataset = synthetic(80);
        let fit = |seed: u64| {
            OrdinalNeural::params()
                .hidden_layers(&[4])
                .max_iterations(50)
                .seed(seed)
                .fit(&dataset)
                .unwrap()
        };
        assert_eq!(fit(11), fit(11));
        assert_ne!(fit(11), fit(12));
    }

    #[test]
    fn convergence_is_reported() {
        let dataset = synthetic(60);
        let stalled = OrdinalNeural::params()
            .max_iterations(3)
            .tolerance(0.0)
            .fit(&dataset)
            .unwrap();
        assert!(!stalled.converged());
        assert_eq!(stalled.iterations(), 3);

        let settled = OrdinalNeural::params()
            .max_iterations(5000)
            .tolerance(1e-4)
            .fit(&dataset)
            .unwrap();
        assert!(settled.converged());
        assert!(settled.iterations() < 5000);
    }

    /// Each head owns a different native quantity, and neither should manufacture the other's.
    #[test]
    fn head_specific_accessors_refuse_the_other_head() {
        let dataset = synthetic(60);
        let row = array![0.5];

        let coral = OrdinalNeural::params()
            .head(OrdinalHead::Coral)
            .max_iterations(50)
            .fit(&dataset)
            .unwrap();
        assert!(coral.task_biases().is_some());
        assert!(coral.latent_score(&row.view()).is_ok());
        assert!(matches!(
            coral.conditional_probabilities(&row.view()),
            Err(OrdinalError::InvalidParameter { name: "head", .. })
        ));

        let corn = OrdinalNeural::params()
            .head(OrdinalHead::Corn)
            .max_iterations(50)
            .fit(&dataset)
            .unwrap();
        assert!(corn.task_biases().is_none());
        assert!(matches!(
            corn.latent_score(&row.view()),
            Err(OrdinalError::InvalidParameter { name: "head", .. })
        ));
        let conditional = corn.conditional_probabilities(&row.view()).unwrap();
        assert_eq!(conditional.len(), 2);
        assert!(conditional.iter().all(|p| *p > 0.0 && *p < 1.0));
    }

    #[test]
    fn probabilities_are_a_distribution_after_fitting() {
        let dataset = synthetic(120);
        for head in [OrdinalHead::Coral, OrdinalHead::Corn] {
            let model = OrdinalNeural::params()
                .head(head)
                .hidden_layers(&[6])
                .max_iterations(300)
                .fit(&dataset)
                .unwrap();

            let batch = model
                .predict_probabilities_batch(dataset.records())
                .unwrap();
            assert_eq!(batch.dim(), (120, 3));
            for row in dataset.records().rows() {
                let probabilities = model.predict_probabilities(&row).unwrap();
                let total: f64 = probabilities.iter().sum();
                assert!(
                    (total - 1.0).abs() < 1e-9,
                    "{head:?}: probabilities summed to {total}"
                );
                assert!(probabilities.iter().all(|p| *p >= 0.0));
            }
        }
    }

    #[test]
    fn coral_biases_never_increase_for_any_raw_values() {
        for raw in [
            array![5.0f64, -30.0, -30.0],
            array![0.0, 40.0, -40.0],
            array![1.0, 0.0, 0.0],
            array![-12.0, 12.0, 0.5],
        ] {
            let biases = coral_biases(&raw);
            for index in 1..biases.len() {
                assert!(
                    biases[index] <= biases[index - 1],
                    "task biases increased: {biases:?}"
                );
            }
        }
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            OrdinalNeural::<f64>::params().alpha(-1.0).check(),
            Err(OrdinalError::InvalidParameter { name: "alpha", .. })
        ));
        assert!(matches!(
            OrdinalNeural::<f64>::params().max_iterations(0).check(),
            Err(OrdinalError::InvalidParameter {
                name: "max_iterations",
                ..
            })
        ));
        assert!(matches!(
            OrdinalNeural::<f64>::params().tolerance(-1.0).check(),
            Err(OrdinalError::InvalidParameter {
                name: "tolerance",
                ..
            })
        ));
        assert!(matches!(
            OrdinalNeural::<f64>::params().learning_rate(0.0).check(),
            Err(OrdinalError::InvalidParameter {
                name: "learning_rate",
                ..
            })
        ));
        assert!(matches!(
            OrdinalNeural::<f64>::params()
                .hidden_layers(&[8, 0])
                .check(),
            Err(OrdinalError::InvalidParameter {
                name: "hidden_layers",
                ..
            })
        ));
        assert!(matches!(
            OrdinalNeural::<f64>::params().n_levels(1).check(),
            Err(OrdinalError::TooFewClasses { .. })
        ));
        assert!(
            OrdinalNeural::<f64>::params()
                .hidden_layers(&[])
                .check()
                .is_ok()
        );
    }

    #[test]
    fn rejects_bad_targets() {
        let records = array![[1.0], [2.0]];
        let single_level = DatasetBase::new(records.clone(), array![0usize, 0]);
        assert!(matches!(
            OrdinalNeural::params().fit(&single_level),
            Err(OrdinalError::TooFewClasses { .. })
        ));

        let out_of_range = DatasetBase::new(records, array![0usize, 5]);
        assert!(matches!(
            OrdinalNeural::params().n_levels(2).fit(&out_of_range),
            Err(OrdinalError::RankOutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_malformed_training_data() {
        let empty = DatasetBase::new(
            Array2::<f64>::zeros((0, 1)),
            Array1::from(Vec::<usize>::new()),
        );
        assert!(matches!(
            OrdinalNeural::params().fit(&empty),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let no_features = DatasetBase::new(Array2::<f64>::zeros((3, 0)), array![0usize, 1, 1]);
        assert!(matches!(
            OrdinalNeural::params().fit(&no_features),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let mismatched =
            DatasetBase::new(Array2::<f64>::zeros((3, 1)), Array1::from(vec![0usize, 1]));
        assert!(matches!(
            OrdinalNeural::params().fit(&mismatched),
            Err(OrdinalError::LengthMismatch {
                records: 3,
                targets: 2
            })
        ));

        let not_finite = DatasetBase::new(array![[f64::NAN], [1.0]], array![0usize, 1]);
        assert!(matches!(
            OrdinalNeural::params().fit(&not_finite),
            Err(OrdinalError::NonFinite { .. })
        ));
    }

    #[test]
    fn predictions_validate_the_feature_width() {
        let dataset = synthetic(40);
        let model = OrdinalNeural::params()
            .max_iterations(20)
            .fit(&dataset)
            .unwrap();
        assert!(matches!(
            model.predict_probabilities(&array![1.0, 2.0].view()),
            Err(OrdinalError::FeatureWidthMismatch {
                expected: 1,
                found: 2
            })
        ));
        assert!(matches!(
            model.cumulative_probabilities(&array![1.0, 2.0].view()),
            Err(OrdinalError::FeatureWidthMismatch { .. })
        ));
        assert!(matches!(
            model.latent_score(&array![1.0, 2.0].view()),
            Err(OrdinalError::FeatureWidthMismatch { .. })
        ));
    }

    /// CORN's shrinking subsets make an unreachable level unfittable; CORAL's do not. The asymmetry
    /// is the point, so both halves are asserted.
    #[test]
    fn corn_rejects_a_level_nothing_reaches() {
        let records = array![[0.0], [1.0], [2.0], [3.0]];
        let dataset = DatasetBase::new(records, array![0usize, 0, 1, 1]);

        assert!(matches!(
            OrdinalNeural::params()
                .head(OrdinalHead::Corn)
                .n_levels(4)
                .fit(&dataset),
            Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                ..
            })
        ));

        let coral = OrdinalNeural::params()
            .head(OrdinalHead::Coral)
            .n_levels(4)
            .max_iterations(50)
            .fit(&dataset)
            .unwrap();
        assert_eq!(coral.n_classes(), 4);
        assert_eq!(coral.task_biases().unwrap().len(), 3);
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

        for head in [OrdinalHead::Coral, OrdinalHead::Corn] {
            let model = OrdinalNeural::<f32>::params()
                .head(head)
                .hidden_layers(&[4])
                .alpha(1e-3)
                .max_iterations(400)
                .fit(&dataset)
                .unwrap();
            let probabilities = model
                .predict_probabilities(&dataset.records().row(0))
                .unwrap();
            let total: f32 = probabilities.iter().sum();
            assert!((total - 1.0).abs() < 1e-5, "{head:?}: summed to {total}");
            assert!(
                model
                    .predict_expected_level(&array![-3.0f32].view())
                    .unwrap()
                    < model
                        .predict_expected_level(&array![3.0f32].view())
                        .unwrap()
            );
        }
    }
}

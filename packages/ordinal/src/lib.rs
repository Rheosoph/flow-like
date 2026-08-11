//! Ordinal regression for flow-like, built on linfa's traits.
//!
//! An *ordinal* target has classes with a meaningful order — `1 < 2 < ... < 6`, or
//! `low < medium < high` — where being off by one level is a smaller mistake than being off by
//! three. linfa offers no estimator for this: its classifiers treat labels as unordered, throwing
//! the ordering away, and its regressors treat the levels as real numbers, inventing distances
//! that the levels do not actually carry.
//!
//! Three estimators are provided:
//!
//! - [`OrdinalLogistic`] — a threshold model: one shared coefficient vector plus `K - 1` ordered
//!   cut points. Choose the CDF with [`Link`] (logit gives proportional odds, probit gives ordered
//!   probit) and the objective with [`OrdinalLoss`] (the cumulative likelihood, or the
//!   all-threshold / immediate-threshold losses that drop the proportional-odds assumption).
//!   Prefer it when you want probabilities or interpretable coefficients.
//! - [`OrdinalRidge`] — regress the rank, then cut the score at learned thresholds. Closed-form,
//!   so it stays cheap when there are many levels or many features.
//! - [`FrankHall`] — not an estimator but a *wrapper*: it decomposes the problem into `K - 1`
//!   binary "is y > k?" questions and hands each to a base classifier you supply. It is therefore
//!   the only one of the three that is not linear in the features, and the way to put a random
//!   forest, an SVM or a naive Bayes model onto an ordered target.
//!
//! [`proportional_odds_report`] checks the assumption the first estimator rests on, which no
//! accuracy number will reveal.
//!
//! Both follow linfa's estimator conventions, so they compose with the rest of the ecosystem:
//! builder-style params guarded by [`linfa::ParamGuard`], fitting through [`linfa::traits::Fit`]
//! over a [`linfa::DatasetBase`], and prediction through [`linfa::traits::PredictInplace`].
//!
//! ```no_run
//! use linfa::prelude::*;
//! use flow_like_ordinal::OrdinalLogistic;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let (train, valid): (linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
//! #                      linfa::DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>) = todo!();
//! let model = OrdinalLogistic::params().alpha(1.0).fit(&train)?;
//! let predicted = model.predict(&valid);
//! # Ok(())
//! # }
//! ```
//!
//! Targets are ranks in `0..n_levels`, lowest first. Mapping the user's labels onto those ranks —
//! and deciding what the order even is — is the caller's job; this crate cannot verify it, and
//! getting it wrong trains a model that is confidently backwards.
//!
//! Dependency-wise this stays light on purpose: linfa *core* (traits and `DatasetBase`) plus
//! ndarray, no algorithm crates and no LAPACK backend. The linear algebra is hand-rolled.

pub mod adjacent_category;
pub mod continuation_ratio;
pub mod diagnostics;
pub mod error;
pub mod frank_hall;
pub mod link;
pub mod logistic;
pub mod math;
pub mod metrics;
pub mod neural;
pub mod ridge;

pub use adjacent_category::{
    AdjacentCategory, AdjacentCategoryParams, AdjacentCategoryValidParams,
};
pub use continuation_ratio::{
    ContinuationRatio, ContinuationRatioParams, ContinuationRatioValidParams,
};
pub use diagnostics::{ProportionalOddsReport, proportional_odds_report};
pub use error::{OrdinalError, Result};
pub use frank_hall::{FrankHall, FrankHallParams, FrankHallValidParams};
pub use link::Link;
pub use logistic::{
    Margin, OrdinalLogistic, OrdinalLogisticParams, OrdinalLogisticValidParams, OrdinalLoss,
};
pub use metrics::{
    KappaWeighting, OrdinalMetrics, accuracy_within, kendall_tau_b, linear_weighted_kappa,
    macro_mean_absolute_error, mean_absolute_rank_error, quadratic_weighted_kappa,
    spearman_rank_correlation, weighted_kappa,
};
pub use neural::{
    Activation, OrdinalHead, OrdinalNeural, OrdinalNeuralParams, OrdinalNeuralValidParams,
};
pub use ridge::{OrdinalRidge, OrdinalRidgeParams, OrdinalRidgeValidParams};

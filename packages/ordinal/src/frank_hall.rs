//! Frank & Hall ordered-binary decomposition: give an ordered target to *any* binary classifier.
//!
//! For `K` ordered levels, fit `K - 1` binary models, where model `k` answers one question — *is
//! `y > k`?* — and read the level back off their answers. Nothing about the base learner is
//! assumed beyond "it can separate two classes", which is what makes this the one estimator here
//! that is not linear in the features: it is not an estimator at all, it is a wrapper. A random
//! forest, an SVM or a naive Bayes classifier plugged into it becomes an ordinal model, and none
//! of them can be used on an ordered target otherwise.
//!
//! The decomposition is deliberately generic over the base learner's *params*, so this crate keeps
//! depending on linfa core alone. The caller supplies the base params; the wrapper only ever calls
//! [`linfa::traits::Fit`] and [`linfa::traits::PredictInplace`] on them.
//!
//! What is lost against [`crate::logistic::OrdinalLogistic`] is a single coherent latent scale:
//! the `K - 1` models are fitted independently, so nothing forces their answers to agree. See
//! [`FrankHall`] for how that is handled at prediction time.
//!
//! Frank, E. & Hall, M. (2001), *A Simple Approach to Ordinal Classification*.

use crate::error::{OrdinalError, Result};
use linfa::dataset::AsSingleTargets;
use linfa::traits::{Fit, PredictInplace};
use linfa::{DatasetBase, Float, ParamGuard};
use ndarray::{Array1, Array2, ArrayBase, Data, Ix2};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

/// Checked hyperparameters for [`FrankHall`].
///
/// `B` is the base learner's parameter object and `E` its error type. `E` cannot be derived from
/// `B` alone — a params type that implements [`ParamGuard`] gets a [`Fit`] impl for *every* error
/// type able to absorb its own — so it is carried here as a marker and defaults to
/// [`linfa::error::Error`], which is what linfa's own estimators use.
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(bound(serialize = "B: Serialize", deserialize = "B: Deserialize<'de>"))
)]
pub struct FrankHallValidParams<B, E = linfa::error::Error> {
    base: B,
    n_levels: Option<usize>,
    #[cfg_attr(feature = "serde", serde(skip))]
    base_error: PhantomData<fn() -> E>,
}

impl<B, E> FrankHallValidParams<B, E> {
    /// The base learner every binary sub-problem is fitted with.
    pub fn base(&self) -> &B {
        &self.base
    }

    /// Declared level count, or `None` to infer it from the observed targets.
    pub fn n_levels(&self) -> Option<usize> {
        self.n_levels
    }
}

/// Unchecked hyperparameters for [`FrankHall`].
///
/// ```no_run
/// # use flow_like_ordinal::frank_hall::FrankHallParams;
/// # use linfa::error::Error;
/// # let base_params = ();
/// // `E` defaults to linfa's own error type; name it when the base learner has its own.
/// let params = FrankHallParams::<_, Error>::new(base_params).n_levels(5);
/// ```
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(bound(serialize = "B: Serialize", deserialize = "B: Deserialize<'de>"))
)]
pub struct FrankHallParams<B, E = linfa::error::Error>(FrankHallValidParams<B, E>);

impl<B, E> FrankHallParams<B, E> {
    /// Wraps a base learner's parameters.
    ///
    /// Mirrors `EnsembleLearnerParams::new` rather than this crate's `Model::params()`: the fitted
    /// model type only exists once the base learner has been fitted, so there is no `FrankHall<M>`
    /// for an associated function to hang off.
    pub fn new(base: B) -> Self {
        Self(FrankHallValidParams {
            base,
            n_levels: None,
            base_error: PhantomData,
        })
    }

    /// Declares the number of ordered levels. Left unset, it is inferred as `max rank + 1`.
    ///
    /// Unlike the linear estimators in this crate, a declared level that never occurs in the
    /// training data is only harmless in the *middle* of the ordering. An absent lowest or highest
    /// level leaves its cut with a single class, which is rejected at fit time rather than
    /// silently shifting every later cut by one.
    pub fn n_levels(mut self, n_levels: usize) -> Self {
        self.0.n_levels = Some(n_levels);
        self
    }

    /// The base learner every binary sub-problem is fitted with.
    pub fn base(&self) -> &B {
        &self.0.base
    }
}

// `E` is a marker that is never held by value, but `derive` would still demand `E: Debug`,
// `E: Clone` and `E: PartialEq` — bounds that real linfa error types do not all satisfy.
impl<B: fmt::Debug, E> fmt::Debug for FrankHallValidParams<B, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrankHallValidParams")
            .field("base", &self.base)
            .field("n_levels", &self.n_levels)
            .finish()
    }
}

impl<B: Clone, E> Clone for FrankHallValidParams<B, E> {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            n_levels: self.n_levels,
            base_error: PhantomData,
        }
    }
}

impl<B: PartialEq, E> PartialEq for FrankHallValidParams<B, E> {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base && self.n_levels == other.n_levels
    }
}

impl<B: fmt::Debug, E> fmt::Debug for FrankHallParams<B, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FrankHallParams").field(&self.0).finish()
    }
}

impl<B: Clone, E> Clone for FrankHallParams<B, E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<B: PartialEq, E> PartialEq for FrankHallParams<B, E> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<B, E> ParamGuard for FrankHallParams<B, E> {
    type Checked = FrankHallValidParams<B, E>;
    type Error = OrdinalError;

    fn check_ref(&self) -> Result<&Self::Checked> {
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

/// A fitted Frank & Hall decomposition: `K - 1` binary models, one per cut.
///
/// # Why hard votes rather than probabilities
///
/// The predicted level is the *count* of models answering "yes, `y > k`". The textbook reading
/// instead differences the cumulative probabilities, `P(y = k) = P(y > k-1) - P(y > k)`, and takes
/// the argmax. That is not used here, for three reasons.
///
/// linfa has no uniform trait for "give me a probability" — probability targets exist on some
/// estimators and not on others — so requiring one would restrict this wrapper to logistic
/// regression and Platt-scaled SVMs and exclude decision trees and forests entirely: the very
/// learners it exists to unlock. Counting needs nothing but `bool`.
///
/// The count is also robust to non-monotone answers. Model 3 saying yes while model 1 says no is
/// impossible for a coherent ordinal model but entirely possible for `K - 1` independent fits, and
/// it drives the probability difference above negative. A count has no such failure mode.
///
/// And it costs nothing when the answers *are* consistent: monotone answers are yes for every cut
/// below the true level and no for the rest, so their count is exactly the level the differencing
/// reading would pick. The two agree wherever the second one is defined.
///
/// The trade-off is that no calibrated per-level probabilities are available. Use
/// [`crate::logistic::OrdinalLogistic`] when those are the point.
///
/// # Example
///
/// The base learner is anything implementing linfa's [`Fit`] over a boolean target, so a function
/// written against these bounds takes a decision tree, an SVM or a naive Bayes classifier
/// unchanged:
///
/// ```no_run
/// use flow_like_ordinal::OrdinalError;
/// use flow_like_ordinal::frank_hall::{FrankHall, FrankHallParams};
/// use linfa::DatasetBase;
/// use linfa::traits::{Fit, Predict, PredictInplace};
/// use ndarray::{Array1, Array2};
///
/// fn fit_ordinal<B, E>(
///     base: B,
///     train: &DatasetBase<Array2<f64>, Array1<usize>>,
/// ) -> Result<Array1<usize>, OrdinalError>
/// where
///     B: Fit<Array2<f64>, Array1<bool>, E>,
///     E: std::error::Error + From<linfa::error::Error>,
///     <B as Fit<Array2<f64>, Array1<bool>, E>>::Object: PredictInplace<Array2<f64>, Array1<bool>>,
/// {
///     let params: FrankHallParams<B, E> = FrankHallParams::new(base);
///     let model: FrankHall<_> = params.n_levels(4).fit(train)?;
///     Ok(model.predict(&train.records))
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FrankHall<M> {
    models: Vec<M>,
    n_classes: usize,
    n_features: usize,
}

impl<M> FrankHall<M> {
    /// The `K - 1` fitted binary models, lowest cut first: `models()[k]` answers `y > k`.
    /// Rebuilds a decomposition from its parts.
    ///
    /// Exists so a caller can substitute the base model type — typically to wrap a base learner
    /// that does not implement `Serialize` in something that does, since `FrankHall<M>` is only
    /// serializable when `M` is. Pair it with [`Self::into_parts`].
    ///
    /// The models must be in threshold order: `models[k]` answers "is y > k?". Passing them out of
    /// order silently reverses the level scale, so this is deliberately explicit rather than a
    /// `From` impl.
    pub fn from_parts(models: Vec<M>, n_classes: usize, n_features: usize) -> Result<Self> {
        if n_classes < 2 {
            return Err(OrdinalError::TooFewClasses { found: n_classes });
        }
        if models.len() + 1 != n_classes {
            return Err(OrdinalError::InvalidParameter {
                name: "models",
                reason: format!(
                    "a {n_classes}-level decomposition needs {} binary models, got {}",
                    n_classes - 1,
                    models.len()
                ),
            });
        }
        Ok(Self {
            models,
            n_classes,
            n_features,
        })
    }

    /// Decomposes into `(models, n_classes, n_features)`, the inverse of [`Self::from_parts`].
    pub fn into_parts(self) -> (Vec<M>, usize, usize) {
        (self.models, self.n_classes, self.n_features)
    }

    pub fn models(&self) -> &[M] {
        &self.models
    }

    pub fn n_classes(&self) -> usize {
        self.n_classes
    }

    pub fn n_features(&self) -> usize {
        self.n_features
    }
}

impl<F, D, T, B, E> Fit<ArrayBase<D, Ix2>, T, OrdinalError> for FrankHallValidParams<B, E>
where
    F: Float,
    D: Data<Elem = F>,
    T: AsSingleTargets<Elem = usize>,
    B: Fit<Array2<F>, Array1<bool>, E>,
    E: std::error::Error + From<linfa::error::Error>,
{
    type Object = FrankHall<<B as Fit<Array2<F>, Array1<bool>, E>>::Object>;

    /// Fits one binary model per cut on ranks in `0..n_levels`, ordered lowest to highest.
    ///
    /// Every cut is checked for separability *before* any model is fitted, so a level that cannot
    /// be cut fails immediately instead of after `K - 2` expensive fits.
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
        // The wrapper never does arithmetic on the features itself, but the learners it exists to
        // unlock do: tree and SVM implementations sort feature values with `partial_cmp`, which
        // panics on NaN. Rejecting it here turns that panic into an error.
        if records.iter().any(|value| !value.is_finite()) {
            return Err(OrdinalError::NonFinite {
                context: "the feature matrix",
            });
        }

        let mut counts = vec![0usize; n_classes];
        for rank in &targets {
            counts[*rank] += 1;
        }
        check_cuts_are_separable(&counts)?;

        // linfa takes ownership of the records through `DatasetBase`, so one copy is unavoidable.
        // Only the targets differ between cuts, so that copy is made once and reused.
        let mut binary = DatasetBase::new(records.to_owned(), Array1::from_elem(n_samples, false))
            .with_weights(dataset.weights.clone());

        let mut models = Vec::with_capacity(n_classes - 1);
        for level in 0..n_classes - 1 {
            binary.targets = targets.iter().map(|rank| *rank > level).collect();
            let model = self.base.fit(&binary).map_err(|error| {
                OrdinalError::Linfa(format!(
                    "base learner failed on the binary problem `y > {level}`: {error}"
                ))
            })?;
            models.push(model);
        }

        Ok(FrankHall {
            models,
            n_classes,
            n_features,
        })
    }
}

impl<F, D, M> PredictInplace<ArrayBase<D, Ix2>, Array1<usize>> for FrankHall<M>
where
    F: Float,
    D: Data<Elem = F>,
    M: PredictInplace<Array2<F>, Array1<bool>>,
{
    fn predict_inplace(&self, x: &ArrayBase<D, Ix2>, y: &mut Array1<usize>) {
        assert_eq!(
            x.nrows(),
            y.len(),
            "The number of data points must match the number of output targets."
        );
        assert_eq!(
            x.ncols(),
            self.n_features,
            "The number of features must match the number the model was fitted on."
        );

        // Bounding `M` over every storage type instead would exclude models that only implement
        // `PredictInplace` for an owned matrix — linfa's own `EnsembleLearner`, i.e. the random
        // forest, among them. One copy is made per call and shared by all `K - 1` models.
        let records = x.to_owned();

        y.fill(0);
        for model in &self.models {
            let mut votes = model.default_target(&records);
            model.predict_inplace(&records, &mut votes);
            for (level, vote) in y.iter_mut().zip(votes.iter()) {
                if *vote {
                    *level += 1;
                }
            }
        }

        // Unreachable for a model this crate fitted, where the vote count cannot exceed `K - 1`,
        // but a deserialized model may carry an `n_classes` that disagrees with its model count.
        let highest = self.n_classes.saturating_sub(1);
        for level in y.iter_mut() {
            *level = (*level).min(highest);
        }
    }

    fn default_target(&self, x: &ArrayBase<D, Ix2>) -> Array1<usize> {
        Array1::zeros(x.nrows())
    }
}

/// Rejects any cut with an empty side, naming the level responsible.
///
/// Cut `k` splits the sample into `y <= k` and `y > k`; if either side is empty the base learner is
/// handed a single-class problem, which most reject and the rest fit meaninglessly. Failing is the
/// only safe answer: skipping the cut would renumber every level above it, and substituting a
/// constant answer would invent a model the caller never asked for. In practice this means the
/// lowest and the highest level must both occur in the training data — a level absent *between*
/// them leaves every cut well posed and is accepted.
fn check_cuts_are_separable(counts: &[usize]) -> Result<()> {
    let total: usize = counts.iter().sum();
    let cuts = counts.len().saturating_sub(1);
    let mut below = 0usize;
    for (level, count) in counts.iter().take(cuts).enumerate() {
        below += *count;
        if below == 0 || below == total {
            let side = if below == 0 { "at or below" } else { "above" };
            return Err(OrdinalError::InvalidParameter {
                name: "n_levels",
                reason: format!(
                    "no training sample lies {side} level {level}, so the binary problem for the \
                     cut `y > {level}` has a single class and cannot be fitted; the lowest and the \
                     highest of the {n} declared levels must both occur in the training data",
                    n = counts.len()
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use linfa::error::Error as LinfaError;
    use linfa::traits::Predict;
    use ndarray::{Axis, array};
    use std::cell::RefCell;

    /// The smallest well-behaved binary learner: the midpoint between the highest negative and the
    /// lowest positive value of feature 0.
    ///
    /// Deterministic, and exact on any linearly separable split, which is all the wrapper's own
    /// logic needs from a base learner. Using a real one would drag in the linfa algorithm crates
    /// this crate is built to avoid.
    #[derive(Debug, Clone, PartialEq)]
    struct MidpointSplitParams;

    #[derive(Debug, Clone, PartialEq)]
    struct MidpointSplit<F> {
        cut: F,
    }

    impl<F: Float> Fit<Array2<F>, Array1<bool>, LinfaError> for MidpointSplitParams {
        type Object = MidpointSplit<F>;

        fn fit(
            &self,
            dataset: &DatasetBase<Array2<F>, Array1<bool>>,
        ) -> std::result::Result<Self::Object, LinfaError> {
            let mut highest_negative: Option<F> = None;
            let mut lowest_positive: Option<F> = None;
            for (row, label) in dataset
                .records
                .rows()
                .into_iter()
                .zip(dataset.targets.iter())
            {
                let value = row[0];
                if *label {
                    lowest_positive = Some(match lowest_positive {
                        Some(current) if current <= value => current,
                        _ => value,
                    });
                } else {
                    highest_negative = Some(match highest_negative {
                        Some(current) if current >= value => current,
                        _ => value,
                    });
                }
            }

            match (highest_negative, lowest_positive) {
                (Some(negative), Some(positive)) => Ok(MidpointSplit {
                    cut: (negative + positive) / F::cast(2.0),
                }),
                _ => Err(LinfaError::Parameters(
                    "one side of the binary split is empty".to_string(),
                )),
            }
        }
    }

    impl<F: Float> PredictInplace<Array2<F>, Array1<bool>> for MidpointSplit<F> {
        fn predict_inplace(&self, x: &Array2<F>, y: &mut Array1<bool>) {
            assert_eq!(x.nrows(), y.len());
            for (answer, row) in y.iter_mut().zip(x.rows().into_iter()) {
                *answer = row[0] > self.cut;
            }
        }

        fn default_target(&self, x: &Array2<F>) -> Array1<bool> {
            Array1::from_elem(x.nrows(), false)
        }
    }

    /// Records the binary targets it is handed, so the decomposition itself can be asserted on.
    #[derive(Debug, Default)]
    struct Recorder {
        seen: RefCell<Vec<Vec<bool>>>,
    }

    impl<F: Float> Fit<Array2<F>, Array1<bool>, LinfaError> for Recorder {
        type Object = ();

        fn fit(
            &self,
            dataset: &DatasetBase<Array2<F>, Array1<bool>>,
        ) -> std::result::Result<(), LinfaError> {
            self.seen
                .borrow_mut()
                .push(dataset.targets.iter().copied().collect());
            Ok(())
        }
    }

    /// One tight cluster per listed level, ten samples each, spaced far enough apart that a
    /// midpoint split separates any grouping of them exactly.
    fn clustered(levels: &[usize]) -> DatasetBase<Array2<f64>, Array1<usize>> {
        let per_level = 10;
        let mut records = Array2::zeros((levels.len() * per_level, 1));
        let mut targets = Vec::with_capacity(levels.len() * per_level);
        for (cluster, level) in levels.iter().enumerate() {
            for index in 0..per_level {
                let row = cluster * per_level + index;
                records[[row, 0]] = 100.0 * *level as f64 + index as f64 * 0.01;
                targets.push(*level);
            }
        }
        DatasetBase::new(records, Array1::from(targets))
    }

    #[test]
    fn recovers_a_separable_ordinal_problem() {
        let dataset = clustered(&[0, 1, 2, 3]);
        let model = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.n_classes(), 4);
        assert_eq!(model.models().len(), 3);
        assert_eq!(model.n_features(), 1);

        let predicted: Array1<usize> = model.predict(&dataset.records);
        assert_eq!(predicted, dataset.targets);
    }

    /// The property the whole design rests on: the level is the number of yes answers, not the
    /// index of the first no.
    #[test]
    fn the_level_is_the_number_of_yes_answers() {
        let dataset = clustered(&[0, 1, 2, 3]);
        let model = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&dataset)
            .unwrap();
        let predicted: Array1<usize> = model.predict(&dataset.records);

        for (index, row) in dataset.records.rows().into_iter().enumerate() {
            let single = row.to_owned().insert_axis(Axis(0));
            let yes = model
                .models()
                .iter()
                .filter(|cut| {
                    let mut answer = Array1::from_elem(1, false);
                    cut.predict_inplace(&single, &mut answer);
                    answer[0]
                })
                .count();
            assert_eq!(predicted[index], yes, "sample {index}");
        }
    }

    #[test]
    fn each_model_answers_whether_the_level_is_above_its_cut() {
        let dataset = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0], [3.0]],
            Array1::from(vec![0usize, 1, 2, 2]),
        );
        let params = FrankHallParams::<_, LinfaError>::new(Recorder::default());
        let model = params.fit(&dataset).unwrap();

        assert_eq!(model.n_classes(), 3);
        let seen = params.base().seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], vec![false, true, true, true]);
        assert_eq!(seen[1], vec![false, false, true, true]);
    }

    /// Independent fits can disagree with the ordering. The count must still yield a valid level
    /// rather than an underflow, a panic or an arbitrary tie-break.
    #[test]
    fn non_monotone_answers_still_produce_a_level() {
        // Cut 0 says "not above level 0" exactly where cut 1 says "above level 1".
        let model = FrankHall {
            models: vec![
                MidpointSplit { cut: 100.0f64 },
                MidpointSplit { cut: -100.0 },
            ],
            n_classes: 3,
            n_features: 1,
        };

        let predicted: Array1<usize> = model.predict(&array![[0.0f64], [200.0], [-200.0]]);
        assert_eq!(predicted[0], 1);
        assert_eq!(predicted[1], 2);
        assert_eq!(predicted[2], 0);
    }

    #[test]
    fn rejects_a_cut_with_an_empty_side() {
        // Level 0 never occurs, so nothing lies below the cut `y > 0`.
        let missing_bottom = DatasetBase::new(
            array![[0.0f64], [1.0], [2.0]],
            Array1::from(vec![1usize, 1, 2]),
        );
        let error = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&missing_bottom)
            .unwrap_err();
        assert!(matches!(error, OrdinalError::InvalidParameter { .. }));
        assert!(error.to_string().contains("level 0"), "{error}");

        // Declaring a top level the data never reaches leaves the cut `y > 2` with no positives.
        let error = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .n_levels(4)
            .fit(&clustered(&[0, 1, 2]))
            .unwrap_err();
        assert!(matches!(error, OrdinalError::InvalidParameter { .. }));
        assert!(error.to_string().contains("level 2"), "{error}");
    }

    /// An absent level between two populated ones keeps every cut well posed, so it must fit.
    #[test]
    fn an_absent_middle_level_is_accepted() {
        let dataset = clustered(&[0, 2]);
        let model = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&dataset)
            .unwrap();

        assert_eq!(model.n_classes(), 3);
        assert_eq!(model.models().len(), 2);
        let predicted: Array1<usize> = model.predict(&dataset.records);
        assert_eq!(predicted, dataset.targets);
    }

    #[test]
    fn infers_the_level_count_and_honours_a_declared_one() {
        let inferred = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&clustered(&[0, 1]))
            .unwrap();
        assert_eq!(inferred.n_classes(), 2);
        assert_eq!(inferred.models().len(), 1);

        let too_few = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .n_levels(2)
            .fit(&clustered(&[0, 1, 2]));
        assert!(matches!(
            too_few,
            Err(OrdinalError::RankOutOfRange { rank: 2, .. })
        ));
    }

    #[test]
    fn rejects_malformed_training_data() {
        let empty = DatasetBase::new(
            Array2::<f64>::zeros((0, 1)),
            Array1::from(Vec::<usize>::new()),
        );
        assert!(matches!(
            FrankHallParams::<_, LinfaError>::new(MidpointSplitParams).fit(&empty),
            Err(OrdinalError::EmptyTrainingSet)
        ));

        let mismatched =
            DatasetBase::new(Array2::<f64>::zeros((3, 1)), Array1::from(vec![0usize, 1]));
        assert!(matches!(
            FrankHallParams::<_, LinfaError>::new(MidpointSplitParams).fit(&mismatched),
            Err(OrdinalError::LengthMismatch {
                records: 3,
                targets: 2
            })
        ));

        let not_finite = DatasetBase::new(array![[f64::NAN], [1.0]], Array1::from(vec![0usize, 1]));
        assert!(matches!(
            FrankHallParams::<_, LinfaError>::new(MidpointSplitParams).fit(&not_finite),
            Err(OrdinalError::NonFinite { .. })
        ));
    }

    /// A base learner that fails must name the cut it failed on; otherwise `K - 1` identical
    /// messages are indistinguishable.
    #[test]
    fn a_failing_base_learner_names_its_cut() {
        #[derive(Debug)]
        struct AlwaysFails;

        impl<F: Float> Fit<Array2<F>, Array1<bool>, LinfaError> for AlwaysFails {
            type Object = ();

            fn fit(
                &self,
                _dataset: &DatasetBase<Array2<F>, Array1<bool>>,
            ) -> std::result::Result<(), LinfaError> {
                Err(LinfaError::NotConverged("stub".to_string()))
            }
        }

        let error = FrankHallParams::<_, LinfaError>::new(AlwaysFails)
            .fit(&clustered(&[0, 1, 2]))
            .unwrap_err();
        assert!(matches!(error, OrdinalError::Linfa(_)));
        assert!(error.to_string().contains("y > 0"), "{error}");
    }

    #[test]
    fn params_are_checked() {
        assert!(matches!(
            FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
                .n_levels(1)
                .check(),
            Err(OrdinalError::TooFewClasses { found: 1 })
        ));
    }

    #[test]
    fn fits_at_f32() {
        let mut records = Array2::<f32>::zeros((30, 1));
        let mut targets = Vec::with_capacity(30);
        for index in 0..30 {
            let level = index / 10;
            records[[index, 0]] = 100.0 * level as f32 + index as f32 * 0.01;
            targets.push(level);
        }
        let dataset = DatasetBase::new(records, Array1::from(targets));

        let model = FrankHallParams::<_, LinfaError>::new(MidpointSplitParams)
            .fit(&dataset)
            .unwrap();
        let predicted: Array1<usize> = model.predict(&dataset.records);
        assert_eq!(predicted, dataset.targets);
    }
}

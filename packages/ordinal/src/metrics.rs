//! Metrics for ordered targets.
//!
//! Plain accuracy is a poor way to judge an ordinal model: predicting "high" when the truth is
//! "medium" is a much smaller mistake than predicting "low", and accuracy scores both as equally
//! wrong. Everything here takes the distance between levels into account.
//!
//! The rank-association metrics ([`kendall_tau_b`], [`spearman_rank_correlation`]) are textbook
//! defined over *pairs* of rows, which reads as an O(n²) double loop — 2·10⁸ pairs at the 20 000
//! row cap the catalog scores against. Because both sides are ranks in `0..K`, they are computed
//! here from the `K × K` contingency table instead: one pass to bin the rows, then a traversal of
//! the level grid, so the cost is O(n + K²) and grows with the *level* count, not the row count.

use crate::error::{OrdinalError, Result};
use ndarray::{ArrayBase, Data, Ix1};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Mean absolute distance between predicted and true level, measured in ranks.
///
/// 0.0 is perfect; 1.0 means being off by one level on average.
pub fn mean_absolute_rank_error(predicted: &[usize], actual: &[usize]) -> Result<f64> {
    check(predicted, actual)?;
    let total: f64 = predicted
        .iter()
        .zip(actual.iter())
        .map(|(p, a)| p.abs_diff(*a) as f64)
        .sum();
    Ok(total / predicted.len() as f64)
}

/// Macro-averaged mean absolute error (MMAE): the per-true-level mean absolute rank error, averaged
/// with equal weight per level.
///
/// [`mean_absolute_rank_error`] averages over rows, so on imbalanced data it reports what the
/// majority level does and stays low even when every rare level is missed. MMAE gives each level
/// one vote, which is why it is the metric that actually moves when a model collapses onto the
/// majority level.
///
/// Levels absent from `actual` are excluded from the average: a level with no instances produced no
/// error, and counting it as 0.0 would reward declaring levels the data never contains.
pub fn macro_mean_absolute_error(
    predicted: &[usize],
    actual: &[usize],
    n_classes: usize,
) -> Result<f64> {
    let table = contingency_table(predicted, actual, n_classes)?;
    let (actual_totals, _) = marginals(&table, n_classes);

    let mut total = 0.0;
    let mut levels_present = 0usize;
    for (actual_level, &level_total) in actual_totals.iter().enumerate() {
        if level_total == 0 {
            continue;
        }
        let row = &table[actual_level * n_classes..(actual_level + 1) * n_classes];
        let errors: f64 = row
            .iter()
            .enumerate()
            .map(|(predicted_level, count)| {
                *count as f64 * actual_level.abs_diff(predicted_level) as f64
            })
            .sum();
        total += errors / level_total as f64;
        levels_present += 1;
    }

    if levels_present == 0 {
        return Err(OrdinalError::EmptyTrainingSet);
    }
    Ok(total / levels_present as f64)
}

/// Share of predictions landing within `tolerance` levels of the truth.
///
/// `tolerance = 0` is exact-match accuracy; `tolerance = 1` is the usual "off by at most one".
pub fn accuracy_within(predicted: &[usize], actual: &[usize], tolerance: usize) -> Result<f64> {
    check(predicted, actual)?;
    let hits = predicted
        .iter()
        .zip(actual.iter())
        .filter(|(p, a)| p.abs_diff(**a) <= tolerance)
        .count();
    Ok(hits as f64 / predicted.len() as f64)
}

/// How fast a weighted kappa's penalty grows with the distance between the true and the predicted
/// level.
///
/// The choice is a statement about the cost structure of the problem, not a tuning knob: use
/// [`Quadratic`](KappaWeighting::Quadratic) when a two-level miss is much worse than twice a
/// one-level miss (the usual assumption, and the standard headline metric), and
/// [`Linear`](KappaWeighting::Linear) when every step along the scale costs the same — grading
/// scales where each level is one unit of loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum KappaWeighting {
    /// Penalty proportional to the level distance.
    Linear,
    /// Penalty proportional to the squared level distance, so near misses are forgiven far more.
    Quadratic,
}

impl KappaWeighting {
    /// Penalty for a `distance`-level disagreement on a scale of `n_classes` levels, normalized so
    /// that agreement costs 0.0 and the widest possible miss costs 1.0.
    ///
    /// The normalization cancels out of kappa's observed/expected ratio; it is kept so the weights
    /// are comparable across scales when inspected directly.
    pub fn weight(self, distance: usize, n_classes: usize) -> f64 {
        let span = n_classes.saturating_sub(1) as f64;
        if span <= 0.0 {
            return 0.0;
        }
        let normalized = distance as f64 / span;
        match self {
            KappaWeighting::Linear => normalized,
            KappaWeighting::Quadratic => normalized * normalized,
        }
    }
}

/// Weighted Cohen's kappa: agreement corrected for what chance alone would achieve, with
/// disagreements penalized according to `weighting`.
///
/// 1.0 is perfect agreement, 0.0 is chance, and negative values mean worse than chance.
///
/// Both weightings share this implementation because they differ only in the penalty curve; the
/// chance correction — expected counts formed from the two marginals under independence — is
/// identical, and getting it right twice is how the two drift apart.
pub fn weighted_kappa(
    predicted: &[usize],
    actual: &[usize],
    n_classes: usize,
    weighting: KappaWeighting,
) -> Result<f64> {
    let table = contingency_table(predicted, actual, n_classes)?;
    let (actual_totals, predicted_totals) = marginals(&table, n_classes);
    let n = predicted.len() as f64;

    let mut numerator_observed = 0.0;
    let mut numerator_expected = 0.0;
    for actual_level in 0..n_classes {
        for predicted_level in 0..n_classes {
            let weight = weighting.weight(actual_level.abs_diff(predicted_level), n_classes);
            numerator_observed += weight * table[actual_level * n_classes + predicted_level] as f64;
            // Expected counts under independence of the two marginals.
            numerator_expected += weight
                * actual_totals[actual_level] as f64
                * predicted_totals[predicted_level] as f64
                / n;
        }
    }

    if numerator_expected == 0.0 {
        // Chance agreement is already perfect (a single level, or both sides constant and equal),
        // so kappa is undefined; report perfect agreement when the two actually match.
        return Ok(if numerator_observed == 0.0 { 1.0 } else { 0.0 });
    }
    Ok(1.0 - numerator_observed / numerator_expected)
}

/// Quadratic weighted kappa: agreement corrected for what chance alone would achieve, with the
/// penalty growing quadratically in the level distance.
///
/// 1.0 is perfect agreement, 0.0 is chance, and negative values mean worse than chance. This is the
/// standard headline metric for ordinal problems.
pub fn quadratic_weighted_kappa(
    predicted: &[usize],
    actual: &[usize],
    n_classes: usize,
) -> Result<f64> {
    weighted_kappa(predicted, actual, n_classes, KappaWeighting::Quadratic)
}

/// Linear weighted kappa: as [`quadratic_weighted_kappa`], but every step along the scale costs the
/// same.
///
/// Quadratic weighting credits a near miss at a quarter of the cost of a two-level miss; when that
/// is not the real cost structure it flatters models that hover next to the truth. Reach for this
/// one when a level is a level.
pub fn linear_weighted_kappa(
    predicted: &[usize],
    actual: &[usize],
    n_classes: usize,
) -> Result<f64> {
    weighted_kappa(predicted, actual, n_classes, KappaWeighting::Linear)
}

/// Kendall's tau-b: rank association between predictions and truth, corrected for ties.
///
/// +1.0 means the prediction orders the rows exactly as the truth does, -1.0 exactly backwards, and
/// 0.0 no association. Unlike kappa this ignores calibration entirely — a model whose levels are all
/// shifted by one still scores 1.0 — which makes it the metric to consult when the question is
/// "does the model get the *order* right".
///
/// tau-b, not tau-a: with `K` levels over `n` rows almost every pair is tied on one side or the
/// other, and the uncorrected coefficient cannot reach 1.0 on tied data at all.
///
/// ```text
/// tau_b = (C - D) / sqrt((n0 - n1) * (n0 - n2))
/// n0 = n(n-1)/2,  n1 = sum of t(t-1)/2 over tied groups in one side,  n2 = likewise for the other
/// ```
///
/// Concordant and discordant pairs are read off the contingency table — a cell's count times the
/// total of all cells strictly below-and-right is concordant, below-and-left discordant — so the
/// cost is O(n + K²) rather than the O(n²) the pairwise definition suggests.
///
/// Returns 0.0 when either side is constant (or there is a single row): the denominator vanishes
/// because a constant carries no ordering to agree with, and "no association measurable" is the
/// honest reading rather than an error, matching how kappa treats its own degenerate case.
pub fn kendall_tau_b(predicted: &[usize], actual: &[usize], n_classes: usize) -> Result<f64> {
    let table = contingency_table(predicted, actual, n_classes)?;
    let (actual_totals, predicted_totals) = marginals(&table, n_classes);

    // Column sums over the rows already visited, i.e. over the strictly higher actual levels.
    // Sweeping upward keeps the pair counting at O(K²) time and O(K) scratch.
    let mut higher_and_lower = vec![0u64; n_classes];
    let mut higher_and_higher = vec![0u64; n_classes];
    let mut concordant = 0u64;
    let mut discordant = 0u64;

    for actual_level in (0..n_classes).rev() {
        let row = &table[actual_level * n_classes..(actual_level + 1) * n_classes];
        for (predicted_level, count) in row.iter().enumerate() {
            concordant += count * higher_and_higher[predicted_level];
            discordant += count * higher_and_lower[predicted_level];
        }

        let mut lower = 0u64;
        for (predicted_level, count) in row.iter().enumerate() {
            higher_and_lower[predicted_level] += lower;
            lower += count;
        }
        let mut higher = 0u64;
        for (predicted_level, count) in row.iter().enumerate().rev() {
            higher_and_higher[predicted_level] += higher;
            higher += count;
        }
    }

    let all_pairs = pair_count(predicted.len() as u64);
    let actual_ties: u64 = actual_totals.iter().copied().map(pair_count).sum();
    let predicted_ties: u64 = predicted_totals.iter().copied().map(pair_count).sum();
    let untied_actual = all_pairs.saturating_sub(actual_ties) as f64;
    let untied_predicted = all_pairs.saturating_sub(predicted_ties) as f64;

    let denominator = (untied_actual * untied_predicted).sqrt();
    if denominator <= 0.0 || !denominator.is_finite() {
        return Ok(0.0);
    }
    let tau = (concordant as f64 - discordant as f64) / denominator;
    Ok(tau.clamp(-1.0, 1.0))
}

/// Spearman rank correlation between predictions and truth.
///
/// The familiar `1 - 6·Σd²/(n(n²-1))` shortcut is only valid when no two values are tied. Ordinal
/// data is nothing but ties — `K` levels spread over `n` rows — so that form is silently wrong on
/// every input this crate produces. This computes Pearson correlation on *midranks* instead, which
/// is the definition the shortcut is derived from.
///
/// Like [`kendall_tau_b`] this measures ordering rather than calibration, and costs O(n + K²): the
/// midrank of a level follows from its count alone, so the correlation is accumulated over the
/// contingency table.
///
/// Returns 0.0 when either side is constant, where the variance — and so the correlation — is
/// undefined.
pub fn spearman_rank_correlation(
    predicted: &[usize],
    actual: &[usize],
    n_classes: usize,
) -> Result<f64> {
    let table = contingency_table(predicted, actual, n_classes)?;
    let (actual_totals, predicted_totals) = marginals(&table, n_classes);
    let n = predicted.len() as f64;

    let actual_midranks = midranks(&actual_totals);
    let predicted_midranks = midranks(&predicted_totals);
    let actual_mean = weighted_mean(&actual_totals, &actual_midranks, n);
    let predicted_mean = weighted_mean(&predicted_totals, &predicted_midranks, n);

    let denominator = (weighted_variance(&actual_totals, &actual_midranks, actual_mean)
        * weighted_variance(&predicted_totals, &predicted_midranks, predicted_mean))
    .sqrt();
    if denominator <= 0.0 || !denominator.is_finite() {
        return Ok(0.0);
    }

    let mut covariance = 0.0;
    for actual_level in 0..n_classes {
        for predicted_level in 0..n_classes {
            let count = table[actual_level * n_classes + predicted_level] as f64;
            covariance += count
                * (actual_midranks[actual_level] - actual_mean)
                * (predicted_midranks[predicted_level] - predicted_mean);
        }
    }
    Ok((covariance / denominator).clamp(-1.0, 1.0))
}

/// Ordinal metrics as an extension trait, mirroring how linfa exposes its own metric families
/// (`ToConfusionMatrix`, `SilhouetteScore`, …) as traits over the target container rather than as
/// loose functions.
///
/// ```no_run
/// use flow_like_ordinal::OrdinalMetrics;
/// # let predicted: ndarray::Array1<usize> = todo!();
/// # let actual: ndarray::Array1<usize> = todo!();
/// let kappa = predicted.quadratic_weighted_kappa(&actual, 5).unwrap();
/// let tau = predicted.kendall_tau_b(&actual, 5).unwrap();
/// let balanced = predicted.macro_mean_absolute_error(&actual, 5).unwrap();
/// ```
pub trait OrdinalMetrics {
    /// Quadratic weighted kappa against `actual`.
    fn quadratic_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64>;
    /// Linear weighted kappa against `actual`.
    fn linear_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64>;
    /// Weighted kappa against `actual` under an explicit penalty curve.
    fn weighted_kappa(
        &self,
        actual: &Self,
        n_classes: usize,
        weighting: KappaWeighting,
    ) -> Result<f64>;
    /// Mean absolute rank distance from `actual`.
    fn mean_absolute_rank_error(&self, actual: &Self) -> Result<f64>;
    /// Mean absolute rank distance from `actual`, averaged per true level.
    fn macro_mean_absolute_error(&self, actual: &Self, n_classes: usize) -> Result<f64>;
    /// Share of predictions within `tolerance` levels of `actual`.
    fn accuracy_within(&self, actual: &Self, tolerance: usize) -> Result<f64>;
    /// Tie-corrected Kendall rank association with `actual`.
    fn kendall_tau_b(&self, actual: &Self, n_classes: usize) -> Result<f64>;
    /// Spearman rank correlation with `actual`, on midranks.
    fn spearman_rank_correlation(&self, actual: &Self, n_classes: usize) -> Result<f64>;
}

impl OrdinalMetrics for [usize] {
    fn quadratic_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        quadratic_weighted_kappa(self, actual, n_classes)
    }

    fn linear_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        linear_weighted_kappa(self, actual, n_classes)
    }

    fn weighted_kappa(
        &self,
        actual: &Self,
        n_classes: usize,
        weighting: KappaWeighting,
    ) -> Result<f64> {
        weighted_kappa(self, actual, n_classes, weighting)
    }

    fn mean_absolute_rank_error(&self, actual: &Self) -> Result<f64> {
        mean_absolute_rank_error(self, actual)
    }

    fn macro_mean_absolute_error(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        macro_mean_absolute_error(self, actual, n_classes)
    }

    fn accuracy_within(&self, actual: &Self, tolerance: usize) -> Result<f64> {
        accuracy_within(self, actual, tolerance)
    }

    fn kendall_tau_b(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        kendall_tau_b(self, actual, n_classes)
    }

    fn spearman_rank_correlation(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        spearman_rank_correlation(self, actual, n_classes)
    }
}

/// Borrows the ranks as a slice, copying only when the array is not contiguous.
fn as_ranks<S: Data<Elem = usize>>(array: &ArrayBase<S, Ix1>) -> Cow<'_, [usize]> {
    match array.as_slice() {
        Some(slice) => Cow::Borrowed(slice),
        None => Cow::Owned(array.iter().copied().collect()),
    }
}

impl<S: Data<Elem = usize>> OrdinalMetrics for ArrayBase<S, Ix1> {
    fn quadratic_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        quadratic_weighted_kappa(&as_ranks(self), &as_ranks(actual), n_classes)
    }

    fn linear_weighted_kappa(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        linear_weighted_kappa(&as_ranks(self), &as_ranks(actual), n_classes)
    }

    fn weighted_kappa(
        &self,
        actual: &Self,
        n_classes: usize,
        weighting: KappaWeighting,
    ) -> Result<f64> {
        weighted_kappa(&as_ranks(self), &as_ranks(actual), n_classes, weighting)
    }

    fn mean_absolute_rank_error(&self, actual: &Self) -> Result<f64> {
        mean_absolute_rank_error(&as_ranks(self), &as_ranks(actual))
    }

    fn macro_mean_absolute_error(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        macro_mean_absolute_error(&as_ranks(self), &as_ranks(actual), n_classes)
    }

    fn accuracy_within(&self, actual: &Self, tolerance: usize) -> Result<f64> {
        accuracy_within(&as_ranks(self), &as_ranks(actual), tolerance)
    }

    fn kendall_tau_b(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        kendall_tau_b(&as_ranks(self), &as_ranks(actual), n_classes)
    }

    fn spearman_rank_correlation(&self, actual: &Self, n_classes: usize) -> Result<f64> {
        spearman_rank_correlation(&as_ranks(self), &as_ranks(actual), n_classes)
    }
}

/// Counts of every `(actual, predicted)` level pair, row-major with the ACTUAL level as the row.
///
/// Every level-aware metric here is a function of this table, so summarizing the rows once is what
/// keeps their cost tied to the level count instead of the row count. Validation lives here too, so
/// the metrics agree on which inputs they reject.
fn contingency_table(predicted: &[usize], actual: &[usize], n_classes: usize) -> Result<Vec<u64>> {
    check(predicted, actual)?;
    if n_classes < 2 {
        return Err(OrdinalError::TooFewClasses { found: n_classes });
    }
    if let Some(&bad) = predicted
        .iter()
        .chain(actual.iter())
        .find(|v| **v >= n_classes)
    {
        return Err(OrdinalError::RankOutOfRange {
            rank: bad,
            n_classes,
        });
    }

    let mut table = vec![0u64; n_classes * n_classes];
    for (p, a) in predicted.iter().zip(actual.iter()) {
        table[a * n_classes + p] += 1;
    }
    Ok(table)
}

/// Row totals (per actual level) and column totals (per predicted level) of a contingency table.
fn marginals(table: &[u64], n_classes: usize) -> (Vec<u64>, Vec<u64>) {
    let mut actual_totals = vec![0u64; n_classes];
    let mut predicted_totals = vec![0u64; n_classes];
    for (actual_level, row) in table.chunks_exact(n_classes).enumerate() {
        for (predicted_level, count) in row.iter().enumerate() {
            actual_totals[actual_level] += count;
            predicted_totals[predicted_level] += count;
        }
    }
    (actual_totals, predicted_totals)
}

/// Midrank of each level: the average 1-based position its rows occupy once the whole column is
/// sorted, which is the tie-aware replacement for a plain rank.
///
/// An absent level yields a placeholder that is only ever multiplied by its own zero count.
fn midranks(counts: &[u64]) -> Vec<f64> {
    let mut consumed = 0u64;
    counts
        .iter()
        .map(|count| {
            let midrank = consumed as f64 + (*count as f64 + 1.0) / 2.0;
            consumed += count;
            midrank
        })
        .collect()
}

/// Mean of per-level `values`, each weighted by how many rows sit in that level.
fn weighted_mean(counts: &[u64], values: &[f64], n: f64) -> f64 {
    counts
        .iter()
        .zip(values.iter())
        .map(|(count, value)| *count as f64 * value)
        .sum::<f64>()
        / n
}

/// Count-weighted sum of squared deviations of per-level `values` from `center`.
fn weighted_variance(counts: &[u64], values: &[f64], center: f64) -> f64 {
    counts
        .iter()
        .zip(values.iter())
        .map(|(count, value)| *count as f64 * (value - center) * (value - center))
        .sum::<f64>()
}

/// Unordered pairs inside a group of `count` items.
fn pair_count(count: u64) -> u64 {
    count * count.saturating_sub(1) / 2
}

fn check(predicted: &[usize], actual: &[usize]) -> Result<()> {
    if predicted.len() != actual.len() {
        return Err(OrdinalError::LengthMismatch {
            records: predicted.len(),
            targets: actual.len(),
        });
    }
    if predicted.is_empty() {
        return Err(OrdinalError::EmptyTrainingSet);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{array, s};

    #[test]
    fn perfect_agreement_scores_one() {
        let values = [0, 1, 2, 3, 2, 1];
        assert!((quadratic_weighted_kappa(&values, &values, 4).unwrap() - 1.0).abs() < 1e-12);
        assert!((linear_weighted_kappa(&values, &values, 4).unwrap() - 1.0).abs() < 1e-12);
        assert!((kendall_tau_b(&values, &values, 4).unwrap() - 1.0).abs() < 1e-12);
        assert!((spearman_rank_correlation(&values, &values, 4).unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(mean_absolute_rank_error(&values, &values).unwrap(), 0.0);
        assert_eq!(macro_mean_absolute_error(&values, &values, 4).unwrap(), 0.0);
        assert_eq!(accuracy_within(&values, &values, 0).unwrap(), 1.0);
    }

    #[test]
    fn near_misses_beat_far_misses() {
        let actual = [0, 1, 2, 3];
        let near = [1, 2, 3, 2];
        let far = [3, 3, 0, 0];
        let near_kappa = quadratic_weighted_kappa(&near, &actual, 4).unwrap();
        let far_kappa = quadratic_weighted_kappa(&far, &actual, 4).unwrap();
        assert!(
            near_kappa > far_kappa,
            "near {near_kappa} should beat far {far_kappa}"
        );
        assert!(
            mean_absolute_rank_error(&near, &actual).unwrap()
                < mean_absolute_rank_error(&far, &actual).unwrap()
        );
    }

    #[test]
    fn tolerance_widens_accuracy() {
        let actual = [0, 1, 2, 3];
        let predicted = [1, 1, 3, 3];
        assert_eq!(accuracy_within(&predicted, &actual, 0).unwrap(), 0.5);
        assert_eq!(accuracy_within(&predicted, &actual, 1).unwrap(), 1.0);
    }

    #[test]
    fn rejects_mismatched_lengths_and_bad_ranks() {
        assert!(matches!(
            quadratic_weighted_kappa(&[0, 1], &[0], 2),
            Err(OrdinalError::LengthMismatch { .. })
        ));
        assert!(matches!(
            quadratic_weighted_kappa(&[0, 9], &[0, 1], 2),
            Err(OrdinalError::RankOutOfRange { .. })
        ));
        assert!(matches!(
            kendall_tau_b(&[0, 1], &[0], 2),
            Err(OrdinalError::LengthMismatch { .. })
        ));
        assert!(matches!(
            spearman_rank_correlation(&[0, 9], &[0, 1], 2),
            Err(OrdinalError::RankOutOfRange { .. })
        ));
        assert!(matches!(
            macro_mean_absolute_error(&[0, 1], &[0, 1], 1),
            Err(OrdinalError::TooFewClasses { .. })
        ));
        assert!(matches!(
            linear_weighted_kappa(&[], &[], 3),
            Err(OrdinalError::EmptyTrainingSet)
        ));
    }

    /// A constant prediction carries no information, so kappa must not reward it.
    #[test]
    fn constant_prediction_scores_at_or_below_chance() {
        let actual = [0, 1, 2, 3, 0, 1, 2, 3];
        let constant = [2, 2, 2, 2, 2, 2, 2, 2];
        let kappa = quadratic_weighted_kappa(&constant, &actual, 4).unwrap();
        assert!(kappa <= 1e-12, "constant prediction scored {kappa}");
        let linear = linear_weighted_kappa(&constant, &actual, 4).unwrap();
        assert!(linear <= 1e-12, "constant prediction scored {linear}");
    }

    #[test]
    fn kappa_weights_are_normalized_to_the_widest_miss() {
        assert_eq!(KappaWeighting::Linear.weight(0, 5), 0.0);
        assert_eq!(KappaWeighting::Quadratic.weight(0, 5), 0.0);
        assert_eq!(KappaWeighting::Linear.weight(4, 5), 1.0);
        assert_eq!(KappaWeighting::Quadratic.weight(4, 5), 1.0);
        assert_eq!(KappaWeighting::Linear.weight(1, 5), 0.25);
        assert_eq!(KappaWeighting::Quadratic.weight(1, 5), 0.0625);
        // A degenerate scale has no distance to normalize by; kappa rejects it before this point.
        assert_eq!(KappaWeighting::Quadratic.weight(1, 1), 0.0);
    }

    /// The whole reason both weightings exist: quadratic forgives a systematic one-level shift that
    /// linear treats as a full step of error.
    #[test]
    fn quadratic_forgives_near_misses_that_linear_charges_for() {
        let actual: Vec<usize> = (0..20).map(|index| index % 5).collect();
        let shifted: Vec<usize> = actual.iter().map(|rank| (rank + 1).min(4)).collect();

        let quadratic = quadratic_weighted_kappa(&shifted, &actual, 5).unwrap();
        let linear = linear_weighted_kappa(&shifted, &actual, 5).unwrap();
        assert!((quadratic - 0.8).abs() < 1e-9, "quadratic {quadratic}");
        assert!((linear - 0.5).abs() < 1e-9, "linear {linear}");
        assert!(quadratic > linear);
    }

    /// …and the mirror image: when the misses are far, quadratic punishes harder than linear.
    #[test]
    fn quadratic_punishes_far_misses_harder_than_linear() {
        let actual: Vec<usize> = (0..20).map(|index| index % 5).collect();
        let inverted: Vec<usize> = actual
            .iter()
            .map(|rank| if *rank < 2 { 4 } else { 0 })
            .collect();

        let quadratic = quadratic_weighted_kappa(&inverted, &actual, 5).unwrap();
        let linear = linear_weighted_kappa(&inverted, &actual, 5).unwrap();
        assert!((quadratic + 0.8).abs() < 1e-9, "quadratic {quadratic}");
        assert!((linear + 0.6).abs() < 1e-9, "linear {linear}");
        assert!(quadratic < linear);
    }

    /// Hand-worked: 5 rows, ties on both sides.
    /// C = 6, D = 0, n0 = 10, n1 = n2 = 2 => 6 / sqrt(8 * 8) = 0.75.
    #[test]
    fn kendall_tau_b_matches_a_hand_worked_example_with_ties() {
        let actual = [0, 0, 1, 1, 2];
        let predicted = [0, 1, 1, 2, 2];
        let tau = kendall_tau_b(&predicted, &actual, 3).unwrap();
        assert!((tau - 0.75).abs() < 1e-12, "tau {tau}");
    }

    #[test]
    fn kendall_tau_b_is_minus_one_when_reversed_and_zero_when_constant() {
        let actual = [0, 1, 2, 3];
        let reversed = [3, 2, 1, 0];
        assert!((kendall_tau_b(&reversed, &actual, 4).unwrap() + 1.0).abs() < 1e-12);

        let constant = [1, 1, 1, 1];
        assert_eq!(kendall_tau_b(&constant, &actual, 4).unwrap(), 0.0);
        assert_eq!(kendall_tau_b(&actual, &constant, 4).unwrap(), 0.0);
        // A single row has no pairs at all.
        assert_eq!(kendall_tau_b(&[1], &[0], 4).unwrap(), 0.0);
    }

    /// tau-b scores ordering, not calibration: a uniform shift keeps the ranking intact.
    #[test]
    fn kendall_tau_b_ignores_a_uniform_shift() {
        let actual = [0, 1, 2, 3, 1, 2];
        let shifted: Vec<usize> = actual.iter().map(|rank| rank + 1).collect();
        assert!((kendall_tau_b(&shifted, &actual, 5).unwrap() - 1.0).abs() < 1e-12);
        // …while kappa, which does score calibration, does not.
        assert!(quadratic_weighted_kappa(&shifted, &actual, 5).unwrap() < 1.0);
    }

    /// Midranks vs the no-ties shortcut. Hand-worked: midranks are [1.5, 1.5, 3, 4] against
    /// [1, 2.5, 2.5, 4], giving Pearson 5/6. The `1 - 6*sum(d^2)/(n(n^2-1))` shortcut would report
    /// 0.85 here — close enough to look right, wrong enough to matter.
    #[test]
    fn spearman_uses_midranks_not_the_no_ties_shortcut() {
        let actual = [0, 0, 1, 2];
        let predicted = [0, 1, 1, 2];
        let rho = spearman_rank_correlation(&predicted, &actual, 3).unwrap();
        assert!((rho - 5.0 / 6.0).abs() < 1e-12, "rho {rho}");
        assert!(
            (rho - 0.85).abs() > 1e-3,
            "rho {rho} matched the tie-blind shortcut"
        );
    }

    #[test]
    fn spearman_is_minus_one_when_reversed_and_zero_when_constant() {
        let actual = [0, 1, 2, 3];
        let reversed = [3, 2, 1, 0];
        assert!((spearman_rank_correlation(&reversed, &actual, 4).unwrap() + 1.0).abs() < 1e-12);

        let constant = [2, 2, 2, 2];
        assert_eq!(
            spearman_rank_correlation(&constant, &actual, 4).unwrap(),
            0.0
        );
        assert_eq!(
            spearman_rank_correlation(&actual, &constant, 4).unwrap(),
            0.0
        );
    }

    /// The point of MMAE: nine easy rows must not bury the one rare level the model always misses.
    #[test]
    fn macro_mae_diverges_from_plain_mae_under_imbalance() {
        let mut actual = vec![0usize; 9];
        actual.push(1);
        let predicted = vec![0usize; 10];

        let plain = mean_absolute_rank_error(&predicted, &actual).unwrap();
        let macro_averaged = macro_mean_absolute_error(&predicted, &actual, 2).unwrap();
        assert!((plain - 0.1).abs() < 1e-12, "plain {plain}");
        assert!(
            (macro_averaged - 0.5).abs() < 1e-12,
            "macro {macro_averaged}"
        );
        assert!(macro_averaged > plain);
    }

    /// With equal counts per level the two coincide, which is what pins the weighting rather than
    /// some other rescaling of the same errors.
    #[test]
    fn macro_mae_equals_plain_mae_when_levels_are_balanced() {
        let actual = [0, 0, 1, 1, 2, 2];
        let predicted = [0, 1, 1, 2, 2, 2];
        let plain = mean_absolute_rank_error(&predicted, &actual).unwrap();
        let macro_averaged = macro_mean_absolute_error(&predicted, &actual, 3).unwrap();
        assert!((plain - 1.0 / 3.0).abs() < 1e-12, "plain {plain}");
        assert!(
            (macro_averaged - plain).abs() < 1e-12,
            "macro {macro_averaged}"
        );
    }

    /// A level nobody was actually in produced no error, so widening the declared scale must not
    /// dilute the score toward zero.
    #[test]
    fn macro_mae_skips_levels_absent_from_the_actuals() {
        let mut actual = vec![0usize; 9];
        actual.push(1);
        let predicted = vec![0usize; 10];
        let two_levels = macro_mean_absolute_error(&predicted, &actual, 2).unwrap();
        let five_levels = macro_mean_absolute_error(&predicted, &actual, 5).unwrap();
        assert!((two_levels - five_levels).abs() < 1e-12);
    }

    #[test]
    fn trait_methods_match_the_free_functions_on_arrays() {
        let actual = array![0usize, 0, 1, 2];
        let predicted = array![0usize, 1, 1, 2];

        assert_eq!(
            predicted.spearman_rank_correlation(&actual, 3).unwrap(),
            spearman_rank_correlation(&[0, 1, 1, 2], &[0, 0, 1, 2], 3).unwrap()
        );
        assert_eq!(
            predicted.kendall_tau_b(&actual, 3).unwrap(),
            kendall_tau_b(&[0, 1, 1, 2], &[0, 0, 1, 2], 3).unwrap()
        );
        assert_eq!(
            predicted.linear_weighted_kappa(&actual, 3).unwrap(),
            predicted
                .weighted_kappa(&actual, 3, KappaWeighting::Linear)
                .unwrap()
        );
        assert_eq!(
            predicted.macro_mean_absolute_error(&actual, 3).unwrap(),
            macro_mean_absolute_error(&[0, 1, 1, 2], &[0, 0, 1, 2], 3).unwrap()
        );
    }

    /// Non-contiguous views take the copying path in `as_ranks`, and must score identically.
    #[test]
    fn strided_views_score_like_contiguous_copies() {
        let actual = array![0usize, 9, 1, 9, 2, 9, 3, 9];
        let predicted = array![0usize, 9, 1, 9, 3, 9, 2, 9];
        let actual_view = actual.slice(s![..;2]);
        let predicted_view = predicted.slice(s![..;2]);
        assert!(actual_view.as_slice().is_none());

        assert_eq!(
            predicted_view.kendall_tau_b(&actual_view, 4).unwrap(),
            kendall_tau_b(&[0, 1, 3, 2], &[0, 1, 2, 3], 4).unwrap()
        );
        assert_eq!(
            predicted_view
                .macro_mean_absolute_error(&actual_view, 4)
                .unwrap(),
            macro_mean_absolute_error(&[0, 1, 3, 2], &[0, 1, 2, 3], 4).unwrap()
        );
    }
}

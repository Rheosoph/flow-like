use crate::{Rng, validate_matrix};
use flow_like_ml_core::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct IsolationForestConfig {
    pub trees: usize,
    pub sample_size: usize,
    pub seed: u64,
}
impl Default for IsolationForestConfig {
    fn default() -> Self {
        Self {
            trees: 100,
            sample_size: 256,
            seed: 42,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
enum IsolationNode {
    Leaf {
        count: usize,
    },
    Split {
        feature: usize,
        threshold: f64,
        left: Box<Self>,
        right: Box<Self>,
    },
}
impl IsolationNode {
    fn path(&self, row: &[f64]) -> Result<f64> {
        let mut node = self;
        for depth in 0..=64 {
            match node {
                Self::Leaf { count } => {
                    require(*count <= 65536, "Invalid isolation leaf count")?;
                    return Ok(depth as f64 + isolation_c(*count));
                }
                Self::Split {
                    feature,
                    threshold,
                    left,
                    right,
                } => {
                    require(
                        *feature < row.len() && threshold.is_finite(),
                        "Invalid isolation split",
                    )?;
                    node = if row[*feature] < *threshold {
                        left
                    } else {
                        right
                    };
                }
            }
        }
        Err(flow_like_ml_core::Error::Invalid(
            "Isolation tree exceeds depth limit".into(),
        ))
    }
}
fn isolation_c(n: usize) -> f64 {
    if n <= 1 {
        0.0
    } else {
        2.0 * (1..n).map(|i| 1.0 / i as f64).sum::<f64>() - 2.0 * (n - 1) as f64 / n as f64
    }
}
fn isolation_tree(
    rows: &[Vec<f64>],
    indices: &[usize],
    depth: usize,
    limit: usize,
    rng: &mut Rng,
) -> IsolationNode {
    if indices.len() <= 1 || depth >= limit {
        return IsolationNode::Leaf {
            count: indices.len(),
        };
    }
    let ranges: Vec<_> = (0..rows[0].len())
        .filter_map(|feature| {
            let min = indices
                .iter()
                .map(|i| rows[*i][feature])
                .fold(f64::INFINITY, f64::min);
            let max = indices
                .iter()
                .map(|i| rows[*i][feature])
                .fold(f64::NEG_INFINITY, f64::max);
            (max > min).then_some((feature, min, max))
        })
        .collect();
    if ranges.is_empty() {
        return IsolationNode::Leaf {
            count: indices.len(),
        };
    }
    let (feature, min, max) = ranges[rng.index(ranges.len())];
    let threshold = min + (max - min) * (1.0 - rng.uniform());
    let (left, right): (Vec<_>, Vec<_>) = indices
        .iter()
        .copied()
        .partition(|i| rows[*i][feature] < threshold);
    if left.is_empty() || right.is_empty() {
        return IsolationNode::Leaf {
            count: indices.len(),
        };
    }
    IsolationNode::Split {
        feature,
        threshold,
        left: Box::new(isolation_tree(rows, &left, depth + 1, limit, rng)),
        right: Box::new(isolation_tree(rows, &right, depth + 1, limit, rng)),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct IsolationForest {
    width: usize,
    sample_size: usize,
    trees: Vec<IsolationNode>,
}
impl IsolationForest {
    pub fn fit(rows: &[Vec<f64>], config: &IsolationForestConfig) -> Result<Self> {
        Self::fit_with_progress(rows, config, |_| Ok(()))
    }
    pub fn fit_with_progress(
        rows: &[Vec<f64>],
        config: &IsolationForestConfig,
        mut progress: impl FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        let width = validate_matrix(rows)?;
        require(
            rows.len() >= 2
                && (1..=10000).contains(&config.trees)
                && (2..=65536).contains(&config.sample_size),
            "Isolation Forest requires two rows, 1..10000 trees and sample size 2..65536",
        )?;
        let size = config.sample_size.min(rows.len());
        let limit = (size as f64).log2().ceil() as usize;
        let mut rng = Rng::new(config.seed);
        let mut trees = Vec::with_capacity(config.trees);
        for i in 0..config.trees {
            progress(i)?;
            let mut indices: Vec<_> = (0..rows.len()).collect();
            rng.shuffle(&mut indices);
            indices.truncate(size);
            trees.push(isolation_tree(rows, &indices, 0, limit, &mut rng));
        }
        progress(config.trees)?;
        Ok(Self {
            width,
            sample_size: size,
            trees,
        })
    }
    pub fn score(&self, row: &[f64]) -> Result<f64> {
        require(
            row.len() == self.width
                && row.iter().all(|v| v.is_finite())
                && !self.trees.is_empty()
                && (2..=65536).contains(&self.sample_size),
            "Isolation Forest input or model is invalid",
        )?;
        let path = self
            .trees
            .iter()
            .map(|t| t.path(row))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum::<f64>()
            / self.trees.len() as f64;
        Ok(2.0f64.powf(-path / isolation_c(self.sample_size)))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoostObjective {
    SquaredError,
    BinaryLogLoss,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct BoostConfig {
    pub objective: BoostObjective,
    pub estimators: usize,
    pub max_depth: usize,
    pub max_bins: usize,
    pub min_leaf: usize,
    pub learning_rate: f64,
    pub l2: f64,
}
impl Default for BoostConfig {
    fn default() -> Self {
        Self {
            objective: BoostObjective::SquaredError,
            estimators: 100,
            max_depth: 3,
            max_bins: 32,
            min_leaf: 2,
            learning_rate: 0.1,
            l2: 1.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
enum RegressionTree {
    Leaf(f64),
    Split {
        feature: usize,
        threshold: f64,
        left: Box<Self>,
        right: Box<Self>,
    },
}
impl RegressionTree {
    fn predict(&self, row: &[f64]) -> Result<f64> {
        let mut node = self;
        for _ in 0..=32 {
            match node {
                Self::Leaf(value) => {
                    require(value.is_finite(), "Invalid boosting leaf")?;
                    return Ok(*value);
                }
                Self::Split {
                    feature,
                    threshold,
                    left,
                    right,
                } => {
                    require(
                        *feature < row.len() && threshold.is_finite(),
                        "Invalid boosting split",
                    )?;
                    node = if row[*feature] <= *threshold {
                        left
                    } else {
                        right
                    };
                }
            }
        }
        Err(flow_like_ml_core::Error::Invalid(
            "Boosting tree exceeds depth limit".into(),
        ))
    }
}
fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

fn boost_tree(
    rows: &[Vec<f64>],
    indices: &[usize],
    gradients: &[f64],
    hessians: &[f64],
    thresholds: &[Vec<f64>],
    depth: usize,
    config: &BoostConfig,
) -> RegressionTree {
    let g = indices.iter().map(|i| gradients[*i]).sum::<f64>();
    let h = indices.iter().map(|i| hessians[*i]).sum::<f64>();
    let leaf = RegressionTree::Leaf(g / (h + config.l2));
    if depth >= config.max_depth || indices.len() < 2 * config.min_leaf {
        return leaf;
    }
    let mut best: Option<(f64, usize, f64)> = None;
    for (feature, cuts) in thresholds.iter().enumerate() {
        let mut gs = vec![0.0; cuts.len() + 1];
        let mut hs = vec![0.0; cuts.len() + 1];
        let mut counts = vec![0usize; cuts.len() + 1];
        for i in indices {
            let bin = cuts.partition_point(|c| rows[*i][feature] > *c);
            gs[bin] += gradients[*i];
            hs[bin] += hessians[*i];
            counts[bin] += 1;
        }
        let (mut lg, mut lh, mut lc) = (0.0, 0.0, 0usize);
        for (bin, cut) in cuts.iter().enumerate() {
            lg += gs[bin];
            lh += hs[bin];
            lc += counts[bin];
            if lc < config.min_leaf || indices.len() - lc < config.min_leaf {
                continue;
            }
            let gain = lg * lg / (lh + config.l2) + (g - lg).powi(2) / (h - lh + config.l2)
                - g * g / (h + config.l2);
            if gain > 1e-12 && best.as_ref().is_none_or(|b| gain > b.0) {
                best = Some((gain, feature, *cut));
            }
        }
    }
    if let Some((_, feature, threshold)) = best {
        let (left, right): (Vec<_>, Vec<_>) = indices
            .iter()
            .copied()
            .partition(|i| rows[*i][feature] <= threshold);
        RegressionTree::Split {
            feature,
            threshold,
            left: Box::new(boost_tree(
                rows,
                &left,
                gradients,
                hessians,
                thresholds,
                depth + 1,
                config,
            )),
            right: Box::new(boost_tree(
                rows,
                &right,
                gradients,
                hessians,
                thresholds,
                depth + 1,
                config,
            )),
        }
    } else {
        leaf
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HistogramGradientBoosting {
    width: usize,
    objective: BoostObjective,
    bias: f64,
    learning_rate: f64,
    trees: Vec<RegressionTree>,
}
impl HistogramGradientBoosting {
    pub fn fit(rows: &[Vec<f64>], targets: &[f64], config: &BoostConfig) -> Result<Self> {
        Self::fit_with_progress(rows, targets, config, |_| Ok(()))
    }
    pub fn fit_with_progress(
        rows: &[Vec<f64>],
        targets: &[f64],
        config: &BoostConfig,
        mut progress: impl FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        let width = validate_matrix(rows)?;
        require(
            targets.len() == rows.len() && targets.iter().all(|v| v.is_finite()),
            "Targets must be finite with one value per row",
        )?;
        require(
            (1..=10000).contains(&config.estimators)
                && (1..=16).contains(&config.max_depth)
                && (2..=256).contains(&config.max_bins)
                && config.min_leaf > 0
                && config.min_leaf <= rows.len(),
            "Invalid histogram boosting tree configuration",
        )?;
        require(
            config.learning_rate.is_finite()
                && config.learning_rate > 0.0
                && config.learning_rate <= 1.0
                && config.l2.is_finite()
                && config.l2 > 0.0,
            "Learning rate must be in (0,1] and L2 must be positive",
        )?;
        let average = targets.iter().sum::<f64>() / targets.len() as f64;
        let bias = if config.objective == BoostObjective::BinaryLogLoss {
            require(
                targets.iter().all(|v| *v == 0.0 || *v == 1.0) && average > 0.0 && average < 1.0,
                "Binary boosting requires both classes encoded as 0 and 1",
            )?;
            (average / (1.0 - average)).ln()
        } else {
            average
        };
        let thresholds: Vec<Vec<f64>> = (0..width)
            .map(|feature| {
                let mut values: Vec<_> = rows.iter().map(|r| r[feature]).collect();
                values.sort_by(f64::total_cmp);
                values.dedup();
                let mut cuts = Vec::new();
                for bin in 1..config.max_bins {
                    let index = bin * values.len() / config.max_bins;
                    if index > 0 && index < values.len() {
                        let cut = values[index - 1];
                        if cuts.last().is_none_or(|v| *v != cut) {
                            cuts.push(cut);
                        }
                    }
                }
                cuts
            })
            .collect();
        let mut predictions = vec![bias; rows.len()];
        let indices: Vec<_> = (0..rows.len()).collect();
        let mut trees = Vec::new();
        for iteration in 0..config.estimators {
            progress(iteration)?;
            let (gradients, hessians): (Vec<_>, Vec<_>) = predictions
                .iter()
                .zip(targets)
                .map(|(p, y)| {
                    if config.objective == BoostObjective::SquaredError {
                        (*y - *p, 1.0)
                    } else {
                        let probability = sigmoid(*p);
                        (
                            *y - probability,
                            (probability * (1.0 - probability)).max(1e-8),
                        )
                    }
                })
                .unzip();
            let tree = boost_tree(
                rows,
                &indices,
                &gradients,
                &hessians,
                &thresholds,
                0,
                config,
            );
            for (p, row) in predictions.iter_mut().zip(rows) {
                *p += config.learning_rate * tree.predict(row)?;
                require(p.is_finite(), "Boosting prediction overflowed")?;
            }
            trees.push(tree);
        }
        progress(config.estimators)?;
        Ok(Self {
            width,
            objective: config.objective,
            bias,
            learning_rate: config.learning_rate,
            trees,
        })
    }
    pub fn predict(&self, row: &[f64]) -> Result<f64> {
        require(
            row.len() == self.width && row.iter().all(|v| v.is_finite()) && !self.trees.is_empty(),
            "Boosting model or feature vector is invalid",
        )?;
        let score = self.bias
            + self.learning_rate
                * self
                    .trees
                    .iter()
                    .map(|tree| tree.predict(row))
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .sum::<f64>();
        require(score.is_finite(), "Boosting score overflowed")?;
        Ok(if self.objective == BoostObjective::BinaryLogLoss {
            sigmoid(score)
        } else {
            score
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PatchCore {
    pub memory_bank: Vec<Vec<f64>>,
    width: usize,
}
impl PatchCore {
    /// Fits the patch memory bank from embeddings of normal images. The caller supplies the frozen image encoder.
    pub fn fit(patches: &[Vec<f64>], coreset_size: usize) -> Result<Self> {
        Self::fit_with_progress(patches, coreset_size, |_| Ok(()))
    }
    pub fn fit_with_progress(
        patches: &[Vec<f64>],
        coreset_size: usize,
        mut progress: impl FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        let width = validate_matrix(patches)?;
        require(
            coreset_size > 0,
            "Patch memory bank must contain at least one patch",
        )?;
        let size = coreset_size.min(patches.len());
        let mut chosen = vec![0];
        let mut nearest = vec![f64::INFINITY; patches.len()];
        progress(0)?;
        while chosen.len() < size {
            progress(chosen.len())?;
            let last = *chosen.last().unwrap();
            for (i, patch) in patches.iter().enumerate() {
                nearest[i] = nearest[i].min(squared_distance(patch, &patches[last]));
            }
            for index in &chosen {
                nearest[*index] = -1.0;
            }
            chosen.push(
                (0..patches.len())
                    .max_by(|a, b| nearest[*a].total_cmp(&nearest[*b]))
                    .unwrap(),
            );
        }
        progress(size)?;
        Ok(Self {
            memory_bank: chosen.into_iter().map(|i| patches[i].clone()).collect(),
            width,
        })
    }
    pub fn score_patches(&self, patches: &[Vec<f64>]) -> Result<Vec<f64>> {
        require(
            validate_matrix(patches)? == self.width
                && validate_matrix(&self.memory_bank)? == self.width,
            "Patch embedding width differs from training",
        )?;
        Ok(patches
            .iter()
            .map(|patch| {
                self.memory_bank
                    .iter()
                    .map(|normal| squared_distance(patch, normal))
                    .fold(f64::INFINITY, f64::min)
                    .sqrt()
            })
            .collect())
    }
    pub fn score(&self, patches: &[Vec<f64>]) -> Result<f64> {
        Ok(self.score_patches(patches)?.into_iter().fold(0.0, f64::max))
    }
}
fn squared_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Padim {
    means: Vec<Vec<f64>>,
    cholesky: Vec<Vec<Vec<f64>>>,
    width: usize,
}
impl Padim {
    /// Fits one regularized Gaussian per spatial patch from normal-image embeddings.
    pub fn fit(images: &[Vec<Vec<f64>>], regularization: f64) -> Result<Self> {
        Self::fit_with_progress(images, regularization, |_| Ok(()))
    }
    pub fn fit_with_progress(
        images: &[Vec<Vec<f64>>],
        regularization: f64,
        mut progress: impl FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        require(
            images.len() >= 2 && regularization.is_finite() && regularization > 0.0,
            "PaDiM needs at least two normal images and positive covariance regularization",
        )?;
        let patches = images[0].len();
        let width = validate_matrix(&images[0])?;
        require(
            width <= 512,
            "PaDiM covariance width exceeds 512; select fewer embedding channels",
        )?;
        for image in images {
            require(
                image.len() == patches && validate_matrix(image)? == width,
                "PaDiM images must share patch layout and embedding width",
            )?;
        }
        let mut means = vec![vec![0.0; width]; patches];
        let mut cholesky = Vec::with_capacity(patches);
        for patch in 0..patches {
            progress(patch)?;
            for image in images {
                for j in 0..width {
                    means[patch][j] += image[patch][j] / images.len() as f64;
                }
            }
            let mut covariance = vec![vec![0.0; width]; width];
            for image in images {
                for i in 0..width {
                    for j in 0..=i {
                        covariance[i][j] += (image[patch][i] - means[patch][i])
                            * (image[patch][j] - means[patch][j])
                            / (images.len() - 1) as f64;
                    }
                }
            }
            for (i, row) in covariance.iter_mut().enumerate() {
                row[i] += regularization;
            }
            cholesky.push(cholesky_factor(&covariance)?);
        }
        progress(patches)?;
        Ok(Self {
            means,
            cholesky,
            width,
        })
    }
    pub fn score_patches(&self, image: &[Vec<f64>]) -> Result<Vec<f64>> {
        require(
            validate_matrix(image)? == self.width
                && image.len() == self.means.len()
                && image.len() == self.cholesky.len(),
            "PaDiM image layout differs from training",
        )?;
        require(
            validate_matrix(&self.means)? == self.width,
            "Invalid PaDiM mean shape",
        )?;
        image
            .iter()
            .enumerate()
            .map(|(patch, values)| {
                let factor = &self.cholesky[patch];
                require(
                    factor.len() == self.width && factor.iter().all(|r| r.len() == self.width),
                    "Invalid PaDiM covariance shape",
                )?;
                let mut solved = vec![0.0; self.width];
                for i in 0..self.width {
                    require(
                        factor[i][i].is_finite() && factor[i][i] > 0.0,
                        "Invalid PaDiM covariance factor",
                    )?;
                    let dot = (0..i).map(|j| factor[i][j] * solved[j]).sum::<f64>();
                    solved[i] = (values[i] - self.means[patch][i] - dot) / factor[i][i];
                }
                Ok(solved.iter().map(|v| v * v).sum::<f64>().sqrt())
            })
            .collect()
    }
}
fn cholesky_factor(matrix: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
    let n = matrix.len();
    let mut result = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let value = matrix[i][j] - (0..j).map(|k| result[i][k] * result[j][k]).sum::<f64>();
            if i == j {
                require(
                    value.is_finite() && value > 0.0,
                    "Covariance is not positive definite; increase regularization",
                )?;
                result[i][j] = value.sqrt();
            } else {
                result[i][j] = value / result[j][j];
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_forest_separates_outliers_and_roundtrips() {
        let mut rng = Rng::new(7);
        let rows: Vec<_> = (0..300)
            .map(|_| {
                (0..2)
                    .map(|_| (0..12).map(|_| rng.uniform()).sum::<f64>() - 6.0)
                    .collect()
            })
            .collect();
        let model = IsolationForest::fit(&rows, &IsolationForestConfig::default()).unwrap();
        assert!(model.score(&[9.0, 9.0]).unwrap() > model.score(&[0.0, 0.0]).unwrap() + 0.1);
        let copy: IsolationForest =
            serde_json::from_slice(&serde_json::to_vec(&model).unwrap()).unwrap();
        assert_eq!(
            model.score(&[9.0, 9.0]).unwrap(),
            copy.score(&[9.0, 9.0]).unwrap()
        );
    }
    #[test]
    fn histogram_boosting_learns_nonlinear_regression_and_classification() {
        let rows: Vec<_> = (0..100).map(|i| vec![i as f64 / 50.0 - 1.0]).collect();
        let targets: Vec<_> = rows.iter().map(|r| r[0] * r[0]).collect();
        let model =
            HistogramGradientBoosting::fit(&rows, &targets, &BoostConfig::default()).unwrap();
        let mse = rows
            .iter()
            .zip(&targets)
            .map(|(r, y)| (model.predict(r).unwrap() - y).powi(2))
            .sum::<f64>()
            / 100.0;
        assert!(mse < 0.005, "mse {mse}");
        let labels: Vec<_> = rows.iter().map(|r| f64::from(r[0].abs() > 0.5)).collect();
        let classifier = HistogramGradientBoosting::fit(
            &rows,
            &labels,
            &BoostConfig {
                objective: BoostObjective::BinaryLogLoss,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(classifier.predict(&[0.8]).unwrap() > 0.9);
        assert!(classifier.predict(&[0.0]).unwrap() < 0.1);
    }
    #[test]
    fn training_callback_can_cancel() {
        let rows = vec![vec![0.0], vec![1.0]];
        assert!(
            IsolationForest::fit_with_progress(&rows, &IsolationForestConfig::default(), |i| {
                require(i < 3, "cancelled")
            })
            .is_err()
        );
    }
    #[test]
    fn patch_memory_and_spatial_gaussians_detect_shifted_patches() {
        let model = PatchCore::fit(&[vec![0.0, 0.0], vec![0.1, 0.1], vec![1.0, 1.0]], 2).unwrap();
        assert_eq!(model.memory_bank.len(), 2);
        assert!(model.score(&[vec![5.0, 5.0]]).unwrap() > 5.0);
        let images = vec![
            vec![vec![0.0, 0.0], vec![5.0, 5.0]],
            vec![vec![0.1, -0.1], vec![5.1, 4.9]],
            vec![vec![-0.1, 0.1], vec![4.9, 5.1]],
        ];
        let model = Padim::fit(&images, 0.01).unwrap();
        let scores = model
            .score_patches(&[vec![0.0, 0.0], vec![0.0, 0.0]])
            .unwrap();
        assert!(scores[0] < 1e-6 && scores[1] > 10.0);
    }
}

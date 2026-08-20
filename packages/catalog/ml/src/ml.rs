//! Sub-Catalog for Machine Learning
//!
//! This module contains various machine learning algorithms and dataset utilities based on the `[linfa]` crate.
//!
//! Note: The `execute` feature must be enabled for actual ML model training and inference.
//! Without it, only node metadata (get_node()) is available.

use flow_like_storage::arrow_schema::{DataType, Field};
use flow_like_types::{Error, Result, Value, anyhow};
use ndarray::{Array1, Array2};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_ordinal::{
    AdjacentCategory, ContinuationRatio, FrankHall, OrdinalLogistic, OrdinalNeural, OrdinalRidge,
};
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, create_id, json::json, sync::Mutex};
#[cfg(feature = "execute")]
use linfa::composing::MultiClassModel;
#[cfg(feature = "execute")]
use linfa::prelude::Pr;
#[cfg(feature = "execute")]
use linfa::{
    DatasetBase,
    traits::{Predict, Transformer},
};
#[cfg(feature = "execute")]
use linfa_bayes::{GaussianNb, MultinomialNb};
#[cfg(feature = "execute")]
use linfa_clustering::{GaussianMixtureModel, KMeans};
#[cfg(feature = "execute")]
use linfa_elasticnet::ElasticNet;
#[cfg(feature = "execute")]
use linfa_ensemble::{AdaBoost, EnsembleLearner};
#[cfg(feature = "execute")]
use linfa_linear::{FittedLinearRegression, TweedieRegressor};
#[cfg(feature = "execute")]
use linfa_logistic::{FittedLogisticRegression, MultiFittedLogisticRegression};
#[cfg(feature = "execute")]
use linfa_nn::distance::L2Dist;
#[cfg(feature = "execute")]
use linfa_preprocessing::linear_scaling::LinearScaler;
#[cfg(feature = "execute")]
use linfa_preprocessing::tf_idf_vectorization::FittedTfIdfVectorizer;
#[cfg(feature = "execute")]
use linfa_svm::Svm;
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree;
#[cfg(feature = "execute")]
use std::fmt;
#[cfg(feature = "execute")]
use std::sync::Arc;

pub mod classification;
pub mod clustering;
pub mod dataset;
pub mod load;
pub mod load_binary;
pub mod metrics;
pub mod model_info;
pub mod ordinal;
pub mod prediction;
pub mod preprocessing;
pub mod reduction;
pub mod regression;
pub mod save;
pub mod save_binary;
pub mod tuning;

// ============================================================================
// Output Schema Types for ML Nodes
// ============================================================================

/// Cluster centroids extracted from a KMeans model
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KMeansCentroids {
    /// Number of clusters (k)
    pub k: usize,
    /// Number of dimensions per centroid
    pub dimensions: usize,
    /// 2D array of centroid coordinates (k × dimensions)
    pub centroids: Vec<Vec<f64>>,
}

/// Coefficients extracted from a Linear Regression model
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinearCoefficients {
    /// Feature coefficients (one per input dimension)
    pub coefficients: Vec<f64>,
    /// The y-intercept (bias term)
    pub intercept: f64,
    /// Number of input features
    pub n_features: usize,
}

/// Confusion matrix result with classification metrics
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfusionMatrixResult {
    /// 2D confusion matrix (rows=actual, cols=predicted)
    pub matrix: Vec<Vec<i64>>,
    /// Class labels in order they appear in the matrix
    pub labels: Vec<String>,
    /// Weighted average precision across all classes
    pub precision: f64,
    /// Weighted average recall across all classes
    pub recall: f64,
    /// Weighted average F1 score across all classes
    pub f1_score: f64,
    /// Total number of samples
    pub total_samples: usize,
}

/// Regression evaluation metrics
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegressionMetrics {
    /// Mean Squared Error
    pub mse: f64,
    /// Root Mean Squared Error
    pub rmse: f64,
    /// Mean Absolute Error
    pub mae: f64,
    /// R² coefficient of determination
    pub r2: f64,
    /// Number of samples evaluated
    pub n_samples: usize,
}

/// Classification accuracy metrics
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AccuracyMetrics {
    /// Accuracy score (0.0 to 1.0)
    pub accuracy: f64,
    /// Number of correct predictions
    pub correct_count: usize,
    /// Total number of predictions
    pub total_count: usize,
}

// ============================================================================
// Hyperparameter Tuning Schema Types
// ============================================================================

/// A single parameter with its possible values for grid search
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ParameterSpec {
    /// Parameter name (e.g., "max_depth", "n_clusters")
    pub name: String,
    /// List of values to try (as JSON values)
    pub values: Vec<Value>,
}

/// Cross-validation results for a single parameter combination
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CVFoldResult {
    /// Fold index (0 to k-1)
    pub fold: usize,
    /// Score on this fold's validation set
    pub score: f64,
}

/// Results from a single parameter combination in grid search
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GridSearchEntry {
    /// Parameter values used for this run
    pub params: HashMap<String, Value>,
    /// Mean CV score across all folds
    pub mean_score: f64,
    /// Standard deviation of CV scores
    pub std_score: f64,
    /// Individual fold scores
    pub fold_scores: Vec<f64>,
    /// Training time in seconds
    pub train_time_secs: f64,
}

/// Complete grid search results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GridSearchResult {
    /// All parameter combinations tried
    pub results: Vec<GridSearchEntry>,
    /// Index of best result
    pub best_index: usize,
    /// Best parameters found
    pub best_params: HashMap<String, Value>,
    /// Best mean CV score
    pub best_score: f64,
    /// Total search time in seconds
    pub total_time_secs: f64,
    /// Number of parameter combinations tried
    pub n_combinations: usize,
    /// Number of CV folds used
    pub n_folds: usize,
}

/// Entry in the AutoML leaderboard
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoMLEntry {
    /// Model type (e.g., "GaussianNaiveBayes", "DecisionTree", "SVM")
    pub model_type: String,
    /// Best parameters found for this model
    pub best_params: HashMap<String, Value>,
    /// Best CV score achieved
    pub cv_score: f64,
    /// Training time in seconds
    pub train_time_secs: f64,
    /// Rank in leaderboard (1 = best)
    pub rank: usize,
}

/// Complete AutoML results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoMLResult {
    /// Leaderboard entries sorted by score
    pub leaderboard: Vec<AutoMLEntry>,
    /// Index of best model in leaderboard
    pub best_model_index: usize,
    /// Total models trained
    pub total_models_tried: usize,
    /// Total elapsed time in seconds
    pub total_time_secs: f64,
    /// Metric used for optimization
    pub metric: String,
}

/// Max number of records for train/prediction
/// TODO: block-wise processing, at least for predictions
pub const MAX_ML_PREDICTION_RECORDS: usize = 20000;

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct ClassEntry {
    id: usize,
    name: String,
}

/// Helper-Module to serialize HashMap as Vec and deserialize Vec as HashMap for the class mappings.
#[cfg(feature = "execute")]
mod vec_as_map {
    use super::ClassEntry;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::HashMap;

    pub fn serialize<S>(
        map_opt: &Option<HashMap<usize, String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match map_opt {
            Some(map) => {
                let mut seq = serializer.serialize_seq(Some(map.len()))?;
                for (id, name) in map {
                    let entry = ClassEntry {
                        id: *id,
                        name: name.clone(),
                    };
                    seq.serialize_element(&entry)?;
                }
                seq.end()
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<HashMap<usize, String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt_vec: Option<Vec<ClassEntry>> = Option::deserialize(deserializer)?;
        Ok(opt_vec.map(|v| v.into_iter().map(|e| (e.id, e.name)).collect()))
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
/// # Linfa models attached with additional metadata
pub struct ModelWithMeta<M> {
    pub model: M,
    /// Optional mapping from class index → class name
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "vec_as_map")]
    pub classes: Option<HashMap<usize, String>>,
}

/// Persistence wrapper for [`linfa_ensemble::EnsembleLearner`].
///
/// Upstream derives neither `Serialize` nor `Deserialize` on the ensemble types, but every field
/// is public, so the real linfa value is kept in memory and mirrored only at the IO boundary.
/// The mirror field names are the on-disk contract and must not be renamed.
#[cfg(feature = "execute")]
pub struct PersistedEnsemble(pub EnsembleLearner<DecisionTree<f64, usize>>);

#[cfg(feature = "execute")]
#[derive(Serialize)]
struct EnsembleMirrorRef<'a> {
    models: &'a Vec<DecisionTree<f64, usize>>,
    model_features: &'a Vec<Vec<usize>>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct EnsembleMirror {
    models: Vec<DecisionTree<f64, usize>>,
    model_features: Vec<Vec<usize>>,
}

#[cfg(feature = "execute")]
impl fmt::Debug for PersistedEnsemble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedEnsemble")
            .field("n_models", &self.0.models.len())
            .finish()
    }
}

#[cfg(feature = "execute")]
impl Serialize for PersistedEnsemble {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        EnsembleMirrorRef {
            models: &self.0.models,
            model_features: &self.0.model_features,
        }
        .serialize(serializer)
    }
}

#[cfg(feature = "execute")]
impl<'de> Deserialize<'de> for PersistedEnsemble {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mirror = EnsembleMirror::deserialize(deserializer)?;
        Ok(PersistedEnsemble(EnsembleLearner {
            models: mirror.models,
            model_features: mirror.model_features,
        }))
    }
}

/// Persistence wrapper for [`linfa_ensemble::AdaBoost`], mirrored for the same reason as
/// [`PersistedEnsemble`].
#[cfg(feature = "execute")]
pub struct PersistedAdaBoost(pub AdaBoost<DecisionTree<f64, usize>, usize>);

#[cfg(feature = "execute")]
#[derive(Serialize)]
struct AdaBoostMirrorRef<'a> {
    models: &'a Vec<DecisionTree<f64, usize>>,
    model_weights: &'a Vec<f64>,
    classes: &'a Vec<usize>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct AdaBoostMirror {
    models: Vec<DecisionTree<f64, usize>>,
    model_weights: Vec<f64>,
    classes: Vec<usize>,
}

#[cfg(feature = "execute")]
impl fmt::Debug for PersistedAdaBoost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedAdaBoost")
            .field("n_models", &self.0.models.len())
            .field("n_classes", &self.0.classes.len())
            .finish()
    }
}

#[cfg(feature = "execute")]
impl Serialize for PersistedAdaBoost {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AdaBoostMirrorRef {
            models: &self.0.models,
            model_weights: &self.0.model_weights,
            classes: &self.0.classes,
        }
        .serialize(serializer)
    }
}

#[cfg(feature = "execute")]
impl<'de> Deserialize<'de> for PersistedAdaBoost {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mirror = AdaBoostMirror::deserialize(deserializer)?;
        Ok(PersistedAdaBoost(AdaBoost {
            models: mirror.models,
            model_weights: mirror.model_weights,
            classes: mirror.classes,
        }))
    }
}

/// Persistence wrapper for a boolean-target [`linfa_ensemble::EnsembleLearner`].
///
/// Mirrored for the same reason as [`PersistedEnsemble`] — upstream derives no serde and every
/// field is public — but over a `bool` target, because the Frank & Hall decomposition asks each
/// base learner a yes/no question ("is the level above this cut?"). Without this newtype a
/// forest-backed `FrankHall` could be fitted but never saved. The mirror field names are the
/// on-disk contract and must not be renamed.
#[cfg(feature = "execute")]
pub struct PersistedBoolEnsemble(pub EnsembleLearner<DecisionTree<f64, bool>>);

#[cfg(feature = "execute")]
#[derive(Serialize)]
struct BoolEnsembleMirrorRef<'a> {
    models: &'a Vec<DecisionTree<f64, bool>>,
    model_features: &'a Vec<Vec<usize>>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct BoolEnsembleMirror {
    models: Vec<DecisionTree<f64, bool>>,
    model_features: Vec<Vec<usize>>,
}

#[cfg(feature = "execute")]
impl fmt::Debug for PersistedBoolEnsemble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedBoolEnsemble")
            .field("n_models", &self.0.models.len())
            .finish()
    }
}

#[cfg(feature = "execute")]
impl Serialize for PersistedBoolEnsemble {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        BoolEnsembleMirrorRef {
            models: &self.0.models,
            model_features: &self.0.model_features,
        }
        .serialize(serializer)
    }
}

#[cfg(feature = "execute")]
impl<'de> Deserialize<'de> for PersistedBoolEnsemble {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mirror = BoolEnsembleMirror::deserialize(deserializer)?;
        Ok(PersistedBoolEnsemble(EnsembleLearner {
            models: mirror.models,
            model_features: mirror.model_features,
        }))
    }
}

/// Lets the wrapper stand in for the ensemble as a Frank & Hall base learner, which requires its
/// base model to predict a boolean per row.
///
/// The trait is spelled out in full because the empty ensemble has to be intercepted: upstream
/// votes with `max_by_key(..).unwrap()` over a per-row map that stays empty when there are no
/// models, and its `default_target` indexes `models[0]`. Neither can happen for a forest this
/// catalog fitted, but a deserialized model carries whatever was on disk.
#[cfg(feature = "execute")]
impl linfa::traits::PredictInplace<Array2<f64>, Array1<bool>> for PersistedBoolEnsemble {
    fn predict_inplace(&self, x: &Array2<f64>, y: &mut Array1<bool>) {
        if self.0.models.is_empty() {
            y.fill(false);
            return;
        }
        linfa::traits::PredictInplace::predict_inplace(&self.0, x, y);
    }

    fn default_target(&self, x: &Array2<f64>) -> Array1<bool> {
        Array1::from_elem(x.nrows(), false)
    }
}

/// A fitted Frank & Hall decomposition, keyed by which base learner backs it.
///
/// `FrankHall<M>` is only serializable when `M` is, so the variants here are exactly the base
/// learners whose fitted model can round-trip through storage: either it derives serde itself, or
/// it is wrapped in a mirror that does — [`PersistedBoolEnsemble`] for the random forest, since
/// `linfa_ensemble::EnsembleLearner` derives neither half.
#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "base")]
pub enum FrankHallModel {
    DecisionTree(FrankHall<DecisionTree<f64, bool>>),
    GaussianNaiveBayes(FrankHall<GaussianNb<f64, bool>>),
    RandomForest(FrankHall<PersistedBoolEnsemble>),
}

#[cfg(feature = "execute")]
impl FrankHallModel {
    /// Number of ordered levels the decomposition spans.
    pub fn n_classes(&self) -> usize {
        match self {
            FrankHallModel::DecisionTree(model) => model.n_classes(),
            FrankHallModel::GaussianNaiveBayes(model) => model.n_classes(),
            FrankHallModel::RandomForest(model) => model.n_classes(),
        }
    }

    /// Feature width the base learners were fitted on.
    pub fn n_features(&self) -> usize {
        match self {
            FrankHallModel::DecisionTree(model) => model.n_features(),
            FrankHallModel::GaussianNaiveBayes(model) => model.n_features(),
            FrankHallModel::RandomForest(model) => model.n_features(),
        }
    }

    /// Predicted level per row.
    pub fn predict_levels(&self, records: &Array2<f64>) -> Array1<usize> {
        match self {
            FrankHallModel::DecisionTree(model) => model.predict(records),
            FrankHallModel::GaussianNaiveBayes(model) => model.predict(records),
            FrankHallModel::RandomForest(model) => model.predict(records),
        }
    }

    /// Human-readable name of the base learner, for model-info output.
    pub fn base_name(&self) -> &'static str {
        match self {
            FrankHallModel::DecisionTree(_) => "Decision Tree",
            FrankHallModel::GaussianNaiveBayes(_) => "Gaussian Naive Bayes",
            FrankHallModel::RandomForest(_) => "Random Forest",
        }
    }
}

/// A fitted k-nearest-neighbour model.
///
/// linfa ships nearest-neighbour *indexes* but no KNN estimator, and none of the index types are
/// serializable (each borrows the batch it was built from). The training matrix is therefore the
/// model: it is stored row-major and queried by brute-force L2 at prediction time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnnModel {
    /// Flattened row-major training features
    pub features: Vec<f64>,
    /// Number of columns per training row
    pub n_features: usize,
    /// Class ids (classification) or target values (regression), one per training row
    pub targets: Vec<f64>,
    /// Number of neighbours to consult
    pub k: usize,
    /// Weight neighbours by inverse distance instead of uniformly
    pub distance_weighted: bool,
}

impl KnnModel {
    /// Returns the `k` nearest training rows to `row` as `(index, distance)`, nearest first.
    fn neighbours(&self, row: &[f64]) -> Result<Vec<(usize, f64)>> {
        if self.n_features == 0 || self.features.is_empty() {
            return Err(anyhow!("KNN model has no training rows"));
        }
        // A truncated or mis-sized `features` buffer makes `chunks_exact` yield nothing, which
        // would leave `scored` empty and turn the k clamp below into a panic.
        if !self.features.len().is_multiple_of(self.n_features) {
            return Err(anyhow!(
                "KNN model is corrupt: {} stored values is not a multiple of {} features per row",
                self.features.len(),
                self.n_features
            ));
        }
        if row.len() != self.n_features {
            return Err(anyhow!(
                "KNN expected {} features, got {}",
                self.n_features,
                row.len()
            ));
        }

        let mut scored: Vec<(usize, f64)> = self
            .features
            .chunks_exact(self.n_features)
            .enumerate()
            .map(|(idx, train_row)| {
                let dist_sq: f64 = train_row
                    .iter()
                    .zip(row.iter())
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                (idx, dist_sq.sqrt())
            })
            .collect();

        let k = self.k.clamp(1, scored.len());
        scored.sort_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    /// Weight for a neighbour at `distance`, honouring `distance_weighted`.
    fn weight_of(&self, distance: f64) -> f64 {
        if self.distance_weighted {
            1.0 / (distance + f64::EPSILON)
        } else {
            1.0
        }
    }

    /// Majority (optionally distance-weighted) vote. Returns `(class_id, confidence)` where
    /// confidence is the winning class's share of the total neighbour weight.
    pub fn predict_class(&self, row: &[f64]) -> Result<(usize, f64)> {
        let neighbours = self.neighbours(row)?;
        let mut tally: HashMap<usize, f64> = HashMap::new();
        let mut total = 0.0;
        for (idx, distance) in &neighbours {
            let weight = self.weight_of(*distance);
            let class = *self
                .targets
                .get(*idx)
                .ok_or_else(|| anyhow!("KNN target row {idx} missing"))?
                as usize;
            *tally.entry(class).or_insert(0.0) += weight;
            total += weight;
        }
        // Ties resolve to the lowest class id so predictions stay reproducible.
        let (class, weight) = tally
            .into_iter()
            .max_by(|(a_class, a_w), (b_class, b_w)| {
                a_w.partial_cmp(b_w)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b_class.cmp(a_class))
            })
            .ok_or_else(|| anyhow!("KNN found no neighbours"))?;
        let confidence = if total > 0.0 { weight / total } else { 0.0 };
        Ok((class, confidence))
    }

    /// Mean (optionally distance-weighted) target of the `k` nearest rows.
    pub fn predict_value(&self, row: &[f64]) -> Result<f64> {
        let neighbours = self.neighbours(row)?;
        let mut weighted_sum = 0.0;
        let mut total = 0.0;
        for (idx, distance) in &neighbours {
            let weight = self.weight_of(*distance);
            let target = *self
                .targets
                .get(*idx)
                .ok_or_else(|| anyhow!("KNN target row {idx} missing"))?;
            weighted_sum += weight * target;
            total += weight;
        }
        if total == 0.0 {
            return Err(anyhow!("KNN neighbour weights summed to zero"));
        }
        Ok(weighted_sum / total)
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
/// # Unified Type for Machine Learning Models from Linfa Crate
pub enum MLModel {
    KMeans(ModelWithMeta<KMeans<f64, L2Dist>>),
    SVMMultiClass(ModelWithMeta<Vec<(usize, Svm<f64, Pr>)>>),
    LinearRegression(ModelWithMeta<FittedLinearRegression<f64>>),
    GaussianNaiveBayes(ModelWithMeta<GaussianNb<f64, usize>>),
    DecisionTree(ModelWithMeta<DecisionTree<f64, usize>>),
    LogisticRegression(ModelWithMeta<FittedLogisticRegression<f64, usize>>),
    MultinomialLogisticRegression(ModelWithMeta<MultiFittedLogisticRegression<f64, usize>>),
    ElasticNet(ModelWithMeta<ElasticNet<f64>>),
    TweedieRegressor(ModelWithMeta<TweedieRegressor<f64>>),
    GaussianMixture(ModelWithMeta<GaussianMixtureModel<f64>>),
    MultinomialNaiveBayes(ModelWithMeta<MultinomialNb<f64, usize>>),
    RandomForest(ModelWithMeta<PersistedEnsemble>),
    AdaBoost(ModelWithMeta<PersistedAdaBoost>),
    SVMRegression(ModelWithMeta<Svm<f64, f64>>),
    OneClassSVM(ModelWithMeta<Svm<f64, bool>>),
    KnnClassifier(ModelWithMeta<KnnModel>),
    KnnRegressor(ModelWithMeta<KnnModel>),
    FeatureScaler(ModelWithMeta<LinearScaler<f64>>),
    TfIdfVectorizer(ModelWithMeta<FittedTfIdfVectorizer>),
    /// Ordered target, proportional-odds model. `classes` maps rank to level label.
    OrdinalLogistic(ModelWithMeta<OrdinalLogistic<f64>>),
    /// Ordered target, rank regression with learned cut points.
    OrdinalRidge(ModelWithMeta<OrdinalRidge<f64>>),
    /// Ordered target via the Frank & Hall decomposition over a binary base learner.
    OrdinalFrankHall(ModelWithMeta<FrankHallModel>),
    /// Ordered target modelled as a sequential progression: P(stop at k | reached k).
    OrdinalContinuationRatio(ModelWithMeta<ContinuationRatio<f64>>),
    /// Ordered target modelled by contrasts between neighbouring levels.
    OrdinalAdjacentCategory(ModelWithMeta<AdjacentCategory<f64>>),
    /// Ordered target with a neural backbone and a rank-consistent CORAL or CORN head.
    OrdinalNeural(ModelWithMeta<OrdinalNeural<f64>>),
}

#[cfg(feature = "execute")]
impl fmt::Display for MLModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Writes one scalar prediction per row into `target_col`.
#[cfg(feature = "execute")]
fn write_scalar_predictions<I>(values: &mut [Value], target_col: &str, predictions: I)
where
    I: IntoIterator<Item = f64>,
{
    for (value, prediction) in values.iter_mut().zip(predictions) {
        if let Value::Object(map) = value {
            map.insert(target_col.to_string(), json!(prediction));
        }
    }
}

/// Writes one class prediction per row into `target_col`, resolving class names when the model
/// carries a mapping and falling back to the raw class id when it does not.
#[cfg(feature = "execute")]
fn write_class_predictions<I>(
    values: &mut [Value],
    target_col: &str,
    predictions: I,
    classes: Option<&HashMap<usize, String>>,
) -> Result<()>
where
    I: IntoIterator<Item = usize>,
{
    for (value, prediction) in values.iter_mut().zip(predictions) {
        if let Value::Object(map) = value {
            match classes {
                Some(classes) => {
                    let class = classes.get(&prediction).ok_or_else(|| {
                        anyhow!(
                            "Couldn't map prediction {} to any of these classes {:?}",
                            prediction,
                            classes
                        )
                    })?;
                    map.insert(target_col.to_string(), json!(class));
                }
                None => {
                    map.insert(target_col.to_string(), json!(prediction));
                }
            };
        }
    }
    Ok(())
}

/// Writes one vector per row into `target_col`, for transformer models.
#[cfg(feature = "execute")]
fn write_vector_outputs(values: &mut [Value], target_col: &str, rows: &Array2<f64>) {
    for (value, row) in values.iter_mut().zip(rows.rows()) {
        if let Value::Object(map) = value {
            map.insert(target_col.to_string(), json!(row.to_vec()));
        }
    }
}

/// Builds an [`MLPrediction`] for a classifier, resolving the class name when available.
#[cfg(feature = "execute")]
fn class_prediction(
    predicted: usize,
    classes: Option<&HashMap<usize, String>>,
    confidence: Option<f64>,
) -> Result<MLPrediction> {
    let class = match classes {
        Some(classes) => Some(classes.get(&predicted).cloned().ok_or_else(|| {
            anyhow!(
                "Couldn't map prediction {} to any of these classes {:?}",
                predicted,
                classes
            )
        })?),
        None => None,
    };
    Ok(MLPrediction {
        score: predicted as f64,
        class,
        confidence,
    })
}

/// First class id of a single-row prediction.
#[cfg(feature = "execute")]
fn first_class_id(predictions: &Array1<usize>) -> Result<usize> {
    predictions
        .first()
        .copied()
        .ok_or_else(|| anyhow!("Got an empty prediction"))
}

/// First scalar of a single-row prediction.
#[cfg(feature = "execute")]
fn first_scalar(predictions: &Array1<f64>) -> Result<f64> {
    predictions
        .first()
        .copied()
        .ok_or_else(|| anyhow!("Got an empty prediction"))
}

/// Largest value in a probability row, used as the confidence of the winning class.
#[cfg(feature = "execute")]
fn max_probability(row: ndarray::ArrayView1<'_, f64>) -> Option<f64> {
    row.iter().copied().fold(None, |acc: Option<f64>, p| {
        Some(acc.map_or(p, |a| a.max(p)))
    })
}

#[cfg(feature = "execute")]
impl MLModel {
    /// Stable identifier for this model kind.
    ///
    /// These strings double as the binary-format discriminator and match the serde variant
    /// names produced by `#[serde(tag = "type")]`, so they are an on-disk contract: renaming one
    /// breaks every model a user has already saved.
    pub fn kind(&self) -> &'static str {
        match self {
            MLModel::KMeans(_) => "KMeans",
            MLModel::SVMMultiClass(_) => "SVMMultiClass",
            MLModel::LinearRegression(_) => "LinearRegression",
            MLModel::GaussianNaiveBayes(_) => "GaussianNaiveBayes",
            MLModel::DecisionTree(_) => "DecisionTree",
            MLModel::LogisticRegression(_) => "LogisticRegression",
            MLModel::MultinomialLogisticRegression(_) => "MultinomialLogisticRegression",
            MLModel::ElasticNet(_) => "ElasticNet",
            MLModel::TweedieRegressor(_) => "TweedieRegressor",
            MLModel::GaussianMixture(_) => "GaussianMixture",
            MLModel::MultinomialNaiveBayes(_) => "MultinomialNaiveBayes",
            MLModel::RandomForest(_) => "RandomForest",
            MLModel::AdaBoost(_) => "AdaBoost",
            MLModel::SVMRegression(_) => "SVMRegression",
            MLModel::OneClassSVM(_) => "OneClassSVM",
            MLModel::KnnClassifier(_) => "KnnClassifier",
            MLModel::KnnRegressor(_) => "KnnRegressor",
            MLModel::FeatureScaler(_) => "FeatureScaler",
            MLModel::TfIdfVectorizer(_) => "TfIdfVectorizer",
            MLModel::OrdinalLogistic(_) => "OrdinalLogistic",
            MLModel::OrdinalRidge(_) => "OrdinalRidge",
            MLModel::OrdinalFrankHall(_) => "OrdinalFrankHall",
            MLModel::OrdinalContinuationRatio(_) => "OrdinalContinuationRatio",
            MLModel::OrdinalAdjacentCategory(_) => "OrdinalAdjacentCategory",
            MLModel::OrdinalNeural(_) => "OrdinalNeural",
        }
    }

    /// Human-readable label shown in logs and model-info output.
    pub fn label(&self) -> &'static str {
        match self {
            MLModel::KMeans(_) => "KMeans Clustering",
            MLModel::SVMMultiClass(_) => "SVM Classification (Multiple Classes)",
            MLModel::LinearRegression(_) => "Linear Regression",
            MLModel::GaussianNaiveBayes(_) => "Gaussian Naive Bayes Classification",
            MLModel::DecisionTree(_) => "Decision Tree Classification",
            MLModel::LogisticRegression(_) => "Logistic Regression (Binary)",
            MLModel::MultinomialLogisticRegression(_) => "Logistic Regression (Multinomial)",
            MLModel::ElasticNet(_) => "Elastic Net Regression",
            MLModel::TweedieRegressor(_) => "Generalized Linear Model (Tweedie)",
            MLModel::GaussianMixture(_) => "Gaussian Mixture Clustering",
            MLModel::MultinomialNaiveBayes(_) => "Multinomial Naive Bayes Classification",
            MLModel::RandomForest(_) => "Random Forest Classification",
            MLModel::AdaBoost(_) => "AdaBoost Classification",
            MLModel::SVMRegression(_) => "SVM Regression",
            MLModel::OneClassSVM(_) => "One-Class SVM (Novelty Detection)",
            MLModel::KnnClassifier(_) => "K-Nearest Neighbours Classification",
            MLModel::KnnRegressor(_) => "K-Nearest Neighbours Regression",
            MLModel::FeatureScaler(_) => "Feature Scaler",
            MLModel::TfIdfVectorizer(_) => "TF-IDF Vectorizer",
            MLModel::OrdinalLogistic(_) => "Ordinal Regression (Proportional Odds)",
            MLModel::OrdinalRidge(_) => "Ordinal Regression (Ridge)",
            MLModel::OrdinalFrankHall(_) => "Ordinal Regression (Frank & Hall)",
            MLModel::OrdinalContinuationRatio(_) => "Ordinal Regression (Continuation Ratio)",
            MLModel::OrdinalAdjacentCategory(_) => "Ordinal Regression (Adjacent Category)",
            MLModel::OrdinalNeural(_) => "Ordinal Regression (Neural, CORAL/CORN)",
        }
    }

    /// Class-id to class-name mapping, when the model was trained on a categorical target.
    ///
    /// Returns `None` for regressors, clusterers and transformers, and also for classifiers whose
    /// target column already held integer class ids.
    pub fn classes(&self) -> Option<&HashMap<usize, String>> {
        match self {
            MLModel::KMeans(m) => m.classes.as_ref(),
            MLModel::SVMMultiClass(m) => m.classes.as_ref(),
            MLModel::LinearRegression(m) => m.classes.as_ref(),
            MLModel::GaussianNaiveBayes(m) => m.classes.as_ref(),
            MLModel::DecisionTree(m) => m.classes.as_ref(),
            MLModel::LogisticRegression(m) => m.classes.as_ref(),
            MLModel::MultinomialLogisticRegression(m) => m.classes.as_ref(),
            MLModel::ElasticNet(m) => m.classes.as_ref(),
            MLModel::TweedieRegressor(m) => m.classes.as_ref(),
            MLModel::GaussianMixture(m) => m.classes.as_ref(),
            MLModel::MultinomialNaiveBayes(m) => m.classes.as_ref(),
            MLModel::RandomForest(m) => m.classes.as_ref(),
            MLModel::AdaBoost(m) => m.classes.as_ref(),
            MLModel::SVMRegression(m) => m.classes.as_ref(),
            MLModel::OneClassSVM(m) => m.classes.as_ref(),
            MLModel::KnnClassifier(m) => m.classes.as_ref(),
            MLModel::KnnRegressor(m) => m.classes.as_ref(),
            MLModel::FeatureScaler(m) => m.classes.as_ref(),
            MLModel::TfIdfVectorizer(m) => m.classes.as_ref(),
            MLModel::OrdinalLogistic(m) => m.classes.as_ref(),
            MLModel::OrdinalRidge(m) => m.classes.as_ref(),
            MLModel::OrdinalFrankHall(m) => m.classes.as_ref(),
            MLModel::OrdinalContinuationRatio(m) => m.classes.as_ref(),
            MLModel::OrdinalAdjacentCategory(m) => m.classes.as_ref(),
            MLModel::OrdinalNeural(m) => m.classes.as_ref(),
        }
    }

    /// Class names ordered by class id, when a mapping is present.
    pub fn class_names(&self) -> Option<Vec<String>> {
        self.classes().map(|classes| {
            let mut names: Vec<_> = classes.iter().collect();
            names.sort_by_key(|(id, _)| *id);
            names.into_iter().map(|(_, name)| name.clone()).collect()
        })
    }

    /// Number of classes or clusters this model distinguishes, when that is knowable.
    pub fn cardinality(&self) -> Option<usize> {
        match self {
            MLModel::KMeans(m) => Some(m.model.centroids().nrows()),
            MLModel::GaussianMixture(m) => Some(m.model.weights().len()),
            MLModel::SVMMultiClass(m) => Some(m.model.len()),
            MLModel::MultinomialLogisticRegression(m) => Some(m.model.classes().len()),
            MLModel::AdaBoost(m) => Some(m.model.0.classes.len()),
            MLModel::OrdinalLogistic(m) => Some(m.model.n_classes()),
            MLModel::OrdinalRidge(m) => Some(m.model.n_classes()),
            MLModel::OrdinalFrankHall(m) => Some(m.model.n_classes()),
            MLModel::OrdinalContinuationRatio(m) => Some(m.model.n_classes()),
            MLModel::OrdinalAdjacentCategory(m) => Some(m.model.n_classes()),
            MLModel::OrdinalNeural(m) => Some(m.model.n_classes()),
            MLModel::LogisticRegression(_) | MLModel::OneClassSVM(_) => Some(2),
            MLModel::LinearRegression(_)
            | MLModel::ElasticNet(_)
            | MLModel::TweedieRegressor(_)
            | MLModel::SVMRegression(_)
            | MLModel::KnnRegressor(_)
            | MLModel::FeatureScaler(_)
            | MLModel::TfIdfVectorizer(_) => None,
            _ => self.classes().map(|classes| classes.len()),
        }
    }

    /// Number of features this model was trained on, where the fitted value records it.
    ///
    /// linfa asserts on a width mismatch deep inside `predict`, which surfaces as a panic rather
    /// than a node error, so prediction checks this first. `None` means the model type does not
    /// expose its input width and the check has to be skipped.
    pub fn expected_features(&self) -> Option<usize> {
        match self {
            MLModel::KMeans(m) => Some(m.model.centroids().ncols()),
            MLModel::GaussianMixture(m) => Some(m.model.means().ncols()),
            MLModel::LinearRegression(m) => Some(m.model.params().len()),
            MLModel::ElasticNet(m) => Some(m.model.hyperplane().len()),
            MLModel::TweedieRegressor(m) => Some(m.model.coef.len()),
            MLModel::LogisticRegression(m) => Some(m.model.params().len()),
            MLModel::MultinomialLogisticRegression(m) => Some(m.model.params().nrows()),
            MLModel::KnnClassifier(m) | MLModel::KnnRegressor(m) => Some(m.model.n_features),
            MLModel::OrdinalLogistic(m) => Some(m.model.n_features()),
            MLModel::OrdinalRidge(m) => Some(m.model.n_features()),
            MLModel::OrdinalFrankHall(m) => Some(m.model.n_features()),
            MLModel::OrdinalContinuationRatio(m) => Some(m.model.n_features()),
            MLModel::OrdinalAdjacentCategory(m) => Some(m.model.n_features()),
            MLModel::OrdinalNeural(m) => Some(m.model.n_features()),
            _ => None,
        }
    }

    /// Rejects a feature matrix whose width does not match what the model was trained on.
    #[allow(clippy::result_large_err)]
    fn ensure_feature_width(&self, provided: usize) -> Result<()> {
        if let Some(expected) = self.expected_features()
            && provided != expected
        {
            return Err(anyhow!(
                "{} was trained on {expected} features but received {provided}. Check that the record column matches the one used for training.",
                self.label()
            ));
        }
        Ok(())
    }

    /// True for models that emit a vector per row rather than a single prediction.
    pub fn is_transformer(&self) -> bool {
        matches!(
            self,
            MLModel::FeatureScaler(_) | MLModel::TfIdfVectorizer(_)
        )
    }

    pub fn to_json_vec(&self) -> Result<Vec<u8>> {
        Ok(flow_like_types::json::to_vec(&self)?)
    }

    /// Serialize the ML model to Fory binary format.
    ///
    /// Uses a wrapper approach: the linfa model is serialized to MessagePack (fast binary),
    /// then wrapped with Fory for schema evolution support.
    pub fn to_fory_vec(&self) -> Result<Vec<u8>> {
        use fory::{Fory, ForyObject};

        // Wrapper struct for Fory serialization
        #[derive(ForyObject)]
        struct MLModelWrapper {
            version: u8,              // Schema version for future evolution
            model_type: String,       // Discriminator for model type
            msgpack_payload: Vec<u8>, // The actual model serialized as MessagePack
        }

        // Use MessagePack for fast, compact inner serialization
        let msgpack_payload = rmp_serde::to_vec(self)
            .map_err(|e| anyhow!("MessagePack serialization failed: {}", e))?;

        let wrapper = MLModelWrapper {
            version: 1,
            model_type: self.kind().to_string(),
            msgpack_payload,
        };

        let mut fory = Fory::default().compatible(true);
        fory.register::<MLModelWrapper>(1)
            .map_err(|e| anyhow!("Failed to register MLModelWrapper: {}", e))?;

        fory.serialize(&wrapper)
            .map_err(|e| anyhow!("Fory serialization failed: {}", e))
    }

    /// Deserialize an ML model from Fory binary format.
    pub fn from_fory_slice(bytes: &[u8]) -> Result<Self> {
        use fory::{Fory, ForyObject};

        #[derive(ForyObject)]
        struct MLModelWrapper {
            version: u8,
            model_type: String,
            msgpack_payload: Vec<u8>,
        }

        let mut fory = Fory::default().compatible(true);
        fory.register::<MLModelWrapper>(1)
            .map_err(|e| anyhow!("Failed to register MLModelWrapper: {}", e))?;

        let wrapper: MLModelWrapper = fory
            .deserialize(bytes)
            .map_err(|e| anyhow!("Fory deserialization failed: {}", e))?;

        // Fory reads bit 0 of the first byte as a null marker and then returns an all-default
        // struct without erroring, so a foreign file (JSON starts with `{` = 0x7B, null bit set)
        // decodes as version 0 instead of failing. Reject that explicitly.
        if wrapper.model_type.is_empty() && wrapper.msgpack_payload.is_empty() {
            let hint = match bytes.first() {
                Some(b'{') | Some(b'[') => {
                    "the file contains JSON — load it with 'Load Model' instead, or re-save it with 'Save Model (Binary)'".to_string()
                }
                byte => format!("unexpected leading byte {byte:02x?}"),
            };
            return Err(anyhow!(
                "Not a valid .flmodel binary model file ({} bytes): {}",
                bytes.len(),
                hint
            ));
        }

        if wrapper.version != 1 {
            return Err(anyhow!(
                "Unsupported MLModel binary format version: {} (expected 1)",
                wrapper.version
            ));
        }

        // Deserialize the MessagePack payload back to MLModel
        let model: MLModel = rmp_serde::from_slice(&wrapper.msgpack_payload)
            .map_err(|e| anyhow!("MessagePack deserialization failed: {}", e))?;

        // The wrapper discriminator is redundant with the serde tag; disagreement means the
        // payload was rewritten out from under the header, so fail loudly instead of guessing.
        if wrapper.model_type != model.kind() {
            return Err(anyhow!(
                "MLModel binary header declares `{}` but the payload decoded as `{}`",
                wrapper.model_type,
                model.kind()
            ));
        }
        Ok(model)
    }

    pub fn predict_on_values(
        &self,
        values: &mut [Value],
        record_col: &str,
        target_col: &str,
    ) -> Result<()> {
        if self.is_transformer() {
            return Err(anyhow!(
                "{} produces a vector per row and cannot be used with Predict. Use the Apply Transform node instead.",
                self.label()
            ));
        }

        let array = values_to_array2_f64(values, record_col)?;
        self.ensure_feature_width(array.ncols())?;
        let dataset = DatasetBase::from(array);
        match self {
            MLModel::KMeans(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(values, target_col, predictions.iter().copied(), None)
            }
            MLModel::LinearRegression(model) => {
                let predictions = model.model.predict(&dataset);
                write_scalar_predictions(values, target_col, predictions.iter().copied());
                Ok(())
            }
            MLModel::SVMMultiClass(model) => {
                let mult_class = MultiClassModel::from_iter(model.model.clone());
                let predictions = mult_class.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::GaussianNaiveBayes(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::DecisionTree(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::LogisticRegression(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::MultinomialLogisticRegression(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::MultinomialNaiveBayes(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::RandomForest(model) => {
                let predictions: Array1<usize> = model.model.0.predict(dataset.records());
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::AdaBoost(model) => {
                let predictions: Array1<usize> = model.model.0.predict(dataset.records());
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::GaussianMixture(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(values, target_col, predictions.iter().copied(), None)
            }
            MLModel::ElasticNet(model) => {
                let predictions = model.model.predict(&dataset);
                write_scalar_predictions(values, target_col, predictions.iter().copied());
                Ok(())
            }
            MLModel::TweedieRegressor(model) => {
                let predictions = model.model.predict(&dataset);
                write_scalar_predictions(values, target_col, predictions.iter().copied());
                Ok(())
            }
            MLModel::SVMRegression(model) => {
                let predictions = model.model.predict(&dataset);
                write_scalar_predictions(values, target_col, predictions.iter().copied());
                Ok(())
            }
            MLModel::OneClassSVM(model) => {
                let predictions = model.model.predict(&dataset);
                // One-class SVM answers "is this an inlier?"; surface it as 1 / 0.
                write_scalar_predictions(
                    values,
                    target_col,
                    predictions
                        .iter()
                        .map(|inlier| if *inlier { 1.0 } else { 0.0 }),
                );
                Ok(())
            }
            MLModel::KnnClassifier(model) => {
                let predictions = dataset
                    .records()
                    .rows()
                    .into_iter()
                    .map(|row| model.model.predict_class(row.as_slice().unwrap_or(&[])))
                    .collect::<Result<Vec<_>>>()?;
                write_class_predictions(
                    values,
                    target_col,
                    predictions.into_iter().map(|(class, _)| class),
                    model.classes.as_ref(),
                )
            }
            MLModel::KnnRegressor(model) => {
                let predictions = dataset
                    .records()
                    .rows()
                    .into_iter()
                    .map(|row| model.model.predict_value(row.as_slice().unwrap_or(&[])))
                    .collect::<Result<Vec<_>>>()?;
                write_scalar_predictions(values, target_col, predictions);
                Ok(())
            }
            MLModel::OrdinalLogistic(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::OrdinalRidge(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::OrdinalFrankHall(model) => {
                let predictions = model.model.predict_levels(dataset.records());
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::OrdinalContinuationRatio(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::OrdinalAdjacentCategory(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::OrdinalNeural(model) => {
                let predictions = model.model.predict(&dataset);
                write_class_predictions(
                    values,
                    target_col,
                    predictions.iter().copied(),
                    model.classes.as_ref(),
                )
            }
            MLModel::FeatureScaler(_) | MLModel::TfIdfVectorizer(_) => Err(anyhow!(
                "{} is a transformer and has no prediction path",
                self.label()
            )),
        }
    }

    /// Applies a transformer model, writing one vector per row into `target_col`.
    ///
    /// This is the counterpart to [`MLModel::predict_on_values`] for models that reshape features
    /// rather than predicting a target.
    pub fn transform_on_values(
        &self,
        values: &mut [Value],
        record_col: &str,
        target_col: &str,
    ) -> Result<()> {
        match self {
            MLModel::FeatureScaler(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = model.model.transform(DatasetBase::from(array));
                write_vector_outputs(values, target_col, dataset.records());
                Ok(())
            }
            MLModel::TfIdfVectorizer(model) => {
                let documents: Vec<String> = values
                    .iter()
                    .enumerate()
                    .map(|(row, value)| {
                        value
                            .get(record_col)
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| {
                                anyhow!("Row {row}: expected a string in column `{record_col}`")
                            })
                    })
                    .collect::<Result<_>>()?;
                let sparse = model
                    .model
                    .transform(&Array1::from(documents))
                    .map_err(|e| anyhow!("TF-IDF transform failed: {e}"))?;
                let dense = sparse.to_dense();
                write_vector_outputs(values, target_col, &dense);
                Ok(())
            }
            other => Err(anyhow!(
                "{} is not a transformer. Use the Predict node instead of Apply Transform.",
                other.label()
            )),
        }
    }

    pub(crate) fn predict_on_vector(&self, vector: Vec<f64>) -> Result<MLPrediction> {
        if self.is_transformer() {
            return Err(anyhow!(
                "{} produces a vector per row and cannot be used with Predict. Use the Apply Transform node instead.",
                self.label()
            ));
        }

        self.ensure_feature_width(vector.len())?;
        let array = Array2::from_shape_vec((1, vector.len()), vector)?;
        let dataset = DatasetBase::from(array);

        match self {
            MLModel::KMeans(model) => Ok(MLPrediction {
                score: first_class_id(&model.model.predict(&dataset))? as f64,
                class: None,
                confidence: None, // Could compute distance to centroid in future
            }),
            MLModel::LinearRegression(model) => Ok(MLPrediction {
                score: first_scalar(&model.model.predict(&dataset))?,
                class: None,
                confidence: None, // Regression doesn't have confidence
            }),
            MLModel::ElasticNet(model) => Ok(MLPrediction {
                score: first_scalar(&model.model.predict(&dataset))?,
                class: None,
                confidence: None,
            }),
            MLModel::TweedieRegressor(model) => Ok(MLPrediction {
                score: first_scalar(&model.model.predict(&dataset))?,
                class: None,
                confidence: None,
            }),
            MLModel::SVMRegression(model) => Ok(MLPrediction {
                score: first_scalar(&model.model.predict(&dataset))?,
                class: None,
                confidence: None,
            }),
            MLModel::SVMMultiClass(model) => {
                let sample = dataset.records().row(0);
                let (predicted_class, confidence) = model
                    .model
                    .iter()
                    .map(|(class_id, classifier)| {
                        let probability = classifier.predict(sample.to_owned());
                        (*class_id, f64::from(*probability))
                    })
                    .max_by(|(_, left), (_, right)| {
                        left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                class_prediction(predicted_class, model.classes.as_ref(), Some(confidence))
            }
            MLModel::GaussianNaiveBayes(model) => class_prediction(
                first_class_id(&model.model.predict(&dataset))?,
                model.classes.as_ref(),
                None,
            ),
            MLModel::MultinomialNaiveBayes(model) => class_prediction(
                first_class_id(&model.model.predict(&dataset))?,
                model.classes.as_ref(),
                None,
            ),
            MLModel::DecisionTree(model) => class_prediction(
                first_class_id(&model.model.predict(&dataset))?,
                model.classes.as_ref(),
                None,
            ),
            MLModel::RandomForest(model) => {
                let predictions: Array1<usize> = model.model.0.predict(dataset.records());
                class_prediction(first_class_id(&predictions)?, model.classes.as_ref(), None)
            }
            MLModel::AdaBoost(model) => {
                let predictions: Array1<usize> = model.model.0.predict(dataset.records());
                class_prediction(first_class_id(&predictions)?, model.classes.as_ref(), None)
            }
            MLModel::LogisticRegression(model) => {
                let probabilities = model.model.predict_probabilities(dataset.records());
                let predicted = first_class_id(&model.model.predict(&dataset))?;
                // `predict_probabilities` always reports the positive class, while `predict` picks
                // a side using the fitted threshold. Taking `max(p, 1-p)` would therefore report
                // the losing class's probability whenever the threshold is not 0.5, so the side is
                // chosen from the label that actually won.
                let confidence = probabilities.first().map(|positive| {
                    if predicted == model.model.labels().pos.class {
                        *positive
                    } else {
                        1.0 - positive
                    }
                });
                class_prediction(predicted, model.classes.as_ref(), confidence)
            }
            MLModel::MultinomialLogisticRegression(model) => {
                let probabilities = model.model.predict_probabilities(dataset.records());
                let confidence = max_probability(probabilities.row(0));
                class_prediction(
                    first_class_id(&model.model.predict(&dataset))?,
                    model.classes.as_ref(),
                    confidence,
                )
            }
            MLModel::GaussianMixture(model) => {
                let responsibilities = model.model.predict_proba(dataset.records());
                Ok(MLPrediction {
                    score: first_class_id(&model.model.predict(&dataset))? as f64,
                    class: None,
                    confidence: max_probability(responsibilities.row(0)),
                })
            }
            MLModel::OneClassSVM(model) => {
                let predictions = model.model.predict(&dataset);
                let inlier = *predictions
                    .first()
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                Ok(MLPrediction {
                    score: if inlier { 1.0 } else { 0.0 },
                    class: Some(if inlier { "inlier" } else { "outlier" }.to_string()),
                    confidence: None,
                })
            }
            MLModel::KnnClassifier(model) => {
                let row = dataset.records().row(0);
                let (class, confidence) =
                    model.model.predict_class(row.as_slice().unwrap_or(&[]))?;
                class_prediction(class, model.classes.as_ref(), Some(confidence))
            }
            MLModel::KnnRegressor(model) => {
                let row = dataset.records().row(0);
                Ok(MLPrediction {
                    score: model.model.predict_value(row.as_slice().unwrap_or(&[]))?,
                    class: None,
                    confidence: None,
                })
            }
            MLModel::OrdinalLogistic(model) => {
                let predicted = first_class_id(&model.model.predict(&dataset))?;
                // The winning level's own probability, which the proportional-odds model exposes
                // directly — unlike the ridge variant below.
                let confidence = model
                    .model
                    .predict_probabilities(&dataset.records().row(0))
                    .ok()
                    .and_then(|probabilities| probabilities.get(predicted).copied());
                class_prediction(predicted, model.classes.as_ref(), confidence)
            }
            MLModel::OrdinalRidge(model) => {
                // Rank regression yields a position on a latent scale, not a probability.
                class_prediction(
                    first_class_id(&model.model.predict(&dataset))?,
                    model.classes.as_ref(),
                    None,
                )
            }
            MLModel::OrdinalFrankHall(model) => {
                // Hard votes across the K-1 binary models, so there is no probability to report.
                let predictions = model.model.predict_levels(dataset.records());
                class_prediction(first_class_id(&predictions)?, model.classes.as_ref(), None)
            }
            MLModel::OrdinalContinuationRatio(model) => {
                let predicted = first_class_id(&model.model.predict(&dataset))?;
                let confidence = model
                    .model
                    .predict_probabilities(&dataset.records().row(0))
                    .ok()
                    .and_then(|p| p.get(predicted).copied());
                class_prediction(predicted, model.classes.as_ref(), confidence)
            }
            MLModel::OrdinalAdjacentCategory(model) => {
                let predicted = first_class_id(&model.model.predict(&dataset))?;
                let confidence = model
                    .model
                    .predict_probabilities(&dataset.records().row(0))
                    .ok()
                    .and_then(|p| p.get(predicted).copied());
                class_prediction(predicted, model.classes.as_ref(), confidence)
            }
            MLModel::OrdinalNeural(model) => {
                let predicted = first_class_id(&model.model.predict(&dataset))?;
                let confidence = model
                    .model
                    .predict_probabilities(&dataset.records().row(0))
                    .ok()
                    .and_then(|p| p.get(predicted).copied());
                class_prediction(predicted, model.classes.as_ref(), confidence)
            }
            MLModel::FeatureScaler(_) | MLModel::TfIdfVectorizer(_) => Err(anyhow!(
                "{} is a transformer and has no prediction path",
                self.label()
            )),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct MLPrediction {
    /// The predicted value (cluster ID for clustering, regression value, or class ID)
    pub score: f64,
    /// The predicted class name (for classification with class mappings)
    pub class: Option<String>,
    /// Confidence score (0.0-1.0) when the model exposes one.
    /// For SVM: the winning one-vs-all probability.
    /// For Logistic Regression: the predicted class's probability.
    /// For Gaussian Mixture: the responsibility of the winning component.
    /// For KNN classification: the winning class's share of the neighbour weight.
    /// `None` for regressors, for KMeans, and for the tree and Naive Bayes models, none of which
    /// expose a calibrated score through this path.
    pub confidence: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
/// Unified Machine Learning Model Type on Board Level
pub struct NodeMLModel {
    pub model_ref: String,
}

#[cfg(feature = "execute")]
pub struct NodeMLModelWrapper {
    pub model: Arc<Mutex<MLModel>>,
}

#[cfg(feature = "execute")]
impl Cacheable for NodeMLModelWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
impl NodeMLModel {
    pub async fn new(ctx: &mut ExecutionContext, model: MLModel) -> Self {
        let id = create_id();
        let model_ref = Arc::new(Mutex::new(model));
        let wrapper = NodeMLModelWrapper {
            model: model_ref.clone(),
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));
        NodeMLModel { model_ref: id }
    }

    pub async fn get_model(&self, ctx: &mut ExecutionContext) -> Result<Arc<Mutex<MLModel>>> {
        let model = ctx
            .cache
            .read()
            .await
            .get(&self.model_ref)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("MLModel not found in cache!"))?;
        let model_wrapper = model
            .as_any()
            .downcast_ref::<NodeMLModelWrapper>()
            .ok_or_else(|| flow_like_types::anyhow!("Could not downcast to NodeMLModelWrapper"))?;
        let model = model_wrapper.model.clone();
        Ok(model)
    }
}

// -----------------------------------
// Utility fns to map Lance Vec<Values> to ndarrays
// TODO: can we merge these using generic types to avoid code duplication for identical behavior?
// -----------------------------------

/// For a column `attr` in Vec<Values> attempt to load all rows as Array2<f64> assuming that `attr` is a FixedSizeList of Vec<f64>
pub fn values_to_array2_f64(values: &[Value], attr: &str) -> Result<Array2<f64>, Error> {
    // Determine dimensions
    let rows = values.len();
    let cols = values
        .first()
        .and_then(|value| value.get(attr))
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .ok_or_else(|| anyhow!("Row 0: expected object with key `{attr}`"))?;

    // Preallocate flat storage
    let mut flat = Vec::with_capacity(rows * cols);

    for (r, value) in values.iter().enumerate() {
        let arr = value.get(attr).and_then(|v| v.as_array()).ok_or_else(|| {
            anyhow!("Row {r}: expected object with key `{attr}`, got `{value:?}`")
        })?;

        if arr.len() != cols {
            return Err(anyhow!(
                "Row {r}: inconsistent length (expected {cols}, got {})",
                arr.len()
            ));
        }

        for (j, x) in arr.iter().enumerate() {
            flat.push(
                x.as_f64()
                    .ok_or_else(|| anyhow!("Row {r}, col {j}: failed to load as f64"))?,
            );
        }
    }
    Ok(Array2::from_shape_vec((rows, cols), flat)?)
}

/// For a column `attr` in Vec<Values> attempt to load all rows as Array1<f64>
pub fn values_to_array1_f64(values: &[Value], attr: &str) -> Result<Array1<f64>> {
    let mut flat = Vec::with_capacity(values.len());
    for (r, value) in values.iter().enumerate() {
        let v = value.get(attr).ok_or_else(|| {
            anyhow!("Row {r}: expected object with key `{attr}`, got `{value:?}`")
        })?;
        flat.push(
            v.as_f64()
                .ok_or_else(|| anyhow!("Row {r}: failed to load as f64"))?,
        );
    }
    Ok(Array1::from(flat))
}

/// For a column `attr` in Vec<Values> attempt to load all rows as Array1<usize>
/// We are assuming that the col contains Strings which we map to unique ids
pub fn values_to_array1_usize(
    values: &[Value],
    attr: &str,
) -> Result<(Array1<usize>, HashMap<usize, String>)> {
    let mut flat = Vec::with_capacity(values.len());
    let mut name_to_id: HashMap<String, usize> = HashMap::new();
    let mut id_to_name: HashMap<usize, String> = HashMap::new();
    let mut next_id = 0;

    for (r, value) in values.iter().enumerate() {
        let v = value.get(attr).ok_or_else(|| {
            anyhow!("Row {r}: expected object with key `{attr}`, got `{value:?}`")
        })?;
        let s = v
            .as_str()
            .ok_or_else(|| anyhow!("Row {r}: failed to load `{attr}` as string, got `{v:?}`"))?;

        // assing new class id or reuse existing
        let id = *name_to_id.entry(s.to_string()).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id_to_name.insert(id, s.to_string());
            id
        });

        flat.push(id);
    }
    Ok((Array1::from(flat), id_to_name))
}

/// Class id per row, plus the id → label map when the target column was categorical.
pub type ClassificationTarget = (Array1<usize>, Option<HashMap<usize, String>>);

/// Auto-detect target column type and convert to Array1<usize> for classification.
///
/// Supports:
/// - String (categorical) → mapped to unique usize IDs, returns class mapping
/// - Integer (u64/i64) → used directly as class IDs, no mapping returned
/// - Float → error (not supported for classification targets)
pub fn values_to_array1_target(values: &[Value], attr: &str) -> Result<ClassificationTarget> {
    if values.is_empty() {
        return Err(anyhow!("Cannot infer target type from empty dataset"));
    }

    // Detect type from first non-null value
    let first_val = values
        .first()
        .and_then(|v| v.get(attr))
        .ok_or_else(|| anyhow!("Row 0: expected object with key `{attr}`"))?;

    match first_val {
        Value::String(_) => {
            // Categorical: use existing string→usize mapping
            let (arr, mapping) = values_to_array1_usize(values, attr)?;
            Ok((arr, Some(mapping)))
        }
        Value::Number(n) if n.is_u64() || n.is_i64() => {
            // Numeric class IDs: use directly
            let mut flat = Vec::with_capacity(values.len());
            for (r, value) in values.iter().enumerate() {
                let v = value.get(attr).ok_or_else(|| {
                    anyhow!("Row {r}: expected object with key `{attr}`, got `{value:?}`")
                })?;
                let id = if let Some(u) = v.as_u64() {
                    u as usize
                } else if let Some(i) = v.as_i64() {
                    if i < 0 {
                        return Err(anyhow!(
                            "Row {r}: negative class ID {i} not allowed for classification"
                        ));
                    }
                    i as usize
                } else {
                    return Err(anyhow!("Row {r}: failed to parse `{attr}` as integer"));
                };
                flat.push(id);
            }
            Ok((Array1::from(flat), None))
        }
        Value::Number(_) => Err(anyhow!(
            "Target column `{attr}` contains floats. Classification requires categorical (string) or integer targets."
        )),
        other => Err(anyhow!(
            "Unsupported target type for column `{attr}`: {:?}",
            other
        )),
    }
}

/// Constant term of the polynomial kernel, which linfa evaluates as `(<x, x'> + c)^degree`.
pub const POLYNOMIAL_KERNEL_CONSTANT: f64 = 1.0;

/// Largest polynomial degree the SVM nodes accept.
///
/// Kernel values grow as `(<x, x'> + 1)^degree`, so even degree 30 spans roughly 27 orders of
/// magnitude on unit-scale features and the solve stops being meaningful long before that. The
/// practical range for a polynomial SVM is 2-5.
pub const MAX_POLYNOMIAL_DEGREE: f64 = 10.0;

/// Validates a polynomial-kernel degree for the SVM family.
///
/// linfa computes the kernel with `powf` (linfa-kernel/src/lib.rs), and `powf` returns NaN for ANY
/// non-integer exponent once the base is negative — and the base here is `<x, x'> + 1`, which goes
/// negative for any pair of rows whose inner product is below -1. That is routine for centred or
/// standardized features. A single NaN kernel entry then either panics inside `Pr::new` during the
/// classifier's Platt scaling, or passes silently: SVR emits NaN (serialized as null) and one-class
/// SVM marks every row an outlier. Restricting the degree to a small whole number closes both.
///
/// Shared by the three SVM nodes so the rule cannot drift between them.
pub fn validate_polynomial_degree(degree: f64) -> Result<()> {
    if !degree.is_finite() || degree < 1.0 {
        return Err(anyhow!(
            "Polynomial kernel degree must be a whole number of at least 1, got {degree}"
        ));
    }
    if degree.fract() != 0.0 {
        return Err(anyhow!(
            "Polynomial kernel degree must be a whole number, got {degree}. A fractional degree makes the kernel NaN for any pair of rows whose inner product is below -1, which silently corrupts the model."
        ));
    }
    if degree > MAX_POLYNOMIAL_DEGREE {
        return Err(anyhow!(
            "Polynomial kernel degree {degree} is too large (maximum {MAX_POLYNOMIAL_DEGREE}); the kernel would span more than 20 orders of magnitude. Typical values are 2 to 5. Note that Kernel Parameter defaults to 30, which is a Gaussian width — set a real degree after switching the kernel to Polynomial."
        ));
    }
    Ok(())
}

/// How the level order of an ordinal target column was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OrdinalOrdering {
    /// Every distinct label parsed as a number, so the order is the numeric one.
    Numeric,
    /// The caller supplied the order explicitly.
    Explicit,
}

/// The ordered levels of an ordinal target.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalLevels {
    /// Level labels from lowest to highest; the index is the rank the model was trained on.
    pub labels: Vec<String>,
    /// Where the ordering came from.
    pub ordering: OrdinalOrdering,
}

/// Canonical text form of a target cell, used as the level label.
fn ordinal_label(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Numeric reading of a target cell, if it has one.
///
/// Integers are tried before floats so that a column of `"1"`, `"2"`, `"10"` orders as 1 < 2 < 10
/// rather than lexicographically. A float parse is the fallback, which also covers `"1.5"` and
/// scientific notation.
fn ordinal_numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().filter(|v| v.is_finite()),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(integer) = trimmed.parse::<i64>() {
                return Some(integer as f64);
            }
            trimmed.parse::<f64>().ok().filter(|v| v.is_finite())
        }
        _ => None,
    }
}

/// Converts an ordinal target column into ranks `0..n_levels`.
///
/// Ordinal models need to know which level is *higher*, and that ordering cannot be guessed from
/// unordered labels — sorting `"low"`, `"medium"`, `"high"` alphabetically would silently train the
/// model on `high < low < medium`. So the order is resolved in exactly two ways:
///
/// 1. `explicit_order` is supplied, and is authoritative.
/// 2. Otherwise every distinct label must read as a number (an integer or a float, whether it is
///    stored as a JSON number or as a string), and the numeric order is used.
///
/// Anything else is an error naming the labels found, rather than a guess.
///
/// Returns the ranks, the rank-to-label mapping used elsewhere in this catalog as
/// `ModelWithMeta::classes`, and the resolved levels.
pub fn values_to_array1_ordinal(
    values: &[Value],
    attr: &str,
    explicit_order: Option<&[String]>,
) -> Result<(Array1<usize>, HashMap<usize, String>, OrdinalLevels)> {
    if values.is_empty() {
        return Err(anyhow!("Cannot infer ordinal levels from an empty dataset"));
    }

    let mut labels: Vec<String> = Vec::with_capacity(values.len());
    for (row, value) in values.iter().enumerate() {
        let cell = value
            .get(attr)
            .ok_or_else(|| anyhow!("Row {row}: expected object with key `{attr}`"))?;
        let label = ordinal_label(cell).ok_or_else(|| {
            anyhow!(
                "Row {row}: `{attr}` holds {cell:?}, expected a string, number or boolean level"
            )
        })?;
        if label.is_empty() {
            return Err(anyhow!(
                "Row {row}: `{attr}` is empty, which is not a level"
            ));
        }
        labels.push(label);
    }

    // Explicit order wins, and every observed label has to appear in it.
    if let Some(order) = explicit_order.filter(|order| !order.is_empty()) {
        let ordered: Vec<String> = order.iter().map(|level| level.trim().to_string()).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        for level in &ordered {
            if !seen.insert(level.as_str()) {
                return Err(anyhow!(
                    "Class order lists `{level}` more than once; each level must appear exactly once"
                ));
            }
        }
        if ordered.len() < 2 {
            return Err(anyhow!(
                "Class order needs at least 2 levels, got {}",
                ordered.len()
            ));
        }

        let rank_of: HashMap<&str, usize> = ordered
            .iter()
            .enumerate()
            .map(|(rank, level)| (level.as_str(), rank))
            .collect();

        let mut ranks = Vec::with_capacity(labels.len());
        for (row, label) in labels.iter().enumerate() {
            let rank = rank_of.get(label.as_str()).ok_or_else(|| {
                anyhow!(
                    "Row {row}: level `{label}` in `{attr}` is not listed in the class order [{}]",
                    ordered.join(", ")
                )
            })?;
            ranks.push(*rank);
        }

        let mapping: HashMap<usize, String> = ordered
            .iter()
            .enumerate()
            .map(|(rank, level)| (rank, level.clone()))
            .collect();
        return Ok((
            Array1::from(ranks),
            mapping,
            OrdinalLevels {
                labels: ordered,
                ordering: OrdinalOrdering::Explicit,
            },
        ));
    }

    // Otherwise fall back to numeric ordering, which requires every label to parse.
    let mut numeric_of: HashMap<String, f64> = HashMap::new();
    let mut unparsed: Vec<String> = Vec::new();
    for (row, value) in values.iter().enumerate() {
        let cell = value
            .get(attr)
            .ok_or_else(|| anyhow!("Row {row}: expected object with key `{attr}`"))?;
        let label = &labels[row];
        if numeric_of.contains_key(label) {
            continue;
        }
        match ordinal_numeric(cell) {
            Some(number) => {
                numeric_of.insert(label.clone(), number);
            }
            None => {
                if !unparsed.contains(label) {
                    unparsed.push(label.clone());
                }
            }
        }
    }

    if !unparsed.is_empty() {
        unparsed.sort();
        unparsed.truncate(12);
        return Err(anyhow!(
            "Ordinal levels in `{attr}` are not numeric ({}), so their order cannot be inferred. Supply the level order explicitly, lowest first.",
            unparsed.join(", ")
        ));
    }

    let mut distinct: Vec<(String, f64)> = numeric_of.into_iter().collect();
    distinct.sort_by(|(left_label, left), (right_label, right)| {
        left.partial_cmp(right)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Two spellings of the same number (`1` and `1.0`) would otherwise order arbitrarily.
            .then_with(|| left_label.cmp(right_label))
    });

    if distinct.len() < 2 {
        return Err(anyhow!(
            "Ordinal models need at least 2 distinct levels in `{attr}`, found {}",
            distinct.len()
        ));
    }

    let rank_of: HashMap<&str, usize> = distinct
        .iter()
        .enumerate()
        .map(|(rank, (label, _))| (label.as_str(), rank))
        .collect();
    let ranks: Vec<usize> = labels.iter().map(|label| rank_of[label.as_str()]).collect();

    let ordered: Vec<String> = distinct.into_iter().map(|(label, _)| label).collect();
    let mapping: HashMap<usize, String> = ordered
        .iter()
        .enumerate()
        .map(|(rank, level)| (rank, level.clone()))
        .collect();

    Ok((
        Array1::from(ranks),
        mapping,
        OrdinalLevels {
            labels: ordered,
            ordering: OrdinalOrdering::Numeric,
        },
    ))
}

/// Infer Schema of New Columns to be added to Lance Tables
/// We map the JSON type of value.attr to a corresponding Arrow type
pub fn make_new_field(value: &Value, attr: &str) -> Result<Field> {
    if let Some(v) = value.get(attr) {
        match v {
            Value::Number(n) if n.is_f64() => Ok(Field::new(attr, DataType::Float64, true)),
            Value::Number(n) if n.is_u64() => Ok(Field::new(attr, DataType::UInt64, true)),
            Value::Number(n) if n.is_i64() => Ok(Field::new(attr, DataType::Int64, true)),
            Value::String(_) => Ok(Field::new(attr, DataType::LargeUtf8, true)),
            other => Err(anyhow!("Unknown type for attr `{}`: {:?}", attr, other)),
        }
    } else {
        Err(anyhow!("Attr `{}` missing in Value", attr))
    }
}

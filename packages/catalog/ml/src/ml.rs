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
use std::collections::HashMap;

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, create_id, json::json, sync::Mutex};
#[cfg(feature = "execute")]
use linfa::composing::MultiClassModel;
#[cfg(feature = "execute")]
use linfa::prelude::Pr;
#[cfg(feature = "execute")]
use linfa::{DatasetBase, traits::Predict};
#[cfg(feature = "execute")]
use linfa_bayes::GaussianNb;
#[cfg(feature = "execute")]
use linfa_clustering::KMeans;
#[cfg(feature = "execute")]
use linfa_linear::FittedLinearRegression;
#[cfg(feature = "execute")]
use linfa_nn::distance::L2Dist;
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
pub mod prediction;
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
}

#[cfg(feature = "execute")]
impl fmt::Display for MLModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MLModel::KMeans(_) => write!(f, "KMeans Clustering"),
            MLModel::LinearRegression(_) => write!(f, "Linear Regression"),
            MLModel::SVMMultiClass(_) => write!(f, "SVM Classification (Multiple Classes)"),
            MLModel::GaussianNaiveBayes(_) => write!(f, "Gaussian Naive Bayes Classification"),
            MLModel::DecisionTree(_) => write!(f, "Decision Tree Classification"),
        }
    }
}

#[cfg(feature = "execute")]
impl MLModel {
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

        let model_type = match self {
            MLModel::KMeans(_) => "KMeans",
            MLModel::SVMMultiClass(_) => "SVMMultiClass",
            MLModel::LinearRegression(_) => "LinearRegression",
            MLModel::GaussianNaiveBayes(_) => "GaussianNaiveBayes",
            MLModel::DecisionTree(_) => "DecisionTree",
        };

        // Use MessagePack for fast, compact inner serialization
        let msgpack_payload = rmp_serde::to_vec(self)
            .map_err(|e| anyhow!("MessagePack serialization failed: {}", e))?;

        let wrapper = MLModelWrapper {
            version: 1,
            model_type: model_type.to_string(),
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

        if wrapper.version != 1 {
            return Err(anyhow!(
                "Unsupported MLModel binary format version: {}",
                wrapper.version
            ));
        }

        // Deserialize the MessagePack payload back to MLModel
        let model: MLModel = rmp_serde::from_slice(&wrapper.msgpack_payload)
            .map_err(|e| anyhow!("MessagePack deserialization failed: {}", e))?;
        Ok(model)
    }

    pub fn predict_on_values(
        &self,
        values: &mut Vec<Value>,
        record_col: &str,
        target_col: &str,
    ) -> Result<()> {
        match self {
            MLModel::KMeans(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                for (value, pred) in values.iter_mut().zip(predictions.iter()) {
                    if let Value::Object(map) = value {
                        map.insert(target_col.to_string(), json!(pred));
                    }
                }
                Ok(())
            }
            MLModel::LinearRegression(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                for (value, pred) in values.iter_mut().zip(predictions.iter()) {
                    if let Value::Object(map) = value {
                        map.insert(target_col.to_string(), json!(pred));
                    }
                }
                Ok(())
            }
            MLModel::SVMMultiClass(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = DatasetBase::from(array);
                let mult_class = MultiClassModel::from_iter(model.model.clone());
                let predictions = mult_class.predict(&dataset);
                for (value, pred) in values.iter_mut().zip(predictions.iter()) {
                    if let Value::Object(map) = value {
                        if let Some(classes) = &model.classes {
                            let class = classes.get(pred).ok_or_else(|| {
                                anyhow!(format!(
                                    "Couldn't map prediction {} to any of these classes {:?}",
                                    pred, classes
                                ))
                            })?;
                            map.insert(target_col.to_string(), json!(class))
                        } else {
                            map.insert(target_col.to_string(), json!(pred))
                        };
                    }
                }
                Ok(())
            }
            MLModel::GaussianNaiveBayes(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                for (value, pred) in values.iter_mut().zip(predictions.iter()) {
                    if let Value::Object(map) = value {
                        if let Some(classes) = &model.classes {
                            let class = classes.get(pred).ok_or_else(|| {
                                anyhow!(format!(
                                    "Couldn't map prediction {} to any of these classes {:?}",
                                    pred, classes
                                ))
                            })?;
                            map.insert(target_col.to_string(), json!(class))
                        } else {
                            map.insert(target_col.to_string(), json!(pred))
                        };
                    }
                }
                Ok(())
            }
            MLModel::DecisionTree(model) => {
                let array = values_to_array2_f64(values, record_col)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                for (value, pred) in values.iter_mut().zip(predictions.iter()) {
                    if let Value::Object(map) = value {
                        if let Some(classes) = &model.classes {
                            let class = classes.get(pred).ok_or_else(|| {
                                anyhow!(format!(
                                    "Couldn't map prediction {} to any of these classes {:?}",
                                    pred, classes
                                ))
                            })?;
                            map.insert(target_col.to_string(), json!(class))
                        } else {
                            map.insert(target_col.to_string(), json!(pred))
                        };
                    }
                }
                Ok(())
            }
        }
    }

    pub(crate) fn predict_on_vector(&self, vector: Vec<f64>) -> Result<MLPrediction> {
        match self {
            MLModel::KMeans(model) => {
                let array = Array2::from_shape_vec((1, vector.len()), vector)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                let cluster_id = *predictions
                    .first()
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                Ok(MLPrediction {
                    score: cluster_id as f64,
                    class: None,
                    confidence: None, // Could compute distance to centroid in future
                })
            }
            MLModel::LinearRegression(model) => {
                let array = Array2::from_shape_vec((1, vector.len()), vector)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                let score = *predictions
                    .first()
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                Ok(MLPrediction {
                    score,
                    class: None,
                    confidence: None, // Regression doesn't have confidence
                })
            }
            MLModel::SVMMultiClass(model) => {
                let array = Array2::from_shape_vec((1, vector.len()), vector)?;
                let sample = array.row(0);
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

                let class = if let Some(classes) = &model.classes {
                    Some(classes.get(&predicted_class).ok_or_else(|| {
                        anyhow!(format!(
                            "Couldn't map prediction {} to any of these classes {:?}",
                            predicted_class, classes
                        ))
                    })?)
                } else {
                    None
                };
                Ok(MLPrediction {
                    score: predicted_class as f64,
                    class: class.cloned(),
                    confidence: Some(confidence),
                })
            }
            MLModel::GaussianNaiveBayes(model) => {
                let array = Array2::from_shape_vec((1, vector.len()), vector)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                let predicted_class = *predictions
                    .first()
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                let class = if let Some(classes) = &model.classes {
                    Some(classes.get(&predicted_class).ok_or_else(|| {
                        anyhow!(format!(
                            "Couldn't map prediction {} to any of these classes {:?}",
                            predicted_class, classes
                        ))
                    })?)
                } else {
                    None
                };
                Ok(MLPrediction {
                    score: predicted_class as f64,
                    class: class.cloned(),
                    confidence: None, // Naive Bayes doesn't expose probabilities easily
                })
            }
            MLModel::DecisionTree(model) => {
                let array = Array2::from_shape_vec((1, vector.len()), vector)?;
                let dataset = DatasetBase::from(array);
                let predictions = model.model.predict(&dataset);
                let predicted_class = *predictions
                    .first()
                    .ok_or_else(|| anyhow!("Got an empty prediction"))?;
                let class = if let Some(classes) = &model.classes {
                    Some(classes.get(&predicted_class).ok_or_else(|| {
                        anyhow!(format!(
                            "Couldn't map prediction {} to any of these classes {:?}",
                            predicted_class, classes
                        ))
                    })?)
                } else {
                    None
                };
                Ok(MLPrediction {
                    score: predicted_class as f64,
                    class: class.cloned(),
                    confidence: None, // Decision tree doesn't expose probability
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct MLPrediction {
    /// The predicted value (cluster ID for clustering, regression value, or class ID)
    pub score: f64,
    /// The predicted class name (for classification with class mappings)
    pub class: Option<String>,
    /// Confidence score (0.0-1.0) when available
    /// For KMeans: inverse of distance to centroid (normalized)
    /// For SVM: winning one-vs-all probability
    /// For Linear Regression: None (no confidence for regression)
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

/// Auto-detect target column type and convert to Array1<usize> for classification.
///
/// Supports:
/// - String (categorical) → mapped to unique usize IDs, returns class mapping
/// - Integer (u64/i64) → used directly as class IDs, no mapping returned
/// - Float → error (not supported for classification targets)
pub fn values_to_array1_target(
    values: &[Value],
    attr: &str,
) -> Result<(Array1<usize>, Option<HashMap<usize, String>>)> {
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

//! Node for extracting feature importance from tree models
//!
//! Supports a single Decision Tree as well as the tree ensembles (Random Forest, AdaBoost),
//! where the per-tree importances are folded back onto the columns of the original training matrix.

use crate::ml::NodeMLModel;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree as LinfaDecisionTree;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Importance of a single input feature (column of the training matrix)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureImportance {
    /// Position of the feature inside the training vector
    pub index: usize,
    /// Label of the feature, taken from the Feature Names input or generated as `feature_<index>`
    pub name: String,
    /// Relative impurity decrease attributed to this feature, normalized so all entries sum to 1.0
    pub importance: f64,
    /// Absolute mean impurity decrease over the splits that used this feature
    pub mean_impurity_decrease: f64,
    /// Number of trees whose feature subset contained this column
    pub trees_using: usize,
}

/// Feature importance of a fitted tree model
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureImportanceResult {
    /// Model kind the importances were computed from
    pub model_type: String,
    /// Number of feature columns covered by the report
    pub n_features: usize,
    /// Number of trees that contributed
    pub n_trees: usize,
    /// How the per-tree importances were combined
    pub aggregation: String,
    /// One entry per feature, in column order
    pub features: Vec<FeatureImportance>,
    /// Column indices ordered by importance, most important first
    pub ranking: Vec<usize>,
    /// Total number of leaves over all contributing trees
    pub num_leaves: usize,
    /// Deepest node depth over all contributing trees
    pub max_depth: usize,
}

/// Folds per-tree importances onto the columns of the original training matrix.
#[cfg(feature = "execute")]
struct ImportanceAccumulator {
    importance: Vec<f64>,
    impurity: Vec<f64>,
    trees_using: Vec<usize>,
    weight_total: f64,
    num_leaves: usize,
    max_depth: usize,
    non_finite: bool,
}

#[cfg(feature = "execute")]
impl ImportanceAccumulator {
    fn new(n_features: usize) -> Self {
        Self {
            importance: vec![0.0; n_features],
            impurity: vec![0.0; n_features],
            trees_using: vec![0; n_features],
            weight_total: 0.0,
            num_leaves: 0,
            max_depth: 0,
            non_finite: false,
        }
    }

    /// Adds one tree. `columns` maps the tree's positional importances back to the columns of the
    /// original matrix; `None` means the tree was trained on every column.
    fn add_tree(
        &mut self,
        tree: &LinfaDecisionTree<f64, usize>,
        columns: Option<&[usize]>,
        weight: f64,
    ) -> Result<()> {
        let importance = tree.feature_importance();
        let impurity = tree.mean_impurity_decrease();
        if let Some(columns) = columns
            && columns.len() != importance.len()
        {
            return Err(anyhow!(
                "Ensemble tree reports {} features but its feature subset lists {} columns",
                importance.len(),
                columns.len()
            ));
        }

        let n_features = self.importance.len();
        // linfa samples each tree's feature subset WITH replacement, so the same column can appear
        // twice in one subset. Counting positions would then report a column as used by more trees
        // than exist, so distinct columns are tallied once per tree.
        let mut seen: HashSet<usize> = HashSet::new();
        for (position, (relative, absolute)) in importance
            .iter()
            .copied()
            .zip(impurity.iter().copied())
            .enumerate()
        {
            let column = columns.map_or(position, |columns| columns[position]);
            if column >= n_features {
                return Err(anyhow!(
                    "Tree references feature column {column}, but only {n_features} columns were derived from the model"
                ));
            }

            // `relative_impurity_decrease` divides by the summed impurity decrease, so a tree that
            // was pruned back to a single leaf yields 0/0 = NaN for every feature.
            let relative = self.finite_or_zero(relative);
            let absolute = self.finite_or_zero(absolute);

            self.importance[column] += weight * relative;
            self.impurity[column] += weight * absolute;
            seen.insert(column);
        }
        for column in seen {
            self.trees_using[column] += 1;
        }

        self.weight_total += weight;
        self.num_leaves += tree.num_leaves();
        self.max_depth = self.max_depth.max(tree.max_depth());
        Ok(())
    }

    fn finite_or_zero(&mut self, value: f64) -> f64 {
        if value.is_finite() {
            value
        } else {
            self.non_finite = true;
            0.0
        }
    }

    fn into_result(
        self,
        model_type: &str,
        n_trees: usize,
        aggregation: String,
        names: &[String],
    ) -> FeatureImportanceResult {
        let divisor = if self.weight_total > 0.0 {
            self.weight_total
        } else {
            1.0
        };
        let mut importance: Vec<f64> = self.importance.iter().map(|v| v / divisor).collect();
        let total: f64 = importance.iter().sum();
        if total > 0.0 {
            for value in importance.iter_mut() {
                *value /= total;
            }
        }

        let features: Vec<FeatureImportance> = importance
            .iter()
            .enumerate()
            .map(|(index, value)| FeatureImportance {
                index,
                name: names
                    .get(index)
                    .filter(|name| !name.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("feature_{index}")),
                importance: *value,
                mean_impurity_decrease: self.impurity[index] / divisor,
                trees_using: self.trees_using[index],
            })
            .collect();

        let mut ranking: Vec<usize> = (0..features.len()).collect();
        ranking.sort_by(|a, b| {
            features[*b]
                .importance
                .partial_cmp(&features[*a].importance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });

        FeatureImportanceResult {
            model_type: model_type.to_string(),
            n_features: features.len(),
            n_trees,
            aggregation,
            features,
            ranking,
            num_leaves: self.num_leaves,
            max_depth: self.max_depth,
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetFeatureImportanceNode {}

impl GetFeatureImportanceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetFeatureImportanceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_feature_importance",
            "Feature Importance",
            "Extract per-feature importance from a Decision Tree, Random Forest or AdaBoost model",
            "AI/ML/Model Info",
        );
        node.set_flowscript_name("ml", "featureImportance");
        node.set_receiver("model");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Trained tree model (Decision Tree, Random Forest or AdaBoost)",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "feature_names",
            "Feature Names",
            "Optional column labels in training order. Unnamed columns fall back to feature_<index>.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the importances are computed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Importance",
            "Per-feature importance with leaf and depth statistics",
            VariableType::Struct,
        )
        .set_schema::<FeatureImportanceResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "importances",
            "Scores",
            "Normalized importance per feature, in column order",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "top_feature",
            "Top Feature",
            "Name of the most important feature",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::MLModel;

        context.deactivate_exec_pin("exec_out").await?;

        let feature_names: Vec<String> = context
            .evaluate_pin("feature_names")
            .await
            .unwrap_or_default();
        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let model_arc = node_model.get_model(context).await?;
        let model = model_arc.lock().await;

        let mut warnings: Vec<String> = Vec::new();
        let (accumulator, n_trees, aggregation) = match &*model {
            MLModel::DecisionTree(wrapper) => {
                let n_features = wrapper.model.feature_importance().len();
                if n_features == 0 {
                    return Err(anyhow!(
                        "Decision tree reports zero features; it was not trained on any column"
                    ));
                }
                let mut accumulator = ImportanceAccumulator::new(n_features);
                accumulator.add_tree(&wrapper.model, None, 1.0)?;
                (accumulator, 1, "single tree".to_string())
            }
            MLModel::RandomForest(wrapper) => {
                let ensemble = &wrapper.model.0;
                if ensemble.models.is_empty() {
                    return Err(anyhow!("Random forest contains no trees"));
                }
                if ensemble.models.len() != ensemble.model_features.len() {
                    return Err(anyhow!(
                        "Random forest holds {} trees but {} feature subsets; the model is inconsistent",
                        ensemble.models.len(),
                        ensemble.model_features.len()
                    ));
                }

                // Each tree is fitted on a random SUBSET of columns, so its positional importances
                // index into `model_features[i]`, not into the original matrix. The forest never
                // stores the original column count, so it is recovered from the highest column any
                // tree selected — trailing columns no tree ever picked stay invisible unless the
                // caller widens the report by passing feature names.
                let derived = ensemble
                    .model_features
                    .iter()
                    .flatten()
                    .copied()
                    .max()
                    .map(|column| column + 1)
                    .unwrap_or(0);
                let n_features = derived.max(feature_names.len());
                if n_features == 0 {
                    return Err(anyhow!(
                        "Random forest trees were trained without any features"
                    ));
                }
                if feature_names.len() > derived {
                    warnings.push(format!(
                        "No tree selected columns {}..{}; they are reported with zero importance",
                        derived,
                        n_features - 1
                    ));
                }

                let mut accumulator = ImportanceAccumulator::new(n_features);
                for (tree, columns) in ensemble.models.iter().zip(ensemble.model_features.iter()) {
                    accumulator.add_tree(tree, Some(columns.as_slice()), 1.0)?;
                }
                let n_trees = ensemble.models.len();
                (accumulator, n_trees, format!("mean over {n_trees} trees"))
            }
            MLModel::AdaBoost(wrapper) => {
                let boosted = &wrapper.model.0;
                if boosted.models.is_empty() {
                    return Err(anyhow!("AdaBoost model contains no trees"));
                }
                if boosted.models.len() != boosted.model_weights.len() {
                    return Err(anyhow!(
                        "AdaBoost holds {} trees but {} weights; the model is inconsistent",
                        boosted.models.len(),
                        boosted.model_weights.len()
                    ));
                }

                // Every boosting round is fitted on the full matrix, so positions are already
                // column indices. Rounds are weighted by their alpha, matching scikit-learn.
                let weight_sum: f64 = boosted.model_weights.iter().sum();
                let weighted = boosted
                    .model_weights
                    .iter()
                    .all(|weight| weight.is_finite() && *weight >= 0.0)
                    && weight_sum > 0.0;
                if !weighted {
                    warnings.push(
                        "AdaBoost round weights are non-finite or sum to zero; falling back to an unweighted mean"
                            .to_string(),
                    );
                }

                let n_features = boosted.models[0].feature_importance().len();
                if n_features == 0 {
                    return Err(anyhow!(
                        "AdaBoost trees report zero features; the model was not trained on any column"
                    ));
                }
                let mut accumulator = ImportanceAccumulator::new(n_features);
                for (tree, weight) in boosted.models.iter().zip(boosted.model_weights.iter()) {
                    let weight = if weighted { *weight } else { 1.0 };
                    accumulator.add_tree(tree, None, weight)?;
                }
                let n_trees = boosted.models.len();
                let aggregation = if weighted {
                    format!("alpha-weighted mean over {n_trees} trees")
                } else {
                    format!("mean over {n_trees} trees")
                };
                (accumulator, n_trees, aggregation)
            }
            other => {
                return Err(anyhow!(
                    "{} has no feature importance. Only tree models expose it: Decision Tree, Random Forest and AdaBoost.",
                    other.label()
                ));
            }
        };

        if accumulator.non_finite {
            warnings.push(
                "At least one tree produced a non-finite impurity decrease (it never split); those contributions were counted as zero"
                    .to_string(),
            );
        }

        let model_type = model.kind();
        let result = accumulator.into_result(model_type, n_trees, aggregation, &feature_names);
        drop(model);

        if !feature_names.is_empty() && feature_names.len() != result.n_features {
            warnings.push(format!(
                "{} feature names supplied for {} features",
                feature_names.len(),
                result.n_features
            ));
        }
        for warning in &warnings {
            context.log_message(warning.as_str(), LogLevel::Warn);
        }

        let importances: Vec<f64> = result
            .features
            .iter()
            .map(|feature| feature.importance)
            .collect();
        let top_feature = result
            .ranking
            .first()
            .and_then(|index| result.features.get(*index))
            .map(|feature| feature.name.clone())
            .unwrap_or_default();

        context.log_message(
            &format!(
                "{} feature importance over {} feature(s), {} ({} leaves, max depth {})",
                model_type,
                result.n_features,
                result.aggregation,
                result.num_leaves,
                result.max_depth
            ),
            LogLevel::Debug,
        );

        context.set_pin_value("result", json!(result)).await?;
        context
            .set_pin_value("importances", json!(importances))
            .await?;
        context
            .set_pin_value("top_feature", json!(top_feature))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}

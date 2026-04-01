//! Node for computing a Confusion Matrix
//!
//! Builds a confusion matrix from predictions vs actuals and computes
//! precision, recall, and F1 score metrics.

use crate::ml::ConfusionMatrixResult;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

#[crate::register_node]
#[derive(Default)]
pub struct ConfusionMatrixNode {}

impl ConfusionMatrixNode {
    pub fn new() -> Self {
        ConfusionMatrixNode {}
    }
}

#[async_trait]
impl NodeLogic for ConfusionMatrixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_eval_confusion_matrix",
            "Confusion Matrix",
            "Build confusion matrix and calculate precision, recall, and F1 score",
            "AI/ML/Metrics",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start confusion matrix calculation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "database",
            "Database",
            "Database connection containing predictions and actuals",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "predictions_col",
            "Predictions Column",
            "Column name containing predicted values",
            VariableType::String,
        )
        .set_default_value(Some(json!("prediction")));

        node.add_input_pin(
            "actuals_col",
            "Actuals Column",
            "Column name containing actual/true values",
            VariableType::String,
        )
        .set_default_value(Some(json!("target")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once confusion matrix calculation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Confusion matrix with precision, recall, and F1 metrics",
            VariableType::Struct,
        )
        .set_schema::<ConfusionMatrixResult>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::MAX_ML_PREDICTION_RECORDS;
        use flow_like::flow::execution::LogLevel;
        use flow_like_types::anyhow;

        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let predictions_col: String = context.evaluate_pin("predictions_col").await?;
        let actuals_col: String = context.evaluate_pin("actuals_col").await?;

        let records = {
            let cached_db = database.load(context).await?;
            cached_db.ensure_flushed().await?;
            let database = cached_db.db.read().await;
            let schema = database.schema().await?;
            let existing_cols: HashSet<String> =
                schema.fields.iter().map(|f| f.name().clone()).collect();

            if !existing_cols.contains(&predictions_col) {
                return Err(anyhow!(
                    "Database doesn't contain predictions column `{}`!",
                    predictions_col
                ));
            }
            if !existing_cols.contains(&actuals_col) {
                return Err(anyhow!(
                    "Database doesn't contain actuals column `{}`!",
                    actuals_col
                ));
            }

            database
                .filter(
                    "true",
                    Some(vec![predictions_col.clone(), actuals_col.clone()]),
                    MAX_ML_PREDICTION_RECORDS,
                    0,
                )
                .await?
        };

        if records.is_empty() {
            return Err(anyhow!("No records found in database"));
        }

        // Collect all unique labels
        let mut labels_set: HashSet<String> = HashSet::new();
        for record in &records {
            if let Some(pred) = record.get(&predictions_col) {
                labels_set.insert(value_to_string(pred));
            }
            if let Some(actual) = record.get(&actuals_col) {
                labels_set.insert(value_to_string(actual));
            }
        }

        let mut labels: Vec<String> = labels_set.into_iter().collect();
        labels.sort();

        let label_to_idx: HashMap<String, usize> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), i))
            .collect();

        let n_classes = labels.len();

        // Build confusion matrix: matrix[actual][predicted]
        let mut matrix: Vec<Vec<usize>> = vec![vec![0; n_classes]; n_classes];

        for record in &records {
            let pred = record
                .get(&predictions_col)
                .map(value_to_string)
                .unwrap_or_default();
            let actual = record
                .get(&actuals_col)
                .map(value_to_string)
                .unwrap_or_default();

            if let (Some(&actual_idx), Some(&pred_idx)) =
                (label_to_idx.get(&actual), label_to_idx.get(&pred))
            {
                matrix[actual_idx][pred_idx] += 1;
            }
        }

        // Calculate per-class metrics
        let mut precisions: Vec<f64> = Vec::with_capacity(n_classes);
        let mut recalls: Vec<f64> = Vec::with_capacity(n_classes);
        let mut supports: Vec<usize> = Vec::with_capacity(n_classes);

        for class_idx in 0..n_classes {
            let true_positive = matrix[class_idx][class_idx];
            let false_positive: usize = (0..n_classes)
                .filter(|&i| i != class_idx)
                .map(|i| matrix[i][class_idx])
                .sum();
            let false_negative: usize = (0..n_classes)
                .filter(|&j| j != class_idx)
                .map(|j| matrix[class_idx][j])
                .sum();

            let precision = if true_positive + false_positive > 0 {
                true_positive as f64 / (true_positive + false_positive) as f64
            } else {
                0.0
            };

            let recall = if true_positive + false_negative > 0 {
                true_positive as f64 / (true_positive + false_negative) as f64
            } else {
                0.0
            };

            let support: usize = matrix[class_idx].iter().sum();

            precisions.push(precision);
            recalls.push(recall);
            supports.push(support);
        }

        let total_support: usize = supports.iter().sum();

        // Weighted averages
        let weighted_precision: f64 = if total_support > 0 {
            precisions
                .iter()
                .zip(supports.iter())
                .map(|(p, s)| p * (*s as f64))
                .sum::<f64>()
                / total_support as f64
        } else {
            0.0
        };

        let weighted_recall: f64 = if total_support > 0 {
            recalls
                .iter()
                .zip(supports.iter())
                .map(|(r, s)| r * (*s as f64))
                .sum::<f64>()
                / total_support as f64
        } else {
            0.0
        };

        let f1_score = if weighted_precision + weighted_recall > 0.0 {
            2.0 * weighted_precision * weighted_recall / (weighted_precision + weighted_recall)
        } else {
            0.0
        };

        context.log_message(
            &format!(
                "Confusion Matrix: {}x{} classes, Precision={:.4}, Recall={:.4}, F1={:.4}",
                n_classes, n_classes, weighted_precision, weighted_recall, f1_score
            ),
            LogLevel::Debug,
        );

        let matrix_i64: Vec<Vec<i64>> = matrix
            .iter()
            .map(|row| row.iter().map(|&v| v as i64).collect())
            .collect();
        let result = ConfusionMatrixResult {
            matrix: matrix_i64,
            labels,
            precision: weighted_precision,
            recall: weighted_recall,
            f1_score,
            total_samples: total_support,
        };

        context.set_pin_value("result", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}

#[cfg(feature = "execute")]
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => format!("{}", v),
    }
}

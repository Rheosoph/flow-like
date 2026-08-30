//! Node for evaluating Classification Accuracy
//!
//! Computes the accuracy of predictions by comparing predicted values against actual/true values.
//! Accuracy = (correct predictions) / (total predictions)

use crate::ml::AccuracyMetrics;
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
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct AccuracyNode {}

impl AccuracyNode {
    pub fn new() -> Self {
        AccuracyNode {}
    }
}

#[async_trait]
impl NodeLogic for AccuracyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_eval_accuracy",
            "Accuracy",
            "Calculate classification accuracy by comparing predictions to actual values",
            "AI/ML/Metrics",
        );
        node.set_flowscript_name("ml", "evalAccuracy");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start accuracy calculation",
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
            "Activated once accuracy calculation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Accuracy metrics including score and counts",
            VariableType::Struct,
        )
        .set_schema::<AccuracyMetrics>();

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

        let total_count = records.len();
        if total_count == 0 {
            return Err(anyhow!("No records found in database"));
        }

        let mut correct_count = 0usize;
        for record in &records {
            let pred = record.get(&predictions_col);
            let actual = record.get(&actuals_col);

            if pred == actual {
                correct_count += 1;
            }
        }

        let accuracy = correct_count as f64 / total_count as f64;

        context.log_message(
            &format!(
                "Accuracy: {:.4} ({}/{} correct)",
                accuracy, correct_count, total_count
            ),
            LogLevel::Debug,
        );

        let result = AccuracyMetrics {
            accuracy,
            correct_count,
            total_count,
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

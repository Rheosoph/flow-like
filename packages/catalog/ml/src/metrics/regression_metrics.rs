//! Node for computing Regression Metrics
//!
//! Calculates common regression evaluation metrics:
//! - MSE (Mean Squared Error)
//! - RMSE (Root Mean Squared Error)
//! - MAE (Mean Absolute Error)
//! - R² (Coefficient of Determination)

use crate::ml::RegressionMetrics;
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
pub struct RegressionMetricsNode {}

impl RegressionMetricsNode {
    pub fn new() -> Self {
        RegressionMetricsNode {}
    }
}

#[async_trait]
impl NodeLogic for RegressionMetricsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_eval_regression",
            "Regression Metrics",
            "Calculate MSE, RMSE, MAE, and R² for regression predictions",
            "AI/ML/Metrics",
        );
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
            "Execution trigger to start regression metrics calculation",
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
            "Column name containing predicted float values",
            VariableType::String,
        )
        .set_default_value(Some(json!("prediction")));

        node.add_input_pin(
            "actuals_col",
            "Actuals Column",
            "Column name containing actual/true float values",
            VariableType::String,
        )
        .set_default_value(Some(json!("target")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once regression metrics calculation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Regression metrics (MSE, RMSE, MAE, R²)",
            VariableType::Struct,
        )
        .set_schema::<RegressionMetrics>();

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

        let mut predictions: Vec<f64> = Vec::with_capacity(records.len());
        let mut actuals: Vec<f64> = Vec::with_capacity(records.len());

        for (i, record) in records.iter().enumerate() {
            let pred = record
                .get(&predictions_col)
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    anyhow!(
                        "Row {}: predictions column `{}` is not a valid float",
                        i,
                        predictions_col
                    )
                })?;

            let actual = record
                .get(&actuals_col)
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    anyhow!(
                        "Row {}: actuals column `{}` is not a valid float",
                        i,
                        actuals_col
                    )
                })?;

            predictions.push(pred);
            actuals.push(actual);
        }

        let n = predictions.len() as f64;

        // MSE: Mean Squared Error
        let mse: f64 = predictions
            .iter()
            .zip(actuals.iter())
            .map(|(p, a)| (p - a).powi(2))
            .sum::<f64>()
            / n;

        // RMSE: Root Mean Squared Error
        let rmse = mse.sqrt();

        // MAE: Mean Absolute Error
        let mae: f64 = predictions
            .iter()
            .zip(actuals.iter())
            .map(|(p, a)| (p - a).abs())
            .sum::<f64>()
            / n;

        // R²: Coefficient of Determination
        let mean_actual: f64 = actuals.iter().sum::<f64>() / n;
        let ss_tot: f64 = actuals.iter().map(|a| (a - mean_actual).powi(2)).sum();
        let ss_res: f64 = predictions
            .iter()
            .zip(actuals.iter())
            .map(|(p, a)| (a - p).powi(2))
            .sum();

        let r2 = if ss_tot > 0.0 {
            1.0 - (ss_res / ss_tot)
        } else {
            0.0
        };

        context.log_message(
            &format!(
                "Regression Metrics: MSE={:.6}, RMSE={:.6}, MAE={:.6}, R²={:.6}",
                mse, rmse, mae, r2
            ),
            LogLevel::Debug,
        );

        let result = RegressionMetrics {
            mse,
            rmse,
            mae,
            r2,
            n_samples: predictions.len(),
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

//! Node for extracting Linear Regression coefficients
//!
//! Returns the coefficients and intercept from a trained Linear Regression model.

use crate::ml::{LinearCoefficients, NodeMLModel};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::json::json;
use flow_like_types::{Result, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct GetLinearCoefficientsNode {}

impl GetLinearCoefficientsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetLinearCoefficientsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_get_linear_coefficients",
            "Get Coefficients",
            "Extract coefficients and intercept from a trained Linear Regression model",
            "AI/ML/Model Info",
        );
        node.set_flowscript_name("ml", "getLinearCoefficients");
        node.set_receiver("model");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10) // Only extracts math params, no raw data
                .set_security(10) // Pure computation, no external calls
                .set_performance(9)
                .set_governance(9)
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
            "Trained Linear Regression model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated when coefficients are extracted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Coefficients",
            "Regression coefficients with intercept",
            VariableType::Struct,
        )
        .set_schema::<LinearCoefficients>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::MLModel;

        context.deactivate_exec_pin("exec_out").await?;

        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let model_arc = node_model.get_model(context).await?;
        let model = model_arc.lock().await;

        match &*model {
            MLModel::LinearRegression(linreg) => {
                let params = linreg.model.params();
                let intercept = linreg.model.intercept();
                let coefficients: Vec<f64> = params.to_vec();
                let n_features = coefficients.len();

                let result = crate::ml::LinearCoefficients {
                    coefficients,
                    intercept,
                    n_features,
                };

                context.log_message(
                    &format!(
                        "Extracted {} coefficients, intercept = {:.4}",
                        n_features, intercept
                    ),
                    LogLevel::Debug,
                );

                context.set_pin_value("result", json!(result)).await?;
                context.activate_exec_pin("exec_out").await?;
                Ok(())
            }
            other => Err(flow_like_types::anyhow!(
                "Expected LinearRegression model, got {}",
                other
            )),
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}

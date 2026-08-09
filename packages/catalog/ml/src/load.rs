//! Node for Deserializing a MLModel from JSON path
//!
//! Deserializes a previously trained and saved MLModel JSON file as the matching MLModel variant.
//! Wraps the MLModel in a cached NodeMLModel.

#[cfg(feature = "execute")]
use crate::ml::MLModel;
use crate::ml::NodeMLModel;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_types::json;
use flow_like_types::{Result, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct LoadMLModelNode {}

impl LoadMLModelNode {
    pub fn new() -> Self {
        LoadMLModelNode {}
    }
}

#[async_trait]
impl NodeLogic for LoadMLModelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "load_ml_model",
            "Load Model",
            "Load Trained ML Model from Path",
            "AI/ML",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts loading the model JSON",
            VariableType::Execution,
        );

        node.add_input_pin(
            "path",
            "Path JSON",
            "Filesystem or storage path pointing at the serialized model JSON",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the model is loaded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Handle to the loaded machine learning model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        // deserialize model
        let bytes = path.get(context, false).await?;
        let ml_model: MLModel = json::from_slice(&bytes)?;
        context.log_message(
            &format!("Loaded Machine Learning Model: {}", ml_model),
            LogLevel::Debug,
        );

        // wrap model + set outputs
        let node_model = NodeMLModel::new(context, ml_model).await;
        context
            .set_pin_value("model", json::json!(node_model))
            .await?;
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

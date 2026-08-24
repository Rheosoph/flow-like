//! Node for Deserializing a MLModel from Fory Binary Format
//!
//! Deserializes a previously trained and saved MLModel binary file as the matching MLModel variant.
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
use flow_like_types::{Result, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct LoadMLModelBinaryNode {}

impl LoadMLModelBinaryNode {
    pub fn new() -> Self {
        LoadMLModelBinaryNode {}
    }
}

#[async_trait]
impl NodeLogic for LoadMLModelBinaryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "load_ml_model_binary",
            "Load Model (Binary)",
            "Load Trained ML Model from Path using fast binary format (Fory)",
            "AI/ML",
        );
        node.set_flowscript_name("ml", "loadBinary");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(9) // Higher performance than JSON
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts loading the model binary",
            VariableType::Execution,
        );

        node.add_input_pin(
            "path",
            "Path",
            "Filesystem or storage path pointing at the serialized model binary (.flmodel)",
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
        use flow_like_types::json;

        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        // mirror the extension Save Model (Binary) writes, so the same path works on both nodes
        let path = path.set_extension(context, "flmodel").await?;

        // deserialize model from Fory binary
        let bytes = path.get(context, false).await?;
        let ml_model = MLModel::from_fory_slice(&bytes)?;
        context.log_message(
            &format!("Loaded Machine Learning Model (Binary): {}", ml_model),
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

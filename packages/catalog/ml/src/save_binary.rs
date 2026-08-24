//! Node for Serializing Trained MLModels as Fory Binary Format
//!
//! Serializes MLModels using Apache Fory for fast, schema-safe binary serialization.
//! Uses .flmodel extension.

use crate::ml::NodeMLModel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Result, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct SaveMLModelBinaryNode {}

impl SaveMLModelBinaryNode {
    pub fn new() -> Self {
        SaveMLModelBinaryNode {}
    }
}

#[async_trait]
impl NodeLogic for SaveMLModelBinaryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "save_ml_model_binary",
            "Save Model (Binary)",
            "Save Trained ML Model to Path using fast binary format (Fory)",
            "AI/ML",
        );
        node.set_flowscript_name("ml", "saveBinary");
        node.set_receiver("model");
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
            "Execution trigger that begins serialization",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Any trained ML model handle to persist",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "path",
            "Path",
            "Destination path where the model binary should be written (.flmodel)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the model file is written",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        // set extension to .flmodel
        let path = path.set_extension(context, "flmodel").await?;

        // serialize model using Fory
        let bytes = {
            let model = node_model.get_model(context).await?;
            let model_guard = model.lock().await;
            model_guard.to_fory_vec()?
        };

        // write
        path.put(context, bytes, false).await?;

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

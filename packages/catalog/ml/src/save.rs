//! Node for Serializing Trained MLModels as JSONs
//!
//! Serializes MLModels as JSONs and writes to a specified path.

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
pub struct SaveMLModelNode {}

impl SaveMLModelNode {
    pub fn new() -> Self {
        SaveMLModelNode {}
    }
}

#[async_trait]
impl NodeLogic for SaveMLModelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "save_ml_model",
            "Save Model",
            "Save Trained ML Model to Path",
            "AI/ML",
        );
        node.set_flowscript_name("ml", "save");
        node.set_receiver("model");
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
            "Path JSON",
            "Destination path where the model JSON should be written",
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
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        // set extension
        let path = path.set_extension(context, "json").await?;

        // serialize model
        let bytes = {
            let model = node_model.get_model(context).await?;
            let model_guard = model.lock().await;
            model_guard.to_json_vec()?
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

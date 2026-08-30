use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FromRawPathNode {}

impl FromRawPathNode {
    pub fn new() -> Self {
        FromRawPathNode {}
    }
}

#[async_trait]
impl NodeLogic for FromRawPathNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "from_raw_path",
            "From Raw Path",
            "Reconstructs a FlowPath from a raw path string using the store reference from a base path",
            "Data/Files/Path",
        );
        node.set_flowscript_name("path", "fromRawPath");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "base_path",
            "Base Path",
            "FlowPath to get the store reference from",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "raw_path",
            "Raw Path",
            "The raw path string to reconstruct",
            VariableType::String,
        );

        node.add_output_pin(
            "path",
            "Path",
            "Reconstructed FlowPath",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base_path: FlowPath = context.evaluate_pin("base_path").await?;
        let raw_path: String = context.evaluate_pin("raw_path").await?;

        let reconstructed = FlowPath::new(raw_path, base_path.store_ref, base_path.cache_store_ref);

        context.set_pin_value("path", json!(reconstructed)).await?;
        Ok(())
    }
}

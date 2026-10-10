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
pub struct IsFileNode {}

impl IsFileNode {
    pub fn new() -> Self {
        IsFileNode {}
    }
}

#[async_trait]
impl NodeLogic for IsFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_is_file",
            "Is File",
            "Checks whether a FlowPath is a file. Returns false if the path is missing.",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "isFile");
        node.set_receiver("path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "is_file",
            "Is File",
            "True if the path is a file",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        let store = path.to_store(context).await?;
        let is_file = store.is_file(&path.object_path()).await?;

        context.set_pin_value("is_file", json!(is_file)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

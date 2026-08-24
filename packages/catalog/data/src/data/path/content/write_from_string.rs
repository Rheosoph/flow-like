use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct WriteStringNode {}

impl WriteStringNode {
    pub fn new() -> Self {
        WriteStringNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "write_string",
            "Write String",
            "Writes a string to a file",
            "Data/Files/Content",
        );
        node.set_flowscript_name("files", "writeString");
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

        node.add_input_pin(
            "content",
            "Content",
            "The content to write as a string",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let path: FlowPath = context.evaluate_pin("path").await?;
        let content: String = context.evaluate_pin("content").await?;

        path.put(context, content.into_bytes(), false).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

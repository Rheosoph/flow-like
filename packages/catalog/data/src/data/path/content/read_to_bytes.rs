use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ReadToBytesNode {}

impl ReadToBytesNode {
    pub fn new() -> Self {
        ReadToBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadToBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_to_bytes",
            "Read to Bytes",
            "Reads the content of a file Fto bytes",
            "Data/Files/Content",
        );
        node.set_flowscript_name("files", "readToBytes");
        node.set_receiver("path");
        node.add_icon("/flow/icons/path.svg"); // Consider a more appropriate icon

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
            "content",
            "Content",
            "The content of the file as bytes",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let path: FlowPath = context.evaluate_pin("path").await?;

        let bytes = path.get(context, false).await?;

        context.set_pin_value("content", json!(bytes)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct WriteBytesNode {}

impl WriteBytesNode {
    pub fn new() -> Self {
        WriteBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "write_bytes",
            "Write Bytes",
            "Writes bytes to a file",
            "Data/Files/Content",
        );
        node.set_flowscript_name("files", "writeBytes");
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

        node.add_input_pin(
            "content",
            "Content",
            "The content to write as bytes",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

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
        let content: Vec<u8> = context.evaluate_pin("content").await?;

        path.put(context, content, false).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

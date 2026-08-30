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
pub struct PutNode {}

impl PutNode {
    pub fn new() -> Self {
        PutNode {}
    }
}

#[async_trait]
impl NodeLogic for PutNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_put",
            "Put",
            "Writes bytes to a file",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "put");
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

        node.add_input_pin("bytes", "Bytes", "Bytes to write", VariableType::Byte)
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
        let bytes_vec: Vec<u8> = context.evaluate_pin("bytes").await?;

        path.put(context, bytes_vec, false).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::path::PathBuf;

use crate::data::path::FlowPath;

#[crate::register_node]
#[derive(Default)]
pub struct PathBufToPathNode {}

impl PathBufToPathNode {
    pub fn new() -> Self {
        PathBufToPathNode {}
    }
}

#[async_trait]
impl NodeLogic for PathBufToPathNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pathbuf_to_path",
            "Local Path to Path",
            "Converts a PathBuf to a Path",
            "Data/Files",
        );
        node.set_flowscript_name("path", "fromLocal");
        node.set_receiver("");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "pathbuf",
            "Local Path",
            "Input PathBuf",
            VariableType::PathBuf,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("path", "Path", "Output Path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pathbuf: PathBuf = context.evaluate_pin("pathbuf").await?;

        context
            .execution_environment()
            .ensure_host_filesystem_access("Local Path to Path")?;

        let path = FlowPath::from_pathbuf(pathbuf, context).await?;
        context.set_pin_value("path", json!(path)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

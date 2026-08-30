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
pub struct PathExistsNode {}

impl PathExistsNode {
    pub fn new() -> Self {
        PathExistsNode {}
    }
}

#[async_trait]
impl NodeLogic for PathExistsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_exists",
            "Path Exists?",
            "Checks if a path exists",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "exists");
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
            "exec_out_exists",
            "Yes",
            "Execution if path exists",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_out_missing",
            "No",
            "Execution if path does not exist",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out_exists").await?;
        context.deactivate_exec_pin("exec_out_missing").await?;

        let path: FlowPath = context.evaluate_pin("path").await?;

        let dynamic = path.to_runtime(context).await?;
        let store = dynamic.store.as_generic();

        let exists = store.head(&dynamic.path).await.is_ok();

        if exists {
            context.deactivate_exec_pin("exec_out_missing").await?;
            context.activate_exec_pin("exec_out_exists").await?;
        } else {
            context.deactivate_exec_pin("exec_out_exists").await?;
            context.activate_exec_pin("exec_out_missing").await?;
        }

        Ok(())
    }
}

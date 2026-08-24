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
pub struct RenameNode {}

impl RenameNode {
    pub fn new() -> Self {
        RenameNode {}
    }
}

#[async_trait]
impl NodeLogic for RenameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_rename",
            "Rename",
            "Renames a file",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "rename");
        node.set_receiver("from");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("from", "From", "Source FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("to", "To", "Destination FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "overwrite",
            "Overwrite",
            "Should the destination file be overwritten?",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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
        let from: FlowPath = context.evaluate_pin("from").await?;
        let to: FlowPath = context.evaluate_pin("to").await?;
        let overwrite: bool = context.evaluate_pin("overwrite").await?;

        let from_path = from.to_runtime(context).await?;
        let to_path = to.to_runtime(context).await?;
        let from_store = from_path.store.as_generic();

        if overwrite {
            from_store.rename(&from_path.path, &to_path.path).await?;
        } else {
            from_store
                .rename_if_not_exists(&from_path.path, &to_path.path)
                .await?;
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

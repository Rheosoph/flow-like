use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::Path;
use flow_like_types::{async_trait, json::json};
use futures::{StreamExt, TryStreamExt};

#[crate::register_node]
#[derive(Default)]
pub struct DeleteNode {}

impl DeleteNode {
    pub fn new() -> Self {
        DeleteNode {}
    }
}

#[async_trait]
impl NodeLogic for DeleteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "storage_delete",
            "Delete",
            "Deletes a file or directory",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "delete");
        node.set_receiver("path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("path", "Path", "Path to delete", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "recursive",
            "Recursive",
            "Delete directories recursively",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Execution if deletion succeeds",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let path: FlowPath = context.evaluate_pin("path").await?;
        let recursive: bool = context.evaluate_pin("recursive").await?;

        let runtime = path.to_runtime(context).await?;
        let generic_store = runtime.store.as_generic();

        if recursive {
            let list = generic_store
                .list(Some(&runtime.path))
                .map_ok(|entry| entry.location)
                .boxed();
            generic_store
                .delete_stream(list)
                .try_collect::<Vec<Path>>()
                .await
                .map_err(|_| flow_like_types::anyhow!("Failed to delete files"))?;
        } else {
            generic_store.delete(&runtime.path).await?;
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

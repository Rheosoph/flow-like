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
pub struct HashFileNode {}

impl HashFileNode {
    pub fn new() -> Self {
        HashFileNode {}
    }
}

#[async_trait]
impl NodeLogic for HashFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_hash_file",
            "Hash File",
            "Hashes a file",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "hashFile");
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

        node.add_output_pin("hash", "Hash", "Output Hash", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        let cache_layer = path.to_cache_layer(context).await?;
        if let Some(cache_layer) = cache_layer {
            let is_dirty = path.is_cache_dirty(context).await;

            if let Err(e) = &is_dirty {
                context.log_message(
                    &format!("Failed to check cache dirty state: {}", e),
                    flow_like::flow::execution::LogLevel::Warn,
                );
            }

            let is_dirty = is_dirty.unwrap_or(true);
            if !is_dirty {
                let path = path.to_runtime(context).await?;
                let hash = cache_layer.hash(&path.path).await?;

                context.set_pin_value("hash", json!(hash)).await?;
                context.activate_exec_pin("exec_out").await?;

                return Ok(());
            }
        }

        let path = path.to_runtime(context).await?;
        let store = path.store;
        let hash = store.hash(&path.path).await?;

        context.set_pin_value("hash", json!(hash)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

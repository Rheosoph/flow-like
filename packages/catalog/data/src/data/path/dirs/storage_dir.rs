use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PathFromStorageDirNode {}

impl PathFromStorageDirNode {
    pub fn new() -> Self {
        PathFromStorageDirNode {}
    }
}

#[async_trait]
impl NodeLogic for PathFromStorageDirNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_from_storage_dir",
            "Storage Dir",
            "Converts the storage directory to a Path",
            "Data/Files/Directories",
        );
        node.set_flowscript_name("files", "storageDir");
        node.add_icon("/flow/icons/path.svg");

        node.add_output_pin("path", "Path", "Output Path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node.add_input_pin(
            "node_scope",
            "Node Scope",
            "Is this node in the node scope?",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let node_scope: bool = context.evaluate_pin("node_scope").await?;

        let path = FlowPath::from_storage_dir(context, node_scope).await?;
        context.set_pin_value("path", json!(path)).await?;
        Ok(())
    }
}

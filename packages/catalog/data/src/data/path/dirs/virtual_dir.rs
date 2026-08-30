use std::sync::Arc;

use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{Cacheable, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct VirtualDirNode {}

impl VirtualDirNode {
    pub fn new() -> Self {
        VirtualDirNode {}
    }
}

#[async_trait]
impl NodeLogic for VirtualDirNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_virtual_dir",
            "Virtual Dir",
            "Creates an in-memory virtual directory path",
            "Data/Files/Directories",
        );
        node.set_flowscript_name("files", "virtualDir");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "name",
            "Name",
            "Virtual directory name",
            VariableType::String,
        )
        .set_default_value(Some(json!("/virtual")));

        node.add_output_pin(
            "path",
            "Path",
            "Virtual directory path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = context.evaluate_pin("name").await?;

        let cache_path = format!("virtual_dir_{}", name);

        if !context.has_cache(&cache_path).await {
            let store = FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ));
            let store: Arc<dyn Cacheable> = Arc::new(store);
            context.set_cache(&cache_path, store).await;
        }

        let virtual_path = FlowPath {
            path: "".to_string(),
            store_ref: cache_path,
            cache_store_ref: None,
        };
        context.set_pin_value("path", json!(virtual_path)).await?;
        Ok(())
    }
}

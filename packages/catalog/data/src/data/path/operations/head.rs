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
pub struct HeadNode {}

impl HeadNode {
    pub fn new() -> Self {
        HeadNode {}
    }
}

#[async_trait]
impl NodeLogic for HeadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_head",
            "Head",
            "Gets the metadata of a file",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "head");
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

        node.add_output_pin("e_tag", "ETag", "Etag", VariableType::String);
        node.add_output_pin(
            "last_modified",
            "Last Modified",
            "Last Modified",
            VariableType::Date,
        );
        node.add_output_pin("size", "Size", "Size", VariableType::Integer);
        node.add_output_pin("version", "Version", "Version", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        let path = path.to_runtime(context).await?;
        let store = path.store.as_generic();
        let metadata = store.head(&path.path).await?;

        context
            .set_pin_value("e_tag", json!(metadata.e_tag.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("last_modified", json!(metadata.last_modified))
            .await?;

        context
            .set_pin_value("size", json!(metadata.size as i64))
            .await?;
        context
            .set_pin_value("version", json!(metadata.version.unwrap_or_default()))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

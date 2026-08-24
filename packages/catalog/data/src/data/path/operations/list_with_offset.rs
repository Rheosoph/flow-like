use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Error, async_trait, json::json};
use futures::StreamExt;

#[crate::register_node]
#[derive(Default)]
pub struct ListWithOffsetNode {}

impl ListWithOffsetNode {
    pub fn new() -> Self {
        ListWithOffsetNode {}
    }
}

#[async_trait]
impl NodeLogic for ListWithOffsetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_list_with_offset",
            "List With Offset",
            "Lists paths in a directory with offset and limit",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "listWithOffset");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("prefix", "Prefix", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("offset", "Offset", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "offset",
            "Offset",
            "Offset to start listing from",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("paths", "Paths", "Output Paths", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let original_prefix: FlowPath = context.evaluate_pin("prefix").await?;
        let original_offset: FlowPath = context.evaluate_pin("offset").await?;

        let prefix = original_prefix.to_runtime(context).await?;
        let offset = original_offset.to_runtime(context).await?;
        let store = prefix.store.as_generic();

        let paths = store
            .list_with_offset(Some(&prefix.path), &offset.path)
            .map(|r| r.map_err(Error::from))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let paths = paths
            .iter()
            .map(|p| {
                let mut new_path = original_prefix.clone();
                new_path.path = p.location.as_ref().to_string();
                new_path
            })
            .collect::<Vec<FlowPath>>();

        context.set_pin_value("paths", json!(paths)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

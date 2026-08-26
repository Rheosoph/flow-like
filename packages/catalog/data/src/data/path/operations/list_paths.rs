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
pub struct ListPathsNode {}

impl ListPathsNode {
    pub fn new() -> Self {
        ListPathsNode {}
    }
}

#[async_trait]
impl NodeLogic for ListPathsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_list_paths",
            "List Paths",
            "Lists all paths in a directory",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "listPaths");
        node.set_receiver("prefix");
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

        node.add_input_pin(
            "recursive",
            "Recursive",
            "List paths recursively",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

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
        let original_path: FlowPath = context.evaluate_pin("prefix").await?;
        let recursive: bool = context.evaluate_pin("recursive").await?;

        let path = original_path.to_runtime(context).await?;
        let store = path.store.as_generic();

        let file_objects = if recursive {
            store
                .list(Some(&path.path))
                .map(|r| r.map_err(Error::from))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let files = store.list_with_delimiter(Some(&path.path)).await?;
            files.objects
        };

        let paths = file_objects
            .iter()
            .map(|p| {
                let mut new_path = original_path.clone();
                new_path.path = p.location.as_ref().to_string();
                new_path
            })
            .collect::<Vec<FlowPath>>();

        context.set_pin_value("paths", json!(paths)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

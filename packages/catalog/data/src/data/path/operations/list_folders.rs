use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Error, async_trait, json::json};
use futures::StreamExt;
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct ListFoldersNode {}

impl ListFoldersNode {
    pub fn new() -> Self {
        ListFoldersNode {}
    }
}

#[async_trait]
impl NodeLogic for ListFoldersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_list_folders",
            "List Folders",
            "Lists folders under a path",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "listFolders");
        node.set_receiver("prefix");
        node.add_icon("/flow/icons/path.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_governance(10)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

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
            "List folders recursively",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("folders", "Folders", "Output Folders", VariableType::Struct)
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
        let prefix = path.path.as_ref();
        let prefix = if prefix.is_empty() {
            "".to_string()
        } else if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{}/", prefix)
        };

        let mut folders = if recursive {
            let file_objects = store
                .list(Some(&path.path))
                .map(|r| r.map_err(Error::from))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            let mut prefixes = HashSet::new();
            for object in file_objects {
                let location = object.location.as_ref();
                if !location.starts_with(&prefix) {
                    continue;
                }

                let remainder = &location[prefix.len()..];
                let parts = remainder
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>();

                if parts.is_empty() {
                    continue;
                }

                let mut end = parts.len();
                if !location.ends_with('/') && end > 0 {
                    end -= 1;
                }

                for i in 0..end {
                    let folder = format!("{}/", parts[..=i].join("/"));
                    prefixes.insert(format!("{}{}", prefix, folder));
                }
            }

            prefixes.into_iter().collect::<Vec<_>>()
        } else {
            let result = store.list_with_delimiter(Some(&path.path)).await?;
            result
                .common_prefixes
                .into_iter()
                .map(|p| p.as_ref().to_string())
                .collect::<Vec<_>>()
        };

        folders.sort();
        let folders = folders
            .into_iter()
            .map(|folder| {
                let mut new_path = original_path.clone();
                new_path.path = folder;
                new_path
            })
            .collect::<Vec<FlowPath>>();

        context.set_pin_value("folders", json!(folders)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

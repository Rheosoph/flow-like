use super::{
    add_exec_out, add_file_output, add_folder_pin, add_item_id_pin, add_scope_pin, add_text_pin,
    add_versions_output,
    client::{LockedAccess, LockedAction, LockedApi},
    locked_node, normalize_folder, optional_text, read_scope, set_detail_outputs,
    types::{GetLockedItemRequest, LockedItemDetail},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, anyhow, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LockedGetItemNode;

impl LockedGetItemNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedGetItemNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_get_item",
            "Get Locked File",
            "Looks up a write-once, revision-safe file by its ID, or by scope, folder and name, with every version and its lock details",
            "get",
            5,
        );
        add_item_id_pin(
            &mut node,
            "ID of the locked file; leave empty to look it up by scope, folder and name",
        );
        add_scope_pin(&mut node);
        add_folder_pin(
            &mut node,
            "Folder of the file when looking it up by name; empty for the root",
        );
        add_text_pin(
            &mut node,
            "name",
            "Name",
            "Name of the file when looking it up without an ID",
        );

        add_exec_out(
            &mut node,
            "The lookup finished, whether or not the file exists",
        );
        add_file_output(&mut node, "The file with its newest locked version");
        add_versions_output(&mut node);
        node.add_output_pin(
            "found",
            "Found",
            "Whether the file exists and is visible to this run",
            VariableType::Boolean,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = optional_text(context.evaluate_pin::<String>("item_id").await?);
        let scope = read_scope(context).await?;
        let folder = normalize_folder(&context.evaluate_pin::<String>("folder").await?);
        let name = optional_text(context.evaluate_pin::<String>("name").await?);
        let request = match (&item_id, &name) {
            (Some(item_id), _) => GetLockedItemRequest {
                item_id: Some(item_id),
                ..Default::default()
            },
            (None, Some(name)) => GetLockedItemRequest {
                item_id: None,
                scope: Some(scope),
                folder: Some(&folder),
                name: Some(name),
            },
            (None, None) => {
                return Err(anyhow!(
                    "Get Locked File needs a Locked File ID or a Name to look up"
                ));
            }
        };
        let api = LockedApi::bind(context, LockedAccess::Read)?;
        let detail: Option<LockedItemDetail> =
            api.find(context, LockedAction::Get, &request).await?;
        match &detail {
            Some(detail) => set_detail_outputs(context, detail).await?,
            None => {
                context.set_pin_value("file", Value::Null).await?;
                context.set_pin_value("versions", json!([])).await?;
            }
        }
        context
            .set_pin_value("found", json!(detail.is_some()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

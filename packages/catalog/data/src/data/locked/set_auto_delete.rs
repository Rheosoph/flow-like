use super::{
    add_exec_out, add_item_id_pin, add_text_pin, add_version_output,
    client::{LockedAccess, LockedAction, LockedApi},
    locked_node, optional_text, read_required, resolve_version_id,
    types::{LockedItemVersion, LockedVersionFlagRequest},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LockedSetAutoDeleteNode;

impl LockedSetAutoDeleteNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedSetAutoDeleteNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_set_auto_delete",
            "Set Locked File Auto Delete",
            "Chooses whether a locked version is deleted automatically once its retention ends and no legal hold remains. The retention itself does not change; project files need app admin rights",
            "setAutoDelete",
            5,
        );
        add_item_id_pin(&mut node, "ID of the locked file");
        add_text_pin(
            &mut node,
            "version_id",
            "Version ID",
            "ID of the version to change; empty uses the newest locked version",
        );
        node.add_input_pin(
            "enabled",
            "Delete After Retention",
            "True deletes the version once its retention ends, false keeps it until someone deletes it; false also cancels a deletion that is due",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        add_exec_out(&mut node, "Automatic deletion was updated");
        add_version_output(&mut node, "The version with its automatic deletion state");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = read_required(context, "item_id", "Locked File ID").await?;
        let version_id = optional_text(context.evaluate_pin::<String>("version_id").await?);
        let enabled = context.evaluate_pin::<bool>("enabled").await?;
        let api = LockedApi::bind(context, LockedAccess::Write)?;
        let version_id = resolve_version_id(context, &api, &item_id, version_id).await?;
        let version: LockedItemVersion = api
            .post(
                context,
                LockedAction::AutoDelete,
                &LockedVersionFlagRequest {
                    item_id: &item_id,
                    version_id: &version_id,
                    enabled,
                },
            )
            .await?;
        context.set_pin_value("version", json!(version)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

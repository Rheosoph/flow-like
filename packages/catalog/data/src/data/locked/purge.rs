use super::{
    add_exec_out, add_file_output, add_item_id_pin, add_text_pin, add_versions_output,
    client::{LockedAccess, LockedAction, LockedApi},
    locked_node, optional_text, read_required, set_detail_outputs,
    types::{LockedItemDetail, LockedVersionRef},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct LockedPurgeNode;

impl LockedPurgeNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedPurgeNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_purge",
            "Delete Expired Locked File",
            "Deletes the stored bytes of a locked file and frees its storage quota. Only works after the retention period ended and without a legal hold; the server refuses while any selected version is still retained. The record itself, including its SHA-256, is kept as evidence",
            "purge",
            5,
        );
        add_item_id_pin(&mut node, "ID of the locked file");
        add_text_pin(
            &mut node,
            "version_id",
            "Version ID",
            "ID of the one version to delete; empty deletes every version of the file",
        );

        add_exec_out(&mut node, "The expired bytes were deleted");
        add_file_output(
            &mut node,
            "The locked file after the deletion, with its remaining stored size",
        );
        add_versions_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = read_required(context, "item_id", "Locked File ID").await?;
        let version_id = optional_text(context.evaluate_pin::<String>("version_id").await?);
        let api = LockedApi::bind(context, LockedAccess::Write)?;
        let detail: LockedItemDetail = api
            .post(
                context,
                LockedAction::Purge,
                &LockedVersionRef {
                    item_id: &item_id,
                    version_id: version_id.as_deref(),
                },
            )
            .await?;
        set_detail_outputs(context, &detail).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

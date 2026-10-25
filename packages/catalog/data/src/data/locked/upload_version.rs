use super::{
    add_exec_out, add_file_output, add_item_id_pin, add_source_pin, add_upload_option_pins,
    add_version_output,
    client::{LockedAccess, LockedApi},
    locked_node, read_required, read_upload_options, set_upload_outputs,
    transfer::upload_locked,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct LockedUploadVersionNode;

impl LockedUploadVersionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedUploadVersionNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_upload_version",
            "Upload Locked File Version",
            "Adds a new write-once version to an existing locked file; earlier versions stay unchanged. Fails when storage is full",
            "uploadVersion",
            6,
        );
        node.set_long_running(true);
        add_item_id_pin(&mut node, "ID of the locked file to add a version to");
        add_source_pin(&mut node);
        add_upload_option_pins(&mut node);

        add_exec_out(&mut node, "The version was uploaded, verified and locked");
        add_file_output(&mut node, "The locked file with the uploaded version");
        add_version_output(&mut node, "The uploaded version with its lock details");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = read_required(context, "item_id", "Locked File ID").await?;
        let path = context.evaluate_pin::<FlowPath>("path").await?;
        let mut fields = read_upload_options(context).await?;
        fields.item_id = Some(item_id);
        let api = LockedApi::bind(context, LockedAccess::Write)?;
        let upload = upload_locked(context, &api, &path, fields).await?;
        set_upload_outputs(context, &upload).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

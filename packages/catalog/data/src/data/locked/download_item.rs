use super::{
    add_exec_out, add_file_output, add_item_id_pin,
    client::{LockedAccess, LockedApi},
    get_detail, locked_node, read_required,
    transfer::download_locked,
    types::LockedFile,
    zero_as_none,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{anyhow, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LockedDownloadItemNode;

impl LockedDownloadItemNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedDownloadItemNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_download_item",
            "Download Locked File",
            "Downloads one version of a write-once, revision-safe file into the run's cache and checks its size and SHA-256 against the record",
            "download",
            6,
        );
        node.set_long_running(true);
        add_item_id_pin(&mut node, "ID of the locked file to download");
        node.add_input_pin(
            "version",
            "Version",
            "Version number to download; 0 downloads the newest locked version",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)))
        .set_options(
            PinOptions::new()
                .set_range((0.0, f64::from(u32::MAX)))
                .build(),
        );

        add_exec_out(
            &mut node,
            "The file was downloaded and its checksum matched",
        );
        node.add_output_pin(
            "path",
            "File",
            "The downloaded bytes in the run's cache",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        add_file_output(&mut node, "The downloaded version with its lock details");
        node.add_output_pin(
            "verified",
            "Verified",
            "True when the storage provider also confirmed the SHA-256 when the version was locked; the downloaded bytes always match the recorded SHA-256",
            VariableType::Boolean,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = read_required(context, "item_id", "Locked File ID").await?;
        let requested = zero_as_none("Version", context.evaluate_pin::<i64>("version").await?)?;
        let api = LockedApi::bind(context, LockedAccess::Read)?;
        let detail = get_detail(context, &api, &item_id).await?;
        let version = match requested {
            None => detail
                .latest_locked()
                .ok_or_else(|| anyhow!("Locked file '{item_id}' has no locked version"))?,
            Some(number) => detail
                .version_by_number(number)
                .ok_or_else(|| anyhow!("Locked file '{item_id}' has no version {number}"))?,
        };
        let path = download_locked(context, &api, version).await?;
        context.set_pin_value("path", json!(path)).await?;
        context
            .set_pin_value("file", json!(LockedFile::new(&detail.item, Some(version))))
            .await?;
        context
            .set_pin_value("verified", json!(version.checksum_verified))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

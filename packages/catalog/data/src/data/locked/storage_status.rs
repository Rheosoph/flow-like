use super::{
    add_exec_out,
    client::{LockedAccess, LockedApi},
    locked_node,
    types::LockedStorageStatus,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LockedStorageStatusNode;

impl LockedStorageStatusNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedStorageStatusNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_storage_status",
            "Locked Storage Status",
            "Reads whether this server offers write-once, revision-safe storage and its retention limits",
            "status",
            5,
        );
        add_exec_out(&mut node, "The status was read");
        node.add_output_pin(
            "status",
            "Status",
            "Provider, enforcement, retention bounds and size limit of locked storage",
            VariableType::Struct,
        )
        .set_schema::<LockedStorageStatus>();
        node.add_output_pin(
            "enabled",
            "Enabled",
            "Whether locked storage is configured on this server",
            VariableType::Boolean,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api = LockedApi::bind(context, LockedAccess::Read)?;
        let status = api.status(context).await?;
        context
            .set_pin_value("enabled", json!(status.enabled))
            .await?;
        context.set_pin_value("status", json!(status)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

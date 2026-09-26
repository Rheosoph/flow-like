#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// # Drop Graph Overlay
/// Deletes a graph overlay definition (metadata only, does not drop underlying tables).
#[crate::register_node]
#[derive(Default)]
pub struct DropGraphOverlayNode {}

impl DropGraphOverlayNode {
    pub fn new() -> Self {
        DropGraphOverlayNode {}
    }
}

#[async_trait]
impl NodeLogic for DropGraphOverlayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "drop_graph_overlay",
            "Drop Graph Overlay",
            "Deletes a graph overlay definition (does not drop underlying tables)",
            "Data/Database/Graph",
        );
        node.set_flowscript_name("db.graph", "dropOverlay");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "overlay_id",
            "Overlay ID",
            "ID of the overlay to delete",
            VariableType::String,
        );
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "Delete from user-scoped database",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Deleted",
            "Overlay deleted successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to delete overlay",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let overlay_id: String = context.evaluate_pin("overlay_id").await?;
        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let connection =
            crate::data::db::vector::connection::open_shared(context, user_scoped).await?;

        match lancegraph::delete_overlay(&connection, &overlay_id).await {
            Ok(()) => {
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(
                    &format!("Database graph-overlay delete failed: {e:#}"),
                    LogLevel::Error,
                );
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}

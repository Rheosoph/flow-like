use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// # List Graph Overlays
/// Lists all graph overlays in the current database scope.
#[crate::register_node]
#[derive(Default)]
pub struct ListGraphOverlaysNode {}

impl ListGraphOverlaysNode {
    pub fn new() -> Self {
        ListGraphOverlaysNode {}
    }
}

#[async_trait]
impl NodeLogic for ListGraphOverlaysNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "list_graph_overlays",
            "List Graph Overlays",
            "Lists all graph overlay definitions in the database",
            "Data/Database/Graph/Meta",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "List overlays from user-scoped database",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done listing overlays",
            VariableType::Execution,
        );
        node.add_output_pin(
            "overlay_ids",
            "Overlay IDs",
            "List of overlay IDs",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "overlay_names",
            "Overlay Names",
            "List of overlay names",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph;

        context.deactivate_exec_pin("exec_out").await?;

        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let context_cache = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let app_id = context_cache.app_id.clone();

        let db = if let Some(credentials) = &context.credentials {
            if user_scoped {
                credentials
                    .to_db_scoped(&context_cache.sub, &app_id)
                    .await?
            } else {
                credentials.to_db(&app_id).await?
            }
        } else if user_scoped {
            let user_dir = context_cache.get_user_dir(false)?;
            let user_dir = user_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_user_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No user database builder found"))?(
                user_dir
            )
        } else {
            let board_dir = context_cache.get_storage(false)?;
            let board_dir = board_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_project_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No database builder found"))?(board_dir)
        };

        let connection = context.app_state.with_lance_session(db).execute().await?;
        let overlays = lancegraph::list_overlays(&connection).await?;

        let ids: Vec<String> = overlays.iter().map(|o| o.id.clone()).collect();
        let names: Vec<String> = overlays.iter().map(|o| o.name.clone()).collect();

        context.set_pin_value("overlay_ids", json!(ids)).await?;
        context.set_pin_value("overlay_names", json!(names)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}

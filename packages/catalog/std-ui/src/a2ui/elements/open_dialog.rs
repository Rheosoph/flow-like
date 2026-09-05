use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct OpenDialog;

impl OpenDialog {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for OpenDialog {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_open_dialog",
            "Open Dialog",
            "Opens a route/page as a modal dialog overlay",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "openDialog");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "route",
            "Route",
            "The route path to open in the dialog (e.g., /settings, /edit/123)",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "Optional dialog title (shown in header)",
            VariableType::String,
        );

        node.add_input_pin(
            "query_params",
            "Query Params",
            "Optional JSON object of query parameters to pass to the route",
            VariableType::String,
        );

        node.add_input_pin(
            "dialog_id",
            "Dialog ID",
            "Optional unique ID for the dialog (for closing specific dialogs)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let route: String = context.evaluate_pin("route").await?;
        let title: Option<String> = context
            .evaluate_pin("title")
            .await
            .ok()
            .filter(|s: &String| !s.is_empty());
        let query_params_str: String = context
            .evaluate_pin("query_params")
            .await
            .unwrap_or_default();
        let dialog_id: Option<String> = context
            .evaluate_pin("dialog_id")
            .await
            .ok()
            .filter(|s: &String| !s.is_empty());

        if route.is_empty() {
            return Err(flow_like_types::anyhow!("Route cannot be empty"));
        }

        let query_params: Option<HashMap<String, String>> = if query_params_str.is_empty() {
            None
        } else {
            match json::from_str(&query_params_str) {
                Ok(params) => Some(params),
                Err(_) => {
                    context.log_message(
                        &format!("Invalid query params JSON: {}", query_params_str),
                        LogLevel::Warn,
                    );
                    None
                }
            }
        };

        context
            .open_dialog(&route, title, query_params, dialog_id)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

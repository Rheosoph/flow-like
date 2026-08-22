use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;
use std::collections::HashMap;

/// Navigates to a different page/route in the application.
#[crate::register_node]
#[derive(Default)]
pub struct NavigateTo;

impl NavigateTo {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for NavigateTo {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_navigate_to",
            "Navigate To",
            "Navigates to a page route",
            "UI/Navigation",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "route",
            "Route",
            "The route to navigate to (e.g., /dashboard, /users/123)",
            VariableType::String,
        );

        node.add_input_pin(
            "query_params",
            "Query Params",
            "Optional query parameters as key-value pairs (e.g., {\"tab\": \"settings\", \"id\": \"123\"})",
            VariableType::Struct,
        )
        .set_default_value(Some(flow_like_types::json::json!({})))
        .set_open_schema();

        node.add_input_pin(
            "replace",
            "Replace",
            "If true, replaces the current history entry instead of adding a new one",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(false)));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let route: String = context.evaluate_pin("route").await?;
        let query_params: Option<HashMap<String, String>> =
            context.evaluate_pin("query_params").await.ok();
        let replace: bool = context.evaluate_pin("replace").await.unwrap_or(false);

        // Build route with query params if provided
        let final_route = if let Some(params) = query_params {
            if params.is_empty() {
                route
            } else {
                let query_string: String = params
                    .iter()
                    .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
                    .collect::<Vec<_>>()
                    .join("&");

                if route.contains('?') {
                    format!("{}&{}", route, query_string)
                } else {
                    format!("{}?{}", route, query_string)
                }
            }
        } else {
            route
        };

        context.navigate_to(&final_route, replace).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::{
        Value,
        json::{from_slice, json},
    };

    fn pin_default(node: &Node, name: &str) -> Option<Value> {
        node.get_pin_by_name(name)
            .and_then(|pin| pin.default_value.as_deref())
            .map(|bytes| from_slice(bytes).expect("pin default must be valid JSON"))
    }

    #[test]
    fn navigation_metadata_requires_only_route_payload() {
        let node = NavigateTo::new().get_node();

        assert!(
            node.get_pin_by_name("route")
                .expect("route input")
                .default_value
                .is_none(),
            "route must remain required at runtime"
        );
        assert_eq!(pin_default(&node, "query_params"), Some(json!({})));
        assert_eq!(pin_default(&node, "replace"), Some(json!(false)));
    }
}

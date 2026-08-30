use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

/// Sets or updates a query parameter in the URL.
///
/// This node sends a message to the frontend to update the URL query params
/// without triggering a navigation (stays on the same page).
#[crate::register_node]
#[derive(Default)]
pub struct SetQueryParam;

impl SetQueryParam {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetQueryParam {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_query_param",
            "Set Query Param",
            "Sets or updates a query parameter in the URL",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "setQueryParam");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "key",
            "Key",
            "The query parameter key to set",
            VariableType::String,
        );

        node.add_input_pin(
            "value",
            "Value",
            "The value to set (empty string removes the param)",
            VariableType::String,
        );

        node.add_input_pin(
            "replace",
            "Replace",
            "If true, replaces the current history entry instead of adding a new one",
            VariableType::Boolean,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let key: String = context.evaluate_pin("key").await?;
        let value: String = context.evaluate_pin("value").await.unwrap_or_default();
        let replace: bool = context.evaluate_pin("replace").await.unwrap_or(true);

        if key.is_empty() {
            return Err(flow_like_types::anyhow!("Query param key cannot be empty"));
        }

        let value_opt = if value.is_empty() { None } else { Some(value) };
        context.set_query_param(&key, value_opt, replace).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

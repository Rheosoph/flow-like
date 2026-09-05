use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets the current route/URL path.
#[crate::register_node]
#[derive(Default)]
pub struct GetCurrentRoute;

impl GetCurrentRoute {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetCurrentRoute {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_current_route",
            "Get Current Route",
            "Gets the current page route from the execution context",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "getCurrentRoute");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "route",
            "Route",
            "The current route path",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let route = context.get_frontend_route().await?.unwrap_or_default();

        context.set_pin_value("route", Value::String(route)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

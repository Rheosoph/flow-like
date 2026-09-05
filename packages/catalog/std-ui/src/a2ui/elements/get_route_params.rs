use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets route parameters from the current URL.
///
/// Route parameters are passed via `_route_params` in the workflow payload.
/// For a route like `/users/:id`, accessing with route `/users/123` would give:
/// `{ "id": "123" }`
#[crate::register_node]
#[derive(Default)]
pub struct GetRouteParams;

impl GetRouteParams {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetRouteParams {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_route_params",
            "Get Route Params",
            "Gets route parameters from the current URL",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "getRouteParam");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "param_name",
            "Param Name",
            "The name of the route parameter to get (optional - if empty, returns all params)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "value",
            "Value",
            "The parameter value (string if param_name specified, object if all params)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the parameter exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let param_name: String = context.evaluate_pin("param_name").await.unwrap_or_default();

        let route_params = context
            .get_frontend_route_params()
            .await?
            .unwrap_or(Value::Object(Default::default()));

        let value_pin = context.get_pin_by_name("value").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        if param_name.is_empty() {
            value_pin.set_value(route_params).await;
            exists_pin.set_value(Value::Bool(true)).await;
        } else if let Some(param_value) = route_params.get(&param_name) {
            value_pin.set_value(param_value.clone()).await;
            exists_pin.set_value(Value::Bool(true)).await;
        } else {
            value_pin.set_value(Value::Null).await;
            exists_pin.set_value(Value::Bool(false)).await;
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

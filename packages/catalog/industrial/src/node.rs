use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};
use schemars::JsonSchema;

pub fn operation_node<I: JsonSchema + serde::Serialize, O: JsonSchema>(
    id: &str,
    name: &str,
    description: &str,
    namespace: &str,
    function: &str,
    category: &str,
) -> Node {
    let mut node = Node::new(id, name, description, category);
    node.set_flowscript_name(namespace, function);
    node.add_icon("/flow/icons/web.svg");
    node.add_input_pin(
        "exec_in",
        "Input",
        "Run this operation",
        VariableType::Execution,
    );
    node.add_input_pin(
        "request",
        "Configuration",
        "Inputs and configuration",
        VariableType::Struct,
    )
    .set_schema::<I>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Done",
        "Operation completed",
        VariableType::Execution,
    );
    let result = node.add_output_pin("result", "Result", "Operation result", VariableType::Struct);
    let schema = serde_json::json!(schemars::schema_for!(O));
    let is_array = schema.get("type").and_then(|v| v.as_str()) == Some("array");
    let mut item = if is_array {
        schema
            .get("items")
            .cloned()
            .unwrap_or(serde_json::json!({}))
    } else {
        schema.clone()
    };
    let kind = match item.get("type").and_then(|v| v.as_str()) {
        Some("boolean") => VariableType::Boolean,
        Some("integer") => VariableType::Integer,
        Some("number") => VariableType::Float,
        Some("string") => VariableType::String,
        _ => VariableType::Struct,
    };
    result.set_data_type(kind.clone());
    if is_array {
        result.set_value_type(flow_like::flow::pin::ValueType::Array);
    }
    if kind == VariableType::Struct {
        if item == serde_json::json!(true) {
            result.set_open_schema();
        } else {
            if is_array && let Some(defs) = schema.get("$defs") {
                item["$defs"] = defs.clone();
            }
            result.schema = serde_json::to_string(&item).ok();
        }
    }
    node
}

#[macro_export]
macro_rules! operation {
    ($node:ident, $id:literal, $name:literal, $desc:literal, $namespace:literal, $function:literal, $category:literal, $input:ty, $output:ty, $handler:path) => {
        $crate::operation!(@impl $node, $id, $name, $desc, $namespace, $function, $category, $input, $output, $handler, false);
    };
    (@impl $node:ident, $id:literal, $name:literal, $desc:literal, $namespace:literal, $function:literal, $category:literal, $input:ty, $output:ty, $handler:path, $listener:expr) => {
        #[flow_like_types::async_trait]
        impl flow_like::flow::node::NodeLogic for $node {
            fn get_node(&self) -> flow_like::flow::node::Node {
                let mut node = $crate::node::operation_node::<$input, $output>($id, $name, $desc, $namespace, $function, $category);
                if $listener { node.set_long_running(true); node.set_can_reference_fns(true); }
                node
            }
            async fn run(&self, context: &mut flow_like::flow::execution::context::ExecutionContext) -> flow_like_types::Result<()> {
                #[cfg(feature = "execute")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let input: $input = context.evaluate_pin("request").await?;
                    let output: $output = $handler(context, input).await?;
                    context.set_pin_value("result", serde_json::json!(output)).await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "execute"))]
                { let _ = context; Err(flow_like_types::anyhow!("Industrial execution is unavailable in this metadata build")) }
            }
        }
    };
}

#[macro_export]
macro_rules! listener {
    ($node:ident, $id:literal, $name:literal, $desc:literal, $namespace:literal, $function:literal, $category:literal, $input:ty, $output:ty, $handler:path) => {
        $crate::operation!(@impl $node, $id, $name, $desc, $namespace, $function, $category, $input, $output, $handler, true);
    };
}

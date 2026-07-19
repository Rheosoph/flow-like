use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Get Widget Element - Resolves an element inside a widget instance.
///
/// Reads the inline widget definition embedded in the `element_ref` produced by
/// Instantiate Widget and outputs an element reference (`__element_id` =
/// `{instance}/{child}`) that plugs into any element node (Set Element Value,
/// Update GeoMap, Push CSV To Chart, …), plus the element's component data.
#[crate::register_node]
#[derive(Default)]
pub struct GetWidgetElement;

impl GetWidgetElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetWidgetElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_widget_get_element",
            "Get Widget Element",
            "Resolves an element inside a widget instance (from Instantiate Widget). The output plugs into any element node (Set Element Value, Update GeoMap, Push CSV To Chart, …).",
            "UI/Container",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Widget",
            "Widget instance reference (from Instantiate Widget)",
            VariableType::Struct,
        );

        node.add_input_pin(
            "element_id",
            "Element ID",
            "ID of the element inside the widget (e.g. 'chart-1')",
            VariableType::String,
        );

        node.add_output_pin(
            "element",
            "Element",
            "The element reference (connect to element nodes)",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the element exists in the widget",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_ref: Value = context.evaluate_pin("element_ref").await?;
        let element_id: String = context.evaluate_pin("element_id").await?;

        let instance_id = element_ref
            .get("instanceId")
            .or_else(|| element_ref.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                flow_like_types::anyhow!("Invalid widget reference: missing 'instanceId'")
            })?
            .to_string();

        let components = element_ref
            .get("component")
            .and_then(|c| c.get("inlineWidgetDef"))
            .and_then(|def| def.get("components"))
            .and_then(|c| c.as_array());

        // Exact id first — a suffix hit must never shadow an exact-id child.
        let suffix = format!("-{element_id}");
        let target = components.and_then(|arr| {
            arr.iter()
                .find(|c| c.get("id").and_then(|id| id.as_str()) == Some(element_id.as_str()))
                .or_else(|| {
                    arr.iter().find(|c| {
                        c.get("id")
                            .and_then(|id| id.as_str())
                            .is_some_and(|id| id.ends_with(&suffix))
                    })
                })
        });

        let element_pin = context.get_pin_by_name("element").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        if let Some(child) = target {
            let child_id = child
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or(&element_id);
            let element = json!({
                "id": child_id,
                "component": child.get("component").cloned().unwrap_or(Value::Null),
                "style": child.get("style").cloned().unwrap_or(Value::Null),
                "__element_id": format!("{instance_id}/{child_id}"),
            });
            element_pin.set_value(element).await;
            exists_pin.set_value(json!(true)).await;
        } else {
            let available: Vec<String> = components
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("id").and_then(|id| id.as_str()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            context.log_message(
                &format!(
                    "Element '{}' not found in widget '{}'. Available elements: [{}]",
                    element_id,
                    instance_id,
                    available.join(", ")
                ),
                flow_like::flow::execution::LogLevel::Warn,
            );
            element_pin.set_value(Value::Null).await;
            exists_pin.set_value(json!(false)).await;
        }

        Ok(())
    }
}

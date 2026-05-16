use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::schema_utils::set_component_schema_by_type;

/// Queries all elements of a specific type from the workflow payload.
///
/// The element data should be included in the workflow payload when triggered from the UI.
/// Returns an array of elements that match the specified component type.
#[crate::register_node]
#[derive(Default)]
pub struct QueryElementsByType;

impl QueryElementsByType {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for QueryElementsByType {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_query_elements_by_type",
            "Query Elements by Type",
            "Gets all elements of a specific component type",
            "UI/Elements/Query",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "component_type",
            "Component Type",
            "The type of component to query (e.g., 'button', 'text', 'textField')",
            VariableType::String,
        );

        node.add_output_pin(
            "elements",
            "Elements",
            "Array of matching elements",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of matching elements",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let component_type: String = context.evaluate_pin("component_type").await?;
        let component_type_lower = component_type.to_lowercase();

        let elements = context.get_frontend_elements().await?;

        let mut matching_elements: Vec<Value> = Vec::new();

        if let Some(elements_map) = elements {
            for (id, element) in elements_map {
                // Check if element's component type matches
                let element_type = element
                    .get("component")
                    .and_then(|c| c.get("type"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_lowercase());

                if element_type == Some(component_type_lower.clone()) {
                    // Include the element with its ID
                    let mut element_with_id = element.clone();
                    if let Some(obj) = element_with_id.as_object_mut() {
                        obj.insert("_id".to_string(), Value::String(id.clone()));
                    }
                    matching_elements.push(element_with_id);
                }
            }
        }

        let count = matching_elements.len() as i64;

        context.log_message(
            &format!("Found {} elements of type '{}'", count, component_type),
            LogLevel::Debug,
        );

        let elements_pin = context.get_pin_by_name("elements").await?;
        elements_pin
            .set_value(Value::Array(matching_elements))
            .await;

        let count_pin = context.get_pin_by_name("count").await?;
        count_pin.set_value(Value::Number(count.into())).await;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let type_pin = match read_only_node.get_pin_by_name("component_type") {
            Some(pin) => pin,
            None => return,
        };

        let type_value = type_pin.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        });

        if let Some(t) = &type_value {
            node.friendly_name = format!("Query {}s", t);

            // Set dynamic schema on output pin based on component type
            if let Some(elements_pin) = node.get_pin_mut_by_name("elements") {
                set_component_schema_by_type(elements_pin, t);
            }
        } else {
            node.friendly_name = "Query Elements by Type".to_string();
        }
    }
}

use super::element_utils::{extract_element_id_from_pin, find_element};
use flow_like::a2ui::components::TextFieldProps;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets the value of an input element.
///
/// Works with text inputs, number inputs, selects, checkboxes, etc.
#[crate::register_node]
#[derive(Default)]
pub struct GetElementValue;

impl GetElementValue {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetElementValue {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_element_value",
            "Get Element Value",
            "Gets the value of an input element",
            "UI/Elements",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the input element",
            VariableType::Struct,
        )
        .set_schema::<TextFieldProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "value",
            "Value",
            "The current value of the input",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the element exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;

        let elements = context.get_frontend_elements().await?;
        let element = elements.as_ref().and_then(|e| find_element(e, &element_id));

        let value_pin = context.get_pin_by_name("value").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        if let Some((found_id, element_value)) = element {
            let raw_value = element_value
                .get("component")
                .and_then(|c| {
                    c.get("value")
                        .or_else(|| c.get("defaultValue"))
                        .or_else(|| c.get("checked"))
                        .or_else(|| c.get("selected"))
                })
                .cloned()
                .unwrap_or(Value::Null);

            // Extract actual value from BoundValue wrapper if present
            // BoundValue can be: { literalString: "..." }, { literalNumber: N }, { literalBool: B }, { path: "..." }, or direct value
            let value = match &raw_value {
                Value::Object(obj) => {
                    if let Some(v) = obj.get("literalString") {
                        v.clone()
                    } else if let Some(v) = obj.get("literalNumber") {
                        v.clone()
                    } else if let Some(v) = obj.get("literalBool") {
                        v.clone()
                    } else if let Some(v) = obj.get("literalOptions") {
                        v.clone()
                    } else if let Some(v) = obj.get("literalJson") {
                        v.as_str()
                            .and_then(|s| flow_like_types::json::from_str::<Value>(s).ok())
                            .unwrap_or_else(|| v.clone())
                    } else if obj.contains_key("path") {
                        // Path binding - return null as we can't resolve it here
                        Value::Null
                    } else {
                        // Not a BoundValue, return as-is
                        raw_value.clone()
                    }
                }
                // Direct value (string, number, bool, etc.)
                _ => raw_value.clone(),
            };

            value_pin.set_value(value.clone()).await;
            exists_pin.set_value(Value::Bool(true)).await;

            context.log_message(
                &format!(
                    "Got value from element {} ({}): {:?}",
                    element_id, found_id, value
                ),
                LogLevel::Debug,
            );
        } else {
            value_pin.set_value(Value::Null).await;
            exists_pin.set_value(Value::Bool(false)).await;

            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Warn,
            );
        }

        Ok(())
    }
}

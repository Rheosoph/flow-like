use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[crate::register_node]
pub struct FormatStringNode {
    regex: Regex,
}

impl FormatStringNode {
    pub fn new() -> Self {
        FormatStringNode {
            regex: Regex::new(r"\{([a-zA-Z0-9_]+)\}").unwrap(),
        }
    }
}

impl Default for FormatStringNode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeLogic for FormatStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_format",
            "Format String",
            "Formats a string with placeholders",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin(
            "format_string",
            "Input",
            "String with placeholders",
            VariableType::String,
        );
        node.add_output_pin(
            "formatted_string",
            "Formatted",
            "Formatted string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let format_string: String = context.evaluate_pin("format_string").await?;
        let mut formatted_string = format_string.clone();

        let placeholders: std::collections::HashSet<String> = self
            .regex
            .captures_iter(&format_string)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .collect();

        for placeholder in placeholders {
            let value: flow_like_types::Value = context.evaluate_pin(&placeholder).await?;
            // If the JSON value is a string, use it directly; otherwise serialize it.
            let replacement = match value {
                flow_like_types::Value::String(s) => s,
                _ => flow_like_types::json::to_string_pretty(&value).unwrap(),
            };
            formatted_string =
                formatted_string.replace(&format!("{{{}}}", placeholder), &replacement);
        }

        context
            .set_pin_value("formatted_string", json!(formatted_string))
            .await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.name != "format_string" && p.pin_type == PinType::Input)
            .collect();

        let format_string: String = node
            .get_pin_by_name("format_string")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let mut current_placeholders = pins
            .iter()
            .map(|p| (p.name.clone(), *p))
            .collect::<HashMap<_, _>>();

        let mut all_placeholders = HashSet::new();
        let mut missing_placeholders = HashSet::new();

        for cap in self.regex.captures_iter(&format_string) {
            if let Some(placeholder) = cap.get(1).map(|m| m.as_str().to_string()) {
                all_placeholders.insert(placeholder.clone());
                if current_placeholders.remove(&placeholder).is_none() {
                    missing_placeholders.insert(placeholder);
                }
            }
        }

        let ids_to_remove = current_placeholders
            .values()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        ids_to_remove.iter().for_each(|id| {
            node.pins.remove(id);
        });

        for placeholder in missing_placeholders {
            node.add_input_pin(&placeholder, &placeholder, "", VariableType::Generic);
        }

        all_placeholders.iter().for_each(|placeholder| {
            let _ = node.match_type(placeholder, board, None, None);
        })
    }
}

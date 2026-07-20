use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use regex::Regex;
use std::collections::{HashMap, HashSet};

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

        // Group by name: earlier updates could have leaked several pins for one placeholder.
        let mut current_placeholders: HashMap<String, Vec<&Pin>> = HashMap::new();
        for pin in &pins {
            current_placeholders
                .entry(pin.name.clone())
                .or_default()
                .push(pin);
        }

        let mut all_placeholders = HashSet::new();
        let mut missing_placeholders = HashSet::new();
        let mut stale_ids = Vec::new();

        for cap in self.regex.captures_iter(&format_string) {
            let Some(placeholder) = cap.get(1).map(|m| m.as_str().to_string()) else {
                continue;
            };
            // A placeholder repeated in the format string still maps to a single pin.
            if !all_placeholders.insert(placeholder.clone()) {
                continue;
            }
            match current_placeholders.remove(&placeholder) {
                Some(mut matched) => {
                    matched.sort_by_key(|pin| (pin.index, pin.id.clone()));
                    stale_ids.extend(matched.iter().skip(1).map(|pin| pin.id.clone()));
                }
                None => {
                    missing_placeholders.insert(placeholder);
                }
            }
        }

        stale_ids.extend(
            current_placeholders
                .values()
                .flatten()
                .map(|pin| pin.id.clone()),
        );
        stale_ids.iter().for_each(|id| {
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

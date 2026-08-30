use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, dynamic_pin_source_literal, remove_unwired_pins},
    pin::{Pin, PinType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
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

    /// Return placeholder names once, in the order of their first appearance.
    ///
    /// Pins are keyed by placeholder name, not by placeholder occurrence. Keeping that contract
    /// here prevents repeated tokens such as `{query}` from being interpreted as new pins on
    /// subsequent `on_update` passes.
    fn placeholders(&self, format_string: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        self.regex
            .captures_iter(format_string)
            .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_string()))
            .filter(|placeholder| seen.insert(placeholder.clone()))
            .collect()
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
        node.set_flowscript_name("string", "format");
        node.set_receiver("format_string");
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

        for placeholder in self.placeholders(&format_string) {
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
        // A wired, absent or non-string format string says nothing about which placeholders the
        // node will actually see at runtime. Reading it as "no placeholders" would delete every
        // placeholder pin along with the wires feeding them.
        node.error = None;
        let Some(format_string) = dynamic_pin_source_literal(node, "format_string") else {
            return;
        };

        let pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.name != "format_string" && p.pin_type == PinType::Input)
            .collect();

        // Group by name: earlier updates could have leaked several pins for one placeholder.
        let mut current_placeholders: HashMap<String, Vec<&Pin>> = HashMap::new();
        for pin in &pins {
            current_placeholders
                .entry(pin.name.clone())
                .or_default()
                .push(pin);
        }

        // A placeholder that names the config pin itself would mint a second pin called
        // `format_string`, and every later lookup of that name would pick whichever the map
        // yielded first — silently writing the placeholder's value over the template.
        let (placeholders, reserved): (Vec<String>, Vec<String>) = self
            .placeholders(&format_string)
            .into_iter()
            .partition(|placeholder| placeholder != "format_string");
        if !reserved.is_empty() {
            node.error = Some(
                "`{format_string}` is the name of this node's own input and cannot be a placeholder. Rename it in the format string."
                    .to_string(),
            );
        }
        let mut missing_placeholders = Vec::new();
        let mut stale_ids = Vec::new();

        for placeholder in &placeholders {
            match current_placeholders.remove(placeholder) {
                Some(mut matched) => {
                    matched.sort_by_key(|pin| (pin.index, pin.id.clone()));
                    stale_ids.extend(matched.iter().skip(1).map(|pin| pin.id.clone()));
                }
                None => {
                    missing_placeholders.push(placeholder.clone());
                }
            }
        }

        stale_ids.extend(
            current_placeholders
                .values()
                .flatten()
                .map(|pin| pin.id.clone()),
        );
        remove_unwired_pins(node, &stale_ids);

        for placeholder in missing_placeholders {
            node.add_input_pin(&placeholder, &placeholder, "", VariableType::Generic);
        }

        placeholders.iter().for_each(|placeholder| {
            let _ = node.match_type(placeholder, board, None, None);
        })
    }
}

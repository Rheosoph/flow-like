use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringToIntNode {}

impl StringToIntNode {
    pub fn new() -> Self {
        StringToIntNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToIntNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_int",
            "To Integer",
            "Parses a string into an integer",
            "Utils/String",
        );
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "String to parse", VariableType::String);
        node.add_input_pin(
            "fallback",
            "Fallback",
            "Value used when parsing fails",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "integer",
            "Integer",
            "The parsed integer",
            VariableType::Integer,
        );
        node.add_output_pin(
            "success",
            "Success",
            "True when the string was a valid integer",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let fallback: i64 = context.evaluate_pin("fallback").await?;

        let parsed = string.trim().parse::<i64>().ok();
        context
            .set_pin_value("integer", json!(parsed.unwrap_or(fallback)))
            .await?;
        context
            .set_pin_value("success", json!(parsed.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringToFloatNode {}

impl StringToFloatNode {
    pub fn new() -> Self {
        StringToFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_float",
            "To Float",
            "Parses a string into a float",
            "Utils/String",
        );
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "String to parse", VariableType::String);
        node.add_input_pin(
            "fallback",
            "Fallback",
            "Value used when parsing fails",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));

        node.add_output_pin("float", "Float", "The parsed float", VariableType::Float);
        node.add_output_pin(
            "success",
            "Success",
            "True when the string was a valid float",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let fallback: f64 = context.evaluate_pin("fallback").await?;

        let parsed = string
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite());
        context
            .set_pin_value("float", json!(parsed.unwrap_or(fallback)))
            .await?;
        context
            .set_pin_value("success", json!(parsed.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringToBoolNode {}

impl StringToBoolNode {
    pub fn new() -> Self {
        StringToBoolNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToBoolNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_bool",
            "To Boolean",
            "Parses a string into a boolean. Accepts true/false, 1/0, yes/no and on/off",
            "Utils/String",
        );
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "String to parse", VariableType::String);
        node.add_input_pin(
            "fallback",
            "Fallback",
            "Value used when parsing fails",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "boolean",
            "Boolean",
            "The parsed boolean",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "success",
            "Success",
            "True when the string was a recognized boolean",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let fallback: bool = context.evaluate_pin("fallback").await?;

        let parsed = match string.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Some(true),
            "false" | "0" | "no" | "n" | "off" => Some(false),
            _ => None,
        };

        context
            .set_pin_value("boolean", json!(parsed.unwrap_or(fallback)))
            .await?;
        context
            .set_pin_value("success", json!(parsed.is_some()))
            .await?;
        Ok(())
    }
}

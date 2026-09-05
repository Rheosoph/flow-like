use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

const RADIX_VALUES: [&str; 4] = ["Binary", "Octal", "Decimal", "Hexadecimal"];

fn radix_of(name: &str) -> u32 {
    match name {
        "Binary" => 2,
        "Octal" => 8,
        "Hexadecimal" => 16,
        _ => 10,
    }
}

fn radix_pin(node: &mut Node) {
    node.add_input_pin("radix", "Base", "Numeric base", VariableType::String)
        .set_default_value(Some(json!("Hexadecimal")))
        .set_options(
            PinOptions::new()
                .set_valid_values(RADIX_VALUES.iter().map(|value| value.to_string()).collect())
                .build(),
        );
}

#[crate::register_node]
#[derive(Default)]
pub struct IntToFloatNode {}

impl IntToFloatNode {
    pub fn new() -> Self {
        IntToFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for IntToFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_to_float",
            "To Float",
            "Converts an integer into a float",
            "Math/Int",
        );
        node.set_flowscript_name("int", "toFloat");
        node.set_receiver("integer");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
        node.add_output_pin("float", "Float", "The converted value", VariableType::Float);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        context
            .set_pin_value("float", json!(integer as f64))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntToRadixNode {}

impl IntToRadixNode {
    pub fn new() -> Self {
        IntToRadixNode {}
    }
}

#[async_trait]
impl NodeLogic for IntToRadixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_to_radix",
            "To Base",
            "Formats an integer as binary, octal, decimal or hexadecimal text",
            "Math/Int",
        );
        node.set_flowscript_name("int", "toRadix");
        node.set_receiver("integer");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);
        radix_pin(&mut node);
        node.add_input_pin(
            "uppercase",
            "Uppercase",
            "Use upper case letters for hexadecimal digits",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "string",
            "String",
            "The formatted number",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        let radix: String = context.evaluate_pin("radix").await?;
        let uppercase: bool = context.evaluate_pin("uppercase").await?;

        let formatted = match radix_of(&radix) {
            2 => format!("{integer:b}"),
            8 => format!("{integer:o}"),
            16 if uppercase => format!("{integer:X}"),
            16 => format!("{integer:x}"),
            _ => integer.to_string(),
        };

        context.set_pin_value("string", json!(formatted)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct IntFromRadixNode {}

impl IntFromRadixNode {
    pub fn new() -> Self {
        IntFromRadixNode {}
    }
}

#[async_trait]
impl NodeLogic for IntFromRadixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_from_radix",
            "From Base",
            "Parses an integer from binary, octal, decimal or hexadecimal text",
            "Math/Int",
        );
        node.set_flowscript_name("int", "fromRadix");
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Text to parse", VariableType::String);
        radix_pin(&mut node);

        node.add_output_pin(
            "integer",
            "Integer",
            "The parsed integer",
            VariableType::Integer,
        );
        node.add_output_pin(
            "success",
            "Success",
            "True when the text was a valid number in that base",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let radix: String = context.evaluate_pin("radix").await?;

        let trimmed = string.trim();
        let radix = radix_of(&radix);
        let cleaned = match radix {
            2 => trimmed.trim_start_matches("0b").trim_start_matches("0B"),
            8 => trimmed.trim_start_matches("0o").trim_start_matches("0O"),
            16 => trimmed.trim_start_matches("0x").trim_start_matches("0X"),
            _ => trimmed,
        };

        let parsed = i64::from_str_radix(cleaned, radix).ok();
        context
            .set_pin_value("integer", json!(parsed.unwrap_or(0)))
            .await?;
        context
            .set_pin_value("success", json!(parsed.is_some()))
            .await?;
        Ok(())
    }
}

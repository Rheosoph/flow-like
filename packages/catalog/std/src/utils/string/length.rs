use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use unicode_segmentation::UnicodeSegmentation;

#[crate::register_node]
#[derive(Default)]
pub struct StringLengthNode {}

impl StringLengthNode {
    pub fn new() -> Self {
        StringLengthNode {}
    }
}

#[async_trait]
impl NodeLogic for StringLengthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_length",
            "String Length",
            "Calculates the length of a string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_version(1);
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "mode",
            "Mode",
            "Characters counts code points, Graphemes counts what a reader sees, Bytes counts UTF-8 bytes",
            VariableType::String,
        )
        .set_default_value(Some(json!("Characters")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Characters".to_string(),
                    "Graphemes".to_string(),
                    "Bytes".to_string(),
                ])
                .build(),
        );

        node.add_output_pin(
            "length",
            "Length",
            "Length of the string",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let mode: String = context.evaluate_pin("mode").await?;

        let length = match mode.as_str() {
            "Bytes" => string.len(),
            "Graphemes" => string.graphemes(true).count(),
            _ => string.chars().count(),
        };

        context.set_pin_value("length", json!(length)).await?;
        Ok(())
    }
}

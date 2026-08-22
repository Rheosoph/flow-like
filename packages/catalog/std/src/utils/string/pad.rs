use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn pad_pins(node: &mut Node) {
    node.add_input_pin("string", "String", "Input String", VariableType::String);
    node.add_input_pin(
        "length",
        "Length",
        "Target length in characters",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(10)));
    node.add_input_pin(
        "padding",
        "Padding",
        "Characters used to fill up the string",
        VariableType::String,
    )
    .set_default_value(Some(json!(" ")));

    node.add_output_pin(
        "padded",
        "Padded",
        "The padded string, unchanged when it is already long enough",
        VariableType::String,
    );
}

fn build_padding(padding: &str, missing: usize) -> String {
    if missing == 0 || padding.is_empty() {
        return String::new();
    }
    padding.chars().cycle().take(missing).collect()
}

fn missing_chars(string: &str, length: i64) -> usize {
    (length.max(0) as usize).saturating_sub(string.chars().count())
}

#[crate::register_node]
#[derive(Default)]
pub struct StringPadStartNode {}

impl StringPadStartNode {
    pub fn new() -> Self {
        StringPadStartNode {}
    }
}

#[async_trait]
impl NodeLogic for StringPadStartNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_pad_start",
            "Pad Start",
            "Fills up a string at the start until it reaches the target length",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());
        pad_pins(&mut node);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let length: i64 = context.evaluate_pin("length").await?;
        let padding: String = context.evaluate_pin("padding").await?;

        let mut padded = build_padding(&padding, missing_chars(&string, length));
        padded.push_str(&string);

        context.set_pin_value("padded", json!(padded)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringPadEndNode {}

impl StringPadEndNode {
    pub fn new() -> Self {
        StringPadEndNode {}
    }
}

#[async_trait]
impl NodeLogic for StringPadEndNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_pad_end",
            "Pad End",
            "Fills up a string at the end until it reaches the target length",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());
        pad_pins(&mut node);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let length: i64 = context.evaluate_pin("length").await?;
        let padding: String = context.evaluate_pin("padding").await?;

        let mut padded = string.clone();
        padded.push_str(&build_padding(&padding, missing_chars(&string, length)));

        context.set_pin_value("padded", json!(padded)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringNormalizeWhitespaceNode {}

impl StringNormalizeWhitespaceNode {
    pub fn new() -> Self {
        StringNormalizeWhitespaceNode {}
    }
}

#[async_trait]
impl NodeLogic for StringNormalizeWhitespaceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_normalize_whitespace",
            "Normalize Whitespace",
            "Collapses runs of whitespace into single spaces and trims the result",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "normalized",
            "Normalized",
            "The normalized string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let normalized = string.split_whitespace().collect::<Vec<_>>().join(" ");
        context
            .set_pin_value("normalized", json!(normalized))
            .await?;
        Ok(())
    }
}

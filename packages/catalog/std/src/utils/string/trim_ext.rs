use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringTrimStartNode {}

impl StringTrimStartNode {
    pub fn new() -> Self {
        StringTrimStartNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTrimStartNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_trim_start",
            "Trim Start",
            "Removes leading whitespace from a string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "trimmed_string",
            "Trimmed String",
            "String without leading whitespace",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("trimmed_string", json!(string.trim_start()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringTrimEndNode {}

impl StringTrimEndNode {
    pub fn new() -> Self {
        StringTrimEndNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTrimEndNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_trim_end",
            "Trim End",
            "Removes trailing whitespace from a string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "trimmed_string",
            "Trimmed String",
            "String without trailing whitespace",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("trimmed_string", json!(string.trim_end()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringTrimMatchesNode {}

impl StringTrimMatchesNode {
    pub fn new() -> Self {
        StringTrimMatchesNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTrimMatchesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_trim_matches",
            "Trim Characters",
            "Removes the given characters from the start and/or end of a string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "characters",
            "Characters",
            "Set of characters to strip",
            VariableType::String,
        )
        .set_default_value(Some(json!(" ")));
        node.add_input_pin("side", "Side", "Where to strip", VariableType::String)
            .set_default_value(Some(json!("Both")))
            .set_options(
                flow_like::flow::pin::PinOptions::new()
                    .set_valid_values(vec![
                        "Both".to_string(),
                        "Start".to_string(),
                        "End".to_string(),
                    ])
                    .build(),
            );

        node.add_output_pin(
            "trimmed_string",
            "Trimmed String",
            "String without the stripped characters",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let characters: String = context.evaluate_pin("characters").await?;
        let side: String = context.evaluate_pin("side").await?;

        let strip = |c: char| characters.contains(c);
        let trimmed = match side.as_str() {
            "Start" => string.trim_start_matches(strip),
            "End" => string.trim_end_matches(strip),
            _ => string.trim_matches(strip),
        };

        context
            .set_pin_value("trimmed_string", json!(trimmed))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringStripPrefixNode {}

impl StringStripPrefixNode {
    pub fn new() -> Self {
        StringStripPrefixNode {}
    }
}

#[async_trait]
impl NodeLogic for StringStripPrefixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_strip_prefix",
            "Strip Prefix",
            "Removes a prefix from a string if it is present",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin("prefix", "Prefix", "Prefix to remove", VariableType::String);

        node.add_output_pin(
            "result",
            "Result",
            "String without the prefix",
            VariableType::String,
        );
        node.add_output_pin(
            "stripped",
            "Stripped",
            "True when the prefix was present",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let prefix: String = context.evaluate_pin("prefix").await?;

        let stripped = string.strip_prefix(&prefix);
        context
            .set_pin_value("result", json!(stripped.unwrap_or(&string)))
            .await?;
        context
            .set_pin_value("stripped", json!(stripped.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringStripSuffixNode {}

impl StringStripSuffixNode {
    pub fn new() -> Self {
        StringStripSuffixNode {}
    }
}

#[async_trait]
impl NodeLogic for StringStripSuffixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_strip_suffix",
            "Strip Suffix",
            "Removes a suffix from a string if it is present",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin("suffix", "Suffix", "Suffix to remove", VariableType::String);

        node.add_output_pin(
            "result",
            "Result",
            "String without the suffix",
            VariableType::String,
        );
        node.add_output_pin(
            "stripped",
            "Stripped",
            "True when the suffix was present",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let suffix: String = context.evaluate_pin("suffix").await?;

        let stripped = string.strip_suffix(&suffix);
        context
            .set_pin_value("result", json!(stripped.unwrap_or(&string)))
            .await?;
        context
            .set_pin_value("stripped", json!(stripped.is_some()))
            .await?;
        Ok(())
    }
}

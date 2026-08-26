use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringSplitOnceNode {}

impl StringSplitOnceNode {
    pub fn new() -> Self {
        StringSplitOnceNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSplitOnceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_split_once",
            "Split Once",
            "Splits a string at the first (or last) occurrence of a separator",
            "Utils/String",
        );
        node.set_flowscript_name("string", "splitOnce");
        node.set_receiver("string");
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "separator",
            "Separator",
            "String to split at",
            VariableType::String,
        );
        node.add_input_pin(
            "from_end",
            "From End",
            "Split at the last occurrence instead of the first",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "before",
            "Before",
            "Text before the separator, the whole string when it was not found",
            VariableType::String,
        );
        node.add_output_pin(
            "after",
            "After",
            "Text after the separator",
            VariableType::String,
        );
        node.add_output_pin(
            "found",
            "Found",
            "True when the separator was found",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let separator: String = context.evaluate_pin("separator").await?;
        let from_end: bool = context.evaluate_pin("from_end").await?;

        let split = if separator.is_empty() {
            None
        } else if from_end {
            string.rsplit_once(&separator)
        } else {
            string.split_once(&separator)
        };

        let (before, after) = match split {
            Some((before, after)) => (before.to_string(), after.to_string()),
            None => (string.clone(), String::new()),
        };

        context.set_pin_value("before", json!(before)).await?;
        context.set_pin_value("after", json!(after)).await?;
        context
            .set_pin_value("found", json!(split.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringSplitWhitespaceNode {}

impl StringSplitWhitespaceNode {
    pub fn new() -> Self {
        StringSplitWhitespaceNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSplitWhitespaceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_split_whitespace",
            "Split Whitespace",
            "Splits a string into words, collapsing runs of whitespace",
            "Utils/String",
        );
        node.set_flowscript_name("string", "splitWhitespace");
        node.set_receiver("string");
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "words",
            "Words",
            "The separated words",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let words: Vec<String> = string.split_whitespace().map(|w| w.to_string()).collect();
        context.set_pin_value("words", json!(words)).await?;
        Ok(())
    }
}

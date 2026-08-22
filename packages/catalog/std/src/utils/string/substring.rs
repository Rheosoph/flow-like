use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn char_range(total: i64, start: i64, length: i64) -> (usize, usize) {
    let start_index = if start < 0 {
        (total + start).max(0)
    } else {
        start.min(total)
    };
    let take = if length < 0 {
        total - start_index
    } else {
        length.min(total - start_index)
    };
    (start_index.max(0) as usize, take.max(0) as usize)
}

#[crate::register_node]
#[derive(Default)]
pub struct StringSubstringNode {}

impl StringSubstringNode {
    pub fn new() -> Self {
        StringSubstringNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSubstringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_substring",
            "Substring",
            "Extracts a range of characters from a string. Negative start counts from the end, length -1 runs to the end.",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "start",
            "Start",
            "First character index, negative counts from the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "length",
            "Length",
            "Number of characters to take, -1 for the rest of the string",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin(
            "substring",
            "Substring",
            "The extracted characters",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let start: i64 = context.evaluate_pin("start").await?;
        let length: i64 = context.evaluate_pin("length").await?;

        let chars: Vec<char> = string.chars().collect();
        let (skip, take) = char_range(chars.len() as i64, start, length);
        let substring: String = chars.into_iter().skip(skip).take(take).collect();

        context.set_pin_value("substring", json!(substring)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringTruncateNode {}

impl StringTruncateNode {
    pub fn new() -> Self {
        StringTruncateNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTruncateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_truncate",
            "Truncate String",
            "Shortens a string to a maximum number of characters, appending an ellipsis when it was cut",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "max_length",
            "Max Length",
            "Maximum number of characters including the ellipsis",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "ellipsis",
            "Ellipsis",
            "Appended when the string was cut",
            VariableType::String,
        )
        .set_default_value(Some(json!("…")));

        node.add_output_pin(
            "truncated",
            "Truncated",
            "The shortened string",
            VariableType::String,
        );
        node.add_output_pin(
            "was_truncated",
            "Was Truncated",
            "True when characters were removed",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let max_length: i64 = context.evaluate_pin("max_length").await?;
        let ellipsis: String = context.evaluate_pin("ellipsis").await?;

        let chars: Vec<char> = string.chars().collect();
        let max_length = max_length.max(0) as usize;
        let was_truncated = chars.len() > max_length;

        let truncated = if was_truncated {
            let budget = max_length.saturating_sub(ellipsis.chars().count());
            let mut result: String = chars.into_iter().take(budget).collect();
            result.push_str(&ellipsis);
            result
        } else {
            string
        };

        context.set_pin_value("truncated", json!(truncated)).await?;
        context
            .set_pin_value("was_truncated", json!(was_truncated))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringSplitAtNode {}

impl StringSplitAtNode {
    pub fn new() -> Self {
        StringSplitAtNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSplitAtNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_split_at",
            "Split At",
            "Splits a string into two halves at a character index",
            "Utils/String",
        );
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "index",
            "Index",
            "Character index to split at, negative counts from the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "before",
            "Before",
            "Characters before the index",
            VariableType::String,
        );
        node.add_output_pin(
            "after",
            "After",
            "Characters from the index onwards",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let index: i64 = context.evaluate_pin("index").await?;

        let chars: Vec<char> = string.chars().collect();
        let (split, _) = char_range(chars.len() as i64, index, -1);

        let before: String = chars.iter().take(split).collect();
        let after: String = chars.into_iter().skip(split).collect();

        context.set_pin_value("before", json!(before)).await?;
        context.set_pin_value("after", json!(after)).await?;
        Ok(())
    }
}

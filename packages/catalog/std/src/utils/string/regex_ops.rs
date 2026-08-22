use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use regex::Regex;

fn pattern_pin(node: &mut Node) {
    node.add_input_pin("string", "String", "Input String", VariableType::String);
    node.add_input_pin(
        "pattern",
        "Pattern",
        "Regular expression pattern",
        VariableType::String,
    );
}

#[crate::register_node]
#[derive(Default)]
pub struct StringRegexMatchNode {}

impl StringRegexMatchNode {
    pub fn new() -> Self {
        StringRegexMatchNode {}
    }
}

#[async_trait]
impl NodeLogic for StringRegexMatchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_regex_match",
            "Regex Match",
            "Checks whether a regular expression matches a string",
            "Utils/String/Regex",
        );
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        pattern_pin(&mut node);

        node.add_output_pin(
            "is_match",
            "Is Match",
            "True when the pattern matches",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "first_match",
            "First Match",
            "The first matching text, empty when there is no match",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;

        let regex = Regex::new(&pattern)?;
        let first = regex.find(&string).map(|m| m.as_str().to_string());

        context
            .set_pin_value("is_match", json!(first.is_some()))
            .await?;
        context
            .set_pin_value("first_match", json!(first.unwrap_or_default()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringRegexFindAllNode {}

impl StringRegexFindAllNode {
    pub fn new() -> Self {
        StringRegexFindAllNode {}
    }
}

#[async_trait]
impl NodeLogic for StringRegexFindAllNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_regex_find_all",
            "Regex Find All",
            "Returns every match of a regular expression in a string",
            "Utils/String/Regex",
        );
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        pattern_pin(&mut node);

        node.add_output_pin(
            "matches",
            "Matches",
            "All matching substrings",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin("count", "Count", "Number of matches", VariableType::Integer);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;

        let regex = Regex::new(&pattern)?;
        let matches: Vec<String> = regex
            .find_iter(&string)
            .map(|m| m.as_str().to_string())
            .collect();

        context
            .set_pin_value("count", json!(matches.len() as i64))
            .await?;
        context.set_pin_value("matches", json!(matches)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringRegexCapturesNode {}

impl StringRegexCapturesNode {
    pub fn new() -> Self {
        StringRegexCapturesNode {}
    }
}

#[async_trait]
impl NodeLogic for StringRegexCapturesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_regex_captures",
            "Regex Captures",
            "Extracts the capture groups of the first regular expression match",
            "Utils/String/Regex",
        );
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        pattern_pin(&mut node);

        node.add_output_pin(
            "groups",
            "Groups",
            "Capture groups, index 0 is the whole match",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "found",
            "Found",
            "True when the pattern matched",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;

        let regex = Regex::new(&pattern)?;
        let groups: Vec<String> = match regex.captures(&string) {
            Some(captures) => captures
                .iter()
                .map(|group| group.map(|g| g.as_str().to_string()).unwrap_or_default())
                .collect(),
            None => Vec::new(),
        };

        context
            .set_pin_value("found", json!(!groups.is_empty()))
            .await?;
        context.set_pin_value("groups", json!(groups)).await?;
        Ok(())
    }
}

use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use regex::Regex;

#[crate::register_node]
#[derive(Default)]
pub struct StringSplitNode {}

impl StringSplitNode {
    pub fn new() -> Self {
        StringSplitNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSplitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_split",
            "Split String",
            "Splits a string into substrings",
            "Utils/String",
        );
        node.add_icon("/flow/icons/split.svg");
        node.set_version(1);
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "separator",
            "Separator",
            "String to split by, an empty separator splits into single characters",
            VariableType::String,
        );
        node.add_input_pin(
            "is_regex",
            "Is Regex",
            "Treat the separator as a regular expression",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of parts, 0 for no limit. The last part keeps the rest",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "skip_empty",
            "Skip Empty",
            "Drop parts that are empty",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "substrings",
            "Substrings",
            "Array of substrings",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let separator: String = context.evaluate_pin("separator").await?;
        let is_regex: bool = context.evaluate_pin("is_regex").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        let skip_empty: bool = context.evaluate_pin("skip_empty").await?;

        let limit = limit.max(0) as usize;

        let mut substrings: Vec<String> = if separator.is_empty() {
            string.chars().map(|c| c.to_string()).collect()
        } else if is_regex {
            let regex = Regex::new(&separator)?;
            match limit {
                0 => regex.split(&string).map(|s| s.to_string()).collect(),
                limit => regex
                    .splitn(&string, limit)
                    .map(|s| s.to_string())
                    .collect(),
            }
        } else {
            match limit {
                0 => string.split(&separator).map(|s| s.to_string()).collect(),
                limit => string
                    .splitn(limit, &separator)
                    .map(|s| s.to_string())
                    .collect(),
            }
        };

        if skip_empty {
            substrings.retain(|part| !part.is_empty());
        }

        context
            .set_pin_value("substrings", json!(substrings))
            .await?;
        Ok(())
    }
}

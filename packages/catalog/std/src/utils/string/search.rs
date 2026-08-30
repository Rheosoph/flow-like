use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn chars_eq(a: char, b: char, ignore_case: bool) -> bool {
    if a == b {
        return true;
    }
    ignore_case && a.to_lowercase().eq(b.to_lowercase())
}

fn window_matches(haystack: &[char], needle: &[char], at: usize, ignore_case: bool) -> bool {
    needle
        .iter()
        .enumerate()
        .all(|(offset, expected)| chars_eq(haystack[at + offset], *expected, ignore_case))
}

pub fn char_index_of(
    haystack: &[char],
    needle: &[char],
    ignore_case: bool,
    from_end: bool,
) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }

    let last_start = haystack.len() - needle.len();
    let candidates: Box<dyn Iterator<Item = usize>> = if from_end {
        Box::new((0..=last_start).rev())
    } else {
        Box::new(0..=last_start)
    };

    candidates
        .into_iter()
        .find(|start| window_matches(haystack, needle, *start, ignore_case))
}

fn ignore_case_pin(node: &mut Node) {
    node.add_input_pin(
        "ignore_case",
        "Ignore Case",
        "Compare without regard to upper/lower case",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIndexOfNode {}

impl StringIndexOfNode {
    pub fn new() -> Self {
        StringIndexOfNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIndexOfNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_index_of",
            "Index Of",
            "Finds the character index of the first occurrence of a substring",
            "Utils/String",
        );
        node.set_flowscript_name("string", "indexOf");
        node.set_receiver("string");
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "substring",
            "Substring",
            "Substring to search for",
            VariableType::String,
        );
        ignore_case_pin(&mut node);

        node.add_output_pin(
            "index",
            "Index",
            "Character index of the match, -1 when not found",
            VariableType::Integer,
        );
        node.add_output_pin(
            "found",
            "Found",
            "True when the substring occurs in the string",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let substring: String = context.evaluate_pin("substring").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let haystack: Vec<char> = string.chars().collect();
        let needle: Vec<char> = substring.chars().collect();
        let found = char_index_of(&haystack, &needle, ignore_case, false);

        context
            .set_pin_value("index", json!(found.map(|i| i as i64).unwrap_or(-1)))
            .await?;
        context
            .set_pin_value("found", json!(found.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringLastIndexOfNode {}

impl StringLastIndexOfNode {
    pub fn new() -> Self {
        StringLastIndexOfNode {}
    }
}

#[async_trait]
impl NodeLogic for StringLastIndexOfNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_last_index_of",
            "Last Index Of",
            "Finds the character index of the last occurrence of a substring",
            "Utils/String",
        );
        node.set_flowscript_name("string", "lastIndexOf");
        node.set_receiver("string");
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "substring",
            "Substring",
            "Substring to search for",
            VariableType::String,
        );
        ignore_case_pin(&mut node);

        node.add_output_pin(
            "index",
            "Index",
            "Character index of the last match, -1 when not found",
            VariableType::Integer,
        );
        node.add_output_pin(
            "found",
            "Found",
            "True when the substring occurs in the string",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let substring: String = context.evaluate_pin("substring").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let haystack: Vec<char> = string.chars().collect();
        let needle: Vec<char> = substring.chars().collect();
        let found = char_index_of(&haystack, &needle, ignore_case, true);

        context
            .set_pin_value("index", json!(found.map(|i| i as i64).unwrap_or(-1)))
            .await?;
        context
            .set_pin_value("found", json!(found.is_some()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringCountMatchesNode {}

impl StringCountMatchesNode {
    pub fn new() -> Self {
        StringCountMatchesNode {}
    }
}

#[async_trait]
impl NodeLogic for StringCountMatchesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_count_matches",
            "Count Matches",
            "Counts non-overlapping occurrences of a substring",
            "Utils/String",
        );
        node.set_flowscript_name("string", "countMatches");
        node.set_receiver("string");
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "substring",
            "Substring",
            "Substring to count",
            VariableType::String,
        );
        ignore_case_pin(&mut node);

        node.add_output_pin(
            "count",
            "Count",
            "Number of non-overlapping occurrences",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let substring: String = context.evaluate_pin("substring").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let haystack: Vec<char> = string.chars().collect();
        let needle: Vec<char> = substring.chars().collect();

        let mut count: i64 = 0;
        let mut cursor = 0usize;
        while !needle.is_empty() && cursor + needle.len() <= haystack.len() {
            match char_index_of(&haystack[cursor..], &needle, ignore_case, false) {
                Some(at) => {
                    count += 1;
                    cursor += at + needle.len();
                }
                None => break,
            }
        }

        context.set_pin_value("count", json!(count)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringContainsAnyNode {}

impl StringContainsAnyNode {
    pub fn new() -> Self {
        StringContainsAnyNode {}
    }
}

#[async_trait]
impl NodeLogic for StringContainsAnyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_contains_any",
            "Contains Any",
            "Checks whether a string contains any of the given substrings",
            "Utils/String",
        );
        node.set_flowscript_name("string", "containsAny");
        node.set_receiver("string");
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "substrings",
            "Substrings",
            "Substrings to search for",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        ignore_case_pin(&mut node);

        node.add_output_pin(
            "contains",
            "Contains",
            "True when at least one substring occurs",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "matched",
            "Matched",
            "The first substring that occurred",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let substrings: Vec<String> = context.evaluate_pin("substrings").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let haystack: Vec<char> = string.chars().collect();
        let matched = substrings.into_iter().find(|candidate| {
            let needle: Vec<char> = candidate.chars().collect();
            char_index_of(&haystack, &needle, ignore_case, false).is_some()
        });

        context
            .set_pin_value("contains", json!(matched.is_some()))
            .await?;
        context
            .set_pin_value("matched", json!(matched.unwrap_or_default()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringStartsWithAnyNode {}

impl StringStartsWithAnyNode {
    pub fn new() -> Self {
        StringStartsWithAnyNode {}
    }
}

#[async_trait]
impl NodeLogic for StringStartsWithAnyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_starts_with_any",
            "Starts With Any",
            "Checks whether a string starts with any of the given prefixes",
            "Utils/String",
        );
        node.set_flowscript_name("string", "startsWithAny");
        node.set_receiver("string");
        node.add_icon("/flow/icons/text-search.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "prefixes",
            "Prefixes",
            "Prefixes to test",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        ignore_case_pin(&mut node);

        node.add_output_pin(
            "starts_with",
            "Starts With",
            "True when the string starts with one of the prefixes",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "matched",
            "Matched",
            "The first prefix that matched",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let prefixes: Vec<String> = context.evaluate_pin("prefixes").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let haystack: Vec<char> = string.chars().collect();
        let matched = prefixes.into_iter().find(|candidate| {
            let needle: Vec<char> = candidate.chars().collect();
            !needle.is_empty()
                && needle.len() <= haystack.len()
                && window_matches(&haystack, &needle, 0, ignore_case)
        });

        context
            .set_pin_value("starts_with", json!(matched.is_some()))
            .await?;
        context
            .set_pin_value("matched", json!(matched.unwrap_or_default()))
            .await?;
        Ok(())
    }
}

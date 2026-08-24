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
pub struct StringIsEmptyNode {}

impl StringIsEmptyNode {
    pub fn new() -> Self {
        StringIsEmptyNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsEmptyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_is_empty",
            "Is Empty",
            "Checks whether a string contains no characters",
            "Utils/String",
        );
        node.set_flowscript_name("string", "isEmpty");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "ignore_whitespace",
            "Ignore Whitespace",
            "Treat whitespace-only strings as empty",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "is_empty",
            "Is Empty",
            "True when the string is empty",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let ignore_whitespace: bool = context.evaluate_pin("ignore_whitespace").await?;

        let is_empty = if ignore_whitespace {
            string.trim().is_empty()
        } else {
            string.is_empty()
        };

        context.set_pin_value("is_empty", json!(is_empty)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringToCharsNode {}

impl StringToCharsNode {
    pub fn new() -> Self {
        StringToCharsNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToCharsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_chars",
            "To Characters",
            "Splits a string into an array of single characters",
            "Utils/String",
        );
        node.set_flowscript_name("string", "toChars");
        node.set_receiver("string");
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "characters",
            "Characters",
            "One entry per character",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let characters: Vec<String> = string.chars().map(|c| c.to_string()).collect();
        context
            .set_pin_value("characters", json!(characters))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringCharAtNode {}

impl StringCharAtNode {
    pub fn new() -> Self {
        StringCharAtNode {}
    }
}

#[async_trait]
impl NodeLogic for StringCharAtNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_char_at",
            "Character At",
            "Returns the character at a index. Negative indices count from the end",
            "Utils/String",
        );
        node.set_flowscript_name("string", "charAt");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "index",
            "Index",
            "Character index, negative counts from the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "character",
            "Character",
            "The character at the index, empty when out of range",
            VariableType::String,
        );
        node.add_output_pin(
            "found",
            "Found",
            "True when the index was in range",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let index: i64 = context.evaluate_pin("index").await?;

        let chars: Vec<char> = string.chars().collect();
        let resolved = if index < 0 {
            chars.len() as i64 + index
        } else {
            index
        };

        let character = if resolved >= 0 && (resolved as usize) < chars.len() {
            Some(chars[resolved as usize].to_string())
        } else {
            None
        };

        context
            .set_pin_value("found", json!(character.is_some()))
            .await?;
        context
            .set_pin_value("character", json!(character.unwrap_or_default()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringReverseNode {}

impl StringReverseNode {
    pub fn new() -> Self {
        StringReverseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringReverseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_reverse",
            "Reverse String",
            "Reverses the characters of a string",
            "Utils/String",
        );
        node.set_flowscript_name("string", "reverse");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "reversed",
            "Reversed",
            "The reversed string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let reversed: String = string.chars().rev().collect();
        context.set_pin_value("reversed", json!(reversed)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringRepeatNode {}

impl StringRepeatNode {
    pub fn new() -> Self {
        StringRepeatNode {}
    }
}

#[async_trait]
impl NodeLogic for StringRepeatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_repeat",
            "Repeat String",
            "Repeats a string a number of times",
            "Utils/String",
        );
        node.set_flowscript_name("string", "repeat");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "count",
            "Count",
            "How often the string is repeated",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_output_pin(
            "repeated",
            "Repeated",
            "The repeated string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let count: i64 = context.evaluate_pin("count").await?;

        let count = count.clamp(0, 100_000) as usize;
        if string.len().saturating_mul(count) > 10_000_000 {
            return Err(flow_like_types::anyhow!(
                "Repeating a {} byte string {} times exceeds the 10 MB limit",
                string.len(),
                count
            ));
        }

        context
            .set_pin_value("repeated", json!(string.repeat(count)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringLinesNode {}

impl StringLinesNode {
    pub fn new() -> Self {
        StringLinesNode {}
    }
}

#[async_trait]
impl NodeLogic for StringLinesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_lines",
            "Lines",
            "Splits a string into its lines",
            "Utils/String",
        );
        node.set_flowscript_name("string", "lines");
        node.set_receiver("string");
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "skip_empty",
            "Skip Empty",
            "Drop lines that are empty or whitespace only",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("lines", "Lines", "One entry per line", VariableType::String)
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let skip_empty: bool = context.evaluate_pin("skip_empty").await?;

        let lines: Vec<String> = string
            .lines()
            .filter(|line| !skip_empty || !line.trim().is_empty())
            .map(|line| line.to_string())
            .collect();

        context.set_pin_value("lines", json!(lines)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringConcatNode {}

impl StringConcatNode {
    pub fn new() -> Self {
        StringConcatNode {}
    }
}

#[async_trait]
impl NodeLogic for StringConcatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_concat",
            "Concat Strings",
            "Appends strings to each other without a separator",
            "Utils/String",
        );
        node.set_flowscript_name("string", "concat");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Part to append", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("string", "String", "Part to append", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin(
            "concatenated",
            "Concatenated",
            "All parts appended in order",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let pins = context.get_pins_by_name("string").await?;

        let mut concatenated = String::new();
        for pin in pins {
            let part: String = context.evaluate_pin_ref(pin).await?;
            concatenated.push_str(&part);
        }

        context
            .set_pin_value("concatenated", json!(concatenated))
            .await?;
        Ok(())
    }
}

use super::case::words;
use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use regex::Regex;

fn smart_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/String");
    node.add_icon("/flow/icons/string.svg");
    node.set_scores(pure_scores());
    node.add_input_pin("string", "String", "Input String", VariableType::String);
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct StringSlugifyNode {}

impl StringSlugifyNode {
    pub fn new() -> Self {
        StringSlugifyNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSlugifyNode {
    fn get_node(&self) -> Node {
        let mut node = smart_node(
            "string_slugify",
            "Slugify",
            "Turns text into a URL safe slug",
        );
        node.set_flowscript_name("string", "slugify");
        node.set_receiver("string");
        node.add_input_pin(
            "separator",
            "Separator",
            "Placed between words",
            VariableType::String,
        )
        .set_default_value(Some(json!("-")));
        node.add_input_pin(
            "max_length",
            "Max Length",
            "Cut the slug at a word boundary, 0 for no limit",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("slug", "Slug", "The slug", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let separator: String = context.evaluate_pin("separator").await?;
        let max_length: i64 = context.evaluate_pin("max_length").await?;

        let ascii = string
            .chars()
            .map(|character| match character {
                'ä' | 'Ä' => "ae".to_string(),
                'ö' | 'Ö' => "oe".to_string(),
                'ü' | 'Ü' => "ue".to_string(),
                'ß' => "ss".to_string(),
                'å' | 'Å' => "aa".to_string(),
                'æ' | 'Æ' => "ae".to_string(),
                'ø' | 'Ø' => "oe".to_string(),
                other if other.is_ascii() => other.to_string(),
                other => deaccent(other),
            })
            .collect::<String>();

        let mut parts: Vec<String> = words(&ascii);
        if max_length > 0 {
            let limit = max_length as usize;
            let mut length = 0;
            parts.retain(|word| {
                let next = if length == 0 {
                    word.len()
                } else {
                    length + separator.len() + word.len()
                };
                if next <= limit {
                    length = next;
                    true
                } else {
                    false
                }
            });
        }

        context
            .set_pin_value("slug", json!(parts.join(&separator)))
            .await?;
        Ok(())
    }
}

/// Strips the common Latin accents so slugs stay readable instead of dropping
/// characters. Anything else falls away, which is what a slug wants.
fn deaccent(character: char) -> String {
    const TABLE: [(&str, char); 6] = [
        ("aàáâãåā", 'a'),
        ("eèéêëē", 'e'),
        ("iìíîïī", 'i'),
        ("oòóôõō", 'o'),
        ("uùúûū", 'u'),
        ("cçć", 'c'),
    ];

    let lower = character.to_lowercase().next().unwrap_or(character);
    for (group, replacement) in TABLE {
        if group.contains(lower) {
            return replacement.to_string();
        }
    }
    if lower == 'ñ' {
        return "n".to_string();
    }
    String::new()
}

#[crate::register_node]
#[derive(Default)]
pub struct StringWordCountNode {}

impl StringWordCountNode {
    pub fn new() -> Self {
        StringWordCountNode {}
    }
}

#[async_trait]
impl NodeLogic for StringWordCountNode {
    fn get_node(&self) -> Node {
        let mut node = smart_node(
            "string_word_count",
            "Word Count",
            "Counts words, sentences and reading time",
        );
        node.set_flowscript_name("string", "wordCount");
        node.set_receiver("string");
        node.add_input_pin(
            "words_per_minute",
            "Words Per Minute",
            "Reading speed used for the estimate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(200)));

        node.add_output_pin("words", "Words", "Number of words", VariableType::Integer);
        node.add_output_pin(
            "characters",
            "Characters",
            "Number of characters",
            VariableType::Integer,
        );
        node.add_output_pin(
            "sentences",
            "Sentences",
            "Number of sentences",
            VariableType::Integer,
        );
        node.add_output_pin(
            "reading_seconds",
            "Reading Seconds",
            "Estimated reading time in seconds",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let words_per_minute: i64 = context.evaluate_pin("words_per_minute").await?;

        let words = string.split_whitespace().count() as i64;
        let sentences = string
            .split(['.', '!', '?', '\n'])
            .filter(|part| !part.trim().is_empty())
            .count() as i64;
        let speed = words_per_minute.max(1);

        context.set_pin_value("words", json!(words)).await?;
        context
            .set_pin_value("characters", json!(string.chars().count() as i64))
            .await?;
        context.set_pin_value("sentences", json!(sentences)).await?;
        context
            .set_pin_value("reading_seconds", json!(words * 60 / speed))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringMaskNode {}

impl StringMaskNode {
    pub fn new() -> Self {
        StringMaskNode {}
    }
}

#[async_trait]
impl NodeLogic for StringMaskNode {
    fn get_node(&self) -> Node {
        let mut node = smart_node(
            "string_mask",
            "Mask",
            "Hides the middle of a value, keeping a few characters visible",
        );
        node.set_flowscript_name("string", "mask");
        node.set_receiver("string");
        node.add_input_pin(
            "keep_start",
            "Keep Start",
            "Characters left visible at the start",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "keep_end",
            "Keep End",
            "Characters left visible at the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));
        node.add_input_pin(
            "mask_character",
            "Mask Character",
            "Character used for the hidden part",
            VariableType::String,
        )
        .set_default_value(Some(json!("*")));
        node.add_input_pin(
            "fixed_width",
            "Fixed Width",
            "Always use this many mask characters so the length is not leaked, 0 keeps the real length",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("masked", "Masked", "The masked value", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let keep_start: i64 = context.evaluate_pin("keep_start").await?;
        let keep_end: i64 = context.evaluate_pin("keep_end").await?;
        let mask_character: String = context.evaluate_pin("mask_character").await?;
        let fixed_width: i64 = context.evaluate_pin("fixed_width").await?;

        let characters: Vec<char> = string.chars().collect();
        let keep_start = keep_start.max(0) as usize;
        let keep_end = keep_end.max(0) as usize;
        let mask = mask_character.chars().next().unwrap_or('*');

        if keep_start + keep_end >= characters.len() {
            let hidden = if fixed_width > 0 {
                fixed_width as usize
            } else {
                characters.len()
            };
            context
                .set_pin_value("masked", json!(mask.to_string().repeat(hidden)))
                .await?;
            return Ok(());
        }

        let hidden = if fixed_width > 0 {
            fixed_width as usize
        } else {
            characters.len() - keep_start - keep_end
        };

        let mut masked: String = characters.iter().take(keep_start).collect();
        masked.push_str(&mask.to_string().repeat(hidden));
        masked.extend(characters.iter().skip(characters.len() - keep_end));

        context.set_pin_value("masked", json!(masked)).await?;
        Ok(())
    }
}

const PATTERNS: [(&str, &str); 6] = [
    ("Emails", r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
    ("URLs", r#"https?://[^\s<>)\]"']+"#),
    ("Numbers", r"-?\d+(?:[.,]\d+)?"),
    ("IP Addresses", r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
    ("Hashtags", r"#[A-Za-z0-9_]+"),
    ("Mentions", r"@[A-Za-z0-9_.]+"),
];

#[crate::register_node]
#[derive(Default)]
pub struct StringExtractNode {}

impl StringExtractNode {
    pub fn new() -> Self {
        StringExtractNode {}
    }
}

#[async_trait]
impl NodeLogic for StringExtractNode {
    fn get_node(&self) -> Node {
        let mut node = smart_node(
            "string_extract",
            "Extract",
            "Pulls every email, link, number or handle out of a text",
        );
        node.set_flowscript_name("string", "extract");
        node.set_receiver("string");
        node.add_input_pin(
            "pattern",
            "Pattern",
            "What to look for",
            VariableType::String,
        )
        .set_default_value(Some(json!("Emails")))
        .set_options(
            PinOptions::new()
                .set_valid_values(PATTERNS.iter().map(|(name, _)| name.to_string()).collect())
                .build(),
        );
        node.add_input_pin(
            "unique",
            "Unique",
            "Drop repeated matches",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "matches",
            "Matches",
            "Everything that matched, in order",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "count",
            "Count",
            "How many matches were found",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;
        let unique: bool = context.evaluate_pin("unique").await?;

        let expression = PATTERNS
            .iter()
            .find(|(name, _)| *name == pattern)
            .map(|(_, expression)| *expression)
            .ok_or_else(|| flow_like_types::anyhow!("Unknown extract pattern {pattern}"))?;

        let regex = Regex::new(expression)?;
        let mut matches: Vec<String> = regex
            .find_iter(&string)
            .map(|found| found.as_str().to_string())
            .collect();

        if unique {
            let mut seen: Vec<String> = Vec::with_capacity(matches.len());
            matches.retain(|found| {
                if seen.contains(found) {
                    false
                } else {
                    seen.push(found.clone());
                    true
                }
            });
        }

        context
            .set_pin_value("count", json!(matches.len() as i64))
            .await?;
        context.set_pin_value("matches", json!(matches)).await?;
        Ok(())
    }
}

use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Splits arbitrary input into lowercase words, honouring separators as well as
/// camelCase and acronym boundaries.
pub fn words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = input.chars().collect();
    for (index, character) in chars.iter().enumerate() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        let previous = index.checked_sub(1).map(|i| chars[i]);
        let next = chars.get(index + 1).copied();
        let starts_word = match previous {
            Some(previous) if previous.is_lowercase() && character.is_uppercase() => true,
            Some(previous)
                if previous.is_uppercase()
                    && character.is_uppercase()
                    && next.is_some_and(|next| next.is_lowercase()) =>
            {
                true
            }
            _ => false,
        };

        if starts_word && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(character.to_lowercase());
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn case_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/String/Case");
    node.add_icon("/flow/icons/string.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("string", "String", "Input String", VariableType::String);
    node.add_output_pin(
        "result",
        "Result",
        "The converted string",
        VariableType::String,
    );

    node
}

#[crate::register_node]
#[derive(Default)]
pub struct StringCapitalizeNode {}

impl StringCapitalizeNode {
    pub fn new() -> Self {
        StringCapitalizeNode {}
    }
}

#[async_trait]
impl NodeLogic for StringCapitalizeNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_capitalize",
            "Capitalize",
            "Upper cases the first character and leaves the rest untouched",
        );
        node.set_flowscript_name("string", "capitalize");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("result", json!(capitalize_word(&string)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringTitleCaseNode {}

impl StringTitleCaseNode {
    pub fn new() -> Self {
        StringTitleCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTitleCaseNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_title_case",
            "Title Case",
            "Converts a string to Title Case",
        );
        node.set_flowscript_name("string", "titleCase");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let result = words(&string)
            .iter()
            .map(|word| capitalize_word(word))
            .collect::<Vec<_>>()
            .join(" ");
        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringSnakeCaseNode {}

impl StringSnakeCaseNode {
    pub fn new() -> Self {
        StringSnakeCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringSnakeCaseNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_snake_case",
            "snake_case",
            "Converts a string to snake_case",
        );
        node.set_flowscript_name("string", "snakeCase");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("result", json!(words(&string).join("_")))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringKebabCaseNode {}

impl StringKebabCaseNode {
    pub fn new() -> Self {
        StringKebabCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringKebabCaseNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_kebab_case",
            "kebab-case",
            "Converts a string to kebab-case",
        );
        node.set_flowscript_name("string", "kebabCase");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("result", json!(words(&string).join("-")))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringCamelCaseNode {}

impl StringCamelCaseNode {
    pub fn new() -> Self {
        StringCamelCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringCamelCaseNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_camel_case",
            "camelCase",
            "Converts a string to camelCase or PascalCase",
        );
        node.set_flowscript_name("string", "camelCase");
        node.set_receiver("string");
        node.add_input_pin(
            "pascal_case",
            "Pascal Case",
            "Upper case the first word as well",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pascal_case: bool = context.evaluate_pin("pascal_case").await?;

        let result = words(&string)
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 && !pascal_case {
                    word.clone()
                } else {
                    capitalize_word(word)
                }
            })
            .collect::<String>();

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

/// The case styles this catalog can read and write. The value is the label the
/// dropdown shows, so detection and conversion speak the same vocabulary and a
/// detected case can be fed straight back into the target pin.
const CASE_STYLES: [&str; 11] = [
    "camelCase",
    "PascalCase",
    "snake_case",
    "SCREAMING_SNAKE_CASE",
    "kebab-case",
    "SCREAMING-KEBAB-CASE",
    "Train-Case",
    "dot.case",
    "path/case",
    "Title Case",
    "Sentence case",
];

/// Reported when the input carries no evidence of a style — a single lowercase
/// word, digits, or nothing at all. Converting still works; there is simply
/// nothing to name.
const UNDETERMINED: &str = "undetermined";

fn is_capitalized(word: &str) -> bool {
    word.chars().next().is_some_and(char::is_uppercase)
}

/// Names the case style `input` is written in.
///
/// Separators are decisive when present, so the checks run from the most
/// specific evidence to the least: a separator plus its capitalisation pattern,
/// then — for a single token — where the uppercase letters fall.
pub fn detect_case(input: &str) -> &'static str {
    let raw_words: Vec<&str> = input
        .split(['_', '-', '.', '/', ' ', '\t', '\n'])
        .filter(|word| !word.is_empty())
        .collect();

    if raw_words.is_empty() {
        return UNDETERMINED;
    }

    let all_upper = raw_words.iter().all(|word| {
        word.chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase)
    });
    let all_capitalized = raw_words.iter().all(|word| is_capitalized(word));

    if input.contains('_') {
        return if all_upper && raw_words.len() > 1 {
            "SCREAMING_SNAKE_CASE"
        } else {
            "snake_case"
        };
    }

    if input.contains('-') {
        return if all_upper && raw_words.len() > 1 {
            "SCREAMING-KEBAB-CASE"
        } else if all_capitalized {
            "Train-Case"
        } else {
            "kebab-case"
        };
    }

    if input.contains('/') {
        return "path/case";
    }

    if input.contains('.') {
        return "dot.case";
    }

    if raw_words.len() > 1 {
        return if all_capitalized {
            "Title Case"
        } else {
            "Sentence case"
        };
    }

    let single = raw_words[0];
    let has_inner_upper = single.chars().skip(1).any(char::is_uppercase);

    if is_capitalized(single) {
        // A lone all-caps token is an acronym, not evidence of a style.
        if has_inner_upper && single.chars().all(|c| !c.is_lowercase()) {
            return UNDETERMINED;
        }
        return "PascalCase";
    }

    if has_inner_upper {
        return "camelCase";
    }

    UNDETERMINED
}

/// Rewrites `input` in `style`. Any input is accepted: [`words`] reduces it to
/// lowercase words first, so the source style never has to be known.
pub fn convert_case(input: &str, style: &str) -> String {
    let words = words(input);
    if words.is_empty() {
        return String::new();
    }

    let joined_capitalized = |separator: &str| {
        words
            .iter()
            .map(|word| capitalize_word(word))
            .collect::<Vec<_>>()
            .join(separator)
    };

    match style {
        "camelCase" | "PascalCase" => words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 && style == "camelCase" {
                    word.clone()
                } else {
                    capitalize_word(word)
                }
            })
            .collect(),
        "snake_case" => words.join("_"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        "kebab-case" => words.join("-"),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        "Train-Case" => joined_capitalized("-"),
        "dot.case" => words.join("."),
        "path/case" => words.join("/"),
        "Title Case" => joined_capitalized(" "),
        "Sentence case" => {
            let sentence = words.join(" ");
            capitalize_word(&sentence)
        }
        _ => words.join(" "),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringConvertCaseNode {}

impl StringConvertCaseNode {
    pub fn new() -> Self {
        StringConvertCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringConvertCaseNode {
    fn get_node(&self) -> Node {
        let mut node = case_node(
            "string_convert_case",
            "Convert Case",
            "Rewrites a string in the chosen case style. The input's own style is detected automatically, so any of the supported styles can be fed in",
        );
        node.set_flowscript_name("string", "convertCase");
        node.set_receiver("string");

        node.add_input_pin(
            "target_case",
            "To Case",
            "The case style to write the string in",
            VariableType::String,
        )
        .set_default_value(Some(json!("snake_case")))
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(CASE_STYLES.iter().map(|style| style.to_string()).collect())
                .build(),
        );

        node.add_output_pin(
            "detected_case",
            "From Case",
            "The case style the input was written in, or \"undetermined\" when it carries no evidence of one",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let target_case: String = context.evaluate_pin("target_case").await?;

        context
            .set_pin_value("detected_case", json!(detect_case(&string)))
            .await?;
        context
            .set_pin_value("result", json!(convert_case(&string, &target_case)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringDetectCaseNode {}

impl StringDetectCaseNode {
    pub fn new() -> Self {
        StringDetectCaseNode {}
    }
}

#[async_trait]
impl NodeLogic for StringDetectCaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_detect_case",
            "Detect Case",
            "Names the case style a string is written in",
            "Utils/String/Case",
        );
        node.set_flowscript_name("string", "detectCase");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_output_pin(
            "detected_case",
            "Case",
            "The detected case style, or \"undetermined\" when the string carries no evidence of one",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        context
            .set_pin_value("detected_case", json!(detect_case(&string)))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every style must round-trip through every other one: that is the whole
    /// point of detecting the input rather than asking for it.
    #[test]
    fn every_style_converts_into_every_other_style() {
        let expected = [
            ("camelCase", "userAccountId"),
            ("PascalCase", "UserAccountId"),
            ("snake_case", "user_account_id"),
            ("SCREAMING_SNAKE_CASE", "USER_ACCOUNT_ID"),
            ("kebab-case", "user-account-id"),
            ("SCREAMING-KEBAB-CASE", "USER-ACCOUNT-ID"),
            ("Train-Case", "User-Account-Id"),
            ("dot.case", "user.account.id"),
            ("path/case", "user/account/id"),
            ("Title Case", "User Account Id"),
            ("Sentence case", "User account id"),
        ];

        for (_, source) in expected {
            for (style, want) in expected {
                assert_eq!(
                    convert_case(source, style),
                    want,
                    "converting {source:?} to {style}"
                );
            }
        }
    }

    #[test]
    fn detection_names_the_style_the_input_is_written_in() {
        for (input, want) in [
            ("userAccountId", "camelCase"),
            ("UserAccountId", "PascalCase"),
            ("user_account_id", "snake_case"),
            ("USER_ACCOUNT_ID", "SCREAMING_SNAKE_CASE"),
            ("user-account-id", "kebab-case"),
            ("USER-ACCOUNT-ID", "SCREAMING-KEBAB-CASE"),
            ("User-Account-Id", "Train-Case"),
            ("user.account.id", "dot.case"),
            ("user/account/id", "path/case"),
            ("User Account Id", "Title Case"),
            ("User account id", "Sentence case"),
        ] {
            assert_eq!(detect_case(input), want, "detecting {input:?}");
        }
    }

    /// Detection feeds a target pin, so it must only ever name a style the
    /// converter accepts.
    #[test]
    fn detection_only_reports_styles_the_converter_knows() {
        for input in [
            "userAccountId",
            "USER_ACCOUNT_ID",
            "User-Account-Id",
            "user",
            "HTTP",
            "",
            "42",
        ] {
            let detected = detect_case(input);
            assert!(
                detected == UNDETERMINED || CASE_STYLES.contains(&detected),
                "{input:?} was detected as {detected:?}, which is not a style"
            );
        }
    }

    #[test]
    fn a_string_with_no_style_to_speak_of_is_undetermined() {
        for input in ["", "   ", "user", "42", "HTTP"] {
            assert_eq!(detect_case(input), UNDETERMINED, "for {input:?}");
        }
    }

    /// Acronyms are where naive splitting produces `h_t_t_p_response`.
    #[test]
    fn acronyms_stay_whole_across_a_conversion() {
        assert_eq!(
            convert_case("HTTPResponseCode", "snake_case"),
            "http_response_code"
        );
        assert_eq!(
            convert_case("parseHTTPResponse", "kebab-case"),
            "parse-http-response"
        );
        assert_eq!(convert_case("io_error_2", "PascalCase"), "IoError2");
    }

    #[test]
    fn an_empty_input_converts_to_an_empty_string() {
        for style in CASE_STYLES {
            assert_eq!(convert_case("", style), "");
            assert_eq!(convert_case("   ", style), "");
        }
    }

    /// An unknown target must not silently drop the string.
    #[test]
    fn an_unrecognised_target_falls_back_to_spaced_words() {
        assert_eq!(
            convert_case("userAccountId", "no-such-style"),
            "user account id"
        );
    }
}

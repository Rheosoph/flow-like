use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn predicate_node(id: &str, label: &str, description: &str, output_description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/String");
    node.add_icon("/flow/icons/string.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("string", "String", "Input String", VariableType::String);
    node.add_output_pin(
        "result",
        "Result",
        output_description,
        VariableType::Boolean,
    );

    node
}

async fn evaluate(
    context: &mut ExecutionContext,
    predicate: impl Fn(&str) -> bool,
) -> flow_like_types::Result<()> {
    let string: String = context.evaluate_pin("string").await?;
    let result = !string.is_empty() && predicate(&string);
    context.set_pin_value("result", json!(result)).await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsNumericNode {}

impl StringIsNumericNode {
    pub fn new() -> Self {
        StringIsNumericNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsNumericNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_numeric",
            "Is Numeric",
            "Checks whether a string can be read as a number",
            "True when the string parses as a number",
        );
        node.set_flowscript_name("string", "isNumeric");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| value.trim().parse::<f64>().is_ok()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsAlphanumericNode {}

impl StringIsAlphanumericNode {
    pub fn new() -> Self {
        StringIsAlphanumericNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsAlphanumericNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_alphanumeric",
            "Is Alphanumeric",
            "Checks whether every character is a letter or a digit",
            "True when all characters are alphanumeric",
        );
        node.set_flowscript_name("string", "isAlphanumeric");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| value.chars().all(char::is_alphanumeric)).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsAsciiNode {}

impl StringIsAsciiNode {
    pub fn new() -> Self {
        StringIsAsciiNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsAsciiNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_ascii",
            "Is ASCII",
            "Checks whether a string only contains ASCII characters",
            "True when the string is pure ASCII",
        );
        node.set_flowscript_name("string", "isAscii");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| value.is_ascii()).await
    }
}

fn is_valid_email(value: &str) -> bool {
    let (local, domain) = match value.split_once('@') {
        Some(parts) => parts,
        None => return false,
    };

    !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..")
        && domain.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '.' || character == '-'
        })
        && !value.chars().any(char::is_whitespace)
}

fn is_valid_url(value: &str) -> bool {
    let rest = match value.split_once("://") {
        Some((scheme, rest)) => {
            if scheme.is_empty()
                || !scheme
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character))
            {
                return false;
            }
            rest
        }
        None => return false,
    };

    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

fn is_valid_uuid(value: &str) -> bool {
    let value = value.trim_matches(['{', '}']);
    let groups: Vec<&str> = value.split('-').collect();

    groups.len() == 5
        && [8usize, 4, 4, 4, 12]
            .iter()
            .zip(groups.iter())
            .all(|(length, group)| {
                group.len() == *length && group.chars().all(|c| c.is_ascii_hexdigit())
            })
}

fn is_valid_ip(value: &str) -> bool {
    if value.contains(':') {
        let groups: Vec<&str> = value.split(':').collect();
        return groups.len() >= 3
            && groups.len() <= 8
            && groups.iter().all(|group| {
                group.is_empty()
                    || (group.len() <= 4 && group.chars().all(|c| c.is_ascii_hexdigit()))
            });
    }

    let groups: Vec<&str> = value.split('.').collect();
    groups.len() == 4
        && groups.iter().all(|group| {
            !group.is_empty()
                && group.len() <= 3
                && group.chars().all(|c| c.is_ascii_digit())
                && group.parse::<u16>().is_ok_and(|octet| octet <= 255)
        })
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsEmailNode {}

impl StringIsEmailNode {
    pub fn new() -> Self {
        StringIsEmailNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsEmailNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_email",
            "Is Email",
            "Checks whether a string looks like an email address",
            "True when the string is a plausible email address",
        );
        node.set_flowscript_name("string", "isEmail");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| is_valid_email(value.trim())).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsUrlNode {}

impl StringIsUrlNode {
    pub fn new() -> Self {
        StringIsUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsUrlNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_url",
            "Is URL",
            "Checks whether a string is a URL with a scheme and a host",
            "True when the string is a plausible URL",
        );
        node.set_flowscript_name("string", "isUrl");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| is_valid_url(value.trim())).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsUuidNode {}

impl StringIsUuidNode {
    pub fn new() -> Self {
        StringIsUuidNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsUuidNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_uuid",
            "Is UUID",
            "Checks whether a string is a UUID",
            "True when the string is a UUID",
        );
        node.set_flowscript_name("string", "isUuid");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| is_valid_uuid(value.trim())).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsIpNode {}

impl StringIsIpNode {
    pub fn new() -> Self {
        StringIsIpNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsIpNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_ip",
            "Is IP Address",
            "Checks whether a string is an IPv4 or IPv6 address",
            "True when the string is an IP address",
        );
        node.set_flowscript_name("string", "isIp");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| is_valid_ip(value.trim())).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StringIsJsonNode {}

impl StringIsJsonNode {
    pub fn new() -> Self {
        StringIsJsonNode {}
    }
}

#[async_trait]
impl NodeLogic for StringIsJsonNode {
    fn get_node(&self) -> Node {
        let mut node = predicate_node(
            "string_is_json",
            "Is JSON",
            "Checks whether a string parses as JSON",
            "True when the string is valid JSON",
        );
        node.set_flowscript_name("string", "isJson");
        node.set_receiver("string");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        evaluate(context, |value| {
            flow_like_types::json::from_str::<flow_like_types::Value>(value.trim()).is_ok()
        })
        .await
    }
}

use super::key::{CompareMode, KEY_DESCRIPTION, resolve_path, sort_key};
use super::sort::{generic_array_pin, harmonized_element_type};
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

const OPERATORS: [&str; 12] = [
    "Equals",
    "Not Equals",
    "Contains",
    "Starts With",
    "Ends With",
    "Greater Than",
    "Greater Or Equal",
    "Less Than",
    "Less Or Equal",
    "Is Empty",
    "Is Not Empty",
    "In List",
];

fn as_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn is_absent(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(map)) => map.is_empty(),
        _ => false,
    }
}

fn ordering_holds(
    mode: CompareMode,
    left: Option<&Value>,
    right: &Value,
    keep: fn(std::cmp::Ordering) -> bool,
) -> bool {
    match (sort_key(mode, left), sort_key(mode, Some(right))) {
        (Some(left), Some(right)) => keep(left.compare(&right)),
        _ => false,
    }
}

fn matches(
    operator: &str,
    mode: CompareMode,
    found: Option<&Value>,
    expected: &str,
    ignore_case: bool,
) -> bool {
    let mut actual = as_text(found);
    let mut expected_text = expected.to_string();
    if ignore_case {
        actual = actual.to_lowercase();
        expected_text = expected_text.to_lowercase();
    }

    let expected_value = Value::String(expected.to_string());

    match operator {
        "Not Equals" => actual != expected_text,
        "Contains" => actual.contains(&expected_text),
        "Starts With" => actual.starts_with(&expected_text),
        "Ends With" => actual.ends_with(&expected_text),
        "Greater Than" => ordering_holds(mode, found, &expected_value, |order| order.is_gt()),
        "Greater Or Equal" => ordering_holds(mode, found, &expected_value, |order| order.is_ge()),
        "Less Than" => ordering_holds(mode, found, &expected_value, |order| order.is_lt()),
        "Less Or Equal" => ordering_holds(mode, found, &expected_value, |order| order.is_le()),
        "Is Empty" => is_absent(found),
        "Is Not Empty" => !is_absent(found),
        "In List" => expected_text
            .split(',')
            .any(|candidate| candidate.trim() == actual.trim()),
        _ => actual == expected_text,
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FilterArrayByNode {}

impl FilterArrayByNode {
    pub fn new() -> Self {
        FilterArrayByNode {}
    }
}

#[async_trait]
impl NodeLogic for FilterArrayByNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_filter_by",
            "Filter",
            "Keeps the elements whose key passes a comparison",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "filterBy");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/filter.svg");
        node.set_scores(pure_scores());

        generic_array_pin(&mut node, "array_in", "Array", "Your Array", false);
        node.add_input_pin("key", "Key", KEY_DESCRIPTION, VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "operator",
            "Operator",
            "How the key is compared against the value",
            VariableType::String,
        )
        .set_default_value(Some(json!("Equals")))
        .set_options(
            PinOptions::new()
                .set_valid_values(OPERATORS.iter().map(|name| name.to_string()).collect())
                .build(),
        );
        node.add_input_pin(
            "value",
            "Value",
            "What to compare against. In List takes a comma separated list",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "compare",
            "Compare As",
            "Comparator used by the ordering operators",
            VariableType::String,
        )
        .set_default_value(Some(json!("Auto")))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    super::key::COMPARE_MODES
                        .iter()
                        .map(|mode| mode.to_string())
                        .collect(),
                )
                .build(),
        );
        node.add_input_pin(
            "ignore_case",
            "Ignore Case",
            "Compare text without regard to upper/lower case",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "invert",
            "Invert",
            "Keep the elements that do not pass instead",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        generic_array_pin(&mut node, "array_out", "Array", "The kept elements", true);
        node.add_output_pin(
            "kept",
            "Kept",
            "How many elements passed",
            VariableType::Integer,
        );
        node.add_output_pin(
            "removed",
            "Removed",
            "How many elements were dropped",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let key: String = context.evaluate_pin("key").await?;
        let operator: String = context.evaluate_pin("operator").await?;
        let value: String = context.evaluate_pin("value").await?;
        let compare: String = context.evaluate_pin("compare").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;
        let invert: bool = context.evaluate_pin("invert").await?;

        let mode = CompareMode::from_name(&compare);
        let total = array.len();

        let kept: Vec<Value> = array
            .into_iter()
            .filter(|element| {
                let found = resolve_path(element, &key);
                matches(&operator, mode, found, &value, ignore_case) != invert
            })
            .collect();

        context
            .set_pin_value("kept", json!(kept.len() as i64))
            .await?;
        context
            .set_pin_value("removed", json!((total - kept.len()) as i64))
            .await?;
        context.set_pin_value("array_out", json!(kept)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = harmonized_element_type(node, board).await;
    }
}

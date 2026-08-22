use super::key::{
    CompareMode, KEY_DESCRIPTION, SortKey, add_compare_pins, add_nulls_pin, compare_keys,
    resolve_path, sort_key, sync_key_pin,
};
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

pub fn generic_array_pin(
    node: &mut Node,
    name: &str,
    label: &str,
    description: &str,
    output: bool,
) {
    let pin = if output {
        node.add_output_pin(name, label, description, VariableType::Generic)
    } else {
        node.add_input_pin(name, label, description, VariableType::Generic)
    };

    pin.set_value_type(ValueType::Array).set_options(
        PinOptions::new()
            .set_enforce_generic_value_type(true)
            .build(),
    );
}

fn array_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Array");
    node.add_icon("/flow/icons/grip.svg");
    node.set_scores(pure_scores());

    generic_array_pin(&mut node, "array_in", "Array", "Your Array", false);
    generic_array_pin(&mut node, "array_out", "Array", description, true);

    node
}

/// Resolves the element type from whatever the array pins are wired to, then
/// mirrors it onto the other side.
pub async fn harmonized_element_type(node: &mut Node, board: &Board) -> VariableType {
    let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
    node.harmonize_type(vec!["array_in", "array_out"], true);

    node.get_pin_by_name("array_in")
        .map(|pin| pin.data_type.clone())
        .unwrap_or(VariableType::Generic)
}

/// The comparator to use when the user left "Compare As" on Auto: a wired
/// scalar array already states its type, so honour it instead of sniffing.
pub async fn resolve_mode(
    context: &mut ExecutionContext,
    requested: &str,
    key: &str,
    array_pin: &str,
) -> flow_like_types::Result<CompareMode> {
    let mode = CompareMode::from_name(requested);
    if mode != CompareMode::Auto || !key.trim().is_empty() {
        return Ok(mode);
    }

    let pin = context.get_pin_by_name(array_pin).await?;
    Ok(CompareMode::from_element_type(&pin.data_type))
}

pub fn decorate(array: Vec<Value>, mode: CompareMode, key: &str) -> Vec<(Option<SortKey>, Value)> {
    array
        .into_iter()
        .map(|value| {
            let resolved = sort_key(mode, resolve_path(&value, key));
            (resolved, value)
        })
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct SortArrayNode {}

impl SortArrayNode {
    pub fn new() -> Self {
        SortArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for SortArrayNode {
    fn get_node(&self) -> Node {
        let mut node = array_node("array_sort", "Sort", "The sorted array");
        node.add_input_pin(
            "descending",
            "Descending",
            "Sort from largest to smallest",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        add_compare_pins(&mut node);
        add_nulls_pin(&mut node);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let descending: bool = context.evaluate_pin("descending").await?;
        let requested: String = context.evaluate_pin("compare").await?;
        let nulls: String = context.evaluate_pin("nulls").await?;
        let key: String = context.evaluate_pin("key").await.unwrap_or_default();

        let mode = resolve_mode(context, &requested, &key, "array_in").await?;
        let nulls_first = nulls == "First";

        // Keys are resolved once per element: comparison runs O(n log n) times and
        // date keys would otherwise be re-parsed on every single comparison.
        let mut decorated = decorate(array, mode, &key);
        decorated.sort_by(|(left, _), (right, _)| {
            compare_keys(left.as_ref(), right.as_ref(), nulls_first, descending)
        });

        let sorted: Vec<Value> = decorated.into_iter().map(|(_, value)| value).collect();
        context.set_pin_value("array_out", json!(sorted)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let element_type = harmonized_element_type(node, board).await;
        sync_key_pin(node, &element_type, KEY_DESCRIPTION);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ReverseArrayNode {}

impl ReverseArrayNode {
    pub fn new() -> Self {
        ReverseArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for ReverseArrayNode {
    fn get_node(&self) -> Node {
        array_node("array_reverse", "Reverse", "The reversed array")
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut array: Vec<Value> = context.evaluate_pin("array_in").await?;
        array.reverse();
        context.set_pin_value("array_out", json!(array)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = harmonized_element_type(node, board).await;
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SliceArrayNode {}

impl SliceArrayNode {
    pub fn new() -> Self {
        SliceArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for SliceArrayNode {
    fn get_node(&self) -> Node {
        let mut node = array_node("array_slice", "Slice", "The selected range of elements");
        node.add_input_pin(
            "start",
            "Start",
            "First index, negative counts from the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "length",
            "Length",
            "Number of elements to take, -1 for the rest of the array",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let start: i64 = context.evaluate_pin("start").await?;
        let length: i64 = context.evaluate_pin("length").await?;

        let total = array.len() as i64;
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

        let sliced: Vec<Value> = array
            .into_iter()
            .skip(start_index.max(0) as usize)
            .take(take.max(0) as usize)
            .collect();

        context.set_pin_value("array_out", json!(sliced)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = harmonized_element_type(node, board).await;
    }
}

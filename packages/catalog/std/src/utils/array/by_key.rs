//! Array nodes that read a key out of every element.

use super::key::{
    CompareMode, KEY_DESCRIPTION, add_compare_pins, add_nulls_pin, compare_keys, resolve_path,
    sync_key_pin,
};
use super::sort::{decorate, generic_array_pin, harmonized_element_type, resolve_mode};
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct KeyedGroup {
    pub key: Value,
    pub count: i64,
    pub items: Vec<Value>,
}

fn keyed_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Array");
    node.add_icon("/flow/icons/grip.svg");
    node.set_scores(pure_scores());

    generic_array_pin(&mut node, "array_in", "Array", "Your Array", false);

    node
}

async fn keyed_inputs(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<(Vec<Value>, String)> {
    let array: Vec<Value> = context.evaluate_pin("array_in").await?;
    let key: String = context.evaluate_pin("key").await.unwrap_or_default();
    Ok((array, key))
}

#[crate::register_node]
#[derive(Default)]
pub struct UniqueArrayNode {}

impl UniqueArrayNode {
    pub fn new() -> Self {
        UniqueArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for UniqueArrayNode {
    fn get_node(&self) -> Node {
        let mut node = keyed_node(
            "array_unique",
            "Unique",
            "The array without duplicate values",
        );
        node.set_flowscript_name("array", "unique");
        node.set_receiver("array_in");
        generic_array_pin(
            &mut node,
            "array_out",
            "Array",
            "The array without duplicates",
            true,
        );
        node.add_output_pin(
            "removed",
            "Removed",
            "How many duplicates were dropped",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (array, key) = keyed_inputs(context).await?;

        let mut seen: Vec<Value> = Vec::new();
        let mut kept: Vec<Value> = Vec::with_capacity(array.len());

        for value in array.iter() {
            let identity = resolve_path(value, &key).cloned().unwrap_or(Value::Null);
            if seen.contains(&identity) {
                continue;
            }
            seen.push(identity);
            kept.push(value.clone());
        }

        context
            .set_pin_value("removed", json!((array.len() - kept.len()) as i64))
            .await?;
        context.set_pin_value("array_out", json!(kept)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let element_type = harmonized_element_type(node, board).await;
        sync_key_pin(
            node,
            &element_type,
            "Field that decides identity, dot notation for nested fields. Empty compares whole elements",
        );
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GroupArrayByNode {}

impl GroupArrayByNode {
    pub fn new() -> Self {
        GroupArrayByNode {}
    }
}

#[async_trait]
impl NodeLogic for GroupArrayByNode {
    fn get_node(&self) -> Node {
        let mut node = keyed_node(
            "array_group_by",
            "Group By",
            "Groups elements that share the same key value",
        );
        node.set_flowscript_name("array", "groupBy");
        node.set_receiver("array_in");
        node.add_input_pin("key", "Key", KEY_DESCRIPTION, VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin(
            "groups",
            "Groups",
            "One entry per distinct key, in first-seen order",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<KeyedGroup>();
        node.add_output_pin(
            "group_count",
            "Group Count",
            "How many distinct keys were found",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (array, key) = keyed_inputs(context).await?;

        let mut keys: Vec<Value> = Vec::new();
        let mut buckets: Vec<Vec<Value>> = Vec::new();

        for value in array {
            let identity = resolve_path(&value, &key).cloned().unwrap_or(Value::Null);
            match keys.iter().position(|seen| seen == &identity) {
                Some(index) => buckets[index].push(value),
                None => {
                    keys.push(identity);
                    buckets.push(vec![value]);
                }
            }
        }

        let groups: Vec<Value> = keys
            .into_iter()
            .zip(buckets)
            .map(|(key, items)| json!({ "key": key, "count": items.len() as i64, "items": items }))
            .collect();

        context
            .set_pin_value("group_count", json!(groups.len() as i64))
            .await?;
        context.set_pin_value("groups", json!(groups)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PluckArrayNode {}

impl PluckArrayNode {
    pub fn new() -> Self {
        PluckArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for PluckArrayNode {
    fn get_node(&self) -> Node {
        let mut node = keyed_node(
            "array_pluck",
            "Pluck",
            "Reads one field out of every element",
        );
        node.set_flowscript_name("array", "pluck");
        node.set_receiver("array_in");
        node.add_input_pin("key", "Key", KEY_DESCRIPTION, VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "skip_missing",
            "Skip Missing",
            "Drop elements that do not have the field instead of emitting null",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "values",
            "Values",
            "The field value of every element",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (array, key) = keyed_inputs(context).await?;
        let skip_missing: bool = context.evaluate_pin("skip_missing").await?;

        let values: Vec<Value> = array
            .iter()
            .filter_map(|value| match resolve_path(value, &key) {
                Some(found) if !(skip_missing && found.is_null()) => Some(found.clone()),
                Some(_) => None,
                None if skip_missing => None,
                None => Some(Value::Null),
            })
            .collect();

        context.set_pin_value("values", json!(values)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    }
}

fn extreme_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = keyed_node(id, label, description);
    add_compare_pins(&mut node);
    add_nulls_pin(&mut node);

    node.add_output_pin("element", "Element", description, VariableType::Generic);
    node.add_output_pin(
        "index",
        "Index",
        "Position of the element in the array",
        VariableType::Integer,
    );
    node.add_output_pin(
        "found",
        "Found",
        "False when the array was empty",
        VariableType::Boolean,
    );

    node
}

async fn extreme_run(
    context: &mut ExecutionContext,
    take_largest: bool,
) -> flow_like_types::Result<()> {
    let (array, key) = keyed_inputs(context).await?;
    let requested: String = context.evaluate_pin("compare").await?;
    let nulls: String = context.evaluate_pin("nulls").await?;

    let mode: CompareMode = resolve_mode(context, &requested, &key, "array_in").await?;
    let nulls_first = nulls == "First";

    let decorated = decorate(array, mode, &key);
    let winner = decorated.iter().enumerate().reduce(|best, candidate| {
        let ordering = compare_keys(
            candidate.1.0.as_ref(),
            best.1.0.as_ref(),
            nulls_first,
            take_largest,
        );
        if ordering.is_lt() { candidate } else { best }
    });

    match winner {
        Some((index, (_, value))) => {
            context.set_pin_value("element", value.clone()).await?;
            context.set_pin_value("index", json!(index as i64)).await?;
            context.set_pin_value("found", json!(true)).await?;
        }
        None => {
            context.set_pin_value("element", Value::Null).await?;
            context.set_pin_value("index", json!(-1)).await?;
            context.set_pin_value("found", json!(false)).await?;
        }
    }

    Ok(())
}

async fn extreme_update(node: &mut Node, board: &Board) {
    let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    let _ = node.match_type("element", board, Some(ValueType::Normal), None);
    node.harmonize_type(vec!["element", "array_in"], true);

    let element_type = node
        .get_pin_by_name("array_in")
        .map(|pin| pin.data_type.clone())
        .unwrap_or(VariableType::Generic);
    sync_key_pin(node, &element_type, KEY_DESCRIPTION);
}

#[crate::register_node]
#[derive(Default)]
pub struct MinByArrayNode {}

impl MinByArrayNode {
    pub fn new() -> Self {
        MinByArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for MinByArrayNode {
    fn get_node(&self) -> Node {
        let mut node = extreme_node(
            "array_min_by",
            "Min By",
            "The element with the smallest key",
        );
        node.set_flowscript_name("array", "minBy");
        node.set_receiver("array_in");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        extreme_run(context, false).await
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        extreme_update(node, board).await;
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MaxByArrayNode {}

impl MaxByArrayNode {
    pub fn new() -> Self {
        MaxByArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for MaxByArrayNode {
    fn get_node(&self) -> Node {
        let mut node = extreme_node("array_max_by", "Max By", "The element with the largest key");
        node.set_flowscript_name("array", "maxBy");
        node.set_receiver("array_in");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        extreme_run(context, true).await
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        extreme_update(node, board).await;
    }
}

use super::key::resolve_path;
use super::sort::generic_array_pin;
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct JoinedPair {
    pub left: Value,
    pub right: Value,
    pub matched: bool,
}

#[crate::register_node]
#[derive(Default)]
pub struct JoinArraysNode {}

impl JoinArraysNode {
    pub fn new() -> Self {
        JoinArraysNode {}
    }
}

#[async_trait]
impl NodeLogic for JoinArraysNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_join_by",
            "Join By Key",
            "Matches the elements of two arrays on a shared key, the way a database join does",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");
        node.set_scores(pure_scores());

        generic_array_pin(&mut node, "array_left", "Left", "Left Array", false);
        generic_array_pin(&mut node, "array_right", "Right", "Right Array", false);
        node.add_input_pin(
            "key_left",
            "Left Key",
            "Field on the left elements, dot notation for nested fields. Empty uses the element itself",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "key_right",
            "Right Key",
            "Field on the right elements. Empty reuses the left key",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "join",
            "Join",
            "Inner keeps only matches, Left keeps every left element",
            VariableType::String,
        )
        .set_default_value(Some(json!("Inner")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Inner".to_string(), "Left".to_string()])
                .build(),
        );

        node.add_output_pin(
            "pairs",
            "Pairs",
            "One entry per match, holding both sides",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JoinedPair>();
        node.add_output_pin(
            "matched",
            "Matched",
            "How many left elements found a partner",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let left: Vec<Value> = context.evaluate_pin("array_left").await?;
        let right: Vec<Value> = context.evaluate_pin("array_right").await?;
        let key_left: String = context.evaluate_pin("key_left").await?;
        let key_right: String = context.evaluate_pin("key_right").await?;
        let join: String = context.evaluate_pin("join").await?;

        let key_right = if key_right.trim().is_empty() {
            key_left.clone()
        } else {
            key_right
        };
        let keep_unmatched = join == "Left";

        // The right side is indexed once; a nested scan turns a 1k x 1k join into
        // a million comparisons.
        let indexed: Vec<(Option<&Value>, &Value)> = right
            .iter()
            .map(|value| (resolve_path(value, &key_right), value))
            .collect();

        let mut pairs: Vec<Value> = Vec::new();
        let mut matched = 0i64;

        for entry in left.iter() {
            let identity = resolve_path(entry, &key_left);
            let partners: Vec<&Value> = match identity {
                Some(identity) if !identity.is_null() => indexed
                    .iter()
                    .filter(|(candidate, _)| *candidate == Some(identity))
                    .map(|(_, value)| *value)
                    .collect(),
                _ => Vec::new(),
            };

            if partners.is_empty() {
                if keep_unmatched {
                    pairs.push(json!({ "left": entry, "right": Value::Null, "matched": false }));
                }
                continue;
            }

            matched += 1;
            for partner in partners {
                pairs.push(json!({ "left": entry, "right": partner, "matched": true }));
            }
        }

        context.set_pin_value("matched", json!(matched)).await?;
        context.set_pin_value("pairs", json!(pairs)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_left", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_right", board, Some(ValueType::Array), None);
    }
}

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
pub struct ZippedPair {
    pub first: Value,
    pub second: Value,
}

#[crate::register_node]
#[derive(Default)]
pub struct ZipArrayNode {}

impl ZipArrayNode {
    pub fn new() -> Self {
        ZipArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for ZipArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_zip",
            "Zip",
            "Pairs up the elements of two arrays, stopping at the shorter one",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("array_first", "First", "First Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );
        node.add_input_pin(
            "array_second",
            "Second",
            "Second Array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node.add_output_pin(
            "pairs",
            "Pairs",
            "One entry per index holding both values",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ZippedPair>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let first: Vec<Value> = context.evaluate_pin("array_first").await?;
        let second: Vec<Value> = context.evaluate_pin("array_second").await?;

        let pairs: Vec<Value> = first
            .into_iter()
            .zip(second)
            .map(|(first, second)| json!({ "first": first, "second": second }))
            .collect();

        context.set_pin_value("pairs", json!(pairs)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SumFieldArrayNode {}

impl SumFieldArrayNode {
    pub fn new() -> Self {
        SumFieldArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for SumFieldArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_sum_field",
            "Sum Field",
            "Adds up one numeric field across an array of structs",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );
        node.add_input_pin(
            "field",
            "Field",
            "Field to add up, empty sums the values themselves",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("sum", "Sum", "Sum of the field", VariableType::Float);
        node.add_output_pin(
            "counted",
            "Counted",
            "How many entries held a number",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let field: String = context.evaluate_pin("field").await?;

        let numbers = array.iter().filter_map(|entry| {
            let value = if field.is_empty() {
                Some(entry)
            } else {
                entry.get(&field)
            };
            value.and_then(|value| value.as_f64())
        });

        let mut sum = 0.0;
        let mut counted = 0i64;
        for number in numbers {
            sum += number;
            counted += 1;
        }

        context.set_pin_value("sum", json!(sum)).await?;
        context.set_pin_value("counted", json!(counted)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    }
}

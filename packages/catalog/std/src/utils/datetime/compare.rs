use super::util::read_date;
use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn pair_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/DateTime/Comparison");
    node.add_icon("/flow/icons/calendar.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("date", "Date", "Date to test", VariableType::Date);
    node.add_input_pin(
        "other",
        "Other",
        "Date to compare against",
        VariableType::Date,
    );
    node.add_output_pin("result", "Result", description, VariableType::Boolean);

    node
}

#[crate::register_node]
#[derive(Default)]
pub struct DateBeforeNode {}

impl DateBeforeNode {
    pub fn new() -> Self {
        DateBeforeNode {}
    }
}

#[async_trait]
impl NodeLogic for DateBeforeNode {
    fn get_node(&self) -> Node {
        pair_node(
            "utils_datetime_before",
            "Is Before",
            "True when the first date lies before the second",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let other = read_date(context, "other").await?;
        context.set_pin_value("result", json!(date < other)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateAfterNode {}

impl DateAfterNode {
    pub fn new() -> Self {
        DateAfterNode {}
    }
}

#[async_trait]
impl NodeLogic for DateAfterNode {
    fn get_node(&self) -> Node {
        pair_node(
            "utils_datetime_after",
            "Is After",
            "True when the first date lies after the second",
        )
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let other = read_date(context, "other").await?;
        context.set_pin_value("result", json!(date > other)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateSameNode {}

impl DateSameNode {
    pub fn new() -> Self {
        DateSameNode {}
    }
}

#[async_trait]
impl NodeLogic for DateSameNode {
    fn get_node(&self) -> Node {
        let mut node = pair_node(
            "utils_datetime_same",
            "Is Same",
            "True when both dates fall into the same unit",
        );
        node.add_input_pin(
            "unit",
            "Unit",
            "Granularity the comparison runs at",
            VariableType::String,
        )
        .set_default_value(Some(json!("Day")))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    super::truncate::UNITS
                        .iter()
                        .map(|unit| unit.to_string())
                        .collect(),
                )
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let other = read_date(context, "other").await?;
        let unit: String = context.evaluate_pin("unit").await?;

        let same = super::truncate::truncate_to(date, &unit, false)
            == super::truncate::truncate_to(other, &unit, false);

        context.set_pin_value("result", json!(same)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateBetweenNode {}

impl DateBetweenNode {
    pub fn new() -> Self {
        DateBetweenNode {}
    }
}

#[async_trait]
impl NodeLogic for DateBetweenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_between",
            "Is Between",
            "True when a date falls inside a range",
            "Utils/DateTime/Comparison",
        );
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Date to test", VariableType::Date);
        node.add_input_pin("start", "Start", "Start of the range", VariableType::Date);
        node.add_input_pin("end", "End", "End of the range", VariableType::Date);
        node.add_input_pin(
            "inclusive",
            "Inclusive",
            "Count the boundaries as inside the range",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "result",
            "Result",
            "True when the date lies in the range",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let start = read_date(context, "start").await?;
        let end = read_date(context, "end").await?;
        let inclusive: bool = context.evaluate_pin("inclusive").await?;

        let (low, high) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let result = if inclusive {
            date >= low && date <= high
        } else {
            date > low && date < high
        };

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

use super::util::{from_value, read_date};
use crate::utils::pure_scores;
use chrono::{DateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

fn pair_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/DateTime");
    node.add_icon("/flow/icons/calendar.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
    node.add_input_pin("other", "Other", "Input Date", VariableType::Date);
    node.add_output_pin("result", "Result", description, VariableType::Date);

    node
}

fn array_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/DateTime");
    node.add_icon("/flow/icons/calendar.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("dates", "Dates", "Input Dates", VariableType::Date)
        .set_value_type(ValueType::Array);
    node.add_output_pin("result", "Result", description, VariableType::Date);
    node.add_output_pin(
        "found",
        "Found",
        "False when the array held no readable date",
        VariableType::Boolean,
    );

    node
}

async fn array_extreme(
    context: &mut ExecutionContext,
    take_largest: bool,
) -> flow_like_types::Result<()> {
    let values: Vec<Value> = context.evaluate_pin("dates").await?;

    let winner = values
        .iter()
        .filter_map(from_value)
        .reduce(|best: DateTime<Utc>, candidate| {
            if (candidate > best) == take_largest {
                candidate
            } else {
                best
            }
        });

    match winner {
        Some(date) => {
            context.set_pin_value("result", json!(date)).await?;
            context.set_pin_value("found", json!(true)).await?;
        }
        None => {
            context.set_pin_value("result", Value::Null).await?;
            context.set_pin_value("found", json!(false)).await?;
        }
    }

    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct DateMinNode {}

impl DateMinNode {
    pub fn new() -> Self {
        DateMinNode {}
    }
}

#[async_trait]
impl NodeLogic for DateMinNode {
    fn get_node(&self) -> Node {
        let mut node = pair_node("utils_datetime_min", "Earlier", "The earlier of two dates");
        node.set_flowscript_name("datetime", "min");
        node.set_receiver("date");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let other = read_date(context, "other").await?;
        context
            .set_pin_value("result", json!(date.min(other)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateMaxNode {}

impl DateMaxNode {
    pub fn new() -> Self {
        DateMaxNode {}
    }
}

#[async_trait]
impl NodeLogic for DateMaxNode {
    fn get_node(&self) -> Node {
        let mut node = pair_node("utils_datetime_max", "Later", "The later of two dates");
        node.set_flowscript_name("datetime", "max");
        node.set_receiver("date");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let other = read_date(context, "other").await?;
        context
            .set_pin_value("result", json!(date.max(other)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateMinOfNode {}

impl DateMinOfNode {
    pub fn new() -> Self {
        DateMinOfNode {}
    }
}

#[async_trait]
impl NodeLogic for DateMinOfNode {
    fn get_node(&self) -> Node {
        let mut node = array_node(
            "utils_datetime_min_of",
            "Earliest Of",
            "The earliest date in an array",
        );
        node.set_flowscript_name("datetime", "minOf");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        array_extreme(context, false).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateMaxOfNode {}

impl DateMaxOfNode {
    pub fn new() -> Self {
        DateMaxOfNode {}
    }
}

#[async_trait]
impl NodeLogic for DateMaxOfNode {
    fn get_node(&self) -> Node {
        let mut node = array_node(
            "utils_datetime_max_of",
            "Latest Of",
            "The latest date in an array",
        );
        node.set_flowscript_name("datetime", "maxOf");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        array_extreme(context, true).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateClampNode {}

impl DateClampNode {
    pub fn new() -> Self {
        DateClampNode {}
    }
}

#[async_trait]
impl NodeLogic for DateClampNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_clamp",
            "Clamp Date",
            "Pulls a date into a range, leaving it alone when it already fits",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "clamp");
        node.set_receiver("date");
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        node.add_input_pin(
            "start",
            "Start",
            "Earliest allowed date",
            VariableType::Date,
        );
        node.add_input_pin("end", "End", "Latest allowed date", VariableType::Date);

        node.add_output_pin(
            "result",
            "Result",
            "The date inside the range",
            VariableType::Date,
        );
        node.add_output_pin(
            "was_clamped",
            "Was Clamped",
            "True when the date had to be moved",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let start = read_date(context, "start").await?;
        let end = read_date(context, "end").await?;

        let (low, high) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let clamped = date.clamp(low, high);

        context.set_pin_value("result", json!(clamped)).await?;
        context
            .set_pin_value("was_clamped", json!(clamped != date))
            .await?;
        Ok(())
    }
}

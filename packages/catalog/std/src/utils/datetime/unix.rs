use super::util::{datetime_from_epoch, read_date};
use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

const UNITS: [&str; 4] = ["Seconds", "Milliseconds", "Microseconds", "Nanoseconds"];

fn unit_pin(node: &mut Node, description: &str) {
    node.add_input_pin("unit", "Unit", description, VariableType::String)
        .set_default_value(Some(json!("Milliseconds")))
        .set_options(
            PinOptions::new()
                .set_valid_values(UNITS.iter().map(|unit| unit.to_string()).collect())
                .build(),
        );
}

#[crate::register_node]
#[derive(Default)]
pub struct DateToUnixNode {}

impl DateToUnixNode {
    pub fn new() -> Self {
        DateToUnixNode {}
    }
}

#[async_trait]
impl NodeLogic for DateToUnixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_to_unix",
            "To Unix Timestamp",
            "Converts a date into an epoch timestamp",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/clock.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        unit_pin(&mut node, "Unit of the produced timestamp");

        node.add_output_pin(
            "timestamp",
            "Timestamp",
            "Epoch timestamp in the selected unit",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let unit: String = context.evaluate_pin("unit").await?;

        let timestamp = match unit.as_str() {
            "Seconds" => date.timestamp(),
            "Microseconds" => date.timestamp_micros(),
            "Nanoseconds" => date.timestamp_nanos_opt().unwrap_or(i64::MAX),
            _ => date.timestamp_millis(),
        };

        context.set_pin_value("timestamp", json!(timestamp)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateFromUnixNode {}

impl DateFromUnixNode {
    pub fn new() -> Self {
        DateFromUnixNode {}
    }
}

#[async_trait]
impl NodeLogic for DateFromUnixNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_from_unix",
            "From Unix Timestamp",
            "Converts an epoch timestamp into a date",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/clock.svg");
        node.set_scores(pure_scores());

        node.add_input_pin(
            "timestamp",
            "Timestamp",
            "Epoch timestamp",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "unit",
            "Unit",
            "Unit of the timestamp. Auto reads it from the magnitude",
            VariableType::String,
        )
        .set_default_value(Some(json!("Auto")))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    std::iter::once("Auto".to_string())
                        .chain(UNITS.iter().map(|unit| unit.to_string()))
                        .collect(),
                )
                .build(),
        );

        node.add_output_pin("date", "Date", "The converted date", VariableType::Date);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let timestamp: i64 = context.evaluate_pin("timestamp").await?;
        let unit: String = context.evaluate_pin("unit").await?;

        let date = match unit.as_str() {
            "Seconds" => chrono::DateTime::from_timestamp(timestamp, 0),
            "Milliseconds" => chrono::DateTime::from_timestamp_millis(timestamp),
            "Microseconds" => chrono::DateTime::from_timestamp_micros(timestamp),
            "Nanoseconds" => Some(chrono::DateTime::from_timestamp_nanos(timestamp)),
            _ => datetime_from_epoch(timestamp),
        }
        .ok_or_else(|| flow_like_types::anyhow!("{timestamp} is not a representable timestamp"))?;

        context.set_pin_value("date", json!(date)).await?;
        Ok(())
    }
}

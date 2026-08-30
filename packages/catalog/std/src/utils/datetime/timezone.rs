use super::util::read_date;
use crate::utils::pure_scores;
use chrono::TimeZone;
use chrono_tz::Tz;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateToTimezoneNode {}

impl DateToTimezoneNode {
    pub fn new() -> Self {
        DateToTimezoneNode {}
    }
}

#[async_trait]
impl NodeLogic for DateToTimezoneNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_to_timezone",
            "To Timezone",
            "Reads a date in another timezone. The instant stays the same, the wall clock changes",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "toTimezone");
        node.set_receiver("date");
        node.add_icon("/flow/icons/clock.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        node.add_input_pin(
            "timezone",
            "Timezone",
            "IANA timezone name, for example Europe/Berlin or America/New_York",
            VariableType::String,
        )
        .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "format",
            "Format",
            "Format for the text output, for example %Y-%m-%d %H:%M",
            VariableType::String,
        )
        .set_default_value(Some(json!("%Y-%m-%d %H:%M:%S")));

        node.add_output_pin(
            "date_out",
            "Date",
            "The same instant carrying the target offset",
            VariableType::Date,
        );
        node.add_output_pin(
            "formatted",
            "Formatted",
            "Local wall clock time as text",
            VariableType::String,
        );
        node.add_output_pin(
            "offset_seconds",
            "Offset Seconds",
            "Offset from UTC at that instant, daylight saving included",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let timezone: String = context.evaluate_pin("timezone").await?;
        let format: String = context.evaluate_pin("format").await?;

        let zone: Tz = timezone
            .trim()
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Unknown timezone {timezone}"))?;

        let local = date.with_timezone(&zone);
        let offset = zone.offset_from_utc_datetime(&date.naive_utc());

        context.set_pin_value("date_out", json!(local)).await?;
        context
            .set_pin_value("formatted", json!(local.format(&format).to_string()))
            .await?;
        context
            .set_pin_value(
                "offset_seconds",
                json!(chrono::Offset::fix(&offset).local_minus_utc() as i64),
            )
            .await?;
        Ok(())
    }
}

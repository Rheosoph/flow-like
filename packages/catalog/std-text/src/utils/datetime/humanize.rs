use super::util::read_date;
use crate::utils::pure_scores;
use chrono::{DateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Coarse buckets, the way a feed timestamp reads: exact enough to be useful,
/// vague enough to stay true a minute later.
fn humanize(delta_seconds: i64) -> String {
    let magnitude = delta_seconds.abs();
    let (amount, unit) = match magnitude {
        0..=44 => (magnitude, "second"),
        45..=2_699 => ((magnitude + 30) / 60, "minute"),
        2_700..=79_199 => ((magnitude + 1_800) / 3_600, "hour"),
        79_200..=2_591_999 => ((magnitude + 43_200) / 86_400, "day"),
        2_592_000..=31_535_999 => ((magnitude + 1_296_000) / 2_592_000, "month"),
        _ => ((magnitude + 15_768_000) / 31_536_000, "year"),
    };

    let amount = amount.max(1);
    let plural = if amount == 1 { "" } else { "s" };

    if magnitude <= 5 {
        return "just now".to_string();
    }
    if delta_seconds < 0 {
        return format!("in {amount} {unit}{plural}");
    }
    format!("{amount} {unit}{plural} ago")
}

#[crate::register_node]
#[derive(Default)]
pub struct DateHumanizeNode {}

impl DateHumanizeNode {
    pub fn new() -> Self {
        DateHumanizeNode {}
    }
}

#[async_trait]
impl NodeLogic for DateHumanizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_humanize",
            "Humanize",
            "Describes how far a date lies from now, for example \"3 days ago\"",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "humanize");
        node.set_receiver("date");
        node.add_icon("/flow/icons/clock.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        node.add_input_pin(
            "reference",
            "Reference",
            "What to measure against. Leave empty for the current time",
            VariableType::Date,
        );

        node.add_output_pin(
            "text",
            "Text",
            "Relative description of the distance",
            VariableType::String,
        );
        node.add_output_pin(
            "is_past",
            "Is Past",
            "True when the date lies before the reference",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "seconds",
            "Seconds",
            "Signed distance in seconds, positive when the date is in the past",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;

        let raw_reference: Value = context
            .evaluate_pin("reference")
            .await
            .unwrap_or(Value::Null);
        let reference: DateTime<Utc> =
            super::util::from_value(&raw_reference).unwrap_or_else(Utc::now);

        let delta = reference.timestamp() - date.timestamp();

        context
            .set_pin_value("text", json!(humanize(delta)))
            .await?;
        context.set_pin_value("is_past", json!(delta > 0)).await?;
        context.set_pin_value("seconds", json!(delta)).await?;
        Ok(())
    }
}

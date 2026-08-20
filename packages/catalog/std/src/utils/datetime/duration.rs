use chrono::{DateTime, Duration as ChronoDuration, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeDurationNode {}

impl DateTimeDurationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeDurationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_duration",
            "Add Duration",
            "Adds or subtracts a duration from a date",
            "Utils/DateTime",
        );

        node.add_input_pin("date", "Date", "Base date", VariableType::Date);
        node.add_input_pin(
            "days",
            "Days",
            "Days to add (negative to subtract)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin("hours", "Hours", "Hours to add", VariableType::Integer)
            .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "minutes",
            "Minutes",
            "Minutes to add",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "seconds",
            "Seconds",
            "Seconds to add",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("result", "Result", "Resulting date", VariableType::Date);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let dt: DateTime<Utc> = context.evaluate_pin("date").await?;
        let days: i64 = context.evaluate_pin("days").await?;
        let hours: i64 = context.evaluate_pin("hours").await?;
        let minutes: i64 = context.evaluate_pin("minutes").await?;
        let seconds: i64 = context.evaluate_pin("seconds").await?;

        let duration = ChronoDuration::days(days)
            + ChronoDuration::hours(hours)
            + ChronoDuration::minutes(minutes)
            + ChronoDuration::seconds(seconds);

        let result = dt
            .checked_add_signed(duration)
            .ok_or_else(|| flow_like_types::anyhow!("DateTime overflow when adding duration"))?;

        context.set_pin_value("result", json!(result)).await?;

        Ok(())
    }
}

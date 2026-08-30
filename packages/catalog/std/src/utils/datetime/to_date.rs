use chrono::{DateTime, Datelike, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeToDateNode {}

impl DateTimeToDateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeToDateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_to_date",
            "To Date",
            "Extracts date components from a DateTime",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "toDate");
        node.set_receiver("date");

        node.add_input_pin(
            "date",
            "Date",
            "DateTime to extract from",
            VariableType::Date,
        );

        node.add_output_pin("year", "Year", "Year", VariableType::Integer);
        node.add_output_pin("month", "Month", "Month (1-12)", VariableType::Integer);
        node.add_output_pin("day", "Day", "Day of month (1-31)", VariableType::Integer);
        node.add_output_pin(
            "weekday",
            "Weekday",
            "Day of week (0=Monday, 6=Sunday)",
            VariableType::Integer,
        );
        node.add_output_pin(
            "day_of_year",
            "Day of Year",
            "Day of year (1-366)",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("error").await?;

        let dt: DateTime<Utc> = context.evaluate_pin("date").await?;

        context
            .set_pin_value("year", json!(dt.year() as i64))
            .await?;
        context
            .set_pin_value("month", json!(dt.month() as i64))
            .await?;
        context.set_pin_value("day", json!(dt.day() as i64)).await?;
        context
            .set_pin_value("weekday", json!(dt.weekday().num_days_from_monday() as i64))
            .await?;
        context
            .set_pin_value("day_of_year", json!(dt.ordinal() as i64))
            .await?;

        Ok(())
    }
}

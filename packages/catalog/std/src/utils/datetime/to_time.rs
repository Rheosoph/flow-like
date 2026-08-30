use chrono::{DateTime, Timelike, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeToTimeNode {}

impl DateTimeToTimeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeToTimeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_to_time",
            "To Time",
            "Extracts time components from a DateTime",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "toTime");
        node.set_receiver("date");

        node.add_input_pin(
            "date",
            "Date",
            "DateTime to extract from",
            VariableType::Date,
        );

        node.add_output_pin("hour", "Hour", "Hour (0-23)", VariableType::Integer);
        node.add_output_pin("minute", "Minute", "Minute (0-59)", VariableType::Integer);
        node.add_output_pin("second", "Second", "Second (0-59)", VariableType::Integer);
        node.add_output_pin(
            "nanosecond",
            "Nanosecond",
            "Nanosecond (0-999999999)",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let dt: DateTime<Utc> = context.evaluate_pin("date").await?;

        context
            .set_pin_value("hour", json!(dt.hour() as i64))
            .await?;
        context
            .set_pin_value("minute", json!(dt.minute() as i64))
            .await?;
        context
            .set_pin_value("second", json!(dt.second() as i64))
            .await?;
        context
            .set_pin_value("nanosecond", json!(dt.nanosecond() as i64))
            .await?;

        Ok(())
    }
}

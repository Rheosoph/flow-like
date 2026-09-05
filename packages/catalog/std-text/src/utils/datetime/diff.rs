use chrono::{DateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeDiffNode {}

impl DateTimeDiffNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SystemTime {
    secs_since_epoch: i64,
    nanos_since_epoch: u32,
}

#[async_trait]
impl NodeLogic for DateTimeDiffNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_diff",
            "DateTime Difference",
            "Calculates the duration between two dates",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "diff");
        node.set_receiver("start");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("start", "Start", "Start date", VariableType::Date);
        node.add_input_pin("end", "End", "End date", VariableType::Date);

        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);
        node.add_output_pin(
            "total_seconds",
            "Total Seconds",
            "Total duration in seconds",
            VariableType::Integer,
        );
        node.add_output_pin("days", "Days", "Number of days", VariableType::Integer);
        node.add_output_pin("hours", "Hours", "Remaining hours", VariableType::Integer);
        node.add_output_pin(
            "minutes",
            "Minutes",
            "Remaining minutes",
            VariableType::Integer,
        );
        node.add_output_pin(
            "seconds",
            "Seconds",
            "Remaining seconds",
            VariableType::Integer,
        );
        node.add_output_pin(
            "human_readable",
            "Human Readable",
            "Human readable duration string",
            VariableType::String,
        );
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let start_value: Value = context.evaluate_pin("start").await?;
        let end_value: Value = context.evaluate_pin("end").await?;

        // Parse start datetime
        let start_dt = if let Ok(dt) =
            flow_like_types::json::from_value::<DateTime<Utc>>(start_value.clone())
        {
            dt
        } else {
            let system_time: SystemTime = match flow_like_types::json::from_value(start_value) {
                Ok(st) => st,
                Err(e) => {
                    context
                        .set_pin_value(
                            "error_message",
                            json!(format!("Invalid start date format: {}", e)),
                        )
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            };

            match DateTime::<Utc>::from_timestamp(
                system_time.secs_since_epoch,
                system_time.nanos_since_epoch,
            ) {
                Some(dt) => dt,
                None => {
                    context
                        .set_pin_value("error_message", json!("Invalid start timestamp"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            }
        };

        // Parse end datetime
        let end_dt =
            if let Ok(dt) = flow_like_types::json::from_value::<DateTime<Utc>>(end_value.clone()) {
                dt
            } else {
                let system_time: SystemTime = match flow_like_types::json::from_value(end_value) {
                    Ok(st) => st,
                    Err(e) => {
                        context
                            .set_pin_value(
                                "error_message",
                                json!(format!("Invalid end date format: {}", e)),
                            )
                            .await?;
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }
                };

                match DateTime::<Utc>::from_timestamp(
                    system_time.secs_since_epoch,
                    system_time.nanos_since_epoch,
                ) {
                    Some(dt) => dt,
                    None => {
                        context
                            .set_pin_value("error_message", json!("Invalid end timestamp"))
                            .await?;
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }
                }
            };

        let duration = end_dt.signed_duration_since(start_dt);
        let total_seconds = duration.num_seconds();

        // Calculate individual components
        let abs_total_seconds = total_seconds.abs();
        let days = abs_total_seconds / 86400;
        let remaining = abs_total_seconds % 86400;
        let hours = remaining / 3600;
        let remaining = remaining % 3600;
        let minutes = remaining / 60;
        let seconds = remaining % 60;

        // Build human readable string
        let mut parts = Vec::new();
        if days > 0 {
            parts.push(format!("{} day{}", days, if days == 1 { "" } else { "s" }));
        }
        if hours > 0 {
            parts.push(format!(
                "{} hour{}",
                hours,
                if hours == 1 { "" } else { "s" }
            ));
        }
        if minutes > 0 {
            parts.push(format!(
                "{} minute{}",
                minutes,
                if minutes == 1 { "" } else { "s" }
            ));
        }
        if seconds > 0 || parts.is_empty() {
            parts.push(format!(
                "{} second{}",
                seconds,
                if seconds == 1 { "" } else { "s" }
            ));
        }

        let human_readable = if total_seconds < 0 {
            format!("-{}", parts.join(", "))
        } else {
            parts.join(", ")
        };

        context
            .set_pin_value("total_seconds", json!(total_seconds))
            .await?;
        context
            .set_pin_value("days", json!(if total_seconds < 0 { -days } else { days }))
            .await?;
        context.set_pin_value("hours", json!(hours)).await?;
        context.set_pin_value("minutes", json!(minutes)).await?;
        context.set_pin_value("seconds", json!(seconds)).await?;
        context
            .set_pin_value("human_readable", json!(human_readable))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

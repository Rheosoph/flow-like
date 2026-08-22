use chrono::{DateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::util::{parse_auto, parse_with_format};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeParseNode {}

impl DateTimeParseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeParseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_parse",
            "Parse DateTime",
            "Parses a string into a DateTime. Auto-detects common formats and epoch timestamps (seconds, milliseconds, microseconds, nanoseconds) or uses a custom format string.",
            "Utils/DateTime",
        );

        node.add_input_pin("input", "Input", "String to parse", VariableType::String);
        node.add_input_pin(
            "format",
            "Format",
            "Optional format string (e.g., '%Y-%m-%d %H:%M:%S'). Leave empty for auto-detection.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("date", "Date", "Parsed date", VariableType::Date);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let format: String = context.evaluate_pin("format").await.unwrap_or_default();

        let dt_utc: Option<DateTime<Utc>> = if format.is_empty() {
            parse_auto(&input)
        } else {
            parse_with_format(&input, &format)
        };

        match dt_utc {
            Some(dt) => {
                context.set_pin_value("date", json!(dt)).await?;
            }
            None => {
                return Err(flow_like_types::anyhow!(
                    "Failed to parse DateTime from input: {}",
                    input
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::util::datetime_from_epoch;

    fn parsed(units: i64) -> String {
        datetime_from_epoch(units)
            .expect("epoch is representable")
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    #[test]
    fn epoch_unit_is_detected_from_magnitude() {
        assert_eq!(parsed(1_787_121_392), "2026-08-19T06:36:32.000Z");
        assert_eq!(parsed(1_787_121_392_487), "2026-08-19T06:36:32.487Z");
        assert_eq!(parsed(1_787_121_392_487_000), "2026-08-19T06:36:32.487Z");
        assert_eq!(
            parsed(1_787_121_392_487_000_000),
            "2026-08-19T06:36:32.487Z"
        );
    }

    #[test]
    fn negative_epochs_keep_their_unit() {
        assert_eq!(parsed(-1_000), "1969-12-31T23:43:20.000Z");
        assert_eq!(parsed(-1_787_121_392_487_000), "1913-05-15T17:23:27.513Z");
    }
}

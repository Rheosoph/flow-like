use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// A bare epoch integer carries no unit, so its magnitude decides — Arrow and
/// Lance hand temporal columns back in microseconds, browsers and most JSON
/// producers in milliseconds, and Unix tooling in seconds. Mirrors
/// `detectEpochUnit` in `packages/ui/lib/date.ts`.
fn datetime_from_epoch(units: i64) -> Option<DateTime<Utc>> {
    const MAX_EPOCH_SECONDS: i64 = 100_000_000_000;
    const MAX_EPOCH_MILLIS: i64 = 100_000_000_000_000;
    const MAX_EPOCH_MICROS: i64 = 100_000_000_000_000_000;

    match units.unsigned_abs() {
        magnitude if magnitude < MAX_EPOCH_SECONDS as u64 => DateTime::from_timestamp(units, 0),
        magnitude if magnitude < MAX_EPOCH_MILLIS as u64 => DateTime::from_timestamp_millis(units),
        magnitude if magnitude < MAX_EPOCH_MICROS as u64 => DateTime::from_timestamp_micros(units),
        _ => Some(DateTime::from_timestamp_nanos(units)),
    }
}

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

        let dt_utc: Option<DateTime<Utc>> = if !format.is_empty() {
            // Try custom format
            if let Ok(dt) = NaiveDateTime::parse_from_str(&input, &format) {
                Some(dt.and_utc())
            } else if let Ok(date) = NaiveDate::parse_from_str(&input, &format) {
                date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc())
            } else {
                None
            }
        } else {
            // Auto-detect format
            // Try RFC3339 first (most common)
            if let Ok(dt) = DateTime::parse_from_rfc3339(&input) {
                Some(dt.with_timezone(&Utc))
            }
            // Try RFC2822
            else if let Ok(dt) = DateTime::parse_from_rfc2822(&input) {
                Some(dt.with_timezone(&Utc))
            }
            // Try epoch timestamp
            else if let Ok(units) = input.parse::<i64>() {
                datetime_from_epoch(units)
            }
            // Try common formats
            else {
                let formats = [
                    "%Y-%m-%d %H:%M:%S",
                    "%Y-%m-%dT%H:%M:%S",
                    "%Y-%m-%d %H:%M:%S%.f",
                    "%Y-%m-%dT%H:%M:%S%.f",
                    "%Y-%m-%d",
                    "%d/%m/%Y",
                    "%m/%d/%Y",
                    "%d.%m.%Y",
                    "%Y/%m/%d",
                    "%d-%m-%Y",
                    "%m-%d-%Y",
                ];

                formats.iter().find_map(|fmt| {
                    if let Ok(dt) = NaiveDateTime::parse_from_str(&input, fmt) {
                        Some(dt.and_utc())
                    } else if let Ok(date) = NaiveDate::parse_from_str(&input, fmt) {
                        date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc())
                    } else {
                        None
                    }
                })
            }
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
    use super::*;

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

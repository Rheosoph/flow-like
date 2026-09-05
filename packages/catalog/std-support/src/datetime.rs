use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::Value;

/// Formats every date the catalog hands out, so parsing one back is lossless.
pub fn to_json(date: DateTime<Utc>) -> Value {
    flow_like_types::json::json!(date)
}

/// A bare epoch integer carries no unit, so its magnitude decides — Arrow and
/// Lance hand temporal columns back in microseconds, browsers and most JSON
/// producers in milliseconds, and Unix tooling in seconds. Mirrors
/// `detectEpochUnit` in `packages/ui/lib/date.ts`.
pub fn datetime_from_epoch(units: i64) -> Option<DateTime<Utc>> {
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

const COMMON_FORMATS: [&str; 11] = [
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

/// Unambiguous formats only — used where a wrong guess silently reorders data.
const STRICT_FORMATS: [&str; 5] = [
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d",
];

fn parse_with(input: &str, formats: &[&str]) -> Option<DateTime<Utc>> {
    formats.iter().find_map(|format| {
        if let Ok(date) = NaiveDateTime::parse_from_str(input, format) {
            Some(date.and_utc())
        } else if let Ok(date) = NaiveDate::parse_from_str(input, format) {
            date.and_hms_opt(0, 0, 0).map(|date| date.and_utc())
        } else {
            None
        }
    })
}

pub fn parse_with_format(input: &str, format: &str) -> Option<DateTime<Utc>> {
    parse_with(input, &[format])
}

/// Everything the Parse DateTime node accepts: RFC 3339, RFC 2822, bare epochs
/// and the common regional layouts.
pub fn parse_auto(input: &str) -> Option<DateTime<Utc>> {
    let input = input.trim();

    if let Ok(date) = DateTime::parse_from_rfc3339(input) {
        return Some(date.with_timezone(&Utc));
    }
    if let Ok(date) = DateTime::parse_from_rfc2822(input) {
        return Some(date.with_timezone(&Utc));
    }
    if let Ok(units) = input.parse::<i64>() {
        return datetime_from_epoch(units);
    }

    parse_with(input, &COMMON_FORMATS)
}

/// Reads a pin value as a date. Numbers are treated as epochs.
pub fn from_value(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::String(text) => parse_auto(text),
        Value::Number(number) => number.as_i64().and_then(datetime_from_epoch),
        _ => None,
    }
}

/// Only recognises values that cannot plausibly be anything else — for the
/// automatic sort comparator, where guessing wrong silently reorders rows.
pub fn from_value_strict(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Utc));
    }
    parse_with(text, &STRICT_FORMATS)
}

/// Reads a Date pin without insisting on RFC 3339 — upstream nodes, HTTP payloads
/// and database columns all hand dates over in their own shape.
pub async fn read_date(
    context: &mut ExecutionContext,
    pin: &str,
) -> flow_like_types::Result<DateTime<Utc>> {
    let value: Value = context.evaluate_pin(pin).await?;
    from_value(&value).ok_or_else(|| {
        flow_like_types::anyhow!("Pin \"{pin}\" does not hold a readable date: {value}")
    })
}

use std::sync::Arc;

use arrow::datatypes::FieldRef;
use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_schema::{DataType, Field, TimeUnit};
use flow_like_types::{
    Result, Value, anyhow,
    json::{Deserialize, Serialize, to_value},
    serde::{Serializer, ser::SerializeMap},
};
use serde_arrow::schema::{SchemaLike, TracingOptions};

pub type ValueBatchReader = Box<dyn RecordBatchReader + Send>;

/// Serializes a JSON value the way `serde_json` does, except that non-negative
/// integers small enough for `i64` are emitted as `i64` instead of `u64`.
///
/// `serde_json::Number` routes every non-negative integer through
/// `serialize_u64`, while serde_arrow's temporal builders (`Timestamp`,
/// `Date32`/`Date64`, `Time32`/`Time64`) only accept `serialize_i64` or
/// `serialize_str`. Without this coercion an epoch value in the column's native
/// unit — exactly what [`record_batch_to_value`] hands back for those columns —
/// can never be written back, so read-modify-upsert fails with
/// "serialize_u64 is not supported". Every other builder accepts `i64` and
/// `u64` interchangeably in this range, so the coercion is invisible to them.
struct TemporalSafeValue<'a>(&'a Value);

impl Serialize for TemporalSafeValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Number(number) => match number.as_u64() {
                Some(value) if value <= i64::MAX as u64 => serializer.serialize_i64(value as i64),
                _ => number.serialize(serializer),
            },
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(values) => serializer.collect_seq(values.iter().map(TemporalSafeValue)),
            Value::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &TemporalSafeValue(value))?;
                }
                map.end()
            }
        }
    }
}

pub fn value_to_record_batch(records: Vec<Value>) -> Result<RecordBatch> {
    value_to_record_batch_with_fields(records, None)
}

pub fn value_to_record_batch_with_fields(
    mut records: Vec<Value>,
    fields: Option<Vec<FieldRef>>,
) -> Result<RecordBatch> {
    // Determine Arrow schema
    let fields = match fields {
        Some(fields) => fields,
        None => infer_fields(&records)?,
    };

    normalize_temporal_values(&mut records, &fields)?;

    // Build a record batch. Schema inference above deliberately sees the raw
    // values so new tables keep their traced column types; only the write is
    // coerced.
    let rows: Vec<TemporalSafeValue> = records.iter().map(TemporalSafeValue).collect();
    let batch: RecordBatch = serde_arrow::to_record_batch(&fields, &rows)?;
    Ok(batch)
}

/// Converts values for the first write to a new table, promoting columns whose
/// non-null values are all RFC3339 instants to UTC millisecond timestamps.
///
/// Existing tables must continue to use `value_to_record_batch_with_fields`
/// with their persisted schema so legacy string columns remain strings.
pub(crate) fn value_to_record_batch_with_utc_timestamp_inference(
    records: Vec<Value>,
) -> Result<RecordBatch> {
    let fields = promote_rfc3339_instants(&records, infer_fields(&records)?);
    value_to_record_batch_with_fields(records, Some(fields))
}

fn infer_fields(records: &[Value]) -> Result<Vec<FieldRef>> {
    let mut fields: Vec<FieldRef> =
        Vec::<FieldRef>::from_samples(records, TracingOptions::default().allow_null_fields(true))?;

    for field in &mut fields {
        if field.name() == "vector" {
            *field = Arc::new(Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    get_vector_dimension(records)? as i32,
                ),
                true,
            ));
        }
    }

    Ok(fields)
}

fn promote_rfc3339_instants(records: &[Value], mut fields: Vec<FieldRef>) -> Vec<FieldRef> {
    for field in &mut fields {
        if matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8)
            && field_is_rfc3339_instant(records, field.name())
        {
            *field = Arc::new(field.as_ref().clone().with_data_type(DataType::Timestamp(
                TimeUnit::Millisecond,
                Some("UTC".into()),
            )));
        }
    }

    fields
}

fn field_is_rfc3339_instant(records: &[Value], field_name: &str) -> bool {
    let mut found_value = false;

    for record in records {
        let value = record
            .as_object()
            .and_then(|record| record.get(field_name))
            .unwrap_or(&Value::Null);

        match value {
            Value::Null => {}
            Value::String(value) if chrono::DateTime::parse_from_rfc3339(value).is_ok() => {
                found_value = true;
            }
            _ => return false,
        }
    }

    found_value
}

/// Reshapes the values bound for temporal columns into something serde_arrow's
/// builders accept, using the target column type — which the type-blind
/// [`TemporalSafeValue`] wrapper cannot see.
fn normalize_temporal_values(records: &mut [Value], fields: &[FieldRef]) -> Result<()> {
    for field in fields {
        let data_type = field.data_type();
        if !matches!(
            data_type,
            DataType::Timestamp(..)
                | DataType::Date32
                | DataType::Date64
                | DataType::Time32(_)
                | DataType::Time64(_)
        ) {
            continue;
        }

        for record in records.iter_mut() {
            let Some(value) = record
                .as_object_mut()
                .and_then(|record| record.get_mut(field.name()))
            else {
                continue;
            };

            match value {
                Value::String(text) => normalize_temporal_string(text, data_type),
                Value::Number(_) => normalize_temporal_number(value, field)?,
                _ => {}
            }
        }
    }

    Ok(())
}

/// Textual instants FlowLike hands to a temporal column, beyond what
/// serde_arrow's builders parse themselves. FlowLike's `Date` type serializes
/// as RFC3339, and a2ui's `date` / `datetime-local` inputs emit the browser's
/// `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM` — the latter has no seconds, so it
/// parses as neither RFC3339 nor `NaiveDateTime`.
///
/// Layouts that need a day-vs-month guess are deliberately absent: `03/04/2026`
/// has no single right reading, and the storage layer must not invent one.
fn parse_instant(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    if let Ok(date_time) = DateTime::parse_from_rfc3339(value) {
        return Some(date_time.with_timezone(&Utc));
    }

    if let Ok(date_time) = value.parse::<NaiveDateTime>() {
        return Some(date_time.and_utc());
    }

    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(date_time) = NaiveDateTime::parse_from_str(value, format) {
            return Some(date_time.and_utc());
        }
    }

    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| date.and_time(NaiveTime::MIN).and_utc())
}

/// Rewrites a textual instant into the one shape the target builder parses,
/// leaving values the builder already accepts byte-identical.
///
/// `Timestamp` keeps old timezone-less tables writable after callers adopted
/// the RFC3339 representation of FlowLike's Date type, and the reverse, so
/// previously accepted naive values keep working against newly-created UTC
/// schemas by being read as UTC.
///
/// `Date64` is left alone on purpose: it is not reachable through
/// [`crate::databases::vector::schema`], and truncating an instant to its day
/// would silently drop the time of day that its millisecond values can carry.
fn normalize_temporal_string(value: &mut String, data_type: &DataType) {
    match data_type {
        DataType::Timestamp(_, timezone) => {
            let utc = match timezone.as_deref() {
                Some(timezone) if timezone.eq_ignore_ascii_case("UTC") => true,
                Some(_) => return,
                None => false,
            };

            let accepted = if utc {
                value.parse::<chrono::DateTime<chrono::Utc>>().is_ok()
            } else {
                value.parse::<chrono::NaiveDateTime>().is_ok()
            };
            if accepted {
                return;
            }

            let Some(instant) = parse_instant(value) else {
                return;
            };
            *value = if utc {
                instant.to_rfc3339()
            } else {
                instant
                    .naive_utc()
                    .format("%Y-%m-%dT%H:%M:%S%.f")
                    .to_string()
            };
        }
        DataType::Date32 => {
            if value.parse::<chrono::NaiveDate>().is_ok() {
                return;
            }

            if let Some(instant) = parse_instant(value) {
                *value = instant.date_naive().format("%Y-%m-%d").to_string();
            }
        }
        _ => {}
    }
}

/// A temporal column holds one integer in its native unit, so a whole number
/// that arrived as a float — an epoch that passed through a Float pin or JSON
/// producer — is rewritten as an integer. This cannot live in
/// [`TemporalSafeValue`], because `Decimal` columns are the mirror image: they
/// take floats and reject integers.
///
/// Date columns additionally reject counts that cannot denote a real date.
/// Without that check an epoch timestamp written to a date column is stored
/// verbatim and silently reads back as a date hundreds of thousands of years
/// out.
fn normalize_temporal_number(value: &mut Value, field: &FieldRef) -> Result<()> {
    let Value::Number(number) = &*value else {
        return Ok(());
    };

    if number.is_f64() {
        let float = number.as_f64().unwrap_or(f64::NAN);
        if float.fract() != 0.0 || !(i64::MIN as f64..i64::MAX as f64).contains(&float) {
            return Err(anyhow!(
                "Column '{}' is {:?} and takes whole numbers in its native unit that fit a 64-bit integer, but got {float}",
                field.name(),
                field.data_type()
            ));
        }
        *value = Value::from(float as i64);
    }

    let Some(units) = value.as_i64() else {
        return Ok(());
    };

    let days = match field.data_type() {
        DataType::Date32 => units,
        DataType::Date64 => units.div_euclid(MILLISECONDS_PER_DAY),
        _ => return Ok(()),
    };

    if !representable_date_days().contains(&days) {
        return Err(anyhow!(
            "Column '{}' is {:?}, counting {} since 1970-01-01, but {units} is outside the range of representable dates — an epoch timestamp written to a date column is the usual cause",
            field.name(),
            field.data_type(),
            if matches!(field.data_type(), DataType::Date32) {
                "days"
            } else {
                "milliseconds"
            }
        ));
    }

    Ok(())
}

const MILLISECONDS_PER_DAY: i64 = 86_400_000;

fn representable_date_days() -> std::ops::RangeInclusive<i64> {
    let epoch = chrono::DateTime::UNIX_EPOCH.date_naive();
    chrono::NaiveDate::MIN
        .signed_duration_since(epoch)
        .num_days()
        ..=chrono::NaiveDate::MAX
            .signed_duration_since(epoch)
            .num_days()
}

fn get_vector_dimension<T>(records: &[T]) -> Result<i32>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    if records.is_empty() {
        return Err(anyhow!("No records to determine vector dimension"));
    }

    for record in records {
        let serialized = to_value(record)?;

        if let Some(map) = serialized.as_object()
            && let Some(Value::Array(vec)) = map.get("vector")
            && !vec.is_empty()
        {
            return Ok(vec.len() as i32);
        }
    }

    Err(anyhow!("Unable to determine vector dimension from records"))
}

pub fn value_to_batch_reader(records: Vec<Value>) -> Result<ValueBatchReader> {
    value_to_batch_reader_with_fields(records, None)
}

pub fn value_to_batch_reader_with_fields(
    records: Vec<Value>,
    fields: Option<Vec<FieldRef>>,
) -> Result<ValueBatchReader> {
    let batch = value_to_record_batch_with_fields(records, fields)?;
    let schema = batch.schema();
    let reader: ValueBatchReader = Box::new(RecordBatchIterator::new(
        [batch].into_iter().map(Ok),
        schema,
    ));

    Ok(reader)
}

pub(crate) fn value_to_batch_reader_with_utc_timestamp_inference(
    records: Vec<Value>,
) -> Result<ValueBatchReader> {
    let batch = value_to_record_batch_with_utc_timestamp_inference(records)?;
    let schema = batch.schema();
    let reader: ValueBatchReader = Box::new(RecordBatchIterator::new(
        [batch].into_iter().map(Ok),
        schema,
    ));

    Ok(reader)
}

pub fn record_batch_to_value(record_batch: &RecordBatch) -> Result<Vec<Value>> {
    let items = serde_arrow::from_record_batch(record_batch)?;
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Date32Array, TimestampMillisecondArray, UInt64Array};
    use flow_like_types::json::{Deserialize, json, to_value};

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct TestStruct {
        id: i32,
        name: String,
    }

    #[test]
    fn test_value_to_batchreader_and_back() -> Result<()> {
        // Mock data as JSON Values
        let records = [
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
            },
        ];

        let records = records
            .iter()
            .map(|r| to_value(r).unwrap())
            .collect::<Vec<Value>>();

        // Convert JSON to RecordBatch
        let record_batch = value_to_record_batch(records.clone())?;

        // Convert RecordBatch back to JSON
        let result = record_batch_to_value(&record_batch)?;

        // Check that the original data and the result match
        assert_eq!(records, result);

        Ok(())
    }

    #[test]
    fn infers_utc_timestamp_for_rfc3339_instants() -> Result<()> {
        let records = vec![
            flow_like_types::json::json!({
                "created_at": "2026-08-09T12:34:56.789Z",
                "label": "2026-08-09"
            }),
            flow_like_types::json::json!({
                "created_at": "2026-08-09T14:34:56.789+02:00",
                "label": "not an instant"
            }),
        ];

        let batch = value_to_record_batch_with_utc_timestamp_inference(records)?;
        assert_eq!(
            batch.schema().field_with_name("created_at")?.data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );
        assert_eq!(
            batch.schema().field_with_name("label")?.data_type(),
            &DataType::LargeUtf8
        );

        let timestamps = batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp");
        assert_eq!(timestamps.value(0), timestamps.value(1));

        Ok(())
    }

    #[test]
    fn preserves_existing_large_utf8_date_column() -> Result<()> {
        let timestamp = "2026-08-09T12:34:56.789Z";
        let batch = value_to_record_batch_with_fields(
            vec![flow_like_types::json::json!({ "created_at": timestamp })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::LargeUtf8,
                false,
            ))]),
        )?;

        assert_eq!(
            batch.schema().field_with_name("created_at")?.data_type(),
            &DataType::LargeUtf8
        );
        assert_eq!(record_batch_to_value(&batch)?[0]["created_at"], timestamp);

        Ok(())
    }

    #[test]
    fn mixed_date_and_text_values_remain_strings() -> Result<()> {
        let records = vec![
            flow_like_types::json::json!({ "value": "2026-08-09T12:34:56.789Z" }),
            flow_like_types::json::json!({ "value": "not a date" }),
        ];

        let batch = value_to_record_batch_with_utc_timestamp_inference(records)?;
        assert_eq!(
            batch.schema().field_with_name("value")?.data_type(),
            &DataType::LargeUtf8
        );

        Ok(())
    }

    #[test]
    fn utc_dates_remain_writable_to_legacy_timezone_less_timestamps() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![flow_like_types::json::json!({
                "created_at": "2026-08-09T14:34:56.789+02:00"
            })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ))]),
        )?;

        assert_eq!(
            batch.schema().field_with_name("created_at")?.data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, None)
        );
        let timestamps = batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp");
        assert_eq!(
            timestamps.value(0),
            chrono::DateTime::parse_from_rfc3339("2026-08-09T12:34:56.789Z")?.timestamp_millis()
        );

        Ok(())
    }

    #[test]
    fn integer_timestamps_survive_a_read_modify_write_round_trip() -> Result<()> {
        let fields: Vec<FieldRef> = vec![Arc::new(Field::new(
            "first_seen_at",
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            true,
        ))];

        let batch = value_to_record_batch_with_fields(
            vec![json!({ "first_seen_at": "2026-08-09T12:34:56.789Z" })],
            Some(fields.clone()),
        )?;

        let rows = record_batch_to_value(&batch)?;
        assert_eq!(
            rows[0]["first_seen_at"].as_i64(),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-08-09T12:34:56.789Z")?
                    .timestamp_millis()
            )
        );

        let rewritten = value_to_record_batch_with_fields(rows.clone(), Some(fields))?;
        assert_eq!(record_batch_to_value(&rewritten)?, rows);

        Ok(())
    }

    #[test]
    fn integer_timestamps_write_to_timezone_less_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "created_at": 1_786_881_600_000_i64 })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ))]),
        )?;

        let timestamps = batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp");
        assert_eq!(timestamps.value(0), 1_786_881_600_000);

        Ok(())
    }

    #[test]
    fn integer_days_write_to_date32_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "day": 20_675 })],
            Some(vec![Arc::new(Field::new("day", DataType::Date32, true))]),
        )?;

        let days = batch
            .column_by_name("day")
            .and_then(|column| column.as_any().downcast_ref::<Date32Array>())
            .expect("day should be a date32 column");
        assert_eq!(days.value(0), 20_675);

        Ok(())
    }

    #[test]
    fn nested_integer_timestamps_write_to_struct_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "meta": { "seen_at": 1_786_881_600_000_i64 } })],
            Some(vec![Arc::new(Field::new(
                "meta",
                DataType::Struct(
                    vec![Field::new(
                        "seen_at",
                        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                        true,
                    )]
                    .into(),
                ),
                true,
            ))]),
        )?;

        assert_eq!(
            record_batch_to_value(&batch)?[0]["meta"]["seen_at"],
            json!(1_786_881_600_000_i64)
        );

        Ok(())
    }

    fn timestamp_column(timezone: Option<&str>) -> Vec<FieldRef> {
        vec![Arc::new(Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Millisecond, timezone.map(Into::into)),
            true,
        ))]
    }

    fn written_millis(batch: &RecordBatch) -> i64 {
        batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp")
            .value(0)
    }

    fn written_days(batch: &RecordBatch) -> i32 {
        batch
            .column_by_name("day")
            .and_then(|column| column.as_any().downcast_ref::<Date32Array>())
            .expect("day should be a date32 column")
            .value(0)
    }

    #[test]
    fn browser_date_and_datetime_local_strings_write_to_timestamp_columns() -> Result<()> {
        for timezone in [Some("UTC"), None] {
            let batch = value_to_record_batch_with_fields(
                vec![json!({ "created_at": "2026-08-19T10:30" })],
                Some(timestamp_column(timezone)),
            )?;
            assert_eq!(written_millis(&batch), 1_787_135_400_000, "{timezone:?}");

            let batch = value_to_record_batch_with_fields(
                vec![json!({ "created_at": "2026-08-19" })],
                Some(timestamp_column(timezone)),
            )?;
            assert_eq!(written_millis(&batch), 1_787_097_600_000, "{timezone:?}");
        }

        Ok(())
    }

    #[test]
    fn rfc3339_instants_write_to_date32_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "day": "2026-08-09T12:34:56.789Z" })],
            Some(vec![Arc::new(Field::new("day", DataType::Date32, true))]),
        )?;
        assert_eq!(written_days(&batch), 20_674);

        let batch = value_to_record_batch_with_fields(
            vec![json!({ "day": "2026-08-09" })],
            Some(vec![Arc::new(Field::new("day", DataType::Date32, true))]),
        )?;
        assert_eq!(written_days(&batch), 20_674);

        Ok(())
    }

    #[test]
    fn date64_string_handling_is_left_untouched() -> Result<()> {
        let fields = vec![Arc::new(Field::new("day", DataType::Date64, true)) as FieldRef];

        let batch = value_to_record_batch_with_fields(
            vec![json!({ "day": "2026-08-09" })],
            Some(fields.clone()),
        )?;
        assert_eq!(
            record_batch_to_value(&batch)?[0]["day"],
            json!(20_674_i64 * 86_400_000)
        );

        assert!(
            value_to_record_batch_with_fields(
                vec![json!({ "day": "2026-08-09T12:34:56.789Z" })],
                Some(fields),
            )
            .is_err(),
            "Date64 must keep rejecting instants rather than silently dropping the time of day"
        );

        Ok(())
    }

    #[test]
    fn whole_floats_write_to_temporal_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "created_at": 1_786_881_600_000.0_f64 })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                true,
            ))]),
        )?;

        let timestamps = batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp");
        assert_eq!(timestamps.value(0), 1_786_881_600_000);

        Ok(())
    }

    #[test]
    fn fractional_floats_are_rejected_by_temporal_columns() -> Result<()> {
        let error = value_to_record_batch_with_fields(
            vec![json!({ "created_at": 1_786_881_600_000.5_f64 })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                true,
            ))]),
        )
        .expect_err("a fractional millisecond has no representation");

        let error = error.to_string();
        assert!(error.contains("created_at"), "{error}");
        assert!(error.contains("whole numbers"), "{error}");

        Ok(())
    }

    #[test]
    fn epoch_timestamps_are_rejected_by_date_columns() -> Result<()> {
        let error = value_to_record_batch_with_fields(
            vec![json!({ "day": 1_786_881_600_i64 })],
            Some(vec![Arc::new(Field::new("day", DataType::Date32, true))]),
        )
        .expect_err("epoch seconds are not a day count");

        let error = error.to_string();
        assert!(error.contains("day"), "{error}");
        assert!(error.contains("epoch timestamp"), "{error}");

        Ok(())
    }

    #[test]
    fn date64_columns_accept_midnight_aligned_milliseconds() -> Result<()> {
        let midnight = 20_675 * 86_400_000_i64;
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "day": midnight })],
            Some(vec![Arc::new(Field::new("day", DataType::Date64, true))]),
        )?;

        assert_eq!(record_batch_to_value(&batch)?[0]["day"], json!(midnight));

        Ok(())
    }

    #[test]
    fn pre_epoch_timestamps_keep_working() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "created_at": -86_400_000_i64 })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                true,
            ))]),
        )?;

        assert_eq!(
            record_batch_to_value(&batch)?[0]["created_at"],
            json!(-86_400_000_i64)
        );

        Ok(())
    }

    #[test]
    fn unsigned_integers_above_i64_max_still_write_to_uint64_columns() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({ "counter": u64::MAX })],
            Some(vec![Arc::new(Field::new(
                "counter",
                DataType::UInt64,
                true,
            ))]),
        )?;

        let counters = batch
            .column_by_name("counter")
            .and_then(|column| column.as_any().downcast_ref::<UInt64Array>())
            .expect("counter should be a uint64 column");
        assert_eq!(counters.value(0), u64::MAX);

        Ok(())
    }

    #[test]
    fn integer_columns_keep_their_inferred_type() -> Result<()> {
        let batch = value_to_record_batch(vec![json!({ "count": 7, "delta": -7 })])?;

        assert_eq!(
            batch.schema().field_with_name("count")?.data_type(),
            &DataType::UInt64
        );
        assert_eq!(
            batch.schema().field_with_name("delta")?.data_type(),
            &DataType::Int64
        );
        assert_eq!(
            record_batch_to_value(&batch)?[0],
            json!({ "count": 7, "delta": -7 })
        );

        Ok(())
    }

    #[test]
    fn naive_dates_remain_writable_to_new_utc_timestamps() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![flow_like_types::json::json!({
                "created_at": "2026-08-09T12:34:56.789"
            })],
            Some(vec![Arc::new(Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                false,
            ))]),
        )?;

        let timestamps = batch
            .column_by_name("created_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("created_at should be a millisecond timestamp");
        assert_eq!(
            timestamps.value(0),
            chrono::DateTime::parse_from_rfc3339("2026-08-09T12:34:56.789Z")?.timestamp_millis()
        );

        Ok(())
    }
}

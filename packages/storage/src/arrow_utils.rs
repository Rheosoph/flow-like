use std::sync::Arc;

use arrow::datatypes::FieldRef;
use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_schema::{DataType, Field, TimeUnit};
use flow_like_types::{
    Result, Value, anyhow,
    json::{Deserialize, Serialize, to_value},
};
use serde_arrow::schema::{SchemaLike, TracingOptions};

pub type ValueBatchReader = Box<dyn RecordBatchReader + Send>;

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

    normalize_timestamp_strings(&mut records, &fields);

    // Build a record batch
    let batch: RecordBatch = serde_arrow::to_record_batch(&fields, &records)?;
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

/// Keep old timezone-less timestamp tables writable after callers adopt the
/// RFC3339 representation used by FlowLike's Date type. The reverse conversion
/// lets previously accepted naive values keep working with newly-created UTC
/// timestamp schemas by interpreting them as UTC.
fn normalize_timestamp_strings(records: &mut [Value], fields: &[FieldRef]) {
    for field in fields {
        let timezone = match field.data_type() {
            DataType::Timestamp(_, timezone) => timezone.as_deref(),
            _ => continue,
        };

        for record in records.iter_mut() {
            let Some(Value::String(value)) = record
                .as_object_mut()
                .and_then(|record| record.get_mut(field.name()))
            else {
                continue;
            };

            match timezone {
                None => {
                    if let Ok(date_time) = chrono::DateTime::parse_from_rfc3339(value) {
                        *value = date_time
                            .naive_utc()
                            .format("%Y-%m-%dT%H:%M:%S%.f")
                            .to_string();
                    }
                }
                Some(timezone) if timezone.eq_ignore_ascii_case("UTC") => {
                    if value.parse::<chrono::DateTime<chrono::Utc>>().is_err()
                        && let Ok(date_time) = value.parse::<chrono::NaiveDateTime>()
                    {
                        *value = date_time.and_utc().to_rfc3339();
                    }
                }
                Some(_) => {}
            }
        }
    }
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
    use arrow_array::TimestampMillisecondArray;
    use flow_like_types::json::{Deserialize, to_value};

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

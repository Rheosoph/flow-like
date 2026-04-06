use std::sync::Arc;

use arrow::datatypes::FieldRef;
use arrow_array::{RecordBatch, RecordBatchIterator};
use arrow_schema::{DataType, Field};
use flow_like_types::{
    Result, Value, anyhow,
    json::{Deserialize, Serialize, to_value},
};
use serde_arrow::schema::{SchemaLike, TracingOptions};

pub type ValueBatchIterator = RecordBatchIterator<
    std::iter::Map<
        std::array::IntoIter<RecordBatch, 1>,
        fn(RecordBatch) -> Result<RecordBatch, arrow_schema::ArrowError>,
    >,
>;

pub fn value_to_record_batch(records: Vec<Value>) -> Result<RecordBatch> {
    value_to_record_batch_with_fields(records, None)
}

pub fn value_to_record_batch_with_fields(
    records: Vec<Value>,
    fields: Option<Vec<FieldRef>>,
) -> Result<RecordBatch> {
    // Determine Arrow schema
    let fields = match fields {
        Some(fields) => fields,
        None => {
            let mut fields: Vec<FieldRef> =
                Vec::<FieldRef>::from_samples(&records, TracingOptions::new())?;

            for field in &mut fields {
                if field.name() == "vector" {
                    *field = Arc::new(Field::new(
                        "vector",
                        DataType::FixedSizeList(
                            Arc::new(Field::new("item", DataType::Float32, true)),
                            get_vector_dimension(&records)? as i32,
                        ),
                        true,
                    ));
                }
            }

            fields
        }
    };

    // Build a record batch
    let batch: RecordBatch = serde_arrow::to_record_batch(&fields, &records)?;
    Ok(batch)
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

pub fn value_to_batch_iterator(
    records: Vec<Value>,
) -> Result<ValueBatchIterator> {
    value_to_batch_iterator_with_fields(records, None)
}

pub fn value_to_batch_iterator_with_fields(
    records: Vec<Value>,
    fields: Option<Vec<FieldRef>>,
) -> Result<ValueBatchIterator> {
    let batch = value_to_record_batch_with_fields(records, fields)?;
    let schema = batch.schema();
    let iterator: ValueBatchIterator = RecordBatchIterator::new([batch].into_iter().map(Ok), schema);

    Ok(iterator)
}

pub fn record_batch_to_value(record_batch: &RecordBatch) -> Result<Vec<Value>> {
    let items = serde_arrow::from_record_batch(record_batch)?;
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}

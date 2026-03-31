use crate::data::datafusion::session::DataFusionSession;
use crate::data::excel::CSVTable;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::HashMap;

/// A row-oriented representation of query results for easy iteration
pub type QueryRow = HashMap<String, Value>;

#[crate::register_node]
#[derive(Default)]
pub struct SqlQueryNode {}

impl SqlQueryNode {
    pub fn new() -> Self {
        SqlQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for SqlQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_sql_query",
            "SQL Query",
            "Execute a SQL query against a DataFusion session. Returns results as both a CSVTable (for analytics) and array of row objects (for iteration).",
            "Data/DataFusion",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session with registered tables",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute (e.g., SELECT * FROM mytable WHERE column > 10)",
            VariableType::String,
        )
        .set_default_value(Some(json!("SELECT * FROM data LIMIT 100")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Query executed successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "table",
            "Table",
            "Query results as a CSVTable (columnar format, good for analytics)",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>();

        node.add_output_pin(
            "rows",
            "Rows",
            "Query results as array of row structs with Flow-Like-compatible values",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows in the result",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 8,
            governance: 9,
            reliability: 8,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let query: String = context.evaluate_pin("query").await?;

        let cached_session = session.load(context).await?;

        context.log_message(&format!("Executing SQL: {}", query), LogLevel::Debug);

        let df = cached_session.ctx.sql(&query).await?;
        let batches = df.collect().await?;

        let csv_table = batches_to_csv_table(&batches)?;
        let rows = batches_to_rows(&batches)?;
        let row_count = csv_table.row_count() as i64;

        context.set_pin_value("table", json!(csv_table)).await?;
        context.set_pin_value("rows", json!(rows)).await?;
        context.set_pin_value("row_count", json!(row_count)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

pub fn batches_to_rows(
    batches: &[flow_like_storage::datafusion::arrow::record_batch::RecordBatch],
) -> flow_like_types::Result<Vec<QueryRow>> {
    if batches.is_empty() {
        return Ok(vec![]);
    }

    let schema = batches[0].schema();
    let headers: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let mut rows: Vec<QueryRow> = Vec::new();

    for batch in batches {
        for row_idx in 0..batch.num_rows() {
            let mut row: QueryRow = HashMap::with_capacity(batch.num_columns());

            for (col_idx, header) in headers.iter().enumerate() {
                let col = batch.column(col_idx);
                let value = array_value_to_json(col.as_ref(), row_idx)?;
                row.insert(header.clone(), value);
            }

            rows.push(row);
        }
    }

    Ok(rows)
}

pub fn batches_to_csv_table(
    batches: &[flow_like_storage::datafusion::arrow::record_batch::RecordBatch],
) -> flow_like_types::Result<CSVTable> {
    use flow_like_types::Value as JsonValue;

    if batches.is_empty() {
        return Ok(CSVTable::new(vec![], vec![], None));
    }

    let schema = batches[0].schema();
    let headers: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();

    let mut rows: Vec<Vec<JsonValue>> = Vec::new();

    for batch in batches {
        for row_idx in 0..batch.num_rows() {
            let mut row: Vec<JsonValue> = Vec::with_capacity(batch.num_columns());

            for col_idx in 0..batch.num_columns() {
                let col = batch.column(col_idx);
                let value = array_value_to_json(col.as_ref(), row_idx)?;
                row.push(value);
            }

            rows.push(row);
        }
    }

    Ok(CSVTable::new(headers, rows, None))
}

fn array_value_to_json(
    array: &dyn flow_like_storage::datafusion::arrow::array::Array,
    idx: usize,
) -> flow_like_types::Result<flow_like_types::Value> {
    use flow_like_storage::datafusion::arrow::array::*;
    use flow_like_storage::datafusion::arrow::datatypes::{DataType, TimeUnit};
    use flow_like_types::Value as JsonValue;

    if array.is_null(idx) {
        return Ok(JsonValue::Null);
    }

    let dt = array.data_type();
    let value = match dt {
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<BooleanArray>().unwrap();
            JsonValue::Bool(arr.value(idx))
        }
        DataType::Int8 => {
            let arr = array.as_any().downcast_ref::<Int8Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int16 => {
            let arr = array.as_any().downcast_ref::<Int16Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt8 => {
            let arr = array.as_any().downcast_ref::<UInt8Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt16 => {
            let arr = array.as_any().downcast_ref::<UInt16Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            let v = arr.value(idx) as f64;
            flow_like_types::json::Number::from_f64(v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            let v = arr.value(idx);
            flow_like_types::json::Number::from_f64(v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        DataType::Utf8 => {
            let arr = array.as_any().downcast_ref::<StringArray>().unwrap();
            JsonValue::String(arr.value(idx).to_string())
        }
        DataType::LargeUtf8 => {
            let arr = array.as_any().downcast_ref::<LargeStringArray>().unwrap();
            JsonValue::String(arr.value(idx).to_string())
        }
        DataType::Decimal128(_, _) => {
            let arr = array.as_any().downcast_ref::<Decimal128Array>().unwrap();
            decimal_string_to_json(arr.value_as_string(idx))
        }
        DataType::Decimal256(_, _) => {
            let arr = array.as_any().downcast_ref::<Decimal256Array>().unwrap();
            decimal_string_to_json(arr.value_as_string(idx))
        }
        DataType::Date32 => {
            let arr = array.as_any().downcast_ref::<Date32Array>().unwrap();
            let days = arr.value(idx);
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            let date = epoch + chrono::Duration::days(days as i64);
            JsonValue::String(date.format("%Y-%m-%d").to_string())
        }
        DataType::Date64 => {
            let arr = array.as_any().downcast_ref::<Date64Array>().unwrap();
            let ms = arr.value(idx);
            let secs = ms / 1000;
            let nsecs = ((ms % 1000) * 1_000_000) as u32;
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nsecs) {
                JsonValue::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            } else {
                JsonValue::Null
            }
        }
        DataType::Timestamp(unit, _) => match unit {
            TimeUnit::Second => {
                let arr = array.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
                timestamp_to_json(arr.value(idx), *unit)
            }
            TimeUnit::Millisecond => {
                let arr = array
                    .as_any()
                    .downcast_ref::<TimestampMillisecondArray>()
                    .unwrap();
                timestamp_to_json(arr.value(idx), *unit)
            }
            TimeUnit::Microsecond => {
                let arr = array
                    .as_any()
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .unwrap();
                timestamp_to_json(arr.value(idx), *unit)
            }
            TimeUnit::Nanosecond => {
                let arr = array
                    .as_any()
                    .downcast_ref::<TimestampNanosecondArray>()
                    .unwrap();
                timestamp_to_json(arr.value(idx), *unit)
            }
        }
        _ => JsonValue::String(format!("{:?}", array.slice(idx, 1))),
    };

    Ok(value)
}

fn decimal_string_to_json(raw: String) -> Value {
    if let Ok(value) = raw.parse::<i64>() {
        return json!(value);
    }

    if let Ok(value) = raw.parse::<u64>() {
        return json!(value);
    }

    let significant_digits = raw.bytes().filter(|byte| byte.is_ascii_digit()).count();
    if significant_digits <= 15 {
        if let Ok(value) = raw.parse::<f64>() {
            if let Some(number) = flow_like_types::json::Number::from_f64(value) {
                return Value::Number(number);
            }
        }
    }

    Value::String(raw)
}

fn timestamp_to_json(value: i64, unit: flow_like_storage::datafusion::arrow::datatypes::TimeUnit) -> Value {
    use flow_like_storage::datafusion::arrow::datatypes::TimeUnit;

    let (seconds, nanoseconds, format) = match unit {
        TimeUnit::Second => (value, 0, "%Y-%m-%dT%H:%M:%S"),
        TimeUnit::Millisecond => (
            value.div_euclid(1_000),
            (value.rem_euclid(1_000) * 1_000_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.3f",
        ),
        TimeUnit::Microsecond => (
            value.div_euclid(1_000_000),
            (value.rem_euclid(1_000_000) * 1_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.6f",
        ),
        TimeUnit::Nanosecond => (
            value.div_euclid(1_000_000_000),
            value.rem_euclid(1_000_000_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.9f",
        ),
    };

    chrono::DateTime::from_timestamp(seconds, nanoseconds)
        .map(|dt| Value::String(dt.format(format).to_string()))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::datafusion::arrow::array::*;
    use flow_like_storage::datafusion::arrow::datatypes::{DataType, Field, Schema};
    use flow_like_storage::datafusion::arrow::record_batch::RecordBatch;
    use flow_like_types::tokio;
    use std::sync::Arc;

    fn create_simple_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Float64, true),
        ]));

        let id_array = Int64Array::from(vec![1, 2, 3]);
        let name_array = StringArray::from(vec!["alice", "bob", "carol"]);
        let value_array = Float64Array::from(vec![Some(10.5), Some(20.0), None]);

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(id_array),
                Arc::new(name_array),
                Arc::new(value_array),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_batches_to_rows_empty() {
        let result = batches_to_rows(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_batches_to_rows_with_data() {
        let batch = create_simple_batch();
        let rows = batches_to_rows(&[batch]).unwrap();

        assert_eq!(rows.len(), 3);

        let first_row = &rows[0];
        assert_eq!(first_row.get("id"), Some(&json!(1)));
        assert_eq!(first_row.get("name"), Some(&json!("alice")));
        assert_eq!(first_row.get("value"), Some(&json!(10.5)));

        let third_row = &rows[2];
        assert_eq!(third_row.get("id"), Some(&json!(3)));
        assert_eq!(third_row.get("name"), Some(&json!("carol")));
        assert_eq!(third_row.get("value"), Some(&Value::Null));
    }

    #[tokio::test]
    async fn test_batches_to_csv_table_empty() {
        let result = batches_to_csv_table(&[]).unwrap();
        assert_eq!(result.row_count(), 0);
    }

    #[tokio::test]
    async fn test_batches_to_csv_table_with_data() {
        let batch = create_simple_batch();
        let table = batches_to_csv_table(&[batch]).unwrap();

        assert_eq!(table.row_count(), 3);
        assert_eq!(table.headers(), vec!["id", "name", "value"]);
    }

    #[tokio::test]
    async fn test_batches_to_rows_multiple_batches() {
        let batch1 = create_simple_batch();
        let batch2 = create_simple_batch();
        let rows = batches_to_rows(&[batch1, batch2]).unwrap();

        assert_eq!(rows.len(), 6);
    }

    #[tokio::test]
    async fn test_array_value_to_json_null() {
        let array = Int64Array::from(vec![Some(1), None, Some(3)]);
        let value = array_value_to_json(&array, 1).unwrap();
        assert_eq!(value, Value::Null);
    }

    #[tokio::test]
    async fn test_array_value_to_json_boolean() {
        let array = BooleanArray::from(vec![true, false, true]);
        assert_eq!(array_value_to_json(&array, 0).unwrap(), json!(true));
        assert_eq!(array_value_to_json(&array, 1).unwrap(), json!(false));
    }

    #[tokio::test]
    async fn test_array_value_to_json_integers() {
        let i8_arr = Int8Array::from(vec![127i8]);
        let i16_arr = Int16Array::from(vec![32767i16]);
        let i32_arr = Int32Array::from(vec![2147483647i32]);
        let i64_arr = Int64Array::from(vec![9223372036854775807i64]);

        assert_eq!(array_value_to_json(&i8_arr, 0).unwrap(), json!(127));
        assert_eq!(array_value_to_json(&i16_arr, 0).unwrap(), json!(32767));
        assert_eq!(array_value_to_json(&i32_arr, 0).unwrap(), json!(2147483647));
        assert_eq!(
            array_value_to_json(&i64_arr, 0).unwrap(),
            json!(9223372036854775807i64)
        );
    }

    #[tokio::test]
    async fn test_array_value_to_json_unsigned_integers() {
        let u8_arr = UInt8Array::from(vec![255u8]);
        let u16_arr = UInt16Array::from(vec![65535u16]);
        let u32_arr = UInt32Array::from(vec![4294967295u32]);
        let u64_arr = UInt64Array::from(vec![18446744073709551615u64]);

        assert_eq!(array_value_to_json(&u8_arr, 0).unwrap(), json!(255));
        assert_eq!(array_value_to_json(&u16_arr, 0).unwrap(), json!(65535));
        assert_eq!(
            array_value_to_json(&u32_arr, 0).unwrap(),
            json!(4294967295u64)
        );
        assert_eq!(
            array_value_to_json(&u64_arr, 0).unwrap(),
            json!(18446744073709551615u64)
        );
    }

    #[tokio::test]
    async fn test_array_value_to_json_floats() {
        let f32_arr = Float32Array::from(vec![3.14f32]);
        let f64_arr = Float64Array::from(vec![2.718281828f64]);

        let f32_val = array_value_to_json(&f32_arr, 0).unwrap();
        let f64_val = array_value_to_json(&f64_arr, 0).unwrap();

        assert!(matches!(f32_val, Value::Number(_)));
        assert!(matches!(f64_val, Value::Number(_)));
    }

    #[tokio::test]
    async fn test_array_value_to_json_strings() {
        let utf8_arr = StringArray::from(vec!["hello world"]);
        let large_utf8_arr = LargeStringArray::from(vec!["large string test"]);

        assert_eq!(
            array_value_to_json(&utf8_arr, 0).unwrap(),
            json!("hello world")
        );
        assert_eq!(
            array_value_to_json(&large_utf8_arr, 0).unwrap(),
            json!("large string test")
        );
    }

    #[tokio::test]
    async fn test_array_value_to_json_date32() {
        let array = Date32Array::from(vec![18628]); // 2021-01-01
        let value = array_value_to_json(&array, 0).unwrap();
        assert_eq!(value, json!("2021-01-01"));
    }

    #[tokio::test]
    async fn test_array_value_to_json_decimal128() {
        let integer_array = Decimal128Array::from_iter_values([12])
            .with_precision_and_scale(20, 0)
            .unwrap();
        let scaled_array = Decimal128Array::from_iter_values([575000])
            .with_precision_and_scale(38, 4)
            .unwrap();

        assert_eq!(array_value_to_json(&integer_array, 0).unwrap(), json!(12));
        assert_eq!(array_value_to_json(&scaled_array, 0).unwrap(), json!(57.5));
    }

    #[tokio::test]
    async fn test_sql_query_node_structure() {
        let node_logic = SqlQueryNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_sql_query");
        assert_eq!(node.friendly_name, "SQL Query");
        assert_eq!(node.version, Some(1));

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Input)
            .collect();
        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Output)
            .collect();
        let rows_pin = output_pins.iter().find(|p| p.name == "rows").unwrap();

        assert!(input_pins.iter().any(|p| p.name == "exec_in"));
        assert!(input_pins.iter().any(|p| p.name == "session"));
        assert!(input_pins.iter().any(|p| p.name == "query"));
        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
        assert!(output_pins.iter().any(|p| p.name == "table"));
        assert!(output_pins.iter().any(|p| p.name == "rows"));
        assert!(output_pins.iter().any(|p| p.name == "row_count"));
        assert_eq!(rows_pin.data_type, VariableType::Struct);
        assert_eq!(rows_pin.value_type, ValueType::Array);
    }
}

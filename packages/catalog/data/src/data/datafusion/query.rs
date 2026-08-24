use crate::data::datafusion::session::DataFusionSession;
use crate::data::excel::CSVTable;
use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
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
            "Execute a SQL statement against a DataFusion session. SELECT returns results as both a CSVTable (for analytics) and array of row objects (for iteration). Registered Lance tables also accept INSERT INTO, and UPDATE/DELETE with a WHERE clause that references at least one column (constant-only conditions like WHERE true are refused, as are subqueries and multi-table forms; writes return a single `count` row). Write any value that comes from outside the flow as a $placeholder and wire it into the pin that appears — never build the SQL string around it.",
            "Data/DataFusion",
        );
        node.set_flowscript_name("df", "sqlQuery");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");
        node.set_version(3);

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
            "SQL query to execute (e.g., SELECT * FROM mytable WHERE column > 10). Use $placeholders for values that come from the flow (SELECT * FROM users WHERE id = $user_id) — each one adds an input pin to wire the value into. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        )
        .set_default_value(Some(json!("SELECT * FROM data LIMIT 100")));

        params::add_params_pin(&mut node, params::SqlFlavor::Query);

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
        .set_value_type(ValueType::Array)
        .set_open_schema();

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

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let query: String = context.evaluate_pin("query").await?;

        // UPDATE/DELETE with subqueries or joined tables cannot be forwarded to
        // Lance faithfully (DataFusion only hands the table plain WHERE
        // conjuncts) — refuse those shapes before planning can mangle them.
        flow_like_storage::databases::sql_guard::validate_lance_dml_sql(&query)?;

        let query_params =
            params::resolve_params(context, &query, params::SqlFlavor::Query).await?;

        let cached_session = session.load(context).await?;

        context.log_message(&format!("Executing SQL: {}", query), LogLevel::Debug);

        let df = cached_session.ctx.sql(&query).await?;
        let df = params::bind(df, &query_params)?;
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
            let arr = typed_column::<BooleanArray>(array)?;
            JsonValue::Bool(arr.value(idx))
        }
        DataType::Int8 => {
            let arr = typed_column::<Int8Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int16 => {
            let arr = typed_column::<Int16Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int32 => {
            let arr = typed_column::<Int32Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Int64 => {
            let arr = typed_column::<Int64Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt8 => {
            let arr = typed_column::<UInt8Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt16 => {
            let arr = typed_column::<UInt16Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt32 => {
            let arr = typed_column::<UInt32Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::UInt64 => {
            let arr = typed_column::<UInt64Array>(array)?;
            JsonValue::Number(arr.value(idx).into())
        }
        DataType::Float32 => {
            let arr = typed_column::<Float32Array>(array)?;
            let v = arr.value(idx) as f64;
            flow_like_types::json::Number::from_f64(v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        DataType::Float64 => {
            let arr = typed_column::<Float64Array>(array)?;
            let v = arr.value(idx);
            flow_like_types::json::Number::from_f64(v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        DataType::Utf8 => {
            let arr = typed_column::<StringArray>(array)?;
            JsonValue::String(arr.value(idx).to_string())
        }
        DataType::LargeUtf8 => {
            let arr = typed_column::<LargeStringArray>(array)?;
            JsonValue::String(arr.value(idx).to_string())
        }
        DataType::Utf8View => {
            let arr = typed_column::<StringViewArray>(array)?;
            JsonValue::String(arr.value(idx).to_string())
        }
        DataType::Decimal128(_, _) => {
            let arr = typed_column::<Decimal128Array>(array)?;
            decimal_string_to_json(arr.value_as_string(idx))
        }
        DataType::Decimal256(_, _) => {
            let arr = typed_column::<Decimal256Array>(array)?;
            decimal_string_to_json(arr.value_as_string(idx))
        }
        DataType::Date32 => {
            let arr = typed_column::<Date32Array>(array)?;
            let days = arr.value(idx);
            let epoch = chrono::DateTime::UNIX_EPOCH.date_naive();
            match chrono::TimeDelta::try_days(days as i64)
                .and_then(|offset| epoch.checked_add_signed(offset))
            {
                Some(date) => JsonValue::String(date.format("%Y-%m-%d").to_string()),
                None => JsonValue::Null,
            }
        }
        DataType::Date64 => {
            let arr = typed_column::<Date64Array>(array)?;
            let ms = arr.value(idx);
            let secs = ms / 1000;
            let nsecs = ((ms % 1000) * 1_000_000) as u32;
            if let Some(dt) = chrono::DateTime::from_timestamp(secs, nsecs) {
                JsonValue::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            } else {
                JsonValue::Null
            }
        }
        DataType::Timestamp(unit, timezone) => match unit {
            TimeUnit::Second => {
                let arr = typed_column::<TimestampSecondArray>(array)?;
                timestamp_to_json(arr.value(idx), *unit, timezone.as_deref())
            }
            TimeUnit::Millisecond => {
                let arr = typed_column::<TimestampMillisecondArray>(array)?;
                timestamp_to_json(arr.value(idx), *unit, timezone.as_deref())
            }
            TimeUnit::Microsecond => {
                let arr = typed_column::<TimestampMicrosecondArray>(array)?;
                timestamp_to_json(arr.value(idx), *unit, timezone.as_deref())
            }
            TimeUnit::Nanosecond => {
                let arr = typed_column::<TimestampNanosecondArray>(array)?;
                timestamp_to_json(arr.value(idx), *unit, timezone.as_deref())
            }
        },
        DataType::List(_) => {
            let arr = typed_column::<ListArray>(array)?;
            list_values_to_json(arr.value(idx).as_ref())?
        }
        DataType::LargeList(_) => {
            let arr = typed_column::<LargeListArray>(array)?;
            list_values_to_json(arr.value(idx).as_ref())?
        }
        DataType::FixedSizeList(_, _) => {
            let arr = typed_column::<FixedSizeListArray>(array)?;
            list_values_to_json(arr.value(idx).as_ref())?
        }
        DataType::Struct(fields) => {
            let arr = typed_column::<StructArray>(array)?;
            let mut object = flow_like_types::json::Map::with_capacity(fields.len());
            for (field, column) in fields.iter().zip(arr.columns().iter()) {
                object.insert(
                    field.name().clone(),
                    array_value_to_json(column.as_ref(), idx)?,
                );
            }
            JsonValue::Object(object)
        }
        _ => {
            use flow_like_storage::arrow::util::display::{ArrayFormatter, FormatOptions};
            let options = FormatOptions::default();
            match ArrayFormatter::try_new(array, &options) {
                Ok(formatter) => {
                    let formatted = formatter.value(idx).to_string();
                    decimal_string_to_json(formatted)
                }
                Err(_) => JsonValue::Null,
            }
        }
    };

    Ok(value)
}

/// Read an Arrow column as its concrete array type. The `DataType` match above already selected
/// the layout, so a mismatch means the schema and the buffers disagree — surface that instead of
/// panicking inside a running flow.
fn typed_column<T: flow_like_storage::datafusion::arrow::array::Array + 'static>(
    array: &dyn flow_like_storage::datafusion::arrow::array::Array,
) -> flow_like_types::Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        flow_like_types::anyhow!(
            "reading SQL result column as {} failed: Arrow reported data type {}",
            std::any::type_name::<T>(),
            array.data_type()
        )
    })
}

/// Materialize the child values of one Arrow list cell as a JSON array. The slice handed in is
/// already narrowed to a single row's values, so every index belongs to that row.
fn list_values_to_json(
    values: &dyn flow_like_storage::datafusion::arrow::array::Array,
) -> flow_like_types::Result<Value> {
    let mut items = Vec::with_capacity(values.len());
    for idx in 0..values.len() {
        items.push(array_value_to_json(values, idx)?);
    }
    Ok(Value::Array(items))
}

fn decimal_string_to_json(raw: String) -> Value {
    if let Ok(value) = raw.parse::<i64>() {
        return json!(value);
    }

    if let Ok(value) = raw.parse::<u64>() {
        return json!(value);
    }

    let significant_digits = raw.bytes().filter(|byte| byte.is_ascii_digit()).count();
    if significant_digits <= 15
        && let Ok(value) = raw.parse::<f64>()
        && let Some(number) = flow_like_types::json::Number::from_f64(value)
    {
        return Value::Number(number);
    }

    Value::String(raw)
}

fn timestamp_to_json(
    value: i64,
    unit: flow_like_storage::datafusion::arrow::datatypes::TimeUnit,
    timezone: Option<&str>,
) -> Value {
    use flow_like_storage::datafusion::arrow::datatypes::TimeUnit;

    let (seconds, nanoseconds, naive_format, seconds_format) = match unit {
        TimeUnit::Second => (value, 0, "%Y-%m-%dT%H:%M:%S", chrono::SecondsFormat::Secs),
        TimeUnit::Millisecond => (
            value.div_euclid(1_000),
            (value.rem_euclid(1_000) * 1_000_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.3f",
            chrono::SecondsFormat::Millis,
        ),
        TimeUnit::Microsecond => (
            value.div_euclid(1_000_000),
            (value.rem_euclid(1_000_000) * 1_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.6f",
            chrono::SecondsFormat::Micros,
        ),
        TimeUnit::Nanosecond => (
            value.div_euclid(1_000_000_000),
            value.rem_euclid(1_000_000_000) as u32,
            "%Y-%m-%dT%H:%M:%S%.9f",
            chrono::SecondsFormat::Nanos,
        ),
    };

    chrono::DateTime::from_timestamp(seconds, nanoseconds)
        .map(|dt| {
            // Arrow timestamps with timezone metadata are physical instants. Normalize their JSON
            // representation to UTC RFC3339 so it can flow directly into a FlowLike Date pin.
            // Preserve the historical suffix-less representation for timezone-less wall clocks.
            let rendered = if timezone.is_some() {
                dt.to_rfc3339_opts(seconds_format, true)
            } else {
                dt.format(naive_format).to_string()
            };
            Value::String(rendered)
        })
        .unwrap_or(Value::Null)
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;
    use flow_like_storage::datafusion::arrow::array::*;
    use flow_like_storage::datafusion::arrow::buffer::{NullBuffer, OffsetBuffer};
    use flow_like_storage::datafusion::arrow::datatypes::{
        DataType, Field, Int32Type, Int64Type, Schema,
    };
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
    async fn date32_values_out_of_range_read_back_as_null() {
        let schema = Arc::new(Schema::new(vec![Field::new("day", DataType::Date32, true)]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Date32Array::from(vec![20_675, i32::MAX]))],
        )
        .unwrap();

        let rows = batches_to_rows(&[batch]).unwrap();
        assert_eq!(rows[0].get("day"), Some(&json!("2026-08-10")));
        assert_eq!(rows[1].get("day"), Some(&Value::Null));
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
    async fn timestamp_json_preserves_utc_awareness_without_changing_legacy_wall_clocks() {
        let value = 1_609_459_200_123_i64; // 2021-01-01T00:00:00.123Z
        let utc = TimestampMillisecondArray::from(vec![value]).with_timezone("UTC");
        let legacy = TimestampMillisecondArray::from(vec![value]);

        assert_eq!(
            array_value_to_json(&utc, 0).unwrap(),
            json!("2021-01-01T00:00:00.123Z")
        );
        assert_eq!(
            array_value_to_json(&legacy, 0).unwrap(),
            json!("2021-01-01T00:00:00.123")
        );
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

    fn subscription_fields() -> (Arc<Field>, Arc<Field>) {
        (
            Arc::new(Field::new("sub", DataType::Utf8, true)),
            Arc::new(Field::new("threshold", DataType::Int64, true)),
        )
    }

    fn subscriptions_list_array() -> ListArray {
        let (sub_field, threshold_field) = subscription_fields();
        let entries = StructArray::from(vec![
            (
                sub_field.clone(),
                Arc::new(StringArray::from(vec!["user-a", "user-b"])) as ArrayRef,
            ),
            (
                threshold_field.clone(),
                Arc::new(Int64Array::from(vec![30i64, 50i64])) as ArrayRef,
            ),
        ]);
        let item_field = Arc::new(Field::new(
            "item",
            DataType::Struct(vec![sub_field, threshold_field].into()),
            true,
        ));

        ListArray::new(
            item_field,
            OffsetBuffer::new(vec![0i32, 2i32].into()),
            Arc::new(entries),
            None,
        )
    }

    #[tokio::test]
    async fn primitive_list_becomes_json_array() {
        let array = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![Some(vec![
            Some(1),
            Some(2),
            None,
        ])]);

        assert_eq!(
            array_value_to_json(&array, 0).unwrap(),
            json!([1, 2, Value::Null])
        );
    }

    #[tokio::test]
    async fn empty_list_becomes_empty_json_array() {
        let array = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
            Some(vec![] as Vec<Option<i32>>),
            None,
        ]);

        assert_eq!(array_value_to_json(&array, 0).unwrap(), json!([]));
        assert_eq!(array_value_to_json(&array, 1).unwrap(), Value::Null);
    }

    #[tokio::test]
    async fn large_and_fixed_size_lists_become_json_arrays() {
        let large = LargeListArray::from_iter_primitive::<Int64Type, _, _>(vec![Some(vec![
            Some(7i64),
            Some(8i64),
        ])]);
        let fixed = FixedSizeListArray::from_iter_primitive::<Int32Type, _, _>(
            vec![Some(vec![Some(4), Some(5)])],
            2,
        );

        assert_eq!(array_value_to_json(&large, 0).unwrap(), json!([7, 8]));
        assert_eq!(array_value_to_json(&fixed, 0).unwrap(), json!([4, 5]));
    }

    #[tokio::test]
    async fn struct_becomes_json_object_and_keeps_null_fields() {
        let (sub_field, threshold_field) = subscription_fields();
        let array = StructArray::from(vec![
            (
                sub_field,
                Arc::new(StringArray::from(vec![Some("user-a"), None])) as ArrayRef,
            ),
            (
                threshold_field,
                Arc::new(Int64Array::from(vec![Some(30i64), None])) as ArrayRef,
            ),
        ]);

        assert_eq!(
            array_value_to_json(&array, 0).unwrap(),
            json!({"sub": "user-a", "threshold": 30})
        );
        assert_eq!(
            array_value_to_json(&array, 1).unwrap(),
            json!({"sub": Value::Null, "threshold": Value::Null})
        );
    }

    #[tokio::test]
    async fn null_struct_row_becomes_json_null() {
        let (sub_field, threshold_field) = subscription_fields();
        let array = StructArray::try_new(
            vec![sub_field, threshold_field].into(),
            vec![
                Arc::new(StringArray::from(vec!["user-a", "user-b"])) as ArrayRef,
                Arc::new(Int64Array::from(vec![30i64, 50i64])) as ArrayRef,
            ],
            Some(NullBuffer::from(vec![true, false])),
        )
        .unwrap();

        assert_eq!(
            array_value_to_json(&array, 0).unwrap(),
            json!({"sub": "user-a", "threshold": 30})
        );
        assert_eq!(array_value_to_json(&array, 1).unwrap(), Value::Null);
    }

    #[tokio::test]
    async fn list_of_structs_becomes_array_of_json_objects() {
        let array = subscriptions_list_array();

        assert_eq!(
            array_value_to_json(&array, 0).unwrap(),
            json!([
                {"sub": "user-a", "threshold": 30},
                {"sub": "user-b", "threshold": 50},
            ])
        );
    }

    #[tokio::test]
    async fn batches_to_rows_keeps_nested_lists_as_arrays() {
        let array = subscriptions_list_array();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "subscriptions",
            array.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(array)]).unwrap();

        let rows = batches_to_rows(&[batch]).unwrap();
        let subscriptions = rows[0].get("subscriptions").unwrap();

        assert!(
            matches!(subscriptions, Value::Array(_)),
            "expected a JSON array, got {subscriptions:?}"
        );
        assert_eq!(
            subscriptions,
            &json!([
                {"sub": "user-a", "threshold": 30},
                {"sub": "user-b", "threshold": 50},
            ])
        );
    }

    #[tokio::test]
    async fn array_agg_named_struct_survives_as_nested_json() {
        use flow_like_storage::datafusion::prelude::SessionContext;

        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT ARRAY_AGG(
                    NAMED_STRUCT(
                        'sub', user_sub,
                        'threshold', relevance_threshold
                    )
                    ORDER BY user_sub
                ) AS subscriptions
                FROM (
                    VALUES
                        ('user-a', 30),
                        ('user-b', 50)
                ) AS subscriptions(user_sub, relevance_threshold)",
            )
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();

        let rows = batches_to_rows(&batches).unwrap();
        assert_eq!(rows.len(), 1);

        let subscriptions = rows[0].get("subscriptions").unwrap();
        assert!(
            matches!(subscriptions, Value::Array(_)),
            "expected a JSON array, got {subscriptions:?}"
        );
        assert_eq!(
            subscriptions,
            &json!([
                {"sub": "user-a", "threshold": 30},
                {"sub": "user-b", "threshold": 50},
            ])
        );
    }

    #[tokio::test]
    async fn test_sql_query_node_structure() {
        let node_logic = SqlQueryNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_sql_query");
        assert_eq!(node.friendly_name, "SQL Query");
        assert_eq!(node.version, Some(3));

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
        assert!(input_pins.iter().any(|p| p.name == "params"));
        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
        assert!(output_pins.iter().any(|p| p.name == "table"));
        assert!(output_pins.iter().any(|p| p.name == "rows"));
        assert!(output_pins.iter().any(|p| p.name == "row_count"));
        assert_eq!(rows_pin.data_type, VariableType::Struct);
        assert_eq!(rows_pin.value_type, ValueType::Array);
    }
}

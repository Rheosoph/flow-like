use std::{collections::HashSet, sync::Arc};

use arrow::datatypes::FieldRef;
use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_schema::{DataType, Field, TimeUnit};
use flow_like_types::{
    Result, Value, anyhow,
    json::{Deserialize, Serialize, to_value},
    serde::{Serializer, ser::SerializeMap},
};
use serde_arrow::schema::{SchemaLike, TracingOptions};

use crate::databases::vector::schema::TableInputRejected;

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

    normalize_temporal_values(&mut records, &fields)
        .map_err(|error| TableInputRejected(error.to_string()))?;
    normalize_geometry_values(&mut records, &fields)?;
    normalize_text_values(&mut records, &fields);

    // Build a record batch. Schema inference above deliberately sees the raw
    // values so new tables keep their traced column types; only the write is
    // coerced.
    let rows: Vec<TemporalSafeValue> = records.iter().map(TemporalSafeValue).collect();
    let batch: RecordBatch = serde_arrow::to_record_batch(&fields, &rows)
        .map_err(|error| TableInputRejected(error.to_string()))?;
    Ok(batch)
}

/// Converts rows bound for the existing `table`, taking every column's type,
/// metadata and nullability from its schema instead of the values: inferred
/// types would turn GeoJSON for a geometry column into a struct, bytes for a
/// Binary column into a list of 64-bit integers, and instants into strings.
///
/// Only the columns some row carries are written, in table order, so partial
/// rows keep their merge semantics.
pub fn value_to_record_batch_for_schema(
    records: Vec<Value>,
    schema: &arrow_schema::Schema,
    table: &str,
) -> Result<RecordBatch> {
    let mut carried = std::collections::BTreeSet::new();
    for (row, record) in records.iter().enumerate() {
        let Value::Object(record) = record else {
            return Err(TableInputRejected(format!(
                "Row {row} for table '{table}' must be a JSON object keyed by column name"
            ))
            .into());
        };
        carried.extend(record.keys().map(String::as_str));
    }

    let unknown: Vec<&str> = carried
        .iter()
        .copied()
        .filter(|name| schema.field_with_name(name).is_err())
        .collect();
    if !unknown.is_empty() {
        return Err(unknown_columns(table, &unknown, schema).into());
    }

    let fields = schema
        .fields()
        .iter()
        .filter(|field| carried.contains(field.name().as_str()))
        .cloned()
        .collect();
    value_to_record_batch_with_fields(records, Some(fields))
}

pub(crate) fn unknown_columns(
    table: &str,
    unknown: &[&str],
    schema: &arrow_schema::Schema,
) -> TableInputRejected {
    TableInputRejected(format!(
        "Table '{table}' has no column {}; its columns are {}",
        quoted_list(unknown.iter().copied()),
        quoted_list(schema.fields().iter().map(|field| field.name().as_str()))
    ))
}

fn quoted_list<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Converts values for the first write to a new table. Top-level columns whose
/// non-null values are all GeoJSON geometries become WGS 84 geometry columns,
/// arrays of byte-sized integers become Binary, and RFC3339 instants become UTC
/// millisecond timestamps.
///
/// Existing tables must continue to use `value_to_record_batch_with_fields`
/// with their persisted schema so legacy columns keep their type.
pub(crate) fn value_to_record_batch_with_utc_timestamp_inference(
    records: Vec<Value>,
) -> Result<RecordBatch> {
    let fields = infer_new_table_fields(&records)?;
    value_to_record_batch_with_fields(records, Some(fields))
}

fn infer_new_table_fields(records: &[Value]) -> Result<Vec<FieldRef>> {
    let value_typed = value_typed_fields(records)?;
    let masked: HashSet<&str> = value_typed
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    let samples: Vec<MaskedRecord> = records
        .iter()
        .map(|record| MaskedRecord {
            record,
            masked: &masked,
        })
        .collect();

    let fields = trace_fields(&samples, records)?
        .into_iter()
        .map(|field| {
            value_typed
                .iter()
                .find(|typed| typed.name() == field.name())
                .cloned()
                .unwrap_or(field)
        })
        .collect();

    Ok(promote_rfc3339_instants(records, fields))
}

/// Serializes a record with the given top-level columns traced as null, so a
/// column typed from its values never has to be expressible to the tracer.
/// GeoJSON is not: Point and Polygon rows nest `coordinates` differently.
struct MaskedRecord<'a> {
    record: &'a Value,
    masked: &'a HashSet<&'a str>,
}

impl Serialize for MaskedRecord<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let Value::Object(entries) = self.record else {
            return self.record.serialize(serializer);
        };
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            if self.masked.contains(key.as_str()) {
                map.serialize_entry(key, &())?;
            } else {
                map.serialize_entry(key, value)?;
            }
        }
        map.end()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueShape {
    Geometry,
    Bytes,
}

/// Columns whose values carry a FlowLike type that serde_arrow cannot see: a
/// Geometry pin's GeoJSON would trace as a struct, and a Bytes pin's array as
/// a list of 64-bit integers.
fn value_typed_fields(records: &[Value]) -> Result<Vec<FieldRef>> {
    let mut seen = HashSet::new();
    records
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|record| record.keys())
        .filter(|name| name.as_str() != "vector" && seen.insert(name.as_str()))
        .filter_map(|name| value_typed_field(records, name).transpose())
        .map(|field| field.map(Arc::new))
        .collect()
}

/// A column whose values all name a geometry kind is a geometry column, so a
/// value that is not a valid WGS 84 geometry fails the write instead of turning
/// the column into a struct.
fn value_typed_field(records: &[Value], name: &str) -> Result<Option<Field>> {
    let mut shape = None;
    let mut nullable = false;
    let mut has_bytes = false;

    for record in records {
        let value = record.get(name).unwrap_or(&Value::Null);
        if value.is_null() {
            nullable = true;
            continue;
        }

        let value_shape = if crate::geometry::names_geometry_kind(value) {
            ValueShape::Geometry
        } else if is_byte_array(value) {
            has_bytes |= value.as_array().is_some_and(|bytes| !bytes.is_empty());
            ValueShape::Bytes
        } else {
            return Ok(None);
        };
        if shape.is_some_and(|shape| shape != value_shape) {
            return Ok(None);
        }
        shape = Some(value_shape);
    }

    Ok(match shape {
        None => None,
        Some(ValueShape::Geometry) => {
            validate_inferred_geometries(records, name)?;
            Some(crate::geometry::geometry_field(name, nullable))
        }
        Some(ValueShape::Bytes) if has_bytes => Some(Field::new(name, DataType::Binary, nullable)),
        Some(ValueShape::Bytes) => None,
    })
}

fn validate_inferred_geometries(records: &[Value], name: &str) -> Result<()> {
    for (row, record) in records.iter().enumerate() {
        let value = record.get(name).unwrap_or(&Value::Null);
        if let Err(error) = crate::geometry::geometry_input(value) {
            return Err(TableInputRejected(format!(
                "Column '{name}' holds GeoJSON geometries, but row {row} is not a valid WGS 84 geometry: {error}"
            ))
            .into());
        }
    }
    Ok(())
}

fn is_byte_array(value: &Value) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .all(|value| value.as_u64().is_some_and(|byte| byte <= u8::MAX as u64))
    })
}

fn infer_fields(records: &[Value]) -> Result<Vec<FieldRef>> {
    trace_fields(records, records)
}

fn trace_fields<T: Serialize>(samples: &[T], records: &[Value]) -> Result<Vec<FieldRef>> {
    let mut fields: Vec<FieldRef> =
        Vec::<FieldRef>::from_samples(samples, TracingOptions::default().allow_null_fields(true))
            .map_err(|error| TableInputRejected(error.to_string()))?;

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

/// Normalize only explicitly declared geometry fields. Ordinary JSON objects retain their type.
fn normalize_geometry_values(records: &mut [Value], fields: &[FieldRef]) -> Result<()> {
    for field in fields {
        crate::geometry::validate_geometry_field(field)?;
    }
    for field in fields
        .iter()
        .filter(|field| crate::geometry::is_geometry_field(field))
    {
        crate::geometry::validate_crs(field)?;
        if field.data_type() != &DataType::Binary {
            return Err(anyhow!(
                "JSON writes require WKB Binary geometry storage for '{}'",
                field.name()
            ));
        }
        for (row, record) in records.iter_mut().enumerate() {
            let Some(value) = record
                .as_object_mut()
                .and_then(|record| record.get_mut(field.name()))
            else {
                continue;
            };
            let bytes = crate::geometry::geometry_input_wkb(value).map_err(|error| {
                TableInputRejected(format!(
                    "Geometry column '{}', row {row}: {error}",
                    field.name()
                ))
            })?;
            *value = bytes.map_or(Value::Null, |bytes| {
                Value::Array(bytes.into_iter().map(Value::from).collect())
            });
        }
    }
    Ok(())
}

/// Declared text columns store objects and arrays as their compact JSON text,
/// the way an update already does.
fn normalize_text_values(records: &mut [Value], fields: &[FieldRef]) {
    for field in fields.iter().filter(|field| {
        matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8)
            && !crate::geometry::is_geometry_field(field)
    }) {
        for record in records.iter_mut() {
            if let Some(value @ (Value::Object(_) | Value::Array(_))) = record
                .as_object_mut()
                .and_then(|record| record.get_mut(field.name()))
            {
                *value = Value::String(value.to_string());
            }
        }
    }
}

/// JSON's data model with lossless byte-array support for Arrow Binary columns.
struct ByteSafeValue(Value);
impl<'de> Deserialize<'de> for ByteSafeValue {
    fn deserialize<D: flow_like_types::serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use flow_like_types::serde::de::{MapAccess, SeqAccess, Visitor};
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = ByteSafeValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value or byte array")
            }
            fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(Value::Null))
            }
            fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(Value::Null))
            }
            fn visit_some<D: flow_like_types::serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> std::result::Result<Self::Value, D::Error> {
                ByteSafeValue::deserialize(d)
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(v.into()))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(v.into()))
            }
            fn visit_f64<E>(self, v: f64) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(Value::from(v)))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(v.into()))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(v.into()))
            }
            fn visit_bytes<E>(self, bytes: &[u8]) -> std::result::Result<Self::Value, E> {
                Ok(ByteSafeValue(Value::Array(
                    bytes.iter().copied().map(Value::from).collect(),
                )))
            }
            fn visit_byte_buf<E: flow_like_types::serde::de::Error>(
                self,
                bytes: Vec<u8>,
            ) -> std::result::Result<Self::Value, E> {
                self.visit_bytes(&bytes)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(ByteSafeValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(ByteSafeValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = flow_like_types::json::Map::new();
                while let Some((key, ByteSafeValue(value))) = map.next_entry()? {
                    values.insert(key, value);
                }
                Ok(ByteSafeValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

fn column_to_values(
    array: &std::sync::Arc<dyn arrow_array::Array>,
    field: &FieldRef,
) -> Result<Vec<Value>> {
    use arrow_array::{Array, FixedSizeListArray, LargeListArray, ListArray, StructArray};
    if crate::geometry::is_geometry_field(field) {
        return crate::geometry::decode_column(array.as_ref(), field);
    }
    if crate::geometry::contains_geometry_field(field) {
        match field.data_type() {
            DataType::Struct(fields) => {
                let values = array
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| anyhow!("Invalid Struct layout"))?;
                return (0..array.len())
                    .map(|row| {
                        if array.is_null(row) {
                            return Ok(Value::Null);
                        }
                        let mut object = serde_json::Map::new();
                        for (field, column) in fields.iter().zip(values.columns()) {
                            let value = column_to_values(&column.slice(row, 1), field)?.remove(0);
                            object.insert(field.name().clone(), value);
                        }
                        Ok(Value::Object(object))
                    })
                    .collect();
            }
            DataType::List(child)
            | DataType::LargeList(child)
            | DataType::FixedSizeList(child, _) => {
                return (0..array.len())
                    .map(|row| {
                        if array.is_null(row) {
                            return Ok(Value::Null);
                        }
                        let values = if let Some(list) = array.as_any().downcast_ref::<ListArray>()
                        {
                            list.value(row)
                        } else if let Some(list) = array.as_any().downcast_ref::<LargeListArray>() {
                            list.value(row)
                        } else if let Some(list) =
                            array.as_any().downcast_ref::<FixedSizeListArray>()
                        {
                            list.value(row)
                        } else {
                            return Err(anyhow!("Invalid geometry list layout"));
                        };
                        Ok(Value::Array(column_to_values(&values, child)?))
                    })
                    .collect();
            }
            _ => {
                return Err(anyhow!(
                    "Unsupported nested geometry layout in '{}'",
                    field.name()
                ));
            }
        }
    }
    let batch = RecordBatch::try_new(
        Arc::new(arrow_schema::Schema::new(vec![field.clone()])),
        vec![array.clone()],
    )?;
    let rows: Vec<ByteSafeValue> = serde_arrow::from_record_batch(&batch)?;
    rows.into_iter()
        .map(|ByteSafeValue(mut row)| {
            row.as_object_mut()
                .and_then(|row| row.remove(field.name()))
                .ok_or_else(|| anyhow!("Missing decoded column '{}'", field.name()))
        })
        .collect()
}

pub fn record_batch_to_value(record_batch: &RecordBatch) -> Result<Vec<Value>> {
    let mut rows = vec![flow_like_types::json::Map::new(); record_batch.num_rows()];
    let mut names = std::collections::HashSet::new();
    for (field, column) in record_batch
        .schema()
        .fields()
        .iter()
        .zip(record_batch.columns())
    {
        if !names.insert(field.name()) {
            return Err(anyhow!(
                "Duplicate result column '{}'; use distinct SQL aliases",
                field.name()
            ));
        }
        let values = column_to_values(column, field)?;
        if values.len() != rows.len() {
            return Err(anyhow!(
                "Decoded row count differs for column '{}'",
                field.name()
            ));
        }
        for (row, value) in rows.iter_mut().zip(values) {
            row.insert(field.name().clone(), value);
        }
    }
    Ok(rows.into_iter().map(Value::Object).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Date32Array, TimestampMillisecondArray, UInt64Array};
    use flow_like_types::json::{Deserialize, json, to_value};

    #[test]
    fn binary_and_geometry_preserve_all_rows() -> Result<()> {
        let point = json!({"type":"Point","coordinates":[13.405,52.52]});
        let fields = vec![
            Arc::new(Field::new("bytes", DataType::Binary, true)),
            Arc::new(crate::geometry::geometry_field("location", true)),
        ];
        let rows = vec![
            json!({"bytes":[0,255,17],"location":point}),
            json!({"bytes":[],"location":null}),
            json!({"bytes":null,"location":point}),
        ];
        let batch = value_to_record_batch_with_fields(rows.clone(), Some(fields))?;
        assert_eq!(record_batch_to_value(&batch)?, rows);
        Ok(())
    }

    fn inferred_type(batch: &RecordBatch, name: &str) -> DataType {
        batch
            .schema()
            .field_with_name(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .data_type()
            .clone()
    }

    #[test]
    fn new_tables_store_geojson_geometries_as_wgs84_geometry() -> Result<()> {
        let rows = vec![
            json!({"id": "a", "geometry": {"type": "Point", "coordinates": [13.405, 52.52]}}),
            json!({"id": "b", "geometry": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]]}}),
            json!({"id": "c", "geometry": null}),
        ];

        let batch = value_to_record_batch_with_utc_timestamp_inference(rows.clone())?;

        assert_eq!(
            batch.schema().field_with_name("geometry")?,
            &crate::geometry::geometry_field("geometry", true)
        );
        assert_eq!(record_batch_to_value(&batch)?, rows);
        Ok(())
    }

    #[test]
    fn new_tables_keep_objects_that_do_not_all_name_a_geometry_kind() -> Result<()> {
        let feature = json!({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [1.0, 2.0]},
            "properties": {"name": "a"}
        });
        let rows = vec![
            json!({"feature": feature, "partly": {"type": "Point", "coordinates": [1.0, 2.0]}}),
            json!({"feature": feature, "partly": {"type": "Unknown", "coordinates": [1.0, 2.0]}}),
        ];
        let batch = value_to_record_batch_with_utc_timestamp_inference(rows)?;

        assert_eq!(record_batch_to_value(&batch)?[0]["feature"], feature);
        for name in ["feature", "partly"] {
            assert!(
                matches!(inferred_type(&batch, name), DataType::Struct(_)),
                "{name}"
            );
            assert!(!crate::geometry::is_geometry_field(
                batch.schema().field_with_name(name)?
            ));
        }
        Ok(())
    }

    #[test]
    fn new_tables_store_annotated_geometries_without_their_foreign_members() -> Result<()> {
        let rows = vec![json!({"annotated": {
            "type": "Point",
            "coordinates": [8.0, 50.0],
            "bbox": [8.0, 50.0, 8.0, 50.0],
            "name": "HQ"
        }})];
        let batch = value_to_record_batch_with_utc_timestamp_inference(rows)?;

        assert!(crate::geometry::is_geometry_field(
            batch.schema().field_with_name("annotated")?
        ));
        assert_eq!(
            record_batch_to_value(&batch)?[0]["annotated"],
            json!({"type": "Point", "coordinates": [8.0, 50.0]})
        );
        Ok(())
    }

    #[test]
    fn new_tables_reject_invalid_geometries_naming_column_and_row() {
        let error = value_to_record_batch_with_utc_timestamp_inference(vec![
            json!({"projected": {"type": "Point", "coordinates": [8.0, 50.0]}}),
            json!({"projected": {"type": "Point", "coordinates": [500000.0, 4649776.0]}}),
        ])
        .expect_err("projected coordinates are not WGS 84");

        assert!(
            error.downcast_ref::<TableInputRejected>().is_some(),
            "{error:#}"
        );
        let message = error.to_string();
        assert!(message.contains("'projected'"), "{message}");
        assert!(message.contains("row 1"), "{message}");
        assert!(message.contains("WGS 84"), "{message}");
    }

    #[test]
    fn declared_text_columns_store_objects_and_arrays_as_json_text() -> Result<()> {
        let batch = value_to_record_batch_with_fields(
            vec![json!({
                "tags": {"addr:city": "Springfield", "levels": [1, 2]},
                "ids": ["a", "b"],
                "count": 3
            })],
            Some(vec![
                Arc::new(Field::new("tags", DataType::Utf8, true)),
                Arc::new(Field::new("ids", DataType::LargeUtf8, true)),
                Arc::new(Field::new("count", DataType::Utf8, true)),
            ]),
        )?;

        assert_eq!(
            record_batch_to_value(&batch)?[0],
            json!({
                "tags": r#"{"addr:city":"Springfield","levels":[1,2]}"#,
                "ids": r#"["a","b"]"#,
                "count": "3"
            })
        );
        Ok(())
    }

    #[test]
    fn declared_geometry_columns_take_features_and_text() -> Result<()> {
        let point = json!({"type": "Point", "coordinates": [13.405, 52.52]});
        let batch = value_to_record_batch_with_fields(
            vec![
                json!({"geom": {"type": "Feature", "geometry": point, "properties": {}}}),
                json!({"geom": point.to_string()}),
                json!({"geom": "POINT (13.405 52.52)"}),
                json!({"geom": {"type": "Feature", "geometry": null, "properties": {}}}),
            ],
            Some(vec![Arc::new(crate::geometry::geometry_field(
                "geom", true,
            ))]),
        )?;

        let rows = record_batch_to_value(&batch)?;
        assert_eq!(
            rows.iter()
                .map(|row| row["geom"].clone())
                .collect::<Vec<_>>(),
            vec![point.clone(), point.clone(), point, Value::Null]
        );
        Ok(())
    }

    #[test]
    fn conversion_failures_are_table_input_rejections() {
        let error = value_to_record_batch_with_fields(
            vec![json!({"count": "seven"})],
            Some(vec![Arc::new(Field::new("count", DataType::Int64, true))]),
        )
        .expect_err("text is not an integer");
        assert!(
            error.downcast_ref::<TableInputRejected>().is_some(),
            "{error:#}"
        );

        let error = value_to_record_batch_for_schema(
            vec![json!({"id": "a", "color": "red"})],
            &existing_table_schema(),
            "places",
        )
        .expect_err("unknown columns are rejected");
        assert!(
            error.downcast_ref::<TableInputRejected>().is_some(),
            "{error:#}"
        );
    }

    #[test]
    fn new_tables_store_byte_arrays_as_binary() -> Result<()> {
        let rows = vec![
            json!({"payload": [0, 255, 17], "wide": [1, 300], "signed": [-1, -2], "ratios": [0.5], "empty": []}),
            json!({"payload": [], "wide": [2], "signed": [-3], "ratios": [1.5], "empty": []}),
            json!({"payload": null, "wide": [4], "signed": [-5], "ratios": [2.5], "empty": []}),
        ];

        let batch = value_to_record_batch_with_utc_timestamp_inference(rows.clone())?;

        assert_eq!(
            batch.schema().field_with_name("payload")?,
            &Field::new("payload", DataType::Binary, true)
        );
        for name in ["wide", "signed", "ratios", "empty"] {
            assert!(
                !matches!(inferred_type(&batch, name), DataType::Binary),
                "{name}"
            );
        }
        assert_eq!(record_batch_to_value(&batch)?, rows);
        Ok(())
    }

    #[test]
    fn value_typing_leaves_the_vector_column_and_existing_tables_alone() -> Result<()> {
        let fields = infer_new_table_fields(&[json!({"vector": [1, 2, 3], "bytes": [1, 2]})])?;
        let vector = fields
            .iter()
            .find(|field| field.name() == "vector")
            .unwrap();
        assert!(matches!(vector.data_type(), DataType::FixedSizeList(_, 3)));

        let point = json!({"type": "Point", "coordinates": [1.0, 2.0]});
        let batch = value_to_record_batch(vec![json!({"bytes": [1, 2], "location": point})])?;
        assert!(!matches!(inferred_type(&batch, "bytes"), DataType::Binary));
        assert!(!crate::geometry::is_geometry_field(
            batch.schema().field_with_name("location")?
        ));
        Ok(())
    }

    fn existing_table_schema() -> arrow_schema::Schema {
        arrow_schema::Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("payload", DataType::Binary, true),
            crate::geometry::geometry_field("location", true),
            Field::new(
                "seen_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                true,
            ),
            Field::new("note", DataType::LargeUtf8, true),
        ])
    }

    #[test]
    fn existing_table_rows_take_geometry_and_binary_columns_from_the_schema() -> Result<()> {
        let schema = existing_table_schema();
        let rows = vec![
            json!({"location": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]]}, "payload": [0, 255, 17], "id": "a"}),
            json!({"id": "b", "location": {"type": "Point", "coordinates": [13.405, 52.52]}, "payload": null}),
            json!({"id": "c", "location": null, "payload": []}),
        ];

        let batch = value_to_record_batch_for_schema(rows.clone(), &schema, "places")?;

        assert_eq!(batch.num_columns(), 3);
        for name in ["id", "payload", "location"] {
            assert_eq!(
                batch.schema().field_with_name(name)?,
                schema.field_with_name(name)?
            );
        }
        assert_eq!(record_batch_to_value(&batch)?, rows);
        Ok(())
    }

    #[test]
    fn existing_table_rows_follow_the_table_column_order() -> Result<()> {
        let batch = value_to_record_batch_for_schema(
            vec![
                json!({"note": "first", "seen_at": "2026-08-09T12:34:56.789Z", "id": "a"}),
                json!({"id": "b", "note": null}),
            ],
            &existing_table_schema(),
            "places",
        )?;

        let names: Vec<String> = batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        assert_eq!(names, ["id", "seen_at", "note"]);

        let seen_at = batch
            .column_by_name("seen_at")
            .and_then(|column| column.as_any().downcast_ref::<TimestampMillisecondArray>())
            .expect("seen_at should keep the table's millisecond timestamp type");
        assert_eq!(
            seen_at.value(0),
            chrono::DateTime::parse_from_rfc3339("2026-08-09T12:34:56.789Z")?.timestamp_millis()
        );
        assert_eq!(record_batch_to_value(&batch)?[1]["seen_at"], Value::Null);
        Ok(())
    }

    #[test]
    fn existing_table_rows_reject_columns_the_table_lacks() -> Result<()> {
        let error = value_to_record_batch_for_schema(
            vec![
                json!({"id": "a", "color": "red"}),
                json!({"id": "b", "Location": null}),
            ],
            &existing_table_schema(),
            "places",
        )
        .expect_err("unknown columns cannot be merged into the table");

        assert_eq!(
            error.to_string(),
            "Table 'places' has no column 'Location', 'color'; its columns are 'id', 'payload', 'location', 'seen_at', 'note'"
        );
        Ok(())
    }

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

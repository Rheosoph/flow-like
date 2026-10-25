use arrow_json::ReaderBuilder;
use datafusion::arrow::array::{GenericStringArray, OffsetSizeTrait};
use datafusion::arrow::{
    array::{
        Array, ArrayRef, BooleanArray, BooleanBuilder, FixedSizeListBuilder, Float32Array,
        Float32Builder, Float64Array, Float64Builder, Int16Array, Int16Builder, Int32Array,
        Int32Builder, Int64Array, Int64Builder, Int8Array, Int8Builder, LargeListBuilder,
        LargeStringArray, LargeStringBuilder, ListArray, ListBuilder, StringArray, StringBuilder,
    },
    datatypes::{DataType, Field, FieldRef},
    error::ArrowError,
};
use std::sync::Arc;

pub type Result<T, E = crate::schema_cast::record_convert::Error> = std::result::Result<T, E>;

macro_rules! cast_string_to_list_array {
    ($string_array:expr, $field_name:expr, $data_type:expr, $builder_type:expr, $primitive_type:ty) => {{
        let item_field = Arc::new(Field::new($field_name, $data_type, true));
        let mut list_builder = ListBuilder::with_capacity($builder_type, $string_array.len())
            .with_field(Arc::clone(&item_field));

        let list_field = Arc::new(Field::new_list("i", item_field, true));
        let mut decoder = ReaderBuilder::new_with_field(Arc::clone(&list_field))
            .build_decoder()
            .map_err(|e| ArrowError::CastError(format!("Failed to create decoder: {e}")))?;

        for value in $string_array {
            match value {
                None => list_builder.append_null(),
                Some(string_value) => {
                    decoder.decode(string_value.as_bytes()).map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode value: {e}"))
                    })?;

                    if let Some(b) = decoder.flush().map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode decoder: {e}"))
                    })? {
                        let list_array = b
                            .column(0)
                            .as_any()
                            .downcast_ref::<ListArray>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to ListArray"
                                        .to_string(),
                                )
                            })?;
                        let primitive_array = list_array
                            .values()
                            .as_any()
                            .downcast_ref::<$primitive_type>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to PrimitiveType"
                                        .to_string(),
                                )
                            })?;
                        primitive_array
                            .iter()
                            .for_each(|maybe_value| match maybe_value {
                                Some(value) => list_builder.values().append_value(value),
                                None => list_builder.values().append_null(),
                            });
                        list_builder.append(true);
                    }
                }
            }
        }

        Ok(Arc::new(list_builder.finish()))
    }};
}

macro_rules! cast_string_to_large_list_array {
    ($string_array:expr, $field_name:expr, $data_type:expr, $builder_type:expr, $primitive_type:ty) => {{
        let item_field = Arc::new(Field::new($field_name, $data_type, true));
        let mut list_builder = LargeListBuilder::with_capacity($builder_type, $string_array.len())
            .with_field(Arc::clone(&item_field));

        let list_field = Arc::new(Field::new_list("i", item_field, true));
        let mut decoder = ReaderBuilder::new_with_field(Arc::clone(&list_field))
            .build_decoder()
            .map_err(|e| ArrowError::CastError(format!("Failed to create decoder: {e}")))?;

        for value in $string_array {
            match value {
                None => list_builder.append_null(),
                Some(string_value) => {
                    decoder.decode(string_value.as_bytes()).map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode value: {e}"))
                    })?;

                    if let Some(b) = decoder.flush().map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode decoder: {e}"))
                    })? {
                        let list_array = b
                            .column(0)
                            .as_any()
                            .downcast_ref::<ListArray>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to ListArray"
                                        .to_string(),
                                )
                            })?;
                        let primitive_array = list_array
                            .values()
                            .as_any()
                            .downcast_ref::<$primitive_type>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to PrimitiveType"
                                        .to_string(),
                                )
                            })?;
                        primitive_array
                            .iter()
                            .for_each(|maybe_value| match maybe_value {
                                Some(value) => list_builder.values().append_value(value),
                                None => list_builder.values().append_null(),
                            });
                        list_builder.append(true);
                    }
                }
            }
        }

        Ok(Arc::new(list_builder.finish()))
    }};
}

macro_rules! cast_string_to_fixed_size_list_array {
    ($string_array:expr, $field_name:expr, $data_type:expr, $builder_type:expr, $primitive_type:ty, $value_length:expr) => {{
        let item_field = Arc::new(Field::new($field_name, $data_type, true));
        let mut list_builder =
            FixedSizeListBuilder::with_capacity($builder_type, $value_length, $string_array.len())
                .with_field(Arc::clone(&item_field));

        let list_field = Arc::new(Field::new_list("i", item_field, true));
        let mut decoder = ReaderBuilder::new_with_field(Arc::clone(&list_field))
            .build_decoder()
            .map_err(|e| ArrowError::CastError(format!("Failed to create decoder: {e}")))?;

        for value in $string_array {
            match value {
                None => {
                    for _ in 0..$value_length {
                        list_builder.values().append_null()
                    }
                    list_builder.append(true)
                }
                Some(string_value) => {
                    decoder.decode(string_value.as_bytes()).map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode value: {e}"))
                    })?;

                    if let Some(b) = decoder.flush().map_err(|e| {
                        ArrowError::CastError(format!("Failed to decode decoder: {e}"))
                    })? {
                        let list_array = b
                            .column(0)
                            .as_any()
                            .downcast_ref::<ListArray>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to ListArray"
                                        .to_string(),
                                )
                            })?;
                        let primitive_array = list_array
                            .values()
                            .as_any()
                            .downcast_ref::<$primitive_type>()
                            .ok_or_else(|| {
                                ArrowError::CastError(
                                    "Failed to decode value: unable to downcast to PrimitiveType"
                                        .to_string(),
                                )
                            })?;
                        primitive_array
                            .iter()
                            .for_each(|maybe_value| match maybe_value {
                                Some(value) => list_builder.values().append_value(value),
                                None => list_builder.values().append_null(),
                            });
                        list_builder.append(true);
                    }
                }
            }
        }

        Ok(Arc::new(list_builder.finish()))
    }};
}

pub(crate) fn cast_string_to_list<StringOffsetSize: OffsetSizeTrait>(
    array: &dyn Array,
    list_item_field: &FieldRef,
) -> Result<ArrayRef, ArrowError> {
    let string_array = array
        .as_any()
        .downcast_ref::<GenericStringArray<StringOffsetSize>>()
        .ok_or_else(|| {
            ArrowError::CastError(
                "Failed to decode value: unable to downcast to StringArray".to_string(),
            )
        })?;

    let field_name = list_item_field.name();

    match list_item_field.data_type() {
        DataType::Utf8 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Utf8,
                StringBuilder::new(),
                StringArray
            )
        }
        DataType::LargeUtf8 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::LargeUtf8,
                LargeStringBuilder::new(),
                LargeStringArray
            )
        }
        DataType::Boolean => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Boolean,
                BooleanBuilder::new(),
                BooleanArray
            )
        }
        DataType::Int8 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Int8,
                Int8Builder::new(),
                Int8Array
            )
        }
        DataType::Int16 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Int16,
                Int16Builder::new(),
                Int16Array
            )
        }
        DataType::Int32 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Int32,
                Int32Builder::new(),
                Int32Array
            )
        }
        DataType::Int64 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Int64,
                Int64Builder::new(),
                Int64Array
            )
        }
        DataType::Float32 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Float32,
                Float32Builder::new(),
                Float32Array
            )
        }
        DataType::Float64 => {
            cast_string_to_list_array!(
                string_array,
                field_name,
                DataType::Float64,
                Float64Builder::new(),
                Float64Array
            )
        }
        _ => Err(ArrowError::CastError(format!(
            "Unsupported list item type: {}",
            list_item_field.data_type()
        ))),
    }
}

pub(crate) fn cast_string_to_large_list<StringOffsetSize: OffsetSizeTrait>(
    array: &dyn Array,
    list_item_field: &FieldRef,
) -> Result<ArrayRef, ArrowError> {
    let string_array = array
        .as_any()
        .downcast_ref::<GenericStringArray<StringOffsetSize>>()
        .ok_or_else(|| {
            ArrowError::CastError(
                "Failed to decode value: unable to downcast to StringArray".to_string(),
            )
        })?;

    let field_name = list_item_field.name();

    match list_item_field.data_type() {
        DataType::Utf8 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Utf8,
                StringBuilder::new(),
                StringArray
            )
        }
        DataType::LargeUtf8 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::LargeUtf8,
                LargeStringBuilder::new(),
                LargeStringArray
            )
        }
        DataType::Boolean => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Boolean,
                BooleanBuilder::new(),
                BooleanArray
            )
        }
        DataType::Int8 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Int8,
                Int8Builder::new(),
                Int8Array
            )
        }
        DataType::Int16 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Int16,
                Int16Builder::new(),
                Int16Array
            )
        }
        DataType::Int32 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Int32,
                Int32Builder::new(),
                Int32Array
            )
        }
        DataType::Int64 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Int64,
                Int64Builder::new(),
                Int64Array
            )
        }
        DataType::Float32 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Float32,
                Float32Builder::new(),
                Float32Array
            )
        }
        DataType::Float64 => {
            cast_string_to_large_list_array!(
                string_array,
                field_name,
                DataType::Float64,
                Float64Builder::new(),
                Float64Array
            )
        }
        _ => Err(ArrowError::CastError(format!(
            "Unsupported list item type: {}",
            list_item_field.data_type()
        ))),
    }
}

pub(crate) fn cast_string_to_fixed_size_list<StringOffsetSize: OffsetSizeTrait>(
    array: &dyn Array,
    list_item_field: &FieldRef,
    value_length: i32,
) -> Result<ArrayRef, ArrowError> {
    let string_array = array
        .as_any()
        .downcast_ref::<GenericStringArray<StringOffsetSize>>()
        .ok_or_else(|| {
            ArrowError::CastError(
                "Failed to decode value: unable to downcast to StringArray".to_string(),
            )
        })?;

    let field_name = list_item_field.name();

    match list_item_field.data_type() {
        DataType::Utf8 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Utf8,
                StringBuilder::new(),
                StringArray,
                value_length
            )
        }
        DataType::LargeUtf8 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::LargeUtf8,
                LargeStringBuilder::new(),
                LargeStringArray,
                value_length
            )
        }
        DataType::Boolean => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Boolean,
                BooleanBuilder::new(),
                BooleanArray,
                value_length
            )
        }
        DataType::Int8 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Int8,
                Int8Builder::new(),
                Int8Array,
                value_length
            )
        }
        DataType::Int16 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Int16,
                Int16Builder::new(),
                Int16Array,
                value_length
            )
        }
        DataType::Int32 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Int32,
                Int32Builder::new(),
                Int32Array,
                value_length
            )
        }
        DataType::Int64 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Int64,
                Int64Builder::new(),
                Int64Array,
                value_length
            )
        }
        DataType::Float32 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Float32,
                Float32Builder::new(),
                Float32Array,
                value_length
            )
        }
        DataType::Float64 => {
            cast_string_to_fixed_size_list_array!(
                string_array,
                field_name,
                DataType::Float64,
                Float64Builder::new(),
                Float64Array,
                value_length
            )
        }
        _ => Err(ArrowError::CastError(format!(
            "Unsupported list item type: {}",
            list_item_field.data_type()
        ))),
    }
}

#[cfg(test)]
mod test {
    use datafusion::arrow::{
        array::{RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
    };

    use crate::schema_cast::record_convert::try_cast_to;

    use super::*;

    fn input_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("a", DataType::Utf8, false),
            Field::new("b", DataType::Utf8, false),
            Field::new("c", DataType::Utf8, false),
        ]))
    }

    fn output_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new(
                "a",
                DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
                false,
            ),
            Field::new(
                "b",
                DataType::LargeList(Arc::new(Field::new("item", DataType::Utf8, true))),
                false,
            ),
            Field::new(
                "c",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Boolean, true)), 3),
                false,
            ),
        ]))
    }

    fn batch_input() -> RecordBatch {
        RecordBatch::try_new(
            input_schema(),
            vec![
                Arc::new(StringArray::from(vec![
                    Some("[1, 2, 3]"),
                    Some("[4, 5, 6]"),
                ])),
                Arc::new(StringArray::from(vec![
                    Some("[\"foo\", \"bar\"]"),
                    Some("[\"baz\", \"qux\"]"),
                ])),
                Arc::new(StringArray::from(vec![
                    Some("[true, false, true]"),
                    Some("[false, true, false]"),
                ])),
            ],
        )
        .expect("record batch should not panic")
    }

    fn batch_expected() -> RecordBatch {
        let mut list_builder = ListBuilder::new(Int32Builder::new());
        list_builder.append_value([Some(1), Some(2), Some(3)]);
        list_builder.append_value([Some(4), Some(5), Some(6)]);
        let list_array = list_builder.finish();

        let mut large_list_builder = LargeListBuilder::new(StringBuilder::new());
        large_list_builder.append_value([Some("foo"), Some("bar")]);
        large_list_builder.append_value([Some("baz"), Some("qux")]);
        let large_list_array = large_list_builder.finish();

        let mut fixed_size_list_builder = FixedSizeListBuilder::new(BooleanBuilder::new(), 3);
        fixed_size_list_builder.values().append_value(true);
        fixed_size_list_builder.values().append_value(false);
        fixed_size_list_builder.values().append_value(true);
        fixed_size_list_builder.append(true);
        fixed_size_list_builder.values().append_value(false);
        fixed_size_list_builder.values().append_value(true);
        fixed_size_list_builder.values().append_value(false);
        fixed_size_list_builder.append(true);
        let fixed_size_list_array = fixed_size_list_builder.finish();

        RecordBatch::try_new(
            output_schema(),
            vec![
                Arc::new(list_array),
                Arc::new(large_list_array),
                Arc::new(fixed_size_list_array),
            ],
        )
        .expect("Failed to create expected RecordBatch")
    }

    #[test]
    fn test_cast_to_list_largelist_fixedsizelist() {
        let input_batch = batch_input();
        let expected = batch_expected();
        let actual = try_cast_to(input_batch, output_schema()).expect("cast should succeed");

        assert_eq!(actual, expected);
    }
}

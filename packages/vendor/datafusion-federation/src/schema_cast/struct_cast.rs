use arrow_json::ReaderBuilder;
use datafusion::arrow::array::{GenericStringArray, OffsetSizeTrait};
use datafusion::arrow::{
    array::{Array, ArrayRef},
    datatypes::Field,
    error::ArrowError,
};
use std::sync::Arc;

pub type Result<T, E = crate::schema_cast::record_convert::Error> = std::result::Result<T, E>;

pub(crate) fn cast_string_to_struct<StringOffsetSize: OffsetSizeTrait>(
    array: &dyn Array,
    struct_field: Arc<Field>,
) -> Result<ArrayRef, ArrowError> {
    let string_array = array
        .as_any()
        .downcast_ref::<GenericStringArray<StringOffsetSize>>()
        .ok_or_else(|| ArrowError::CastError("Failed to downcast to StringArray".to_string()))?;

    let mut decoder = ReaderBuilder::new_with_field(struct_field)
        .build_decoder()
        .map_err(|e| ArrowError::CastError(format!("Failed to create JSON decoder: {e}")))?;

    for value in string_array {
        match value {
            Some(v) => {
                decoder.decode(v.as_bytes()).map_err(|e| {
                    ArrowError::CastError(format!("Failed to decode struct array: {e}"))
                })?;
            }
            None => {
                decoder.decode("null".as_bytes()).map_err(|e| {
                    ArrowError::CastError(format!("Failed to decode struct array: {e}"))
                })?;
            }
        }
    }

    let record = match decoder.flush() {
        Ok(Some(record)) => record,
        Ok(None) => {
            return Err(ArrowError::CastError(
                "Failed to flush decoder: No record".to_string(),
            ));
        }
        Err(e) => {
            return Err(ArrowError::CastError(format!(
                "Failed to decode struct array: {e}"
            )));
        }
    };
    // struct_field is single struct column
    Ok(Arc::clone(record.column(0)))
}

#[cfg(test)]
mod test {
    use datafusion::arrow::{
        array::{Int32Builder, RecordBatch, StringArray, StringBuilder, StructBuilder},
        datatypes::{DataType, Field, Schema, SchemaRef},
    };

    use crate::schema_cast::record_convert::try_cast_to;

    use super::*;

    fn input_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new(
            "struct_string",
            DataType::Utf8,
            true,
        )]))
    }

    fn output_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new(
            "struct",
            DataType::Struct(
                vec![
                    Field::new("name", DataType::Utf8, false),
                    Field::new("age", DataType::Int32, false),
                ]
                .into(),
            ),
            true,
        )]))
    }

    fn batch_input() -> RecordBatch {
        RecordBatch::try_new(
            input_schema(),
            vec![Arc::new(StringArray::from(vec![
                Some(r#"{"name":"John","age":30}"#),
                None,
                None,
                Some(r#"{"name":"Jane","age":25}"#),
            ]))],
        )
        .expect("record batch should not panic")
    }

    fn batch_expected() -> RecordBatch {
        let name_field = Field::new("name", DataType::Utf8, false);
        let age_field = Field::new("age", DataType::Int32, false);

        let mut struct_builder = StructBuilder::new(
            vec![name_field, age_field],
            vec![
                Box::new(StringBuilder::new()),
                Box::new(Int32Builder::new()),
            ],
        );

        struct_builder
            .field_builder::<StringBuilder>(0)
            .expect("should return field builder")
            .append_value("John");
        struct_builder
            .field_builder::<Int32Builder>(1)
            .expect("should return field builder")
            .append_value(30);
        struct_builder.append(true);

        struct_builder
            .field_builder::<StringBuilder>(0)
            .expect("should return field builder")
            .append_null();
        struct_builder
            .field_builder::<Int32Builder>(1)
            .expect("should return field builder")
            .append_null();
        struct_builder.append(false);

        struct_builder
            .field_builder::<StringBuilder>(0)
            .expect("should return field builder")
            .append_null();
        struct_builder
            .field_builder::<Int32Builder>(1)
            .expect("should return field builder")
            .append_null();
        struct_builder.append(false);

        struct_builder
            .field_builder::<StringBuilder>(0)
            .expect("should return field builder")
            .append_value("Jane");
        struct_builder
            .field_builder::<Int32Builder>(1)
            .expect("should return field builder")
            .append_value(25);
        struct_builder.append(true);

        let struct_array = struct_builder.finish();

        RecordBatch::try_new(output_schema(), vec![Arc::new(struct_array)])
            .expect("Failed to create expected RecordBatch")
    }

    #[test]
    fn test_cast_to_struct() {
        let input_batch = batch_input();
        let expected = batch_expected();

        let actual = try_cast_to(input_batch, output_schema()).expect("cast should succeed");

        assert_eq!(actual, expected);
    }
}

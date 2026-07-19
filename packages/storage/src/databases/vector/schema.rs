use std::{collections::HashSet, sync::Arc};

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use flow_like_types::{Result, anyhow};

/// Agent-friendly description of one column in a LanceDB table.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct DatabaseSchemaField {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default = "default_nullable")]
    pub nullable: bool,
    #[serde(default)]
    pub vector_size: Option<u32>,
}

fn default_nullable() -> bool {
    true
}

fn validate_field_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(anyhow!("Column name must be 1-128 characters"));
    }

    let mut chars = name.chars();
    let first = chars.next().expect("name was checked as non-empty");
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(anyhow!(
            "Column name '{name}' is invalid (use ASCII letters, numbers, and underscores; do not start with a number)"
        ));
    }

    if matches!(
        name.to_ascii_lowercase().as_str(),
        "_rowid" | "_distance" | "_relevance_score"
    ) {
        return Err(anyhow!("Column name '{name}' is reserved by LanceDB"));
    }

    Ok(())
}

fn field_data_type(field: &DatabaseSchemaField) -> Result<DataType> {
    let normalized = field.data_type.trim().to_ascii_lowercase();
    let scalar = match normalized.as_str() {
        "string" | "text" | "utf8" => Some(DataType::Utf8),
        "bool" | "boolean" => Some(DataType::Boolean),
        "int8" => Some(DataType::Int8),
        "int16" => Some(DataType::Int16),
        "int32" | "integer" => Some(DataType::Int32),
        "int64" | "bigint" => Some(DataType::Int64),
        "uint8" => Some(DataType::UInt8),
        "uint16" => Some(DataType::UInt16),
        "uint32" => Some(DataType::UInt32),
        "uint64" => Some(DataType::UInt64),
        "float32" | "float" => Some(DataType::Float32),
        "float64" | "double" => Some(DataType::Float64),
        "binary" | "bytes" => Some(DataType::Binary),
        "date" | "date32" => Some(DataType::Date32),
        "timestamp" | "datetime" | "timestamp_ms" => {
            Some(DataType::Timestamp(TimeUnit::Millisecond, None))
        }
        "vector" | "vector_float32" => None,
        _ => {
            return Err(anyhow!(
                "Unsupported type '{}' for column '{}'. Supported types: string, boolean, int8, int16, int32, int64, uint8, uint16, uint32, uint64, float32, float64, binary, date32, timestamp, vector",
                field.data_type,
                field.name
            ));
        }
    };

    if let Some(data_type) = scalar {
        if field.vector_size.is_some() {
            return Err(anyhow!(
                "vector_size is only valid for vector columns (column '{}')",
                field.name
            ));
        }
        return Ok(data_type);
    }

    let vector_size = field.vector_size.ok_or_else(|| {
        anyhow!(
            "Vector column '{}' requires a positive vector_size",
            field.name
        )
    })?;
    let vector_size = i32::try_from(vector_size)
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| {
            anyhow!(
                "vector_size for column '{}' must be between 1 and {}",
                field.name,
                i32::MAX
            )
        })?;

    Ok(DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, false)),
        vector_size,
    ))
}

/// Validate a simplified schema and convert it into the Arrow schema LanceDB expects.
pub fn database_fields_to_arrow_schema(fields: &[DatabaseSchemaField]) -> Result<Schema> {
    if fields.is_empty() {
        return Err(anyhow!("A table schema requires at least one field"));
    }
    if fields.len() > 256 {
        return Err(anyhow!("A table schema supports at most 256 fields"));
    }

    let mut names = HashSet::with_capacity(fields.len());
    let mut arrow_fields = Vec::with_capacity(fields.len());
    for field in fields {
        validate_field_name(&field.name)?;
        let normalized_name = field.name.to_ascii_lowercase();
        if !names.insert(normalized_name) {
            return Err(anyhow!("Duplicate column name '{}'", field.name));
        }
        arrow_fields.push(Field::new(
            &field.name,
            field_data_type(field)?,
            field.nullable,
        ));
    }

    Ok(Schema::new(arrow_fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, data_type: &str) -> DatabaseSchemaField {
        DatabaseSchemaField {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: true,
            vector_size: None,
        }
    }

    #[test]
    fn converts_scalar_and_vector_fields() {
        let mut embedding = field("embedding", "vector");
        embedding.nullable = false;
        embedding.vector_size = Some(384);

        let schema = database_fields_to_arrow_schema(&[
            field("ticket_id", "string"),
            field("created_at", "timestamp"),
            embedding,
        ])
        .unwrap();

        assert_eq!(schema.fields().len(), 3);
        assert_eq!(
            schema.field_with_name("created_at").unwrap().data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, None)
        );
        assert_eq!(
            schema.field_with_name("embedding").unwrap().data_type(),
            &DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 384)
        );
        assert!(!schema.field_with_name("embedding").unwrap().is_nullable());
    }

    #[test]
    fn rejects_invalid_and_duplicate_names() {
        assert!(database_fields_to_arrow_schema(&[field("bad-name", "string")]).is_err());
        assert!(
            database_fields_to_arrow_schema(&[
                field("TicketId", "string"),
                field("ticketid", "string")
            ])
            .is_err()
        );
        assert!(database_fields_to_arrow_schema(&[field("_rowid", "int64")]).is_err());
    }

    #[test]
    fn rejects_invalid_type_and_vector_size_combinations() {
        assert!(database_fields_to_arrow_schema(&[field("value", "object")]).is_err());

        let vector_without_size = field("embedding", "vector");
        assert!(database_fields_to_arrow_schema(&[vector_without_size]).is_err());

        let mut scalar_with_size = field("value", "float32");
        scalar_with_size.vector_size = Some(3);
        assert!(database_fields_to_arrow_schema(&[scalar_with_size]).is_err());
    }
}

use std::{collections::HashSet, sync::Arc};

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use flow_like_types::{Result, anyhow};
use lance::datatypes::{LANCE_UNENFORCED_PRIMARY_KEY, LANCE_UNENFORCED_PRIMARY_KEY_POSITION};
use lancedb::table::FieldMetadataUpdate;

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
    /// Marks the table key (Lance unenforced primary key). At most one column; it must be required.
    #[serde(default)]
    pub primary_key: bool,
}

fn default_nullable() -> bool {
    true
}

/// A request the table key rules refuse. The message names the table and column and is
/// safe to show to the caller.
#[derive(Debug)]
pub struct PrimaryKeyRejected(pub String);

impl std::fmt::Display for PrimaryKeyRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PrimaryKeyRejected {}

/// Row values or column names a table write cannot accept. The message names the column and
/// the problem and is safe to show to the caller, who can correct the input and retry.
#[derive(Debug)]
pub struct TableInputRejected(pub String);

impl std::fmt::Display for TableInputRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TableInputRejected {}

/// Lance orders keys by explicit position; the legacy boolean flag reads as position 0.
fn primary_key_position(field: &Field) -> Option<u32> {
    let metadata = field.metadata();
    metadata
        .get(LANCE_UNENFORCED_PRIMARY_KEY_POSITION)
        .and_then(|position| position.parse().ok())
        .or_else(|| {
            metadata
                .get(LANCE_UNENFORCED_PRIMARY_KEY)
                .filter(|flag| matches!(flag.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
                .map(|_| 0)
        })
}

/// Top-level key columns in Lance's order: positioned keys first, then legacy flags.
pub fn primary_key_columns(schema: &Schema) -> Vec<String> {
    let mut keys: Vec<_> = schema
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            primary_key_position(field).map(|position| (position == 0, position, index, field))
        })
        .collect();
    keys.sort_by_key(|(legacy, position, index, _)| (*legacy, *position, *index));
    keys.into_iter()
        .map(|(.., field)| field.name().clone())
        .collect()
}

/// Why `field` cannot become the table key, or `None` when it can. Lance rejects a
/// nullable key whenever a schema carrying the marker is converted, and merge_insert
/// source batches carry it. The types are the ones Lance's inserted-key conflict filter
/// hashes; it skips any other type, which would leave concurrent inserts unchecked.
pub fn primary_key_ineligibility(field: &Field) -> Option<String> {
    if field.is_nullable() {
        return Some("it is optional (nullable); only a required column can be the key".into());
    }
    if !matches!(
        field.data_type(),
        DataType::Int32
            | DataType::Int64
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Binary
            | DataType::LargeBinary
    ) {
        return Some(format!(
            "its type {} is not supported; the key must be Int32, Int64, UInt32, UInt64, Utf8, LargeUtf8, Binary or LargeBinary",
            field.data_type()
        ));
    }
    None
}

const PRIMARY_KEY_POSITION: &str = "1";

pub fn with_primary_key_marker(field: Field) -> Field {
    let mut metadata = field.metadata().clone();
    metadata.remove(LANCE_UNENFORCED_PRIMARY_KEY);
    metadata.insert(
        LANCE_UNENFORCED_PRIMARY_KEY_POSITION.to_string(),
        PRIMARY_KEY_POSITION.into(),
    );
    field.with_metadata(metadata)
}

pub fn without_primary_key_marker(
    metadata: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut metadata = metadata.clone();
    metadata.remove(LANCE_UNENFORCED_PRIMARY_KEY);
    metadata.remove(LANCE_UNENFORCED_PRIMARY_KEY_POSITION);
    metadata
}

/// Marks an existing column as the key. lancedb's `set_unenforced_primary_key` refuses
/// UInt32 and UInt64, which Lance's inserted-key filter hashes, so the marker is written
/// directly; Lance still rejects any change once a key is set.
pub fn primary_key_marker_update(column: &str) -> FieldMetadataUpdate {
    let mut update = FieldMetadataUpdate::new(column);
    update.metadata.insert(
        LANCE_UNENFORCED_PRIMARY_KEY_POSITION.to_string(),
        Some(PRIMARY_KEY_POSITION.into()),
    );
    update
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
        "binary" | "bytes" | "geometry" => Some(DataType::Binary),
        "date" | "date32" => Some(DataType::Date32),
        // FlowLike Date values represent instants and serialize as RFC3339,
        // so their physical timestamp column must carry UTC timezone metadata.
        "timestamp:ms:utc" | "timestamp" | "datetime" | "timestamp_ms" => Some(
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
        ),
        "vector" | "vector_float32" => None,
        _ => {
            return Err(anyhow!(
                "Unsupported type '{}' for column '{}'. Supported types: string, boolean, int8, int16, int32, int64, uint8, uint16, uint32, uint64, float32, float64, binary, geometry, date32, timestamp:ms:UTC, vector",
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
    let mut key: Option<&str> = None;
    for field in fields {
        validate_field_name(&field.name)?;
        let normalized_name = field.name.to_ascii_lowercase();
        if !names.insert(normalized_name) {
            return Err(anyhow!("Duplicate column name '{}'", field.name));
        }
        let data_type = field_data_type(field)?;
        let arrow_field = if field.data_type.trim().eq_ignore_ascii_case("geometry") {
            crate::geometry::geometry_field(&field.name, field.nullable)
        } else {
            Field::new(&field.name, data_type, field.nullable)
        };
        if !field.primary_key {
            arrow_fields.push(arrow_field);
            continue;
        }
        if let Some(existing) = key.replace(&field.name) {
            return Err(anyhow!(
                "A table has at most one key column, but both '{existing}' and '{}' are marked as the key",
                field.name
            ));
        }
        if let Some(reason) = primary_key_ineligibility(&arrow_field) {
            return Err(anyhow!(
                "Column '{}' cannot be the table key: {reason}",
                field.name
            ));
        }
        arrow_fields.push(with_primary_key_marker(arrow_field));
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
            primary_key: false,
        }
    }

    fn key(name: &str, data_type: &str) -> DatabaseSchemaField {
        DatabaseSchemaField {
            nullable: false,
            primary_key: true,
            ..field(name, data_type)
        }
    }

    #[test]
    fn marks_one_required_key_column() {
        let schema = database_fields_to_arrow_schema(&[
            key("ticket_id", "string"),
            field("title", "string"),
        ])
        .unwrap();

        assert_eq!(primary_key_columns(&schema), vec!["ticket_id".to_string()]);
        assert_eq!(
            schema
                .field_with_name("ticket_id")
                .unwrap()
                .metadata()
                .get(LANCE_UNENFORCED_PRIMARY_KEY_POSITION)
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn rejects_ineligible_or_repeated_key_columns() {
        let error = database_fields_to_arrow_schema(&[key("a", "string"), key("b", "int64")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("'a'") && error.contains("'b'"), "{error}");

        let optional = DatabaseSchemaField {
            nullable: true,
            ..key("id", "string")
        };
        let error = database_fields_to_arrow_schema(&[optional])
            .unwrap_err()
            .to_string();
        assert!(error.contains("nullable"), "{error}");

        let error = database_fields_to_arrow_schema(&[key("score", "float64")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Float64"), "{error}");
    }

    #[test]
    fn reads_both_key_marker_spellings() {
        let legacy = Field::new("legacy", DataType::Int64, false)
            .with_metadata([(LANCE_UNENFORCED_PRIMARY_KEY.to_string(), "Yes".to_string())].into());
        let positioned = with_primary_key_marker(Field::new("id", DataType::Utf8, false));
        let schema = Schema::new(vec![
            legacy,
            Field::new("value", DataType::Utf8, true),
            positioned,
        ]);

        assert_eq!(
            primary_key_columns(&schema),
            vec!["id".to_string(), "legacy".to_string()]
        );
    }

    #[test]
    fn converts_scalar_and_vector_fields() {
        let mut embedding = field("embedding", "vector");
        embedding.nullable = false;
        embedding.vector_size = Some(384);

        let schema = database_fields_to_arrow_schema(&[
            field("ticket_id", "string"),
            field("created_at", "timestamp:ms:UTC"),
            embedding,
        ])
        .unwrap();

        assert_eq!(schema.fields().len(), 3);
        assert_eq!(
            schema.field_with_name("created_at").unwrap().data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );
        assert_eq!(
            schema.field_with_name("embedding").unwrap().data_type(),
            &DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 384)
        );
        assert!(!schema.field_with_name("embedding").unwrap().is_nullable());
    }

    #[test]
    fn legacy_timestamp_aliases_remain_utc_compatible() {
        for alias in ["timestamp", "datetime", "timestamp_ms"] {
            let schema = database_fields_to_arrow_schema(&[field("created_at", alias)]).unwrap();
            assert_eq!(
                schema.field_with_name("created_at").unwrap().data_type(),
                &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                "legacy alias {alias} must retain UTC instant semantics"
            );
        }
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

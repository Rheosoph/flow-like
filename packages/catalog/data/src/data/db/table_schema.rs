//! The shape a table's Arrow schema takes once it has been serialised onto a pin.

use flow_like_types::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One column of a table.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TableField {
    /// Column name.
    pub name: String,
    /// Arrow data type: a bare name such as `"Utf8"` or `"Int64"` for simple
    /// types, an object for nested ones such as lists and structs.
    pub data_type: Value,
    /// Whether the column accepts nulls.
    pub nullable: bool,
    /// Dictionary id, set only on dictionary-encoded columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dict_id: Option<i64>,
    /// Whether the dictionary is ordered, set only on dictionary-encoded columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dict_is_ordered: Option<bool>,
    /// Column level key/value metadata, defined by whoever wrote the table.
    #[serde(default)]
    pub metadata: Value,
}

/// A table's schema, as written to a `schema` pin.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TableSchema {
    /// The columns, in storage order.
    pub fields: Vec<TableField>,
    /// Table level key/value metadata, defined by whoever wrote the table.
    #[serde(default)]
    pub metadata: Value,
}

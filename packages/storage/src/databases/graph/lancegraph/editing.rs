//! Governed, update-only edits of one ontology object or one join-table
//! relationship row.
//!
//! The saved ontology resolves the table, the identity and the editable
//! columns, so a caller only ever supplies identity values and property
//! values. Nothing is inserted: exactly one existing row changes, or the
//! request fails without writing.

use super::{
    EdgeMappingDef, GraphOverlayDef, NodeMappingDef, effective_node_id_column_checked,
    filter_identifier, foreign_key_row_identity, required_object_columns, resolve_edge_mapping,
    resolve_object_mapping, resolve_object_projection,
};
use crate::arrow_utils::{
    record_batch_to_value, unknown_columns, value_to_record_batch_for_schema,
};
use crate::databases::vector::lancedb::update_sql_literal;
use crate::databases::vector::schema::TableInputRejected;
use crate::geometry::is_geometry_field;
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use flow_like_types::json::Map;
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use lancedb::Connection;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::Table;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const MAX_UPDATED_COLUMNS: usize = 64;
const IDENTIFIES: &str = "identifies";
const LINKS: &str = "links";

#[derive(Debug, Clone, Deserialize)]
pub struct ObjectPropertyUpdate {
    pub object_type: String,
    pub id: Value,
    pub updates: Map<String, Value>,
    pub expected: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelationshipPropertyUpdate {
    pub relationship_type: String,
    pub source: Value,
    pub target: Value,
    pub updates: Map<String, Value>,
    pub expected: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayRowUpdateOutcome {
    Updated,
    Stale,
}

/// `row` is the saved row on `Updated`, and the current row on `Stale`, where
/// nothing was written because a sent value no longer matched `expected`.
#[derive(Debug, Clone, Serialize)]
pub struct OverlayRowUpdateResult {
    pub outcome: OverlayRowUpdateOutcome,
    pub row: Value,
}

/// The row an edit targets could not be changed as one unit. The message is
/// safe to show to the caller.
#[derive(Debug)]
pub enum OverlayRowUpdateRejected {
    NotFound(String),
    NotUnique(String),
    NotApplied(String),
    /// The write committed, but to `rows` rows instead of one: another writer
    /// added a row with the same identity while the edit was saving.
    AppliedToSeveral {
        rows: u64,
        message: String,
    },
}

impl std::fmt::Display for OverlayRowUpdateRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NotFound(message)
            | Self::NotUnique(message)
            | Self::NotApplied(message)
            | Self::AppliedToSeveral { message, .. } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for OverlayRowUpdateRejected {}

/// How many rows an edit wrote. A rejected edit can still have written rows,
/// so callers that record writes must ask this before mapping the error.
pub fn overlay_rows_written(result: &Result<OverlayRowUpdateResult>) -> u64 {
    match result {
        Ok(result) if result.outcome == OverlayRowUpdateOutcome::Updated => 1,
        Ok(_) => 0,
        Err(error) => error
            .chain()
            .find_map(|cause| cause.downcast_ref::<OverlayRowUpdateRejected>())
            .map_or(0, |rejection| match rejection {
                OverlayRowUpdateRejected::AppliedToSeveral { rows, .. } => *rows,
                _ => 0,
            }),
    }
}

struct EditTarget<'a> {
    table: &'a str,
    label: &'a str,
    noun: &'static str,
    identity: Vec<(String, Value)>,
    locked: HashMap<String, &'static str>,
    projection: Vec<String>,
    hidden: Vec<String>,
}

pub async fn update_overlay_object(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    request: ObjectPropertyUpdate,
) -> Result<OverlayRowUpdateResult> {
    let mapping = resolve_object_mapping(overlay, &request.object_type).ok_or_else(|| {
        TableInputRejected(format!(
            "Object type '{}' was not found in the ontology",
            request.object_type
        ))
    })?;
    let identity = checked_identity_column(overlay, mapping)?;
    let projection = resolve_object_projection(
        connection,
        &mapping.table,
        &mapping.property_columns,
        overlay.property_projection_mode,
        required_object_columns(mapping, &identity),
    )
    .await?;
    let locked = table_locked_columns(overlay, &mapping.table)?;

    update_projected_row(
        connection,
        EditTarget {
            table: &mapping.table,
            label: &mapping.label,
            noun: "object",
            identity: vec![(identity, request.id)],
            locked,
            projection,
            hidden: Vec::new(),
        },
        request.updates,
        request.expected,
    )
    .await
}

pub async fn update_overlay_relationship(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    request: RelationshipPropertyUpdate,
) -> Result<OverlayRowUpdateResult> {
    let edge = resolve_edge_mapping(overlay, &request.relationship_type).ok_or_else(|| {
        TableInputRejected(format!(
            "Relationship type '{}' was not found in the ontology",
            request.relationship_type
        ))
    })?;
    let locked = relationship_locked_columns(overlay, edge)?;
    let projection = resolve_object_projection(
        connection,
        &edge.table,
        &edge.property_columns,
        overlay.property_projection_mode,
        vec![edge.src_column.clone(), edge.dst_column.clone()],
    )
    .await?;

    update_projected_row(
        connection,
        EditTarget {
            table: &edge.table,
            label: &edge.label,
            noun: "relationship",
            identity: vec![
                (edge.src_column.clone(), request.source),
                (edge.dst_column.clone(), request.target),
            ],
            locked,
            projection,
            hidden: vec![edge.src_column.clone(), edge.dst_column.clone()],
        },
        request.updates,
        request.expected,
    )
    .await
}

fn checked_identity_column(overlay: &GraphOverlayDef, mapping: &NodeMappingDef) -> Result<String> {
    Ok(effective_node_id_column_checked(overlay, &mapping.label)
        .map_err(|error| TableInputRejected(error.to_string()))?
        .unwrap_or_else(|| mapping.id_column.clone()))
}

/// Columns that identify an object or store a relationship on `table`, for
/// every mapping stored there and not only the edited one. Changing any of
/// them would move some object to another identity or re-point some
/// relationship, so none of them is a plain property.
fn table_locked_columns(
    overlay: &GraphOverlayDef,
    table: &str,
) -> Result<HashMap<String, &'static str>> {
    let mut locked = HashMap::new();
    for node in overlay.nodes.iter().filter(|node| node.table == table) {
        locked.insert(checked_identity_column(overlay, node)?, IDENTIFIES);
        locked.insert(node.id_column.clone(), IDENTIFIES);
    }
    for edge in overlay.edges.iter().filter(|edge| edge.table == table) {
        locked.entry(edge.src_column.clone()).or_insert(LINKS);
        locked.entry(edge.dst_column.clone()).or_insert(LINKS);
    }
    Ok(locked)
}

/// A relationship's own row is editable only in a join table. One stored as a
/// foreign key lives on its object's row, whose identity is the object's id,
/// not the (source, target) pair.
fn relationship_locked_columns(
    overlay: &GraphOverlayDef,
    edge: &EdgeMappingDef,
) -> Result<HashMap<String, &'static str>> {
    let foreign_key = foreign_key_row_identity(
        &overlay.nodes,
        &overlay.edges,
        &edge.table,
        &edge.src_column,
        &edge.dst_column,
    )
    .map_err(|error| TableInputRejected(error.to_string()))?;
    if foreign_key.is_some() {
        return Err(TableInputRejected(format!(
            "Relationship '{}' is stored on its object's row; edit that object instead",
            edge.label
        ))
        .into());
    }
    table_locked_columns(overlay, &edge.table)
}

fn is_nested(field: &Field) -> bool {
    matches!(
        field.data_type(),
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(_, _)
            | DataType::Struct(_)
            | DataType::Map(_, _)
    ) && !is_geometry_field(field)
}

async fn update_projected_row(
    connection: &Connection,
    target: EditTarget<'_>,
    updates: Map<String, Value>,
    expected: Map<String, Value>,
) -> Result<OverlayRowUpdateResult> {
    validate_request(&target, &updates, &expected)?;

    let table = connection
        .open_table(target.table)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open table '{}': {}", target.table, error))?;
    let schema = table
        .schema()
        .await
        .map_err(|error| anyhow!("Failed to read schema of '{}': {}", target.table, error))?;
    let expressions = update_expressions(target.table, &schema, &updates)?;
    let predicate = identity_predicate(&target, &schema)?;
    let current = read_unique_row(&table, &target, &predicate, &updates).await?;

    if is_stale(target.table, &schema, &updates, expected, &current)? {
        let row = read_projected_row(&table, &target, &predicate)
            .await?
            .ok_or_else(|| not_found(&target))?;
        return Ok(OverlayRowUpdateResult {
            outcome: OverlayRowUpdateOutcome::Stale,
            row,
        });
    }

    apply_update(&table, &target, &predicate, expressions).await?;
    let row = match read_projected_row(&table, &target, &predicate).await {
        Ok(Some(row)) => row,
        Ok(None) => saved_row_without_read_back(&target, &schema, current, updates),
        Err(error) => {
            tracing::warn!(
                %error,
                table = target.table,
                "Saved ontology row could not be read back"
            );
            saved_row_without_read_back(&target, &schema, current, updates)
        }
    };
    Ok(OverlayRowUpdateResult {
        outcome: OverlayRowUpdateOutcome::Updated,
        row,
    })
}

/// The committed values when the saved row can't be read back, because
/// another writer removed it right after this edit committed. Reporting that
/// as a failure would hide a write that happened.
fn saved_row_without_read_back(
    target: &EditTarget<'_>,
    schema: &Schema,
    mut current: Map<String, Value>,
    updates: Map<String, Value>,
) -> Value {
    let saved = canonical_values(target.table, schema, updates.clone()).unwrap_or(updates);
    current.extend(saved);
    for column in &target.hidden {
        current.remove(column);
    }
    Value::Object(current)
}

/// Reads the identity and the columns about to change, requiring exactly one
/// matching row so a missing or duplicated identity never gets written.
async fn read_unique_row(
    table: &Table,
    target: &EditTarget<'_>,
    predicate: &str,
    updates: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut columns = target
        .identity
        .iter()
        .map(|(column, _)| column.clone())
        .collect::<Vec<_>>();
    for column in updates.keys() {
        if !columns.contains(column) {
            columns.push(column.clone());
        }
    }
    let mut rows = query_rows(table, target.table, predicate, columns, 2).await?;
    if rows.len() > 1 {
        return Err(OverlayRowUpdateRejected::NotUnique(
            "Several rows share this identity, so nothing was changed".to_string(),
        )
        .into());
    }
    match rows.pop() {
        Some(Value::Object(row)) => Ok(row),
        _ => Err(not_found(target).into()),
    }
}

async fn apply_update(
    table: &Table,
    target: &EditTarget<'_>,
    predicate: &str,
    expressions: Vec<(String, String)>,
) -> Result<()> {
    let mut operation = table.update().only_if(predicate);
    for (column, literal) in expressions {
        operation = operation.column(column, literal);
    }
    let result = operation.execute().await.map_err(|error| {
        anyhow!(
            "Failed to update '{}' in table '{}': {}",
            target.label,
            target.table,
            error
        )
    })?;
    match result.rows_updated {
        1 => Ok(()),
        0 => Err(OverlayRowUpdateRejected::NotApplied(
            "The row changed while saving; refresh and try again".to_string(),
        )
        .into()),
        rows => Err(OverlayRowUpdateRejected::AppliedToSeveral {
            rows,
            message: format!(
                "Another change added a row with the same identity while saving, so this \
                 change was saved to {rows} rows; refresh and check them"
            ),
        }
        .into()),
    }
}

fn validate_request(
    target: &EditTarget<'_>,
    updates: &Map<String, Value>,
    expected: &Map<String, Value>,
) -> Result<()> {
    if updates.is_empty() {
        return Err(TableInputRejected("No property changes to save".to_string()).into());
    }
    if updates.len() > MAX_UPDATED_COLUMNS {
        return Err(TableInputRejected(format!(
            "At most {MAX_UPDATED_COLUMNS} properties can be saved at once"
        ))
        .into());
    }
    let projected = target
        .projection
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for column in updates.keys() {
        if !expected.contains_key(column) {
            return Err(TableInputRejected(format!(
                "'{column}' has no expected value, so a concurrent change could not be detected"
            ))
            .into());
        }
        if let Some(reason) = target.locked.get(column) {
            return Err(TableInputRejected(locked_message(column, reason, target.noun)).into());
        }
        if !projected.contains(column.as_str()) {
            return Err(TableInputRejected(format!(
                "'{column}' is not a property of '{}'",
                target.label
            ))
            .into());
        }
    }
    Ok(())
}

fn locked_message(column: &str, reason: &str, noun: &str) -> String {
    if reason == LINKS {
        format!("'{column}' links this {noun} to another object and can't be edited")
    } else {
        format!("'{column}' identifies this {noun} and can't be edited")
    }
}

fn update_expressions(
    table: &str,
    schema: &Schema,
    updates: &Map<String, Value>,
) -> Result<Vec<(String, String)>> {
    let mut expressions = Vec::with_capacity(updates.len());
    for (column, value) in updates {
        let Ok(field) = schema.field_with_name(column) else {
            return Err(unknown_columns(table, &[column.as_str()], schema).into());
        };
        if is_nested(field) {
            return Err(TableInputRejected(format!(
                "Editing nested values in '{column}' is not supported"
            ))
            .into());
        }
        if value.is_null() && !field.is_nullable() {
            return Err(TableInputRejected(format!("'{column}' can't be empty")).into());
        }
        expressions.push((column.clone(), update_sql_literal(table, field, value)?));
    }
    Ok(expressions)
}

fn identity_predicate(target: &EditTarget<'_>, schema: &Schema) -> Result<String> {
    let clauses = target
        .identity
        .iter()
        .map(|(column, value)| -> Result<String> {
            let field = schema.field_with_name(column).map_err(|_| {
                TableInputRejected(format!(
                    "Identity column '{column}' does not exist in table '{}'",
                    target.table
                ))
            })?;
            Ok(format!(
                "{} = {}",
                filter_identifier(column),
                identity_sql_literal(field, value)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(clauses.join(" AND "))
}

/// Types an identity value for its column. A bare integer never matches a
/// timestamp or date column in a Lance filter, so those are cast explicitly.
fn identity_sql_literal(field: &Field, value: &Value) -> Result<String> {
    let data_type = field.data_type();
    let literal = match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            text_identity(value).map(|text| format!("'{}'", text.replace('\'', "''")))
        }
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
            signed_identity(value).map(|number| number.to_string())
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            unsigned_identity(value).map(|number| number.to_string())
        }
        DataType::Float32 | DataType::Float64 => {
            float_identity(value).map(|number| number.to_string())
        }
        DataType::Boolean => {
            bool_identity(value).map(|flag| String::from(if flag { "TRUE" } else { "FALSE" }))
        }
        DataType::Timestamp(unit, _) => signed_identity(value).map(|number| {
            format!(
                "CAST({number} AS TIMESTAMP({}))",
                timestamp_precision(*unit)
            )
        }),
        DataType::Date32 => signed_identity(value).map(|days| format!("CAST({days} AS DATE)")),
        other => {
            return Err(TableInputRejected(format!(
                "Objects identified by a {other} column can't be edited"
            ))
            .into());
        }
    };
    literal.ok_or_else(|| {
        TableInputRejected(format!(
            "Identity column '{}' is {data_type}; cannot match {}",
            field.name(),
            describe_value(value)
        ))
        .into()
    })
}

fn timestamp_precision(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Second => 0,
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
    }
}

fn text_identity(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    };
    text.filter(|text| !text.is_empty())
}

fn float_identity(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    };
    number.filter(|number| number.is_finite())
}

fn bool_identity(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn signed_identity(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn unsigned_identity(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::String(text) => format!("'{text}'"),
        other => other.to_string(),
    }
}

fn not_found(target: &EditTarget<'_>) -> OverlayRowUpdateRejected {
    let identity = target
        .identity
        .iter()
        .map(|(_, value)| describe_value(value))
        .collect::<Vec<_>>()
        .join(" -> ");
    OverlayRowUpdateRejected::NotFound(format!(
        "'{}' {identity} was not found; it may have been deleted",
        target.label
    ))
}

/// Compares the sent `expected` values with the stored ones after both went
/// through the column's type, so an ISO instant equals the stored epoch and a
/// Float32 equals its widened read-back.
fn is_stale(
    table: &str,
    schema: &Schema,
    updates: &Map<String, Value>,
    expected: Map<String, Value>,
    current: &Map<String, Value>,
) -> Result<bool> {
    let expected = expected
        .into_iter()
        .filter(|(column, _)| updates.contains_key(column))
        .collect::<Map<String, Value>>();
    let canonical = canonical_values(table, schema, expected)?;
    let null = Value::Null;
    Ok(updates.keys().any(|column| {
        canonical.get(column).unwrap_or(&null) != current.get(column).unwrap_or(&null)
    }))
}

/// `values` as they read back after being stored in their columns.
fn canonical_values(
    table: &str,
    schema: &Schema,
    values: Map<String, Value>,
) -> Result<Map<String, Value>> {
    let batch = value_to_record_batch_for_schema(vec![Value::Object(values)], schema, table)?;
    Ok(match record_batch_to_value(&batch)?.pop() {
        Some(Value::Object(row)) => row,
        _ => Map::new(),
    })
}

async fn query_rows(
    table: &Table,
    table_name: &str,
    predicate: &str,
    columns: Vec<String>,
    limit: usize,
) -> Result<Vec<Value>> {
    let batches = table
        .query()
        .only_if(predicate)
        .select(Select::Columns(columns))
        .limit(limit)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to read from '{}': {}", table_name, error))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|error| anyhow!("Failed to collect from '{}': {}", table_name, error))?;
    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(record_batch_to_value(batch)?);
    }
    Ok(rows)
}

async fn read_projected_row(
    table: &Table,
    target: &EditTarget<'_>,
    predicate: &str,
) -> Result<Option<Value>> {
    let mut rows = query_rows(table, target.table, predicate, target.projection.clone(), 1).await?;
    let Some(Value::Object(mut row)) = rows.pop() else {
        return Ok(None);
    };
    for column in &target.hidden {
        row.remove(column);
    }
    Ok(Some(Value::Object(row)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{RecordBatch, StringArray};
    use flow_like_types::json::json;
    use std::sync::Arc;

    fn json_map(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn person_target(id: &str) -> EditTarget<'static> {
        EditTarget {
            table: "people",
            label: "Person",
            noun: "object",
            identity: vec![("id".to_string(), json!(id))],
            locked: HashMap::new(),
            projection: vec!["id".to_string(), "name".to_string()],
            hidden: Vec::new(),
        }
    }

    async fn rename_person(table: &Table, id: &str) -> Result<OverlayRowUpdateResult> {
        let target = person_target(id);
        let schema = table.schema().await?;
        let predicate = identity_predicate(&target, &schema)?;
        let updates = json_map(json!({ "name": "Changed" }));
        let expressions = update_expressions(target.table, &schema, &updates)?;
        apply_update(table, &target, &predicate, expressions).await?;
        Ok(OverlayRowUpdateResult {
            outcome: OverlayRowUpdateOutcome::Updated,
            row: Value::Null,
        })
    }

    #[tokio::test]
    async fn apply_update_reports_rows_written_when_it_reached_several() -> Result<()> {
        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path)?;
        let connection = lancedb::connect(&test_path).execute().await?;
        let people = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("name", DataType::Utf8, true),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["p1", "p1", "p2"])),
                Arc::new(StringArray::from(vec![
                    Some("First"),
                    Some("Second"),
                    Some("Other"),
                ])),
            ],
        )?;
        connection
            .create_table("people", vec![people])
            .execute()
            .await?;
        let table = connection.open_table("people").execute().await?;

        let several = rename_person(&table, "p1").await;
        let missing = rename_person(&table, "p9").await;
        let changed = query_rows(
            &table,
            "people",
            "name = 'Changed'",
            vec!["id".to_string()],
            10,
        )
        .await?;
        std::fs::remove_dir_all(&test_path).ok();

        assert_eq!(overlay_rows_written(&several), 2);
        let several = several.expect_err("a write that reached two rows is not a clean update");
        assert!(
            matches!(
                several.downcast_ref::<OverlayRowUpdateRejected>(),
                Some(OverlayRowUpdateRejected::AppliedToSeveral { rows: 2, .. })
            ),
            "{several:#}"
        );
        assert_eq!(changed.len(), 2, "the committed write must stay visible");
        assert_eq!(overlay_rows_written(&missing), 0);
        let missing = missing.expect_err("a write that reached no row is not applied");
        assert!(
            matches!(
                missing.downcast_ref::<OverlayRowUpdateRejected>(),
                Some(OverlayRowUpdateRejected::NotApplied(_))
            ),
            "{missing:#}"
        );
        Ok(())
    }

    #[test]
    fn overlay_rows_written_counts_only_committed_writes() {
        let result = |outcome| {
            Ok(OverlayRowUpdateResult {
                outcome,
                row: Value::Null,
            })
        };
        assert_eq!(
            overlay_rows_written(&result(OverlayRowUpdateOutcome::Updated)),
            1
        );
        assert_eq!(
            overlay_rows_written(&result(OverlayRowUpdateOutcome::Stale)),
            0
        );
        let not_unique: Result<OverlayRowUpdateResult> =
            Err(OverlayRowUpdateRejected::NotUnique("ambiguous".to_string()).into());
        assert_eq!(overlay_rows_written(&not_unique), 0);
    }

    #[test]
    fn saved_row_without_read_back_carries_the_committed_values() {
        let schema = Schema::new(vec![
            Field::new("source", DataType::Utf8, false),
            Field::new("target", DataType::Utf8, false),
            Field::new(
                "seen_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                true,
            ),
            Field::new("note", DataType::Utf8, true),
        ]);
        let target = EditTarget {
            table: "knows",
            label: "KNOWS",
            noun: "relationship",
            identity: vec![
                ("source".to_string(), json!("p1")),
                ("target".to_string(), json!("p2")),
            ],
            locked: HashMap::new(),
            projection: vec!["seen_at".to_string(), "note".to_string()],
            hidden: vec!["source".to_string(), "target".to_string()],
        };
        let current = json_map(json!({
            "source": "p1",
            "target": "p2",
            "seen_at": 1_786_968_000_000_i64,
            "note": "met",
        }));
        let updates = json_map(json!({
            "seen_at": "2026-08-16T12:00:00.000Z",
            "note": "colleagues",
        }));

        let row = saved_row_without_read_back(&target, &schema, current, updates);

        assert_eq!(
            row,
            json!({ "seen_at": 1_786_881_600_000_i64, "note": "colleagues" })
        );
    }
}

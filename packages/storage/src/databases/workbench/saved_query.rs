//! Persistence for Data Studio saved queries and views. Logical definitions
//! live in a revisioned manifest row in the reserved
//! `__saved_queries_manifest__` table, allowing cross-process compare-and-swap
//! mutations. Databases using the original one-row-per-query representation in
//! `__saved_queries__` are migrated lazily as a best-effort convenience for
//! pre-release development data. Mixed-version writers are not supported: all
//! processes sharing a database must use the manifest format.

use crate::arrow_utils::record_batch_to_value;
use flow_like_types::{Result, Value, anyhow, create_id};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, Table};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const SAVED_QUERIES_TABLE: &str = "__saved_queries__";
const SAVED_QUERIES_MANIFEST_TABLE: &str = "__saved_queries_manifest__";
const SAVED_QUERIES_MANIFEST_ID: &str = "__saved_queries_manifest__";
const MANIFEST_WRITE_RETRIES: usize = 32;

/// A stored query or view. `kind` distinguishes a runnable (optionally
/// parametrized) query from a composable view usable as a named virtual table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQueryDef {
    pub id: String,
    pub app_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: SavedQueryKind,
    pub surface: SavedQuerySurface,
    #[serde(default)]
    pub overlay_id: Option<String>,
    pub sql: String,
    /// Opaque JSON-Schema-shaped parameter definition owned by the UI. The
    /// backend never interprets it; parameter values are bound from the execute
    /// payload at run time.
    #[serde(default)]
    pub param_schema: Option<Value>,
    /// Opaque chart/graph configuration owned by the UI.
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQueryKind {
    Query,
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQuerySurface {
    Native,
    Overlay,
}

/// Returns an RFC 3339 update timestamp that is strictly later than the
/// previously persisted timestamp, even if the wall clock stalls or moves
/// backwards. Invalid legacy timestamps fall back to the current time.
pub fn next_updated_at(previous: &str) -> String {
    let now = chrono::Utc::now();
    let after_previous = chrono::DateTime::parse_from_rfc3339(previous)
        .ok()
        .and_then(|previous| {
            previous
                .with_timezone(&chrono::Utc)
                .checked_add_signed(chrono::Duration::nanoseconds(1))
        });
    let next = match after_previous {
        Some(after_previous) if after_previous > now => after_previous,
        _ => now,
    };
    next.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// Result of an atomic saved-query manifest mutation. View-name conflicts are
/// distinct from optimistic revision conflicts so HTTP and Tauri surfaces can
/// return the appropriate user-facing error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedQuerySaveResult {
    Saved,
    RevisionConflict,
    ViewNameConflict { conflicting_query_id: String },
    ViewLimitExceeded { limit: usize },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SavedQueryManifest {
    #[serde(default)]
    queries: Vec<SavedQueryDef>,
}

async fn collect_rows(table: &Table, filter: Option<String>) -> Result<Vec<Value>> {
    let mut query = table.query();
    if let Some(filter) = filter {
        query = query.only_if(filter);
    }
    let result = query
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to query saved queries: {}", error))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|error| anyhow!("Failed to collect saved queries: {}", error))?;
    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(record_batch_to_value(batch)?);
    }
    Ok(rows)
}

async fn open_latest_table(connection: &Connection, table_name: &str) -> Result<Table> {
    let table = connection
        .open_table(table_name)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open table '{}': {}", table_name, error))?;
    // Connections default to manual read consistency. A newly opened table can
    // therefore still observe the connection session's cached manifest unless
    // it is explicitly refreshed, which would defeat the outer CAS retry.
    table
        .checkout_latest()
        .await
        .map_err(|error| anyhow!("Failed to refresh table '{}': {}", table_name, error))?;
    Ok(table)
}

async fn load_manifest_from_table(table: &Table) -> Result<Option<(SavedQueryManifest, String)>> {
    let filter = format!("id = '{SAVED_QUERIES_MANIFEST_ID}'");
    let rows = collect_rows(table, Some(filter)).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let definition = row
        .get("definition_json")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Saved query manifest has no definition_json"))?;
    let revision = row
        .get("updated_at")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Saved query manifest has no revision"))?;
    let manifest = serde_json::from_str(definition)
        .map_err(|error| anyhow!("Failed to parse saved query manifest: {}", error))?;
    Ok(Some((manifest, revision.to_string())))
}

async fn load_legacy_saved_queries(table: &Table) -> Result<Vec<SavedQueryDef>> {
    let rows = collect_rows(table, None).await?;
    let mut queries: Vec<SavedQueryDef> = Vec::new();
    for row in rows {
        let query_id = row.get("id").and_then(Value::as_str).unwrap_or("<unknown>");
        if query_id == SAVED_QUERIES_MANIFEST_ID {
            continue;
        }
        if let Some(definition) = row.get("definition_json").and_then(Value::as_str) {
            match serde_json::from_str(definition) {
                Ok(query) => queries.push(query),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        query_id,
                        "Skipping saved query with unparseable definition"
                    );
                }
            }
        }
    }
    queries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(queries)
}

pub async fn list_saved_queries(connection: &Connection) -> Result<Vec<SavedQueryDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    let mut queries = if table_names
        .iter()
        .any(|name| name == SAVED_QUERIES_MANIFEST_TABLE)
    {
        let table = open_latest_table(connection, SAVED_QUERIES_MANIFEST_TABLE).await?;
        load_manifest_from_table(&table)
            .await?
            .ok_or_else(|| anyhow!("Saved query manifest table has no manifest row"))?
            .0
            .queries
    } else if table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        let table = open_latest_table(connection, SAVED_QUERIES_TABLE).await?;
        match load_manifest_from_table(&table).await? {
            Some((manifest, _)) => manifest.queries,
            None => load_legacy_saved_queries(&table).await?,
        }
    } else {
        Vec::new()
    };
    queries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(queries)
}

pub async fn find_saved_query(
    connection: &Connection,
    query_id: &str,
) -> Result<Option<SavedQueryDef>> {
    Ok(list_saved_queries(connection)
        .await?
        .into_iter()
        .find(|query| query.id == query_id))
}

pub async fn load_saved_query(connection: &Connection, query_id: &str) -> Result<SavedQueryDef> {
    find_saved_query(connection, query_id)
        .await?
        .ok_or_else(|| anyhow!("Saved query '{}' not found", query_id))
}

fn saved_queries_schema() -> Arc<arrow::datatypes::Schema> {
    use arrow::datatypes::{DataType, Field, Schema};

    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("app_id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("surface", DataType::Utf8, false),
        Field::new("definition_json", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ]))
}

fn manifest_batch(
    manifest: &SavedQueryManifest,
    revision: &str,
) -> Result<(Arc<arrow::datatypes::Schema>, arrow::array::RecordBatch)> {
    use arrow::array::{RecordBatch, StringArray};

    let schema = saved_queries_schema();
    let definition_json = serde_json::to_string(manifest)
        .map_err(|error| anyhow!("Failed to serialize saved query manifest: {}", error))?;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![SAVED_QUERIES_MANIFEST_ID])),
            Arc::new(StringArray::from(vec![""])),
            Arc::new(StringArray::from(vec!["Saved query manifest"])),
            Arc::new(StringArray::from(vec!["manifest"])),
            Arc::new(StringArray::from(vec!["manifest"])),
            Arc::new(StringArray::from(vec![definition_json.as_str()])),
            Arc::new(StringArray::from(vec![revision])),
        ],
    )?;
    Ok((schema, batch))
}

async fn open_after_create_race(connection: &Connection, create_error: &str) -> Result<Table> {
    let mut last_open_error = None;
    for attempt in 0..8_u32 {
        match open_latest_table(connection, SAVED_QUERIES_MANIFEST_TABLE).await {
            Ok(table) => return Ok(table),
            Err(error) => last_open_error = Some(error.to_string()),
        }
        let delay_ms = 5_u64 << attempt.min(5);
        flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    Err(anyhow!(
        "Failed to initialize saved query manifest ({create_error}); concurrent open also failed ({})",
        last_open_error.unwrap_or_else(|| "unknown open error".to_string())
    ))
}

async fn load_or_initialize_manifest(
    connection: &Connection,
) -> Result<(Table, SavedQueryManifest, String)> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if table_names
        .iter()
        .any(|name| name == SAVED_QUERIES_MANIFEST_TABLE)
    {
        let table = open_latest_table(connection, SAVED_QUERIES_MANIFEST_TABLE).await?;
        let (manifest, revision) = load_manifest_from_table(&table)
            .await?
            .ok_or_else(|| anyhow!("Saved query manifest table has no manifest row"))?;
        return Ok((table, manifest, revision));
    }

    // This importer preserves pre-release development data. It is deliberately
    // not a rolling-upgrade bridge: an older process can continue writing the
    // legacy table after this snapshot, so mixed-version writers are unsupported.
    let manifest = if table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        let legacy_table = open_latest_table(connection, SAVED_QUERIES_TABLE).await?;
        match load_manifest_from_table(&legacy_table).await? {
            // Compatibility with the short-lived inline-manifest format.
            Some((manifest, _)) => manifest,
            None => SavedQueryManifest {
                queries: load_legacy_saved_queries(&legacy_table).await?,
            },
        }
    } else {
        SavedQueryManifest::default()
    };

    let revision = create_id();
    let (_, batch) = manifest_batch(&manifest, &revision)?;
    match connection
        .create_table(SAVED_QUERIES_MANIFEST_TABLE, vec![batch])
        .execute()
        .await
    {
        Ok(table) => Ok((table, manifest, revision)),
        Err(create_error) => {
            // Table creation is the migration's cross-process election: only
            // one writer can create the reserved table. Every loser reopens
            // the winner's complete seed rather than inserting a duplicate key.
            let table = open_after_create_race(connection, &create_error.to_string()).await?;
            let (manifest, revision) = load_manifest_from_table(&table)
                .await?
                .ok_or_else(|| anyhow!("Saved query manifest initialization did not commit"))?;
            Ok((table, manifest, revision))
        }
    }
}

fn view_name_conflict<'a>(
    queries: &'a [SavedQueryDef],
    candidate: &SavedQueryDef,
) -> Option<&'a SavedQueryDef> {
    if candidate.kind != SavedQueryKind::View {
        return None;
    }
    let normalized_name = candidate.name.to_lowercase();
    queries.iter().find(|query| {
        query.id != candidate.id
            && query.kind == SavedQueryKind::View
            && query.surface == candidate.surface
            && (candidate.surface != SavedQuerySurface::Overlay
                || query.overlay_id == candidate.overlay_id)
            && query.name.to_lowercase() == normalized_name
    })
}

fn same_view_scope(left: &SavedQueryDef, right: &SavedQueryDef) -> bool {
    left.surface == right.surface
        && (left.surface != SavedQuerySurface::Overlay || left.overlay_id == right.overlay_id)
}

fn view_scope_count(queries: &[SavedQueryDef], candidate: &SavedQueryDef) -> usize {
    if candidate.kind != SavedQueryKind::View {
        return 0;
    }
    queries
        .iter()
        .filter(|query| query.kind == SavedQueryKind::View && same_view_scope(query, candidate))
        .count()
}

fn view_limit_exceeded(queries: &[SavedQueryDef], candidate: &SavedQueryDef) -> bool {
    if candidate.kind != SavedQueryKind::View {
        return false;
    }
    let already_in_scope = queries.iter().any(|query| {
        query.id == candidate.id
            && query.kind == SavedQueryKind::View
            && same_view_scope(query, candidate)
    });
    !already_in_scope && view_scope_count(queries, candidate) >= super::MAX_WORKBENCH_VIEWS
}

async fn save_manifest_if_revision(
    table: &Table,
    manifest: &SavedQueryManifest,
    expected_revision: &str,
) -> Result<bool> {
    let next_revision = create_id();
    let (schema, batch) = manifest_batch(manifest, &next_revision)?;
    let expected = expected_revision.replace('\'', "''");
    let mut merger = table.merge_insert(&["id"]);
    merger.when_matched_update_all(Some(format!("target.updated_at = '{expected}'")));
    let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
        arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
    );
    let result = merger
        .execute(reader)
        .await
        .map_err(|error| anyhow!("Failed to update saved query manifest: {}", error))?;
    Ok(result.num_updated_rows == 1)
}

/// Atomically creates a saved query while enforcing case-insensitive view-name
/// uniqueness across all writers sharing this database.
pub async fn save_saved_query(
    connection: &Connection,
    query: &SavedQueryDef,
) -> Result<SavedQuerySaveResult> {
    for _ in 0..MANIFEST_WRITE_RETRIES {
        let (table, mut manifest, revision) = load_or_initialize_manifest(connection).await?;
        if let Some(conflict) = view_name_conflict(&manifest.queries, query) {
            return Ok(SavedQuerySaveResult::ViewNameConflict {
                conflicting_query_id: conflict.id.clone(),
            });
        }
        if view_limit_exceeded(&manifest.queries, query) {
            return Ok(SavedQuerySaveResult::ViewLimitExceeded {
                limit: super::MAX_WORKBENCH_VIEWS,
            });
        }
        if manifest
            .queries
            .iter()
            .any(|existing| existing.id == query.id)
        {
            return Ok(SavedQuerySaveResult::RevisionConflict);
        }
        manifest.queries.push(query.clone());
        if save_manifest_if_revision(&table, &manifest, &revision).await? {
            return Ok(SavedQuerySaveResult::Saved);
        }
    }
    Err(anyhow!(
        "Saved query write could not commit after {} concurrent retries",
        MANIFEST_WRITE_RETRIES
    ))
}

/// Atomically updates an existing saved query only when its persisted query
/// revision still matches the one the caller loaded. The independent manifest
/// CAS serializes name allocation with unrelated creates and renames.
pub async fn save_saved_query_if_unchanged(
    connection: &Connection,
    query: &SavedQueryDef,
    expected_updated_at: &str,
) -> Result<SavedQuerySaveResult> {
    for _ in 0..MANIFEST_WRITE_RETRIES {
        let (table, mut manifest, revision) = load_or_initialize_manifest(connection).await?;
        let Some(index) = manifest
            .queries
            .iter()
            .position(|existing| existing.id == query.id)
        else {
            return Ok(SavedQuerySaveResult::RevisionConflict);
        };
        if manifest.queries[index].updated_at != expected_updated_at {
            return Ok(SavedQuerySaveResult::RevisionConflict);
        }
        if let Some(conflict) = view_name_conflict(&manifest.queries, query) {
            return Ok(SavedQuerySaveResult::ViewNameConflict {
                conflicting_query_id: conflict.id.clone(),
            });
        }
        if view_limit_exceeded(&manifest.queries, query) {
            return Ok(SavedQuerySaveResult::ViewLimitExceeded {
                limit: super::MAX_WORKBENCH_VIEWS,
            });
        }
        manifest.queries[index] = query.clone();
        if save_manifest_if_revision(&table, &manifest, &revision).await? {
            return Ok(SavedQuerySaveResult::Saved);
        }
    }
    Err(anyhow!(
        "Saved query update could not commit after {} concurrent retries",
        MANIFEST_WRITE_RETRIES
    ))
}

pub async fn delete_saved_query(connection: &Connection, query_id: &str) -> Result<()> {
    let table_names = connection.table_names().execute().await?;
    if !table_names
        .iter()
        .any(|name| name == SAVED_QUERIES_TABLE || name == SAVED_QUERIES_MANIFEST_TABLE)
    {
        return Ok(());
    }
    for _ in 0..MANIFEST_WRITE_RETRIES {
        let (table, mut manifest, revision) = load_or_initialize_manifest(connection).await?;
        let old_len = manifest.queries.len();
        manifest.queries.retain(|query| query.id != query_id);
        if manifest.queries.len() == old_len {
            return Ok(());
        }
        if save_manifest_if_revision(&table, &manifest, &revision).await? {
            return Ok(());
        }
    }
    Err(anyhow!(
        "Saved query delete could not commit after {} concurrent retries",
        MANIFEST_WRITE_RETRIES
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    fn saved_query(id: &str, name: &str, kind: SavedQueryKind) -> SavedQueryDef {
        SavedQueryDef {
            id: id.to_string(),
            app_id: "app".to_string(),
            name: name.to_string(),
            description: None,
            kind,
            surface: SavedQuerySurface::Native,
            overlay_id: None,
            sql: "SELECT 1".to_string(),
            param_schema: None,
            viz_config: None,
            default_limit: None,
            created_at: format!("created-{id}"),
            updated_at: format!("updated-{id}"),
        }
    }

    fn assert_one_saved_one_name_conflict(left: SavedQuerySaveResult, right: SavedQuerySaveResult) {
        assert!(
            matches!(
                (&left, &right),
                (
                    SavedQuerySaveResult::Saved,
                    SavedQuerySaveResult::ViewNameConflict { .. }
                ) | (
                    SavedQuerySaveResult::ViewNameConflict { .. },
                    SavedQuerySaveResult::Saved
                )
            ),
            "expected one saved result and one name conflict, got {left:?} and {right:?}"
        );
    }

    fn assert_one_saved_one_limit_conflict(
        left: SavedQuerySaveResult,
        right: SavedQuerySaveResult,
    ) {
        assert!(
            matches!(
                (&left, &right),
                (
                    SavedQuerySaveResult::Saved,
                    SavedQuerySaveResult::ViewLimitExceeded { .. }
                ) | (
                    SavedQuerySaveResult::ViewLimitExceeded { .. },
                    SavedQuerySaveResult::Saved
                )
            ),
            "expected one saved result and one view-limit conflict, got {left:?} and {right:?}"
        );
    }

    async fn test_connections() -> Result<(String, Connection, Connection)> {
        let path = format!("./tmp/saved-query-manifest-{}", create_id());
        std::fs::create_dir_all(&path)?;
        let left = lancedb::connect(&path).execute().await?;
        let right = lancedb::connect(&path).execute().await?;
        Ok((path, left, right))
    }

    fn legacy_batch(query: &SavedQueryDef) -> Result<arrow::array::RecordBatch> {
        use arrow::array::{RecordBatch, StringArray};

        let definition_json = serde_json::to_string(query)?;
        RecordBatch::try_new(
            saved_queries_schema(),
            vec![
                Arc::new(StringArray::from(vec![query.id.as_str()])),
                Arc::new(StringArray::from(vec![query.app_id.as_str()])),
                Arc::new(StringArray::from(vec![query.name.as_str()])),
                Arc::new(StringArray::from(vec![match query.kind {
                    SavedQueryKind::Query => "query",
                    SavedQueryKind::View => "view",
                }])),
                Arc::new(StringArray::from(vec![match query.surface {
                    SavedQuerySurface::Native => "native",
                    SavedQuerySurface::Overlay => "overlay",
                }])),
                Arc::new(StringArray::from(vec![definition_json.as_str()])),
                Arc::new(StringArray::from(vec![query.updated_at.as_str()])),
            ],
        )
        .map_err(Into::into)
    }

    #[test]
    fn update_timestamp_is_strictly_monotonic_when_clock_is_behind() {
        let previous = chrono::DateTime::parse_from_rfc3339("2099-01-01T00:00:00.000000000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let next = chrono::DateTime::parse_from_rfc3339(&next_updated_at(
            &previous.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        ))
        .unwrap()
        .with_timezone(&chrono::Utc);
        assert_eq!(next, previous + chrono::Duration::nanoseconds(1));
    }

    #[tokio::test]
    async fn create_does_not_replace_an_existing_query_id() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let original = saved_query("same-id", "Original", SavedQueryKind::Query);
        assert_eq!(
            save_saved_query(&left, &original).await?,
            SavedQuerySaveResult::Saved
        );

        let replacement = saved_query("same-id", "Replacement", SavedQueryKind::Query);
        assert_eq!(
            save_saved_query(&right, &replacement).await?,
            SavedQuerySaveResult::RevisionConflict
        );
        assert_eq!(load_saved_query(&left, "same-id").await?.name, "Original");

        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_creates_claim_case_insensitive_view_name_once() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let first = saved_query("first", "Revenue", SavedQueryKind::View);
        let second = saved_query("second", "rEvEnUe", SavedQueryKind::View);

        let (first_result, second_result) = tokio::join!(
            save_saved_query(&left, &first),
            save_saved_query(&right, &second)
        );
        assert_one_saved_one_name_conflict(first_result?, second_result?);

        let saved = list_saved_queries(&left).await?;
        assert_eq!(
            saved
                .iter()
                .filter(|query| query.name.to_lowercase() == "revenue")
                .count(),
            1
        );
        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_create_and_rename_serialize_name_claim() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let original = saved_query("original", "Original", SavedQueryKind::View);
        assert_eq!(
            save_saved_query(&left, &original).await?,
            SavedQuerySaveResult::Saved
        );

        let created = saved_query("created", "Shared", SavedQueryKind::View);
        let mut renamed = original.clone();
        renamed.name = "sHaReD".to_string();
        renamed.updated_at = "renamed".to_string();
        let (create_result, rename_result) = tokio::join!(
            save_saved_query(&right, &created),
            save_saved_query_if_unchanged(&left, &renamed, &original.updated_at)
        );
        assert_one_saved_one_name_conflict(create_result?, rename_result?);

        let saved = list_saved_queries(&left).await?;
        assert_eq!(
            saved
                .iter()
                .filter(|query| query.name.to_lowercase() == "shared")
                .count(),
            1
        );
        assert_eq!(
            saved.iter().filter(|query| query.id == "original").count(),
            1
        );
        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_renames_claim_view_name_once() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let first = saved_query("first", "First", SavedQueryKind::View);
        let second = saved_query("second", "Second", SavedQueryKind::View);
        assert_eq!(
            save_saved_query(&left, &first).await?,
            SavedQuerySaveResult::Saved
        );
        assert_eq!(
            save_saved_query(&left, &second).await?,
            SavedQuerySaveResult::Saved
        );

        let mut first_renamed = first.clone();
        first_renamed.name = "Target".to_string();
        first_renamed.updated_at = "first-renamed".to_string();
        let mut second_renamed = second.clone();
        second_renamed.name = "tArGeT".to_string();
        second_renamed.updated_at = "second-renamed".to_string();
        let (first_result, second_result) = tokio::join!(
            save_saved_query_if_unchanged(&left, &first_renamed, &first.updated_at),
            save_saved_query_if_unchanged(&right, &second_renamed, &second.updated_at)
        );
        assert_one_saved_one_name_conflict(first_result?, second_result?);

        let saved = list_saved_queries(&left).await?;
        assert_eq!(
            saved
                .iter()
                .filter(|query| query.name.to_lowercase() == "target")
                .count(),
            1
        );
        assert_eq!(saved.len(), 2);
        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_legacy_migration_preserves_legacy_and_new_queries() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let legacy = saved_query("legacy", "Legacy", SavedQueryKind::Query);
        left.create_table(SAVED_QUERIES_TABLE, vec![legacy_batch(&legacy)?])
            .execute()
            .await?;

        let first = saved_query("first", "First", SavedQueryKind::Query);
        let second = saved_query("second", "Second", SavedQueryKind::Query);
        let (first_result, second_result) = tokio::join!(
            save_saved_query(&left, &first),
            save_saved_query(&right, &second)
        );
        assert_eq!(first_result?, SavedQuerySaveResult::Saved);
        assert_eq!(second_result?, SavedQuerySaveResult::Saved);

        let saved = list_saved_queries(&left).await?;
        assert_eq!(saved.len(), 3);
        assert!(saved.iter().any(|query| query.id == legacy.id));
        assert!(saved.iter().any(|query| query.id == first.id));
        assert!(saved.iter().any(|query| query.id == second.id));

        let table = left
            .open_table(SAVED_QUERIES_MANIFEST_TABLE)
            .execute()
            .await?;
        assert!(load_manifest_from_table(&table).await?.is_some());
        drop(table);
        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_views_cannot_exceed_surface_limit() -> Result<()> {
        let (path, left, right) = test_connections().await?;
        let limit = super::super::MAX_WORKBENCH_VIEWS;
        let manifest = SavedQueryManifest {
            queries: (0..limit - 1)
                .map(|index| {
                    saved_query(
                        &format!("existing-{index}"),
                        &format!("Existing{index}"),
                        SavedQueryKind::View,
                    )
                })
                .collect(),
        };
        let revision = create_id();
        let (_, batch) = manifest_batch(&manifest, &revision)?;
        left.create_table(SAVED_QUERIES_MANIFEST_TABLE, vec![batch])
            .execute()
            .await?;

        let first = saved_query("first", "First", SavedQueryKind::View);
        let second = saved_query("second", "Second", SavedQueryKind::View);
        let (first_result, second_result) = tokio::join!(
            save_saved_query(&left, &first),
            save_saved_query(&right, &second)
        );
        assert_one_saved_one_limit_conflict(first_result?, second_result?);

        let saved = list_saved_queries(&left).await?;
        assert_eq!(
            saved
                .iter()
                .filter(|query| query.kind == SavedQueryKind::View)
                .count(),
            limit
        );
        drop(left);
        drop(right);
        std::fs::remove_dir_all(path).ok();
        Ok(())
    }
}

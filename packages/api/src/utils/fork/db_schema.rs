//! Schema-only project-database forking.
//!
//! Under [`super::ForkDatabaseMode::SchemaOnly`] the object mirror skips
//! every user table's `.lance` directory (see [`super::policy::storage_skip`])
//! and the destination's tables are recreated empty from the source schema
//! instead. Reserved artifact tables (`__x__`) are Data Studio configuration
//! and are carried whole by the mirror, so they are not touched here.
//!
//! Indices are **not** reproduced: `list_indices` reports a display-stringified
//! index type that `index()` cannot round-trip, and a vector index cannot exist
//! on an empty table at all. Callers surface that as a warning.

use crate::{error::ApiError, state::AppState};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};

/// One source table's Arrow schema, ready to ship to the desktop in an
/// offline bundle. `arrow-schema` is built with `serde`, and
/// `GET /apps/{id}/db/{table}/schema` already returns this shape over HTTP.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ForkTableSchema {
    pub table: String,
    /// serde-serialized `arrow_schema::Schema`
    #[schema(value_type = Object)]
    pub schema: serde_json::Value,
}

/// Reads the Arrow schema of every non-reserved table in `app_id`'s project
/// database. Reserved (`__x__`) tables are skipped — they ride along whole.
pub async fn read_project_db_schemas(
    state: &AppState,
    app_id: &str,
) -> Result<Vec<ForkTableSchema>, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let connection = credentials
        .to_db(app_id)
        .await
        .map_err(ApiError::internal_error)?
        .execute()
        .await
        .map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("open project db '{app_id}': {e}"))
        })?;

    let table_names = connection.table_names().execute().await.map_err(|e| {
        ApiError::internal_error(flow_like_types::anyhow!("list source tables: {e}"))
    })?;

    let mut schemas = Vec::new();
    for table in table_names {
        if flow_like_catalog_core::is_reserved_table(&table) {
            continue;
        }
        let store = LanceDBVectorStore::from_connection(connection.clone(), table.clone()).await;
        let schema = store.schema().await.map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("read schema of '{table}': {e}"))
        })?;
        let schema = serde_json::to_value(&schema).map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "serialize schema of '{table}': {e}"
            ))
        })?;
        schemas.push(ForkTableSchema { table, schema });
    }
    Ok(schemas)
}

/// Recreates every non-reserved source table as an **empty** table in the
/// destination app's project database. Returns the created table names.
pub async fn copy_project_db_schemas(
    state: &AppState,
    src_app_id: &str,
    dst_app_id: &str,
) -> Result<Vec<String>, ApiError> {
    let schemas = read_project_db_schemas(state, src_app_id).await?;
    if schemas.is_empty() {
        return Ok(Vec::new());
    }

    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let destination = credentials
        .to_db(dst_app_id)
        .await
        .map_err(ApiError::internal_error)?
        .execute()
        .await
        .map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "open project db '{dst_app_id}': {e}"
            ))
        })?;

    let mut created = Vec::new();
    for entry in schemas {
        let schema: flow_like_storage::arrow_schema::Schema = serde_json::from_value(entry.schema)
            .map_err(|e| {
                ApiError::internal_error(flow_like_types::anyhow!(
                    "decode schema of '{}': {e}",
                    entry.table
                ))
            })?;
        let mut store =
            LanceDBVectorStore::from_connection(destination.clone(), entry.table.clone()).await;
        store.create_empty_table(schema, true).await.map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!(
                "create empty table '{}': {e}",
                entry.table
            ))
        })?;
        created.push(entry.table);
    }
    Ok(created)
}

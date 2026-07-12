use std::sync::Arc;

use anyhow::anyhow;
use flow_like::{
    credentials::SharedCredentials,
    flow_like_storage::{
        Path,
        arrow_schema::Schema,
        databases::vector::{
            VectorStore,
            lancedb::{IndexConfigDto, LanceDBVectorStore, record_batches_to_vec},
        },
        datafusion::prelude::SessionContext,
    },
};
use tauri::AppHandle;

use crate::{
    functions::{TauriFunctionError, flow::storage::current_user_sub},
    state::TauriFlowLikeState,
};

/// Validates a table name: alphanumeric, hyphens, underscores, dots only; no path traversal.
fn validate_table_name(name: &str) -> flow_like_types::Result<()> {
    if name.is_empty() || name.len() > 256 {
        return Err(flow_like_types::anyhow!(
            "Table name must be 1-256 characters"
        ));
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(flow_like_types::anyhow!(
            "Table name contains forbidden characters"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(flow_like_types::anyhow!(
            "Table name contains invalid characters (allowed: alphanumeric, -, _, .)"
        ));
    }
    Ok(())
}

/// Max offset to prevent unbounded scans.
const MAX_OFFSET: u64 = 100_000;

async fn db_connection_inner(
    app_handle: &AppHandle,
    app_id: String,
    table_name: Option<String>,
    credentials: Option<Arc<SharedCredentials>>,
    user_scoped: bool,
    sub: Option<String>,
) -> flow_like_types::Result<LanceDBVectorStore> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    let table_name = table_name.unwrap_or("default".to_string());
    validate_table_name(&table_name)?;
    let project_db_dir = Path::from("apps")
        .child(app_id.clone())
        .child("storage")
        .child("db");
    let db = if let Some(credentials) = &credentials {
        if user_scoped {
            let sub = sub.ok_or_else(|| {
                flow_like_types::anyhow!(
                    "User subject (sub) is required for user-scoped database access"
                )
            })?;
            credentials.to_db_scoped(&sub, &app_id).await?
        } else {
            credentials.to_db(&app_id).await?
        }
    } else if user_scoped {
        let sub = match sub {
            Some(sub) => sub,
            None => current_user_sub(app_handle)
                .await
                .map_err(|e| flow_like_types::anyhow!(e.to_string()))?,
        };
        let user_db_dir = Path::from("users")
            .child(sub)
            .child("apps")
            .child(app_id.clone())
            .child("db");
        flow_like_state
            .config
            .read()
            .await
            .callbacks
            .build_user_database
            .clone()
            .ok_or(flow_like_types::anyhow!("No user database builder found"))?(user_db_dir)
    } else {
        flow_like_state
            .config
            .read()
            .await
            .callbacks
            .build_project_database
            .clone()
            .ok_or(flow_like_types::anyhow!("No database builder found"))?(project_db_dir)
    };

    let db = db.execute().await?;
    let mut db = LanceDBVectorStore::from_connection(db, table_name).await;
    if let Some(opts) = &flow_like_state
        .config
        .read()
        .await
        .callbacks
        .lance_write_options
    {
        db.set_write_options(opts.clone());
    }
    Ok(db)
}

#[tauri::command(async)]
pub async fn db_table_names(
    app_handle: AppHandle,
    app_id: String,
    table_name: Option<String>,
    credentials: Option<Arc<SharedCredentials>>,
) -> Result<Vec<String>, TauriFunctionError> {
    let db = db_connection_inner(&app_handle, app_id, table_name, credentials, false, None).await?;
    let table_names = db
        .list_tables()
        .await?
        .into_iter()
        .filter(|name| !flow_like_catalog::is_reserved_table(name))
        .collect();
    Ok(table_names)
}

#[tauri::command(async)]
pub async fn db_table_names_user(
    app_handle: AppHandle,
    app_id: String,
    table_name: Option<String>,
    credentials: Option<Arc<SharedCredentials>>,
    sub: Option<String>,
) -> Result<Vec<String>, TauriFunctionError> {
    let db = db_connection_inner(&app_handle, app_id, table_name, credentials, true, sub).await?;
    let table_names = db
        .list_tables()
        .await?
        .into_iter()
        .filter(|name| !flow_like_catalog::is_reserved_table(name))
        .collect();
    Ok(table_names)
}

#[tauri::command(async)]
pub async fn db_count(
    app_handle: AppHandle,
    app_id: String,
    table_name: Option<String>,
    credentials: Option<Arc<SharedCredentials>>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<usize, TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        table_name,
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let cnt = db.count(None).await?;
    Ok(cnt)
}

#[tauri::command(async)]
pub async fn db_schema(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<Schema, TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let schema = db.schema().await?;
    Ok(schema)
}

#[tauri::command(async)]
pub async fn db_list(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    limit: Option<u64>,
    offset: Option<u64>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<Vec<flow_like_types::Value>, TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let limit = limit.unwrap_or(25).min(250) as usize;
    let offset = offset.unwrap_or(0).min(MAX_OFFSET) as usize;
    let items = db.list(None, limit, offset).await?;
    Ok(items)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VectorQueryPayload {
    pub column: String,
    pub vector: Vec<f64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QueryTablePayload {
    sql: Option<String>,
    vector_query: Option<VectorQueryPayload>,
    filter: Option<String>,
    fts_term: Option<String>,
    rerank: Option<bool>,
    select: Option<Vec<String>>,
}

#[tauri::command(async)]
pub async fn db_query(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    limit: Option<u64>,
    offset: Option<u64>,
    payload: QueryTablePayload,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<Vec<flow_like_types::Value>, TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name.clone()),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let limit = limit.unwrap_or(25).min(250) as usize;
    let offset = offset.unwrap_or(0).min(MAX_OFFSET) as usize;
    if let Some(sql) = payload.sql {
        let context = SessionContext::new();
        let fusion = db.to_datafusion().await?;
        context
            .register_table(table_name, Arc::new(fusion))
            .map_err(|e| anyhow!(e))?;
        let df = context.sql(&sql).await.map_err(|e| anyhow!(e))?;
        let items = df.collect().await.map_err(|e| anyhow!(e))?;
        let items = record_batches_to_vec(Some(items))?;
        return Ok(items);
    }

    match (payload.vector_query, payload.fts_term, payload.filter) {
        (Some(vector_query), None, filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .vector_search(
                    vector_query.vector,
                    filter_str,
                    payload.select,
                    limit,
                    offset,
                )
                .await?;
            Ok(items)
        }
        (None, Some(fts_term), filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .fts_search(&fts_term, filter_str, payload.select, None, limit, offset)
                .await?;
            Ok(items)
        }
        (Some(vector_query), Some(fts_term), filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .hybrid_search(
                    vector_query.vector,
                    &fts_term,
                    filter_str,
                    payload.select,
                    None,
                    limit,
                    offset,
                    payload.rerank.unwrap_or(true),
                )
                .await?;
            Ok(items)
        }
        (None, None, Some(filter)) => {
            let items = db.filter(&filter, payload.select, limit, offset).await?;
            Ok(items)
        }
        _ => Err(anyhow::anyhow!("No query parameters provided").into()),
    }
}

#[tauri::command(async)]
pub async fn db_indices(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<Vec<IndexConfigDto>, TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let indices = db.list_indices().await?;
    Ok(indices)
}

#[tauri::command(async)]
pub async fn db_delete(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    query: String,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.delete(&query).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn db_add(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    items: Vec<flow_like_types::Value>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let mut db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.insert(items).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn build_index(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    column: String,
    index_type: String,
    optimize: Option<bool>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.index(&column, Some(&index_type)).await?;
    if optimize.unwrap_or(false) {
        db.optimize(false).await?;
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn db_optimize(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    keep_versions: Option<bool>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.optimize(keep_versions.unwrap_or(false)).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn db_update(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    filter: String,
    updates: std::collections::HashMap<String, flow_like_types::Value>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.update(&filter, updates).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn db_drop_columns(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    columns: Vec<String>,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    let column_refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
    db.drop_columns(&column_refs).await?;
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AddColumnPayload {
    pub name: String,
    pub sql_expression: String,
}

#[tauri::command(async)]
pub async fn db_add_column(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    column: AddColumnPayload,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.add_column(&column.name, &column.sql_expression).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn db_alter_column(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    column: String,
    nullable: bool,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.make_column_nullable(&column, nullable).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn db_drop_index(
    app_handle: AppHandle,
    app_id: String,
    table_name: String,
    credentials: Option<Arc<SharedCredentials>>,
    index_name: String,
    user_scoped: Option<bool>,
    sub: Option<String>,
) -> Result<(), TauriFunctionError> {
    let db = db_connection_inner(
        &app_handle,
        app_id,
        Some(table_name),
        credentials,
        user_scoped.unwrap_or(false),
        sub,
    )
    .await?;
    db.drop_index(&index_name).await?;
    Ok(())
}

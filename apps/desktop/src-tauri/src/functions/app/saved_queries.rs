use flow_like::flow_like_storage::{
    databases::graph::lancegraph,
    databases::workbench::{
        self, WorkbenchSurface, WorkbenchView,
        saved_query::{self, SavedQueryDef, SavedQueryKind, SavedQuerySurface},
    },
    lancedb::Connection,
};
use flow_like_types::{Value, create_id};
use serde::Deserialize;
use tauri::AppHandle;

use super::graph::graph_connection;
use crate::functions::TauriFunctionError;

#[derive(Debug, Deserialize)]
pub struct SavedQueryInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: String,
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
    pub sql: String,
    #[serde(default)]
    pub param_schema: Option<Value>,
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SavedQueryUpdateInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub overlay_id: Option<String>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub param_schema: Option<Value>,
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
    #[serde(default)]
    pub expected_updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteQueryInput {
    pub sql: String,
    #[serde(default)]
    pub params: Option<Value>,
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn parse_kind(value: &str) -> flow_like_types::Result<SavedQueryKind> {
    match value {
        "query" => Ok(SavedQueryKind::Query),
        "view" => Ok(SavedQueryKind::View),
        other => Err(flow_like_types::anyhow!(
            "Unknown saved query kind '{}'",
            other
        )),
    }
}

fn parse_surface(value: &str) -> flow_like_types::Result<SavedQuerySurface> {
    match value {
        "native" => Ok(SavedQuerySurface::Native),
        "overlay" => Ok(SavedQuerySurface::Overlay),
        other => Err(flow_like_types::anyhow!(
            "Unknown query surface '{}'",
            other
        )),
    }
}

async fn collect_views(
    connection: &Connection,
    surface: SavedQuerySurface,
    overlay_id: Option<&str>,
) -> flow_like_types::Result<Vec<WorkbenchView>> {
    let all = saved_query::list_saved_queries(connection).await?;
    Ok(all
        .into_iter()
        .filter(|query| query.kind == SavedQueryKind::View && query.surface == surface)
        .filter(|query| {
            surface != SavedQuerySurface::Overlay || query.overlay_id.as_deref() == overlay_id
        })
        .map(|query| WorkbenchView {
            name: query.name,
            sql: query.sql,
        })
        .collect())
}

#[tauri::command(async)]
pub async fn query_saved_list(
    app_handle: AppHandle,
    app_id: String,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let queries = saved_query::list_saved_queries(&conn).await?;
    serde_json::to_value(queries).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn query_saved_get(
    app_handle: AppHandle,
    app_id: String,
    query_id: String,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let query = saved_query::load_saved_query(&conn, &query_id).await?;
    serde_json::to_value(query).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn query_saved_create(
    app_handle: AppHandle,
    app_id: String,
    payload: SavedQueryInput,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let def = SavedQueryDef {
        id: create_id(),
        app_id: app_id.clone(),
        name: payload.name,
        description: payload.description,
        kind: parse_kind(&payload.kind)?,
        surface: parse_surface(&payload.surface)?,
        overlay_id: payload.overlay_id,
        sql: payload.sql,
        param_schema: payload.param_schema,
        viz_config: payload.viz_config,
        default_limit: payload.default_limit,
        created_at: now.clone(),
        updated_at: now,
    };
    workbench::validate_workbench_sql(&def.sql)?;
    saved_query::save_saved_query(&conn, &def).await?;
    serde_json::to_value(def).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn query_saved_update(
    app_handle: AppHandle,
    app_id: String,
    query_id: String,
    payload: SavedQueryUpdateInput,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let previous = saved_query::load_saved_query(&conn, &query_id).await?;

    if let Some(expected) = payload.expected_updated_at.as_deref()
        && expected != previous.updated_at
    {
        return Err(TauriFunctionError::new(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }

    let mut def = previous.clone();
    if let Some(name) = payload.name {
        def.name = name;
    }
    if payload.description.is_some() {
        def.description = payload.description;
    }
    if let Some(kind) = payload.kind {
        def.kind = parse_kind(&kind)?;
    }
    if let Some(surface) = payload.surface {
        def.surface = parse_surface(&surface)?;
    }
    if payload.overlay_id.is_some() {
        def.overlay_id = payload.overlay_id;
    }
    if let Some(sql) = payload.sql {
        def.sql = sql;
    }
    if payload.param_schema.is_some() {
        def.param_schema = payload.param_schema;
    }
    if payload.viz_config.is_some() {
        def.viz_config = payload.viz_config;
    }
    if payload.default_limit.is_some() {
        def.default_limit = payload.default_limit;
    }
    def.updated_at = chrono::Utc::now().to_rfc3339();

    workbench::validate_workbench_sql(&def.sql)?;
    let saved =
        saved_query::save_saved_query_if_unchanged(&conn, &def, &previous.updated_at).await?;
    if !saved {
        return Err(TauriFunctionError::new(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }
    serde_json::to_value(def).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn query_saved_delete(
    app_handle: AppHandle,
    app_id: String,
    query_id: String,
    user_scoped: Option<bool>,
) -> Result<(), TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    saved_query::delete_saved_query(&conn, &query_id).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn query_execute_sql(
    app_handle: AppHandle,
    app_id: String,
    payload: ExecuteQueryInput,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let params = payload.params.unwrap_or(Value::Null);

    let (surface, views) = match parse_surface(&payload.surface)? {
        SavedQuerySurface::Native => {
            let views = collect_views(&conn, SavedQuerySurface::Native, None).await?;
            (WorkbenchSurface::Native, views)
        }
        SavedQuerySurface::Overlay => {
            let overlay_id = payload.overlay_id.as_deref().ok_or_else(|| {
                flow_like_types::anyhow!("An overlay id is required for the overlay surface")
            })?;
            let overlay = lancegraph::load_overlay(&conn, overlay_id).await?;
            let views = collect_views(&conn, SavedQuerySurface::Overlay, Some(overlay_id)).await?;
            (WorkbenchSurface::Overlay(overlay), views)
        }
    };

    let result = workbench::execute_readonly_sql(
        &conn,
        surface,
        views,
        &payload.sql,
        &params,
        payload.limit,
    )
    .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

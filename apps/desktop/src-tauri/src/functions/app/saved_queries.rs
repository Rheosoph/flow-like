use flow_like::flow_like_storage::{
    databases::graph::lancegraph,
    databases::workbench::{
        self, WorkbenchSurface, WorkbenchView,
        saved_query::{
            self, SavedQueryDef, SavedQueryKind, SavedQuerySaveResult, SavedQuerySurface,
        },
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

fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct SavedQueryUpdateInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub overlay_id: Option<Option<String>>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub param_schema: Option<Option<Value>>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub viz_config: Option<Option<Value>>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub default_limit: Option<Option<usize>>,
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
        .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(flow_like_types::anyhow!(
            "Table name contains invalid characters"
        ));
    }
    if flow_like_catalog::is_reserved_table(name) {
        return Err(flow_like_types::anyhow!(
            "Table name is reserved for internal use"
        ));
    }
    Ok(())
}

async fn validate_saved_query(
    connection: &Connection,
    def: &SavedQueryDef,
) -> flow_like_types::Result<()> {
    match def.surface {
        SavedQuerySurface::Overlay => {
            let overlay_id = def
                .overlay_id
                .as_deref()
                .filter(|overlay_id| !overlay_id.trim().is_empty())
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "A non-empty overlay id is required for overlay queries"
                    )
                })?;
            lancegraph::load_overlay(connection, overlay_id)
                .await
                .map_err(|_| {
                    flow_like_types::anyhow!("The selected graph overlay does not exist")
                })?;
        }
        SavedQuerySurface::Native if def.overlay_id.is_some() => {
            return Err(flow_like_types::anyhow!(
                "overlay_id must be omitted for native queries"
            ));
        }
        SavedQuerySurface::Native => {}
    }
    workbench::validate_workbench_sql(&def.sql)?;

    if def.kind == SavedQueryKind::View {
        if def.sql.contains('$') {
            return Err(flow_like_types::anyhow!(
                "Views must not declare parameters"
            ));
        }
        validate_table_name(&def.name)?;
        let tables = connection.table_names().execute().await?;
        if tables
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&def.name))
        {
            return Err(flow_like_types::anyhow!(
                "View name '{}' collides with an existing table",
                def.name
            ));
        }
    }
    Ok(())
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
    validate_saved_query(&conn, &def).await?;
    match saved_query::save_saved_query(&conn, &def).await? {
        SavedQuerySaveResult::Saved => {}
        SavedQuerySaveResult::ViewNameConflict { .. } => {
            return Err(TauriFunctionError::new(&format!(
                "View name '{}' is already used by another saved view on this surface",
                def.name
            )));
        }
        SavedQuerySaveResult::RevisionConflict => {
            return Err(TauriFunctionError::new(
                "A saved query with this ID was created concurrently. Retry the request.",
            ));
        }
        SavedQuerySaveResult::ViewLimitExceeded { limit } => {
            return Err(TauriFunctionError::new(&format!(
                "A workbench surface can contain at most {limit} saved views"
            )));
        }
    }
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

    let expected_updated_at = payload.expected_updated_at.clone().ok_or_else(|| {
        TauriFunctionError::new("expected_updated_at is required when updating a saved query")
    })?;
    if expected_updated_at != previous.updated_at {
        return Err(TauriFunctionError::new(
            "This saved query was modified elsewhere. Reload and try again.",
        ));
    }

    let mut def = previous.clone();
    if let Some(name) = payload.name {
        def.name = name;
    }
    if let Some(description) = payload.description {
        def.description = description;
    }
    if let Some(kind) = payload.kind {
        def.kind = parse_kind(&kind)?;
    }
    if let Some(surface) = payload.surface {
        def.surface = parse_surface(&surface)?;
    }
    if let Some(overlay_id) = payload.overlay_id {
        def.overlay_id = overlay_id;
    }
    if let Some(sql) = payload.sql {
        def.sql = sql;
    }
    if let Some(param_schema) = payload.param_schema {
        def.param_schema = param_schema;
    }
    if let Some(viz_config) = payload.viz_config {
        def.viz_config = viz_config;
    }
    if let Some(default_limit) = payload.default_limit {
        def.default_limit = default_limit;
    }
    def.updated_at = saved_query::next_updated_at(&previous.updated_at);

    validate_saved_query(&conn, &def).await?;
    match saved_query::save_saved_query_if_unchanged(&conn, &def, &expected_updated_at).await? {
        SavedQuerySaveResult::Saved => {}
        SavedQuerySaveResult::RevisionConflict => {
            return Err(TauriFunctionError::new(
                "This saved query was modified elsewhere. Reload and try again.",
            ));
        }
        SavedQuerySaveResult::ViewNameConflict { .. } => {
            return Err(TauriFunctionError::new(&format!(
                "View name '{}' is already used by another saved view on this surface",
                def.name
            )));
        }
        SavedQuerySaveResult::ViewLimitExceeded { limit } => {
            return Err(TauriFunctionError::new(&format!(
                "A workbench surface can contain at most {limit} saved views"
            )));
        }
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

#[cfg(test)]
mod tests {
    use super::SavedQueryUpdateInput;
    use serde_json::json;

    #[test]
    fn nullable_update_fields_distinguish_omission_and_clear() {
        let omitted: SavedQueryUpdateInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(omitted.description, None);
        assert_eq!(omitted.default_limit, None);

        let cleared: SavedQueryUpdateInput = serde_json::from_value(json!({
            "description": null,
            "overlay_id": null,
            "param_schema": null,
            "viz_config": null,
            "default_limit": null
        }))
        .unwrap();
        assert_eq!(cleared.description, Some(None));
        assert_eq!(cleared.overlay_id, Some(None));
        assert_eq!(cleared.param_schema, Some(None));
        assert_eq!(cleared.viz_config, Some(None));
        assert_eq!(cleared.default_limit, Some(None));
    }
}

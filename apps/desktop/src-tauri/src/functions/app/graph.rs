use flow_like::flow_like_storage::{
    Path,
    databases::graph::lancegraph::{
        self, EdgeMappingDef, GraphOverlayDef, LanceGraphStore, NodeMappingDef,
    },
    databases::graph::{GraphStore, TraversalDirection},
    lancedb::Connection,
};
use flow_like_catalog::{
    DEFAULT_GRAPH_NEIGHBORS_DIRECTION, DEFAULT_GRAPH_OVERLAY_LIMIT, DEFAULT_GRAPH_QUERY_LIMIT,
    DEFAULT_GRAPH_SAMPLE_SIZE,
};
use flow_like_types::create_id;
use serde::Deserialize;
use tauri::AppHandle;

use crate::{
    functions::{TauriFunctionError, flow::storage::current_user_sub},
    state::TauriFlowLikeState,
};

async fn graph_connection(
    app_handle: &AppHandle,
    app_id: &str,
    user_scoped: bool,
) -> flow_like_types::Result<Connection> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    let project_db_dir = Path::from("apps")
        .child(app_id)
        .child("storage")
        .child("db");

    let builder = if user_scoped {
        let sub = current_user_sub(app_handle)
            .await
            .map_err(|e| flow_like_types::anyhow!(e.to_string()))?;
        let user_db_dir = Path::from("users")
            .child(sub)
            .child("apps")
            .child(app_id)
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

    builder
        .execute()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to connect to database: {}", e))
}

#[tauri::command(async)]
pub async fn graph_list_overlays(
    app_handle: AppHandle,
    app_id: String,
    user_scoped: Option<bool>,
) -> Result<Vec<GraphOverlayDef>, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlays = lancegraph::list_overlays(&conn).await?;
    Ok(overlays)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOverlayPayload {
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<NodeMappingDef>,
    pub edges: Vec<EdgeMappingDef>,
    #[serde(alias = "default_limit")]
    pub default_limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_create_overlay(
    app_handle: AppHandle,
    app_id: String,
    payload: CreateOverlayPayload,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let overlay = GraphOverlayDef {
        id: create_id(),
        name: payload.name,
        description: payload.description,
        nodes: payload.nodes,
        edges: payload.edges,
        default_limit: payload.default_limit.unwrap_or(DEFAULT_GRAPH_OVERLAY_LIMIT),
        created_at: now.clone(),
        updated_at: now,
    };
    lancegraph::save_overlay(&conn, &overlay).await?;
    Ok(overlay)
}

#[tauri::command(async)]
pub async fn graph_get_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    Ok(overlay)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOverlayPayload {
    pub name: Option<String>,
    pub description: Option<String>,
    pub nodes: Option<Vec<NodeMappingDef>>,
    pub edges: Option<Vec<EdgeMappingDef>>,
    #[serde(alias = "default_limit")]
    pub default_limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_update_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: UpdateOverlayPayload,
    user_scoped: Option<bool>,
) -> Result<GraphOverlayDef, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let mut overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;

    if let Some(name) = payload.name {
        overlay.name = name;
    }
    if let Some(desc) = payload.description {
        overlay.description = Some(desc);
    }
    if let Some(nodes) = payload.nodes {
        overlay.nodes = nodes;
    }
    if let Some(edges) = payload.edges {
        overlay.edges = edges;
    }
    if let Some(limit) = payload.default_limit {
        overlay.default_limit = limit;
    }
    overlay.updated_at = chrono::Utc::now().to_rfc3339();

    lancegraph::save_overlay(&conn, &overlay).await?;
    Ok(overlay)
}

#[tauri::command(async)]
pub async fn graph_delete_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<(), TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    lancegraph::delete_overlay(&conn, &overlay_id).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn graph_get_schema(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let schema = store.schema().await?;
    serde_json::to_value(schema).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn graph_validate_overlay(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await;
    match store {
        Ok(_) => Ok(serde_json::json!({"ok": true, "issues": []})),
        Err(e) => Ok(serde_json::json!({"ok": false, "issues": [e.to_string()]})),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CypherPayload {
    pub query: String,
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_cypher(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: CypherPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let params = match payload.params {
        Some(map) => serde_json::Value::Object(map),
        None => serde_json::Value::Null,
    };
    let result = store
        .cypher(
            &payload.query,
            params,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubgraphPayload {
    pub seeds: Vec<SubgraphSeed>,
    pub depth: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubgraphSeed {
    pub label: String,
    pub id: serde_json::Value,
}

#[tauri::command(async)]
pub async fn graph_subgraph(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SubgraphPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let seeds: Vec<(String, serde_json::Value)> =
        payload.seeds.into_iter().map(|s| (s.label, s.id)).collect();
    let result = store
        .subgraph(
            seeds,
            payload.depth.unwrap_or(1),
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchNodesPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_search_nodes(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SearchNodesPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store.search_nodes(&payload.query, payload.limit).await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborsPayload {
    pub label: String,
    #[serde(alias = "node_id")]
    pub node_id: serde_json::Value,
    pub depth: Option<usize>,
    pub direction: Option<String>,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_neighbors(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: NeighborsPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let direction = match payload
        .direction
        .as_deref()
        .unwrap_or(DEFAULT_GRAPH_NEIGHBORS_DIRECTION)
    {
        "incoming" => TraversalDirection::Incoming,
        "both" => TraversalDirection::Both,
        _ => TraversalDirection::Outgoing,
    };
    let result = store
        .neighbors(
            &payload.label,
            payload.node_id,
            payload.depth.unwrap_or(1),
            direction,
            payload.limit,
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[tauri::command(async)]
pub async fn graph_sql(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    payload: SqlPayload,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store
        .sql(
            &payload.query,
            Some(payload.limit.unwrap_or(DEFAULT_GRAPH_QUERY_LIMIT)),
        )
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

#[tauri::command(async)]
pub async fn graph_sample(
    app_handle: AppHandle,
    app_id: String,
    overlay_id: String,
    label: String,
    n: Option<usize>,
    user_scoped: Option<bool>,
) -> Result<serde_json::Value, TauriFunctionError> {
    let conn = graph_connection(&app_handle, &app_id, user_scoped.unwrap_or(false)).await?;
    let overlay = lancegraph::load_overlay(&conn, &overlay_id).await?;
    let store = LanceGraphStore::new(conn, overlay, None).await?;
    let result = store
        .sample(&label, n.unwrap_or(DEFAULT_GRAPH_SAMPLE_SIZE))
        .await?;
    serde_json::to_value(result).map_err(|e| e.into())
}

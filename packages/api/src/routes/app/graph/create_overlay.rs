use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_OVERLAY_LIMIT;
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct CreateOverlayPayload {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Vec<Object>)]
    pub nodes: Vec<flow_like_catalog_core::NodeLabelMapping>,
    #[schema(value_type = Vec<Object>)]
    pub edges: Vec<flow_like_catalog_core::EdgeLabelMapping>,
    #[serde(default = "default_limit")]
    pub default_limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_GRAPH_OVERLAY_LIMIT
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph",
    tag = "graph",
    description = "Create a new graph overlay.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = CreateOverlayPayload,
    responses(
        (status = 200, description = "Created overlay", body = Object),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/graph", skip(state, user, payload))]
pub async fn create_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<CreateOverlayPayload>,
) -> Result<Json<flow_like_catalog_core::GraphOverlay>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;

    let now = chrono::Utc::now().to_rfc3339();
    let overlay_id = uuid::Uuid::new_v4().to_string();

    let def = lancegraph::GraphOverlayDef {
        id: overlay_id.clone(),
        name: payload.name.clone(),
        description: payload.description.clone(),
        nodes: payload
            .nodes
            .iter()
            .map(|n| lancegraph::NodeMappingDef {
                label: n.label.clone(),
                table: n.table.clone(),
                id_column: n.id_column.clone(),
                display_column: n.display_column.clone(),
                property_columns: n
                    .property_columns
                    .iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name.clone(),
                        data_type: p.data_type.clone(),
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&n.style).unwrap_or_default(),
            })
            .collect(),
        edges: payload
            .edges
            .iter()
            .map(|e| lancegraph::EdgeMappingDef {
                label: e.label.clone(),
                table: e.table.clone(),
                src_column: e.src_column.clone(),
                dst_column: e.dst_column.clone(),
                src_label: e.src_label.clone(),
                dst_label: e.dst_label.clone(),
                src_node_column: e.src_node_column.clone(),
                dst_node_column: e.dst_node_column.clone(),
                property_columns: e
                    .property_columns
                    .iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name.clone(),
                        data_type: p.data_type.clone(),
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&e.style).unwrap_or_default(),
            })
            .collect(),
        default_limit: payload.default_limit,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    lancegraph::save_overlay(&connection, &def).await?;

    let result = flow_like_catalog_core::GraphOverlay {
        id: overlay_id,
        name: payload.name,
        description: payload.description,
        nodes: payload.nodes,
        edges: payload.edges,
        default_limit: payload.default_limit,
        created_at: now.clone(),
        updated_at: now,
    };

    Ok(Json(result))
}

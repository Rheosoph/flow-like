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
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpdateOverlayPayload {
    pub name: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub nodes: Option<Vec<flow_like_catalog_core::NodeLabelMapping>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub edges: Option<Vec<flow_like_catalog_core::EdgeLabelMapping>>,
    pub default_limit: Option<usize>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/graph/{overlay_id}",
    tag = "graph",
    description = "Update a graph overlay.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpdateOverlayPayload,
    responses(
        (status = 200, description = "Updated overlay", body = Object),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/graph/{overlay_id}",
    skip(state, user, payload)
)]
pub async fn update_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdateOverlayPayload>,
) -> Result<Json<flow_like_catalog_core::GraphOverlay>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let mut def = lancegraph::load_overlay(&connection, &overlay_id).await?;

    if let Some(name) = payload.name {
        def.name = name;
    }
    if let Some(description) = payload.description {
        def.description = Some(description);
    }
    if let Some(nodes) = payload.nodes {
        def.nodes = nodes
            .into_iter()
            .map(|n| lancegraph::NodeMappingDef {
                label: n.label,
                table: n.table,
                id_column: n.id_column,
                display_column: n.display_column,
                property_columns: n
                    .property_columns
                    .into_iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&n.style).unwrap_or_default(),
            })
            .collect();
    }
    if let Some(edges) = payload.edges {
        def.edges = edges
            .into_iter()
            .map(|e| lancegraph::EdgeMappingDef {
                label: e.label,
                table: e.table,
                src_column: e.src_column,
                dst_column: e.dst_column,
                src_label: e.src_label,
                dst_label: e.dst_label,
                src_node_column: e.src_node_column,
                dst_node_column: e.dst_node_column,
                property_columns: e
                    .property_columns
                    .into_iter()
                    .map(|p| lancegraph::PropertyColumnDef {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::to_value(&e.style).unwrap_or_default(),
            })
            .collect();
    }
    if let Some(default_limit) = payload.default_limit {
        def.default_limit = default_limit;
    }
    def.updated_at = chrono::Utc::now().to_rfc3339();

    lancegraph::save_overlay(&connection, &def).await?;

    Ok(Json(super::list_overlays::def_to_overlay(def)))
}

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

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph",
    tag = "graph",
    description = "List all graph overlays for the app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses(
        (status = 200, description = "List of overlays", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/graph", skip(state, user))]
pub async fn list_overlays(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<Vec<flow_like_catalog_core::GraphOverlay>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let mut defs = lancegraph::list_overlays(&connection).await?;
    if user.is_connected_app() {
        defs = defs
            .into_iter()
            .filter(|definition| definition.exposed)
            .map(crate::routes::app::connection::remote_ontologies::sanitize_remote_contract)
            .collect();
    }

    let overlays: Vec<flow_like_catalog_core::GraphOverlay> =
        defs.into_iter().map(def_to_overlay).collect();

    Ok(Json(overlays))
}

pub fn def_to_overlay(d: lancegraph::GraphOverlayDef) -> flow_like_catalog_core::GraphOverlay {
    flow_like_catalog_core::GraphOverlay {
        id: d.id,
        name: d.name,
        description: d.description,
        nodes: d
            .nodes
            .into_iter()
            .map(|n| flow_like_catalog_core::NodeLabelMapping {
                id: n.id,
                api_name: n.api_name,
                label: n.label,
                table: n.table,
                id_column: n.id_column,
                display_column: n.display_column,
                property_columns: n
                    .property_columns
                    .into_iter()
                    .map(|p| flow_like_catalog_core::PropertyColumn {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::from_value(n.style).unwrap_or_default(),
            })
            .collect(),
        edges: d
            .edges
            .into_iter()
            .map(|e| flow_like_catalog_core::EdgeLabelMapping {
                id: e.id,
                api_name: e.api_name,
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
                    .map(|p| flow_like_catalog_core::PropertyColumn {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
                style: serde_json::from_value(e.style).unwrap_or_default(),
            })
            .collect(),
        object_views: d
            .object_views
            .into_iter()
            .map(|view| flow_like_catalog_core::ObjectViewDefinition {
                object_type: view.object_type,
                title_property: view.title_property,
                prominent_properties: view.prominent_properties,
            })
            .collect(),
        actions: d
            .actions
            .into_iter()
            .map(|action| flow_like_catalog_core::OntologyActionDefinition {
                id: action.id,
                name: action.name,
                description: action.description,
                object_type: action.object_type,
                board_id: action.board_id,
                board_version: action.board_version,
                start_node_id: action.start_node_id,
                event_id: action.event_id,
                enabled: action.enabled,
                allow_bulk: action.allow_bulk,
                parameter_schema: action.parameter_schema,
            })
            .collect(),
        exposed: d.exposed,
        bindings_enabled: d.bindings_enabled,
        default_limit: d.default_limit,
        created_at: d.created_at,
        updated_at: d.updated_at,
    }
}

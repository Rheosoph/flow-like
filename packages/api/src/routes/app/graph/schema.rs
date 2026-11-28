use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::graph::GraphOperation;
use flow_like_storage::databases::graph::{GraphLabelInfo, GraphSchemaResult};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/{overlay_id}/schema",
    tag = "graph",
    description = "Get the schema (labels and properties) of a graph overlay.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses(
        (status = 200, description = "Graph schema", body = Object),
        (status = 400, description = "A table of the overlay could not be read"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "Reading the schema ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/graph/{overlay_id}/schema",
    skip(state, user, scope)
)]
pub async fn graph_schema(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<flow_like_catalog_core::GraphSchema>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let schema_result: GraphSchemaResult = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Schema {
            overlay: Box::new(overlay),
        },
    )
    .await?;

    Ok(Json(flow_like_catalog_core::GraphSchema {
        node_labels: schema_result
            .node_labels
            .into_iter()
            .map(label_info)
            .collect(),
        edge_labels: schema_result
            .edge_labels
            .into_iter()
            .map(label_info)
            .collect(),
    }))
}

fn label_info(label: GraphLabelInfo) -> flow_like_catalog_core::GraphLabelInfo {
    flow_like_catalog_core::GraphLabelInfo {
        label: label.label,
        table: label.table,
        properties: label
            .properties
            .into_iter()
            .map(|property| flow_like_catalog_core::GraphPropertyInfo {
                name: property.name,
                data_type: property.data_type,
                nullable: property.nullable,
                metadata: property.metadata,
            })
            .collect(),
    }
}

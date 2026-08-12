use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::{GraphStore, lancegraph};

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
    name = "GET /apps/{app_id}/graph/{overlay_id}/schema",
    skip(state, user, scope)
)]
pub async fn graph_schema(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<flow_like_catalog_core::GraphSchema>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let (connection, overlay) =
        super::load_scoped_overlay(&state, &user, &app_id, &overlay_id, &scope).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;
    let schema_result = store.schema().await?;

    let schema = flow_like_catalog_core::GraphSchema {
        node_labels: schema_result
            .node_labels
            .into_iter()
            .map(|l| flow_like_catalog_core::GraphLabelInfo {
                label: l.label,
                table: l.table,
                properties: l
                    .properties
                    .into_iter()
                    .map(|p| flow_like_catalog_core::GraphPropertyInfo {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
            })
            .collect(),
        edge_labels: schema_result
            .edge_labels
            .into_iter()
            .map(|l| flow_like_catalog_core::GraphLabelInfo {
                label: l.label,
                table: l.table,
                properties: l
                    .properties
                    .into_iter()
                    .map(|p| flow_like_catalog_core::GraphPropertyInfo {
                        name: p.name,
                        data_type: p.data_type,
                        nullable: p.nullable,
                    })
                    .collect(),
            })
            .collect(),
    };

    Ok(Json(schema))
}

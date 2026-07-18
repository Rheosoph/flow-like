use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

use super::upsert_nodes::UpsertResult;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpsertEdgesPayload {
    /// Edge label (type) to write into; must exist in the overlay schema.
    pub label: String,
    /// Edge rows to upsert. Each row must carry the label's source and target id columns.
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<flow_like_types::Value>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/edges",
    tag = "graph",
    description = "Insert or update edges of a label in a graph overlay's underlying table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpsertEdgesPayload,
    responses(
        (status = 200, description = "Rows written", body = UpsertResult),
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/edges",
    skip(state, user, payload)
)]
pub async fn upsert_edges(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpsertEdgesPayload>,
) -> Result<Json<UpsertResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let (connection, overlay) =
        super::load_scoped_overlay(&state, &user, &app_id, &overlay_id, &scope).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;

    let upserted = store.upsert_edges(&payload.label, payload.rows).await?;

    Ok(Json(UpsertResult { upserted }))
}

use crate::{
    audit_branch, ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct UpsertNodesPayload {
    /// Node label (type) to write into; must exist in the overlay schema.
    pub label: String,
    /// Node rows to upsert. Each row must carry the label's id column value.
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<flow_like_types::Value>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct UpsertResult {
    pub upserted: usize,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/nodes",
    tag = "graph",
    description = "Insert or update nodes of a label in a graph overlay's underlying table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = UpsertNodesPayload,
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
    name = "POST /apps/{app_id}/graph/{overlay_id}/nodes",
    skip(state, user, scope, payload)
)]
pub async fn upsert_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpsertNodesPayload>,
) -> Result<Json<UpsertResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let (connection, overlay) =
        super::load_scoped_overlay_for_write(&state, &user, &app_id, &overlay_id, &scope).await?;
    let store = lancegraph::LanceGraphStore::new(connection, overlay, None).await?;

    let upserted = store.upsert_nodes(&payload.label, payload.rows).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "graph.nodes.upsert",
        "GraphOverlay",
        overlay_id,
        "Saved graph nodes",
        serde_json::json!({
            "row_count": upserted,
            "user_scoped": scope.is_user_scoped(),
        })
    );

    Ok(Json(UpsertResult { upserted }))
}

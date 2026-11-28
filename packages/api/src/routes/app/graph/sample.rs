use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_catalog_core::DEFAULT_GRAPH_SAMPLE_SIZE;
use flow_like_query::graph::GraphOperation;

#[derive(Debug, serde::Deserialize)]
pub struct SampleParams {
    pub scope: Option<String>,
    pub label: String,
    #[serde(default = "default_n")]
    pub n: usize,
}

fn default_n() -> usize {
    DEFAULT_GRAPH_SAMPLE_SIZE
}

impl SampleParams {
    fn scope_params(&self) -> super::super::db::ScopeParams {
        super::super::db::ScopeParams {
            scope: self.scope.clone(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/graph/{overlay_id}/sample",
    tag = "graph",
    description = "Sample nodes or edges from a label for UI previews.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project"),
        ("label" = String, Query, description = "Label to sample from"),
        ("n" = Option<usize>, Query, description = "Number of samples (default 10)")
    ),
    responses(
        (status = 200, description = "Sample data", body = Vec<flow_like_types::Value>),
        (status = 400, description = "The label is not part of the overlay, or the sample failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The sample ran out of time")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/graph/{overlay_id}/sample",
    skip(state, user, params)
)]
pub async fn sample_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(params): Query<SampleParams>,
) -> Result<Json<Vec<flow_like_types::Value>>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let scope = params.scope_params();

    let results = super::read_overlay(
        &state,
        &user,
        &permission,
        &app_id,
        &overlay_id,
        &scope,
        |overlay| GraphOperation::Sample {
            overlay: Box::new(overlay),
            label: params.label,
            n: params.n.min(super::MAX_SAMPLE_ROWS),
        },
    )
    .await?;

    Ok(Json(results))
}

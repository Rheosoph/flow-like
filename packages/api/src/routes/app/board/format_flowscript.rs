use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct FormatFlowScriptBody {
    pub flowscript: String,
    /// Re-emit `//@n:<id>` anchor comments in the formatted output (default: true). Anchors in
    /// the input always survive the parse; `false` strips them from the result.
    #[serde(default)]
    pub anchors: Option<bool>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct FormatFlowScriptResponse {
    pub flowscript: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flowscript/format",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = FormatFlowScriptBody,
    responses(
        (status = 200, description = "The canonically formatted FlowScript source text", body = FormatFlowScriptResponse),
        (status = 400, description = "The source does not parse"),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flowscript/format",
    skip(state, user, params)
)]
pub async fn format_flowscript(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, _board_id)): Path<(String, String)>,
    Json(params): Json<FormatFlowScriptBody>,
) -> Result<Json<FormatFlowScriptResponse>, ApiError> {
    // Formatting is pure text-domain (`render(parse(text))`) and never touches the board, but it
    // is part of the board-editing surface: require the same read access as `get_flowscript`.
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let formatted =
        flow_like::flow::ast::format_flowscript(&params.flowscript, params.anchors.unwrap_or(true))
            .map_err(|error| {
                ApiError::bad_request(format!(
                    "FlowScript parse error at {}:{}: {}",
                    error.line, error.col, error.message
                ))
            })?;

    Ok(Json(FormatFlowScriptResponse {
        flowscript: formatted,
    }))
}

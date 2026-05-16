use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::secrets::filter_board_secrets, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::Board;
use flow_like_types::anyhow;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Deserialize, Debug, ToSchema)]
pub struct VersionQuery {
    /// expected format: "MAJOR_MINOR_PATCH", e.g. "1_0_3"
    pub version: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/templates/{template_id}",
    tag = "templates",
    description = "Get a template by ID and optional version.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Template payload", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "GET /apps/{app_id}/templates/{template_id}", skip(state, user))]
pub async fn get_template(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, template_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<Board>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadTemplates);
    let sub = permission.sub()?;

    let version_opt = if let Some(ver_str) = params.version {
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::internal_error(anyhow!(
                    "version must be in MAJOR_MINOR_PATCH format"
                )));
            }
        }
    } else {
        None
    };

    let mut template = state
        .scoped_template(
            &sub,
            &app_id,
            &template_id,
            &state,
            version_opt,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;

    // Templates are serialized `proto::Board` and carry the same
    // variable structure as live boards (`Board.variables` +
    // `Layer.variables`). Strip secrets before the response leaves —
    // template viewers don't need the source's secret values.
    filter_board_secrets(&mut template);

    Ok(Json(template))
}

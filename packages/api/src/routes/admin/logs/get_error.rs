//! Get a single error report by id (used when a user shares the reference id).

use crate::entity::error_report;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::admin::logs::list_errors::ErrorReportRecord;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use sea_orm::EntityTrait;

#[utoipa::path(
    get,
    path = "/admin/logs/errors/{error_id}",
    tag = "admin",
    params(("error_id" = String, Path, description = "Error report identifier")),
    responses(
        (status = 200, description = "Error report detail", body = ErrorReportRecord),
        (status = 404, description = "Error not found")
    ),
    description = "Look up an error report by its public reference id."
)]
pub async fn get_error(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(error_id): Path<String>,
) -> Result<Json<ErrorReportRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;

    let model = error_report::Entity::find_by_id(error_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    Ok(Json(model.into()))
}

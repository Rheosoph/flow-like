use crate::{
    entity::fork_job, error::ApiError, middleware::jwt::AppUser, state::AppState,
    utils::fork::job::ForkJobView,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::EntityTrait;

/// Poll a fork that `POST /apps/{app_id}/fork` accepted with `202`.
/// Only the user who started the job can see it; anyone else gets `404`.
#[utoipa::path(
    get,
    path = "/apps/fork/jobs/{job_id}",
    tag = "forking",
    description = "Read the status of an asynchronous fork job started by the calling user.",
    params(
        ("job_id" = String, Path, description = "Fork job id returned with the 202 response"),
    ),
    responses(
        (status = 200, description = "Current job state; `report` is present once `status` is `DONE`", body = ForkJobView),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No such job for this user, or the job was aborted")
    )
)]
#[tracing::instrument(name = "GET /apps/fork/jobs/{job_id}", skip(state, user))]
pub async fn get_fork_job(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ForkJobView>, ApiError> {
    let sub = user.sub()?;
    let job = fork_job::Entity::find_by_id(job_id.as_str())
        .one(&state.db)
        .await?
        .filter(|job| job.user_id == sub)
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(Json(ForkJobView::from(&job)))
}

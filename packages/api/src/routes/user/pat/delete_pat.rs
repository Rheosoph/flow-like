use crate::{entity::pat, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/user/pat/{pat_id}",
    tag = "user",
    params(
        ("pat_id" = String, Path, description = "Personal access token ID")
    ),
    responses(
        (status = 200, description = "Personal access token deleted"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "DELETE /user/pat/:pat_id", skip(state, user))]
pub async fn delete_pat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(pat_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;

    pat::Entity::delete_many()
        .filter(pat::Column::Id.eq(pat_id).and(pat::Column::UserId.eq(sub)))
        .exec(&state.db)
        .await?;

    Ok(Json(()))
}

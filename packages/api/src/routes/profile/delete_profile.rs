use crate::{entity::profile, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
};

/// Soft-delete a profile by ID (sets deleted_at timestamp)
#[utoipa::path(
    delete,
    path = "/profile/{profile_id}",
    tag = "profile",
    params(
        ("profile_id" = String, Path, description = "Profile ID to delete")
    ),
    responses(
        (status = 200, description = "Profile deleted successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Profile not found")
    )
)]
#[tracing::instrument(name = "DELETE /profile/{profile_id}", skip(state, user))]
pub async fn delete_profile(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(profile_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    if profile_id.trim().is_empty() {
        return Err(ApiError::bad_request("Profile ID is required"));
    }

    let profile = profile::Entity::find()
        .filter(
            profile::Column::Id
                .eq(&profile_id)
                .and(profile::Column::UserId.eq(&sub)),
        )
        .one(&state.db)
        .await?;

    if let Some(existing) = profile {
        if existing.deleted_at.is_some() {
            return Ok(Json(()));
        }

        let active_profile_count = profile::Entity::find()
            .filter(
                profile::Column::UserId
                    .eq(&sub)
                    .and(profile::Column::DeletedAt.is_null()),
            )
            .count(&state.db)
            .await?;

        if active_profile_count <= 1 {
            return Err(ApiError::conflict("Cannot delete your only profile"));
        }

        let mut active_model: profile::ActiveModel = existing.into();
        active_model.deleted_at = Set(Some(chrono::Utc::now().naive_utc()));
        active_model.updated_at = Set(chrono::Utc::now().naive_utc());
        active_model.update(&state.db).await?;
    }

    Ok(Json(()))
}

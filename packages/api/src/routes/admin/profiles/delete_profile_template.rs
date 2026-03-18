use crate::{
    audit, entity::template_profile, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::profile::Profile;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/admin/profiles/{profile_id}",
    tag = "admin",
    params(
        ("profile_id" = String, Path, description = "Profile template ID to delete")
    ),
    responses(
        (status = 200, description = "Deleted profile templates"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "DELETE /admin/profiles/{profile_id}", skip(state, user))]
pub async fn delete_profile_template(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(profile_id): Path<String>,
) -> Result<Json<Vec<Profile>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteBits)
        .await?;

    let audit_profile_id = profile_id.clone();
    let profiles = template_profile::Entity::delete_many()
        .filter(template_profile::Column::Id.eq(profile_id))
        .exec_with_returning(&state.db)
        .await?;

    let profiles: Vec<Profile> = profiles.into_iter().map(Profile::from).collect();

    audit!(
        state,
        user,
        "admin.profile.delete",
        "profile_template",
        audit_profile_id,
        "Profile template deleted"
    );
    Ok(Json(profiles))
}

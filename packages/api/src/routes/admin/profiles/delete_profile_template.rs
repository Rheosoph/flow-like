use crate::{
    audit,
    entity::{home_default, template_profile},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::profile::Profile;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::validation::validate_template_id;

fn check_template_can_be_deleted(has_home_default: bool) -> Result<(), ApiError> {
    if has_home_default {
        return Err(ApiError::conflict(
            "Remove this template's published home default in Home defaults before deleting the template. Profiles using that default will then follow the main default.",
        ));
    }
    Ok(())
}

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
    user.check_global_permission(&state, GlobalPermission::WriteProfile)
        .await?;
    validate_template_id(&profile_id)?;

    let audit_profile_id = profile_id.clone();
    let profiles = state
        .transaction(|txn| {
            let profile_id = profile_id.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "profile-template", &[&profile_id])
                    .await?;
                check_template_can_be_deleted(
                    home_default::Entity::find_by_id(&profile_id)
                        .one(txn)
                        .await?
                        .is_some(),
                )?;
                let profiles = template_profile::Entity::delete_many()
                    .filter(template_profile::Column::Id.eq(profile_id))
                    .exec_with_returning(txn)
                    .await?;
                Ok::<_, ApiError>(profiles)
            })
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_home_default_prevents_template_deletion_with_actionable_conflict() {
        let error = check_template_can_be_deleted(true).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
        assert!(error.public_message().unwrap().contains("Home defaults"));
        assert!(check_template_can_be_deleted(false).is_ok());
    }
}

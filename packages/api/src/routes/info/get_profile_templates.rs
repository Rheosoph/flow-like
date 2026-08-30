use crate::{entity::template_profile, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like::profile::Profile;
use sea_orm::EntityTrait;

#[tracing::instrument(name = "GET /info/profiles", skip(state, user))]
pub async fn get_profile_templates(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<Profile>>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    let profiles = template_profile::Entity::find().all(&state.db).await?;
    let profiles: Vec<Profile> = profiles.into_iter().map(Profile::from).collect();

    Ok(Json(profiles))
}

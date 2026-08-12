use crate::{entity::template_profile, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like::profile::{Profile, Settings};
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

impl From<template_profile::Model> for Profile {
    fn from(model: template_profile::Model) -> Self {
        let created_string = model.created_at.and_utc().to_rfc3339();
        let updated_string = model.updated_at.and_utc().to_rfc3339();
        Profile {
            id: model.id,
            name: model.name,
            description: model.description,
            icon: model.icon,
            apps: Some(vec![]),
            shortcuts: Some(vec![]),
            secure: model.secure,
            bits: model.bit_ids.unwrap_or_default(),
            custom_bits: vec![],
            hub: model.hub,
            hubs: model.hubs.unwrap_or_default(),
            interests: model.interests.unwrap_or_default(),
            settings: Settings::default(),
            tags: model.tags.unwrap_or_default(),
            theme: model.theme,
            thumbnail: model.thumbnail,
            created: created_string,
            updated: updated_string,
        }
    }
}

impl From<Profile> for template_profile::Model {
    fn from(profile: Profile) -> Self {
        template_profile::Model {
            id: profile.id,
            name: profile.name,
            description: profile.description,
            icon: profile.icon,
            secure: profile.secure,
            apps: None,
            bit_ids: Some(profile.bits),
            hub: profile.hub,
            hubs: Some(profile.hubs),
            interests: Some(profile.interests),
            settings: None,
            tags: Some(profile.tags),
            thumbnail: profile.thumbnail,
            theme: None,
            created_at: chrono::DateTime::parse_from_rfc3339(&profile.created)
                .unwrap_or_default()
                .naive_utc(),
            updated_at: chrono::DateTime::parse_from_rfc3339(&profile.updated)
                .unwrap_or_default()
                .naive_utc(),
        }
    }
}

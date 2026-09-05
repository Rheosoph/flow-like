use crate::{
    entity::template_profile, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::profile::Profile;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, to_value};

use super::validation::prepare_template;

#[tracing::instrument(
    name = "PUT /admin/profiles/{profile_id}",
    skip(state, user, profile_data)
)]
pub async fn upsert_profile_template(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(profile_id): Path<String>,
    Json(mut profile_data): Json<Profile>,
) -> Result<Json<Profile>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteProfile)
        .await?;
    prepare_template(&profile_id, &mut profile_data)?;

    state
        .transaction(|txn| {
            let profile_id = profile_id.clone();
            let profile_data = profile_data.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "profile-template", &[&profile_id])
                    .await?;

                let profile = template_profile::Entity::find()
                    .filter(template_profile::Column::Id.eq(profile_id))
                    .one(txn)
                    .await?;

                let apps: Option<Vec<Value>> = profile_data.apps.map(|apps| {
                    apps.into_iter()
                        .map(|app| to_value(app).unwrap_or(Value::Null))
                        .collect()
                });
                let settings = to_value(profile_data.settings)?;

                if let Some(existing_profile) = profile {
                    let apps = match apps {
                        Some(apps) => Some(Value::Array(apps)),
                        None => existing_profile.apps.clone(),
                    };
                    let mut updated_profile: template_profile::ActiveModel =
                        existing_profile.into();
                    updated_profile.name = Set(profile_data.name.clone());
                    updated_profile.description = Set(profile_data.description.clone());
                    updated_profile.icon = Set(profile_data.icon.clone());
                    updated_profile.bit_ids = Set(Some(profile_data.bits.clone().into()));
                    updated_profile.hub = Set(profile_data.hub.clone());
                    updated_profile.hubs = Set(Some(profile_data.hubs.clone().into()));
                    updated_profile.interests = Set(Some(profile_data.interests.clone().into()));
                    updated_profile.tags = Set(Some(profile_data.tags.clone().into()));
                    updated_profile.theme = Set(profile_data.theme.clone());
                    updated_profile.apps = Set(apps.clone());
                    updated_profile.thumbnail = Set(profile_data.thumbnail.clone());
                    updated_profile.settings = Set(Some(settings.clone()));
                    updated_profile.secure = Set(profile_data.secure);
                    updated_profile.updated_at = Set(chrono::Utc::now().fixed_offset());
                    let updated_profile = updated_profile.update(txn).await?;
                    return Ok(Json(Profile::from(updated_profile)));
                }

                let apps = apps.map(Value::Array);
                let new_profile = template_profile::ActiveModel {
                    id: Set(profile_data.id),
                    name: Set(profile_data.name),
                    description: Set(profile_data.description),
                    icon: Set(profile_data.icon),
                    secure: Set(profile_data.secure),
                    bit_ids: Set(Some(profile_data.bits.into())),
                    hub: Set(profile_data.hub),
                    hubs: Set(Some(profile_data.hubs.into())),
                    interests: Set(Some(profile_data.interests.into())),
                    settings: Set(Some(settings)),
                    tags: Set(Some(profile_data.tags.into())),
                    thumbnail: Set(profile_data.thumbnail),
                    apps: Set(apps),
                    theme: Set(profile_data.theme),
                    created_at: Set(chrono::Utc::now().fixed_offset()),
                    updated_at: Set(chrono::Utc::now().fixed_offset()),
                };

                let new_profile = new_profile.insert(txn).await?;

                Ok::<_, ApiError>(Json(Profile::from(new_profile)))
            })
        })
        .await
}

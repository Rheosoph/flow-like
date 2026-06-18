use crate::{
    entity::profile,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        profile::{
            delete_old_image, find_profile_for_user, generate_upload_url,
            is_profile_unique_violation,
        },
        user::ensure_user_exists,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::profile::{ProfileApp, ProfileShortcut, Settings};
use flow_like_types::{Value, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use serde_json::to_value;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ProfileBody {
    pub name: Option<String>,
    pub description: Option<String>,
    /// File extension for icon upload (e.g., "png", "jpg"). If set, server will generate a signed URL.
    pub icon_upload_ext: Option<String>,
    /// File extension for thumbnail upload (e.g., "png", "jpg"). If set, server will generate a signed URL.
    pub thumbnail_upload_ext: Option<String>,
    pub interests: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    pub theme: Option<Value>,
    pub bit_ids: Option<Vec<String>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub apps: Option<Vec<ProfileApp>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub shortcuts: Option<Vec<ProfileShortcut>>,
    pub hub: Option<String>,
    pub hubs: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    pub settings: Option<Settings>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpsertProfileResponse {
    #[schema(value_type = Object)]
    pub profile: profile::Model,
    /// Signed URL for uploading icon (if requested)
    pub icon_upload_url: Option<String>,
    /// Signed URL for uploading thumbnail (if requested)
    pub thumbnail_upload_url: Option<String>,
}

#[utoipa::path(
    post,
    path = "/profile/{profile_id}",
    tag = "profile",
    params(
        ("profile_id" = String, Path, description = "Profile ID to create or update")
    ),
    request_body = ProfileBody,
    responses(
        (status = 200, description = "Profile created or updated successfully", body = UpsertProfileResponse),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "POST /profile/{profile_id}", skip(state, user, profile_body))]
pub async fn upsert_profile(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(profile_id): Path<String>,
    Json(profile_body): Json<ProfileBody>,
) -> Result<Json<UpsertProfileResponse>, ApiError> {
    let sub = user.sub()?;
    if profile_id.trim().is_empty() {
        return Err(ApiError::bad_request("Profile ID is required"));
    }

    ensure_user_exists(&state, &sub).await?;
    let found_profile = find_profile_for_user(&state.db, &sub, &profile_id).await?;

    if let Some(found_profile) = found_profile {
        if found_profile.deleted_at.is_some() {
            return Err(ApiError::gone("Profile has been deleted"));
        }

        let mut active_model: profile::ActiveModel = found_profile.clone().into();

        if let Some(name) = profile_body.name {
            active_model.name = Set(name);
        }
        if let Some(description) = profile_body.description {
            active_model.description = Set(Some(description));
        }
        if let Some(interests) = profile_body.interests {
            active_model.interests = Set(Some(interests));
        }
        if let Some(tags) = profile_body.tags {
            active_model.tags = Set(Some(tags));
        }
        if let Some(theme) = profile_body.theme {
            active_model.theme = Set(Some(theme));
        }
        if let Some(bit_ids) = profile_body.bit_ids {
            active_model.bit_ids = Set(Some(bit_ids));
        }
        if let Some(apps) = profile_body.apps {
            let apps: Vec<Value> = apps.iter().map(|v| to_value(v).unwrap()).collect();
            let apps: Value = Value::Array(apps);
            active_model.apps = Set(Some(apps));
        }
        if let Some(shortcuts) = profile_body.shortcuts {
            let shortcuts: Vec<Value> = shortcuts.iter().map(|v| to_value(v).unwrap()).collect();
            let shortcuts: Value = Value::Array(shortcuts);
            active_model.shortcuts = Set(Some(shortcuts));
        }
        if let Some(settings) = profile_body.settings {
            let settings = to_value(&settings)?;
            active_model.settings = Set(Some(settings));
        }
        if let Some(hubs) = profile_body.hubs {
            active_model.hubs = Set(Some(hubs));
        }

        // Handle icon upload request
        let icon_upload_url = if let Some(ext) = &profile_body.icon_upload_ext {
            if let Some(old_icon_id) = &found_profile.icon {
                delete_old_image(&state, &sub, old_icon_id).await?;
            }
            let (upload_url, image_id) = generate_upload_url(&state, &sub, ext).await?;
            active_model.icon = Set(Some(image_id));
            Some(upload_url)
        } else {
            None
        };

        // Handle thumbnail upload request
        let thumbnail_upload_url = if let Some(ext) = &profile_body.thumbnail_upload_ext {
            if let Some(old_thumb_id) = &found_profile.thumbnail {
                delete_old_image(&state, &sub, old_thumb_id).await?;
            }
            let (upload_url, image_id) = generate_upload_url(&state, &sub, ext).await?;
            active_model.thumbnail = Set(Some(image_id));
            Some(upload_url)
        } else {
            None
        };

        active_model.updated_at = Set(chrono::Utc::now().naive_utc());

        let updated_profile = active_model.update(&state.db).await?;
        return Ok(Json(UpsertProfileResponse {
            profile: updated_profile,
            icon_upload_url,
            thumbnail_upload_url,
        }));
    }

    let ProfileBody {
        name,
        description,
        icon_upload_ext,
        thumbnail_upload_ext,
        interests,
        tags,
        theme,
        bit_ids,
        apps,
        shortcuts,
        hub,
        hubs,
        settings,
    } = profile_body;

    let apps = if let Some(apps) = apps {
        let apps: Vec<Value> = apps.iter().map(to_value).collect::<Result<_, _>>()?;
        Some(Value::Array(apps))
    } else {
        None
    };

    let settings = if let Some(settings) = settings {
        Some(to_value(&settings)?)
    } else {
        None
    };

    let shortcuts = if let Some(shortcuts) = shortcuts {
        let shortcuts: Vec<Value> = shortcuts.iter().map(to_value).collect::<Result<_, _>>()?;
        Some(Value::Array(shortcuts))
    } else {
        None
    };

    let hub = hub
        .or_else(|| hubs.as_ref().and_then(|h| h.first().cloned()))
        .unwrap_or_else(|| "https://api.flow-like.com".to_string());

    // Generate upload URLs for the new profile
    let (mut icon_upload_url, icon_id) = if let Some(ext) = &icon_upload_ext {
        let (url, img_id) = generate_upload_url(&state, &sub, ext).await?;
        (Some(url), Some(img_id))
    } else {
        (None, None)
    };

    let (mut thumbnail_upload_url, thumbnail_id) = if let Some(ext) = &thumbnail_upload_ext {
        let (url, img_id) = generate_upload_url(&state, &sub, ext).await?;
        (Some(url), Some(img_id))
    } else {
        (None, None)
    };

    let make_new_profile = |id: String| {
        let now = chrono::Utc::now().naive_utc();
        profile::ActiveModel {
            id: Set(id),
            user_id: Set(sub.clone()),
            name: Set(name.clone().unwrap_or_default()),
            description: Set(description.clone()),
            icon: Set(icon_id.clone()),
            thumbnail: Set(thumbnail_id.clone()),
            interests: Set(interests.clone()),
            tags: Set(tags.clone()),
            theme: Set(theme.clone()),
            bit_ids: Set(bit_ids.clone()),
            apps: Set(apps.clone()),
            shortcuts: Set(shortcuts.clone()),
            settings: Set(settings.clone()),
            hub: Set(hub.clone()),
            hubs: Set(hubs.clone()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
    };

    let created_profile = match make_new_profile(profile_id.clone()).insert(&state.db).await {
        Ok(created_profile) => created_profile,
        Err(e) if is_profile_unique_violation(&e) => {
            if let Some(existing) = find_profile_for_user(&state.db, &sub, &profile_id).await? {
                icon_upload_url = None;
                thumbnail_upload_url = None;
                existing
            } else {
                let fallback_profile_id = create_id();
                tracing::warn!(
                    "profile id '{}' hit a legacy global uniqueness constraint; creating '{}' for user '{}'",
                    profile_id,
                    fallback_profile_id,
                    sub
                );
                make_new_profile(fallback_profile_id)
                    .insert(&state.db)
                    .await?
            }
        }
        Err(e) => return Err(e.into()),
    };

    if created_profile.deleted_at.is_some() {
        return Err(ApiError::gone("Profile has been deleted"));
    }

    Ok(Json(UpsertProfileResponse {
        profile: created_profile,
        icon_upload_url,
        thumbnail_upload_url,
    }))
}

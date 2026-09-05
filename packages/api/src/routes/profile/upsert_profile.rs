use crate::{
    entity::profile,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        profile::{
            find_profile_for_user, is_profile_unique_violation,
            media::{
                cleanup_upload, finalize_upload, mutation_matches, normalize_image_extension,
                prepare_upload,
            },
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
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
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
    /// Complete an image allocated by a previous upload request.
    pub icon_upload_id: Option<String>,
    pub thumbnail_upload_id: Option<String>,
    pub interests: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "super::deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<Object>)]
    pub theme: Option<Option<Value>>,
    pub bit_ids: Option<Vec<String>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub apps: Option<Vec<ProfileApp>>,
    #[schema(value_type = Option<Vec<Object>>)]
    pub shortcuts: Option<Vec<ProfileShortcut>>,
    #[serde(
        default,
        deserialize_with = "super::deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<Object>)]
    pub home_layout: Option<Option<Value>>,
    #[serde(
        default,
        deserialize_with = "super::deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub home_default_id: Option<Option<String>>,
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
    pub icon_upload_id: Option<String>,
    pub thumbnail_upload_id: Option<String>,
    pub upload_pending: bool,
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
    Json(mut profile_body): Json<ProfileBody>,
) -> Result<Json<UpsertProfileResponse>, ApiError> {
    let sub = user.sub()?;
    if let Some(name) = &profile_body.name {
        profile_body.name = Some(super::validate_profile_name(name)?);
    }
    if (profile_body.icon_upload_ext.is_some() && profile_body.icon_upload_id.is_some())
        || (profile_body.thumbnail_upload_ext.is_some()
            && profile_body.thumbnail_upload_id.is_some())
    {
        return Err(ApiError::bad_request(
            "Prepare and complete an image upload in separate requests",
        ));
    }

    for extension in [
        &profile_body.icon_upload_ext,
        &profile_body.thumbnail_upload_ext,
    ]
    .into_iter()
    .flatten()
    {
        normalize_image_extension(extension)?;
    }
    super::validate_home_patch(&profile_body.home_layout, &profile_body.home_default_id)?;
    if profile_id.trim().is_empty() {
        return Err(ApiError::bad_request("Profile ID is required"));
    }

    ensure_user_exists(&state, &sub).await?;
    let found_profile = find_profile_for_user(&state.db, &sub, &profile_id).await?;

    if let Some(found_profile) = found_profile {
        if found_profile.deleted_at.is_some() {
            return Err(ApiError::gone("Profile has been deleted"));
        }

        let mut published_icon = None;
        let mut published_thumbnail = None;
        let mut upload_pending = false;
        if let Some(upload_id) = &profile_body.icon_upload_id {
            published_icon = finalize_upload(
                &state,
                &sub,
                upload_id,
                &format!("profile:{profile_id}:icon"),
                found_profile.icon.as_deref(),
            )
            .await?;
            upload_pending |= published_icon.is_none();
        }
        if let Some(upload_id) = &profile_body.thumbnail_upload_id {
            published_thumbnail = finalize_upload(
                &state,
                &sub,
                upload_id,
                &format!("profile:{profile_id}:thumbnail"),
                found_profile.thumbnail.as_deref(),
            )
            .await?;
            upload_pending |= published_thumbnail.is_none();
        }
        if upload_pending {
            return Ok(Json(UpsertProfileResponse {
                profile: found_profile,
                icon_upload_url: None,
                thumbnail_upload_url: None,
                icon_upload_id: None,
                thumbnail_upload_id: None,
                upload_pending: true,
            }));
        }
        let mut active_model: profile::ActiveModel = found_profile.clone().into();
        if let Some(icon) = published_icon {
            active_model.icon = Set(Some(icon));
        }
        if let Some(thumbnail) = published_thumbnail {
            active_model.thumbnail = Set(Some(thumbnail));
        }

        if let Some(name) = profile_body.name {
            active_model.name = Set(name);
        }
        if let Some(description) = profile_body.description {
            active_model.description = Set(Some(description));
        }
        if let Some(interests) = profile_body.interests {
            active_model.interests = Set(Some(interests.into()));
        }
        if let Some(tags) = profile_body.tags {
            active_model.tags = Set(Some(tags.into()));
        }
        if let Some(theme) = profile_body.theme {
            active_model.theme = Set(theme);
        }
        if let Some(bit_ids) = profile_body.bit_ids {
            active_model.bit_ids = Set(Some(bit_ids.into()));
        }
        if let Some(apps) = profile_body.apps {
            let apps: Vec<Value> = apps.iter().map(to_value).collect::<Result<_, _>>()?;
            let apps: Value = Value::Array(apps);
            active_model.apps = Set(Some(apps));
        }
        if let Some(shortcuts) = profile_body.shortcuts {
            let shortcuts: Vec<Value> = shortcuts.iter().map(to_value).collect::<Result<_, _>>()?;
            let shortcuts: Value = Value::Array(shortcuts);
            active_model.shortcuts = Set(Some(shortcuts));
        }
        if let Some(settings) = profile_body.settings {
            let settings = to_value(&settings)?;
            active_model.settings = Set(Some(settings));
        }
        if let Some(layout) = profile_body.home_layout {
            active_model.home_layout = Set(layout);
        }
        if let Some(default_id) = profile_body.home_default_id {
            active_model.home_default_id = Set(default_id);
        }
        if let Some(hubs) = profile_body.hubs {
            active_model.hubs = Set(Some(hubs.into()));
        }

        let (icon_upload_url, icon_upload_id) = if let Some(ext) = &profile_body.icon_upload_ext {
            let (url, id) = prepare_upload(
                &state,
                &sub,
                ext,
                &format!("profile:{profile_id}:icon"),
                found_profile.icon.as_deref(),
            )
            .await?;
            (Some(url), Some(id))
        } else {
            (None, None)
        };
        let (thumbnail_upload_url, thumbnail_upload_id) =
            if let Some(ext) = &profile_body.thumbnail_upload_ext {
                let (url, id) = prepare_upload(
                    &state,
                    &sub,
                    ext,
                    &format!("profile:{profile_id}:thumbnail"),
                    found_profile.thumbnail.as_deref(),
                )
                .await?;
                (Some(url), Some(id))
            } else {
                (None, None)
            };

        active_model.updated_at = Set(chrono::Utc::now().fixed_offset());

        let updated_profile = if profile_body.icon_upload_id.is_some()
            || profile_body.thumbnail_upload_id.is_some()
        {
            let intended = active_model.clone();
            let mut update = profile::Entity::update_many()
                .set(active_model)
                .filter(profile::Column::Id.eq(&profile_id))
                .filter(profile::Column::UserId.eq(&sub))
                .filter(profile::Column::DeletedAt.is_null());
            if profile_body.icon_upload_id.is_some() {
                update = update.filter(match &found_profile.icon {
                    Some(icon) => profile::Column::Icon.eq(icon),
                    None => profile::Column::Icon.is_null(),
                });
            }
            if profile_body.thumbnail_upload_id.is_some() {
                update = update.filter(match &found_profile.thumbnail {
                    Some(thumbnail) => profile::Column::Thumbnail.eq(thumbnail),
                    None => profile::Column::Thumbnail.is_null(),
                });
            }
            let result = update.exec(&state.db).await?;
            let latest = find_profile_for_user(&state.db, &sub, &profile_id)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if latest.deleted_at.is_some() {
                return Err(ApiError::gone("Profile has been deleted"));
            }
            if result.rows_affected != 1
                && !mutation_matches(
                    &intended,
                    &latest.clone().into(),
                    profile::Column::UpdatedAt,
                )
            {
                return Err(ApiError::conflict(
                    "The profile image changed during upload. Please try again",
                ));
            }
            latest
        } else {
            active_model.update(&state.db).await?
        };
        if let Some(upload_id) = &profile_body.icon_upload_id {
            cleanup_upload(&state, &sub, upload_id, found_profile.icon.as_deref()).await;
        }
        if let Some(upload_id) = &profile_body.thumbnail_upload_id {
            cleanup_upload(&state, &sub, upload_id, found_profile.thumbnail.as_deref()).await;
        }
        return Ok(Json(UpsertProfileResponse {
            profile: updated_profile,
            icon_upload_url,
            thumbnail_upload_url,
            icon_upload_id,
            thumbnail_upload_id,
            upload_pending: false,
        }));
    }

    if profile_body.icon_upload_id.is_some() || profile_body.thumbnail_upload_id.is_some() {
        return Err(ApiError::NOT_FOUND);
    }

    let ProfileBody {
        name,
        description,
        icon_upload_ext,
        thumbnail_upload_ext,
        icon_upload_id: _,
        thumbnail_upload_id: _,
        interests,
        tags,
        theme,
        bit_ids,
        apps,
        shortcuts,
        home_layout,
        home_default_id,
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

    let make_new_profile = |id: String| {
        let now = chrono::Utc::now().fixed_offset();
        profile::ActiveModel {
            id: Set(id),
            user_id: Set(sub.clone()),
            name: Set(name.clone().unwrap_or_else(|| "New Profile".to_string())),
            description: Set(description.clone()),
            icon: Set(None),
            thumbnail: Set(None),
            interests: Set(interests.clone().map(Into::into)),
            tags: Set(tags.clone().map(Into::into)),
            theme: Set(theme.clone().flatten()),
            bit_ids: Set(bit_ids.clone().map(Into::into)),
            apps: Set(apps.clone()),
            shortcuts: Set(shortcuts.clone()),
            home_layout: Set(home_layout.clone().flatten()),
            home_default_id: Set(home_default_id.clone().flatten()),
            settings: Set(settings.clone()),
            hub: Set(hub.clone()),
            hubs: Set(hubs.clone().map(Into::into)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
    };

    let created_profile = match make_new_profile(profile_id.clone()).insert(&state.db).await {
        Ok(created_profile) => created_profile,
        Err(e) if is_profile_unique_violation(&e) => {
            if let Some(existing) = find_profile_for_user(&state.db, &sub, &profile_id).await? {
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

    let (icon_upload_url, icon_upload_id) = if let Some(ext) = &icon_upload_ext {
        let (url, id) = prepare_upload(
            &state,
            &sub,
            ext,
            &format!("profile:{}:icon", created_profile.id),
            created_profile.icon.as_deref(),
        )
        .await?;
        (Some(url), Some(id))
    } else {
        (None, None)
    };
    let (thumbnail_upload_url, thumbnail_upload_id) = if let Some(ext) = &thumbnail_upload_ext {
        let (url, id) = prepare_upload(
            &state,
            &sub,
            ext,
            &format!("profile:{}:thumbnail", created_profile.id),
            created_profile.thumbnail.as_deref(),
        )
        .await?;
        (Some(url), Some(id))
    } else {
        (None, None)
    };

    Ok(Json(UpsertProfileResponse {
        profile: created_profile,
        icon_upload_url,
        thumbnail_upload_url,
        icon_upload_id,
        thumbnail_upload_id,
        upload_pending: false,
    }))
}

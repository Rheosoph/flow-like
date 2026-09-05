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
use axum::{Extension, Json, extract::State};
use flow_like::profile::{ProfileApp, ProfileShortcut, Settings};
use flow_like_types::{Value, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use serde_json::to_value;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SyncProfileRequest {
    pub id: String,
    pub name: String,
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
    pub hubs: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    pub settings: Option<Settings>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncProfileResponse {
    pub synced: Vec<String>,
    pub created: Vec<SyncedProfile>,
    pub updated: Vec<UpdatedProfile>,
    pub skipped: Vec<String>,
    /// IDs of profiles that were soft-deleted on the server (tombstones).
    /// Clients should delete these locally and stop syncing them.
    pub deleted: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncedProfile {
    pub local_id: String,
    pub server_id: String,
    /// Signed URL for uploading icon (if requested)
    pub icon_upload_url: Option<String>,
    /// Signed URL for uploading thumbnail (if requested)
    pub thumbnail_upload_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdatedProfile {
    pub id: String,
    /// Signed URL for uploading icon (if requested)
    pub icon_upload_url: Option<String>,
    /// Signed URL for uploading thumbnail (if requested)
    pub thumbnail_upload_url: Option<String>,
}

/// Sync multiple profiles from desktop to server
/// For existing profiles (matched by ID), updates if local is newer
/// For new profiles, creates with the client-provided ID and returns the mapping
/// Returns signed URLs for direct S3 upload when icon/thumbnail uploads are requested
#[utoipa::path(
    post,
    path = "/profile/sync",
    tag = "profile",
    request_body = Vec<SyncProfileRequest>,
    responses(
        (status = 200, description = "Profiles synced successfully", body = SyncProfileResponse),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "POST /profile/sync", skip(state, user, profiles))]
pub async fn sync_profiles(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(profiles): Json<Vec<SyncProfileRequest>>,
) -> Result<Json<SyncProfileResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;
    println!(
        "[ProfileSync] sync_profiles called by user={}, profile_count={}",
        sub,
        profiles.len()
    );
    for (i, p) in profiles.iter().enumerate() {
        println!(
            "[ProfileSync]   profile[{}]: id={}, name={}, icon_ext={:?}, thumb_ext={:?}",
            i, p.id, p.name, p.icon_upload_ext, p.thumbnail_upload_ext
        );
    }

    let mut created: Vec<SyncedProfile> = Vec::new();
    let mut updated: Vec<UpdatedProfile> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for profile_req in profiles {
        super::validate_home_patch(&profile_req.home_layout, &profile_req.home_default_id)?;
        if profile_req.id.trim().is_empty() {
            return Err(ApiError::bad_request("Profile ID is required"));
        }

        // Check if profile exists on server (including soft-deleted)
        let found_profile = find_profile_for_user(&state.db, &sub, &profile_req.id).await?;

        if let Some(existing) = found_profile {
            // If this profile was soft-deleted, tell the client to delete it locally
            if existing.deleted_at.is_some() {
                println!(
                    "[ProfileSync] Profile {} is soft-deleted, returning as tombstone",
                    profile_req.id
                );
                deleted.push(profile_req.id.clone());
                continue;
            }
            println!(
                "[ProfileSync] Profile {} found in DB, updated_at={}",
                profile_req.id, existing.updated_at
            );
            // Update existing profile metadata only if local is newer.
            // If the client sends no timestamp we cannot determine freshness → skip.
            let should_update = if let Some(local_updated) = &profile_req.updated_at {
                match chrono::DateTime::parse_from_rfc3339(local_updated) {
                    Ok(local_time) => local_time > existing.updated_at,
                    Err(_) => false,
                }
            } else {
                false
            };

            if should_update {
                println!("[ProfileSync] Updating profile {}", profile_req.id);
                let mut active_model: profile::ActiveModel = existing.clone().into();

                active_model.name = Set(profile_req.name.clone());
                active_model.description = Set(profile_req.description.clone());
                active_model.interests = Set(profile_req.interests.clone().map(Into::into));
                active_model.tags = Set(profile_req.tags.clone().map(Into::into));
                active_model.theme = Set(profile_req.theme.clone());
                if let Some(layout) = profile_req.home_layout.clone() {
                    active_model.home_layout = Set(layout);
                }
                if let Some(default_id) = profile_req.home_default_id.clone() {
                    active_model.home_default_id = Set(default_id);
                }
                active_model.bit_ids = Set(profile_req.bit_ids.clone().map(Into::into));

                if let Some(apps) = profile_req.apps {
                    active_model.apps = Set(Some(to_value(&apps)?));
                }

                if let Some(shortcuts) = profile_req.shortcuts {
                    active_model.shortcuts = Set(Some(to_value(&shortcuts)?));
                }

                if let Some(settings) = profile_req.settings {
                    let settings = to_value(&settings)?;
                    active_model.settings = Set(Some(settings));
                }

                active_model.hubs = Set(profile_req.hubs.clone().map(Into::into));

                // Handle icon upload request
                let icon_upload_url = if let Some(ext) = &profile_req.icon_upload_ext {
                    // Delete old icon if exists
                    if let Some(old_icon_id) = &existing.icon {
                        delete_old_image(&state, &sub, old_icon_id).await?;
                    }
                    let (upload_url, image_id) = generate_upload_url(&state, &sub, ext).await?;
                    active_model.icon = Set(Some(image_id));
                    Some(upload_url)
                } else {
                    None
                };

                // Handle thumbnail upload request
                let thumbnail_upload_url = if let Some(ext) = &profile_req.thumbnail_upload_ext {
                    // Delete old thumbnail if exists
                    if let Some(old_thumb_id) = &existing.thumbnail {
                        delete_old_image(&state, &sub, old_thumb_id).await?;
                    }
                    let (upload_url, image_id) = generate_upload_url(&state, &sub, ext).await?;
                    active_model.thumbnail = Set(Some(image_id));
                    Some(upload_url)
                } else {
                    None
                };

                active_model.updated_at = Set(chrono::Utc::now().fixed_offset());
                active_model.update(&state.db).await?;

                updated.push(UpdatedProfile {
                    id: profile_req.id.clone(),
                    icon_upload_url,
                    thumbnail_upload_url,
                });
            } else {
                // Timestamp says "do not update", but we may still need a one-time media backfill.
                // This happens when local profile points to a local image path while DB has no media id.
                let needs_icon_backfill =
                    profile_req.icon_upload_ext.is_some() && existing.icon.is_none();
                let needs_thumbnail_backfill =
                    profile_req.thumbnail_upload_ext.is_some() && existing.thumbnail.is_none();

                if needs_icon_backfill || needs_thumbnail_backfill {
                    println!(
                        "[ProfileSync] Backfilling media for profile {} (icon_missing={}, thumbnail_missing={})",
                        profile_req.id, needs_icon_backfill, needs_thumbnail_backfill
                    );

                    let mut active_model: profile::ActiveModel = existing.clone().into();

                    let icon_upload_url = if needs_icon_backfill {
                        if let Some(ext) = &profile_req.icon_upload_ext {
                            let (upload_url, image_id) =
                                generate_upload_url(&state, &sub, ext).await?;
                            active_model.icon = Set(Some(image_id));
                            Some(upload_url)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let thumbnail_upload_url = if needs_thumbnail_backfill {
                        if let Some(ext) = &profile_req.thumbnail_upload_ext {
                            let (upload_url, image_id) =
                                generate_upload_url(&state, &sub, ext).await?;
                            active_model.thumbnail = Set(Some(image_id));
                            Some(upload_url)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    active_model.updated_at = Set(chrono::Utc::now().fixed_offset());
                    active_model.update(&state.db).await?;

                    updated.push(UpdatedProfile {
                        id: profile_req.id.clone(),
                        icon_upload_url,
                        thumbnail_upload_url,
                    });
                } else {
                    skipped.push(profile_req.id.clone());
                }
            }
        } else {
            // Prefer the client-provided ID so sync stays idempotent, but fall
            // back to a fresh server ID if that global primary key is already
            // owned by another account.
            let mut server_id = profile_req.id.clone();
            println!(
                "[ProfileSync] Creating new profile: local_id={}, server_id={}",
                profile_req.id, server_id
            );

            let apps = if let Some(apps) = profile_req.apps {
                Some(to_value(&apps)?)
            } else {
                None
            };

            let shortcuts = if let Some(shortcuts) = profile_req.shortcuts {
                Some(to_value(&shortcuts)?)
            } else {
                None
            };

            let settings = if let Some(settings) = profile_req.settings {
                Some(to_value(&settings)?)
            } else {
                None
            };

            let default_hub = if state.platform_config.domain.is_empty() {
                "api.flow-like.com".to_string()
            } else {
                state.platform_config.domain.clone()
            };

            // Generate upload URLs for the new profile
            let (icon_upload_url, icon_id) = if let Some(ext) = &profile_req.icon_upload_ext {
                let (url, id) = generate_upload_url(&state, &sub, ext).await?;
                (Some(url), Some(id))
            } else {
                (None, None)
            };

            let (thumbnail_upload_url, thumbnail_id) =
                if let Some(ext) = &profile_req.thumbnail_upload_ext {
                    let (url, id) = generate_upload_url(&state, &sub, ext).await?;
                    (Some(url), Some(id))
                } else {
                    (None, None)
                };

            let mut created_server_id: Option<String> = None;
            let mut skipped_existing_local = false;

            for attempt in 0..4 {
                let now = chrono::Utc::now().fixed_offset();
                let new_profile = profile::ActiveModel {
                    id: Set(server_id.clone()),
                    user_id: Set(sub.clone()),
                    name: Set(profile_req.name.clone()),
                    description: Set(profile_req.description.clone()),
                    icon: Set(icon_id.clone()),
                    thumbnail: Set(thumbnail_id.clone()),
                    interests: Set(profile_req.interests.clone().map(Into::into)),
                    tags: Set(profile_req.tags.clone().map(Into::into)),
                    theme: Set(profile_req.theme.clone()),
                    bit_ids: Set(profile_req.bit_ids.clone().map(Into::into)),
                    apps: Set(apps.clone()),
                    shortcuts: Set(shortcuts.clone()),
                    home_layout: Set(profile_req.home_layout.clone().flatten()),
                    home_default_id: Set(profile_req.home_default_id.clone().flatten()),
                    settings: Set(settings.clone()),
                    hub: Set(default_hub.clone()),
                    hubs: Set(profile_req
                        .hubs
                        .clone()
                        .or(Some(vec![default_hub.clone()]))
                        .map(Into::into)),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                };

                match new_profile.insert(&state.db).await {
                    Ok(_) => {
                        created_server_id = Some(server_id.clone());
                        break;
                    }
                    Err(e) if is_profile_unique_violation(&e) => {}
                    Err(e) => return Err(e.into()),
                }

                if let Some(existing) = find_profile_for_user(&state.db, &sub, &server_id).await?
                    && server_id == profile_req.id
                {
                    if existing.deleted_at.is_some() {
                        deleted.push(profile_req.id.clone());
                    } else {
                        skipped.push(profile_req.id.clone());
                    }
                    skipped_existing_local = true;
                    break;
                }

                if attempt == 3 {
                    return Err(ApiError::conflict("Could not allocate a unique profile ID"));
                }

                if server_id == profile_req.id {
                    println!(
                        "[ProfileSync] Profile ID collision for local_id={} on a legacy global uniqueness constraint; retrying with a generated server id",
                        profile_req.id
                    );
                }

                server_id = create_id();
                println!(
                    "[ProfileSync] Profile ID collision for local_id={}, retrying with server_id={}",
                    profile_req.id, server_id
                );
            }

            if let Some(server_id) = created_server_id {
                created.push(SyncedProfile {
                    local_id: profile_req.id.clone(),
                    server_id,
                    icon_upload_url,
                    thumbnail_upload_url,
                });
            } else if !skipped_existing_local {
                return Err(ApiError::conflict("Could not allocate a unique profile ID"));
            }
        }
    }

    let synced: Vec<String> = created
        .iter()
        .map(|p| p.server_id.clone())
        .chain(updated.iter().map(|p| p.id.clone()))
        .collect();

    println!(
        "[ProfileSync] Done: created={}, updated={}, skipped={}, synced={}, deleted={}",
        created.len(),
        updated.len(),
        skipped.len(),
        synced.len(),
        deleted.len()
    );

    Ok(Json(SyncProfileResponse {
        synced,
        created,
        updated,
        skipped,
        deleted,
    }))
}

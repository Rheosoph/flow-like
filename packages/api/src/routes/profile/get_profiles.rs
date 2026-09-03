use crate::{
    entity::profile, error::ApiError, middleware::jwt::AppUser,
    routes::profile::sign_profile_image, state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;

/// Profile response with signed image URLs in icon/thumbnail fields.
/// Soft-deleted profiles include `deleted_at`; clients should remove them locally.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileResponse {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    /// Signed URL for icon (constructed from stored CUID)
    pub icon: Option<String>,
    /// Signed URL for thumbnail (constructed from stored CUID)
    pub thumbnail: Option<String>,
    pub interests: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    pub theme: Option<serde_json::Value>,
    pub bit_ids: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    pub apps: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub shortcuts: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub settings: Option<serde_json::Value>,
    pub hub: String,
    pub hubs: Option<Vec<String>>,
    #[schema(value_type = String)]
    pub created_at: chrono::NaiveDateTime,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
    /// Set for soft-deleted profiles (tombstones). Clients should delete these locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub deleted_at: Option<chrono::NaiveDateTime>,
}

/// Get all profiles for the authenticated user.
/// Includes soft-deleted profiles (with `deleted_at` set) so clients can clean up locally.
/// Tombstones older than 30 days are automatically hard-deleted.
#[utoipa::path(
    get,
    path = "/profile",
    tag = "profile",
    responses(
        (status = 200, description = "User profiles including tombstones", body = Vec<ProfileResponse>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /profile", skip(state, user))]
pub async fn get_profiles(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<ProfileResponse>>, ApiError> {
    let sub = user.sub()?;

    // Purge tombstones older than 30 days
    let cutoff = chrono::Utc::now().naive_utc() - chrono::Duration::days(30);
    profile::Entity::delete_many()
        .filter(
            profile::Column::UserId
                .eq(&sub)
                .and(profile::Column::DeletedAt.is_not_null())
                .and(profile::Column::DeletedAt.lt(cutoff)),
        )
        .exec(&state.db)
        .await?;

    let all_profiles = profile::Entity::find()
        .filter(profile::Column::UserId.eq(&sub))
        .all(&state.db)
        .await?;

    let mut result = Vec::with_capacity(all_profiles.len());

    for p in all_profiles {
        let is_deleted = p.deleted_at.is_some();

        // Skip signing images for tombstoned profiles
        let (icon, thumbnail) = if is_deleted {
            (None, None)
        } else {
            let icon = if let Some(icon_id) = &p.icon {
                sign_profile_image(&p.user_id, icon_id, &state).await.ok()
            } else {
                None
            };
            let thumbnail = if let Some(thumb_id) = &p.thumbnail {
                sign_profile_image(&p.user_id, thumb_id, &state).await.ok()
            } else {
                None
            };
            (icon, thumbnail)
        };

        result.push(ProfileResponse {
            id: p.id,
            user_id: p.user_id,
            name: p.name,
            description: p.description,
            icon,
            thumbnail,
            interests: p.interests,
            tags: p.tags,
            theme: p.theme,
            bit_ids: p.bit_ids,
            apps: p.apps,
            shortcuts: p.shortcuts,
            settings: p.settings,
            hub: p.hub,
            hubs: p.hubs,
            created_at: p.created_at,
            updated_at: p.updated_at,
            deleted_at: p.deleted_at,
        });
    }

    Ok(Json(result))
}

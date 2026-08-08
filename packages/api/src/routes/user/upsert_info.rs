use std::time::Duration;

use crate::{
    entity::user,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::user::{avatar_file_name, identity::sanitize_display_name},
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const ALLOWED_AVATAR_EXTENSIONS: &[&str] = &["webp", "png", "jpg", "jpeg", "gif", "avif"];

/// The extension is user input that ends up in a storage path, so it is matched
/// against an allowlist rather than sanitized.
fn normalize_avatar_extension(extension: &str) -> Result<&'static str, ApiError> {
    let lowered = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    ALLOWED_AVATAR_EXTENSIONS
        .iter()
        .find(|allowed| **allowed == lowered)
        .copied()
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "Unsupported avatar extension {:?}, expected one of {}",
                extension,
                ALLOWED_AVATAR_EXTENSIONS.join(", ")
            ))
        })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct UpsertInfoBody {
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_extension: Option<String>,
    pub accepted_terms_version: Option<String>,
    pub tutorial_completed: Option<bool>,
    pub dev_mode: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct UpsertInfoResponse {
    pub signed_url: Option<String>,
}

#[utoipa::path(
    put,
    path = "/user/info",
    tag = "user",
    request_body = UpsertInfoBody,
    responses(
        (status = 200, description = "User info updated successfully", body = UpsertInfoResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "PUT /user/info", skip_all)]
pub async fn upsert_info(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<UpsertInfoBody>,
) -> Result<Json<UpsertInfoResponse>, ApiError> {
    let sub = user.sub()?;

    let mut response = UpsertInfoResponse { signed_url: None };
    let info = user.user_info(&state).await?;

    let email = info.email.clone();
    let preferred_username = info.preferred_username.clone();

    let current_user = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut updated_user: user::ActiveModel = current_user.clone().into();

    if let Some(email) = email
        && current_user.email != Some(email.clone())
    {
        updated_user.email = Set(Some(email));
    }

    if let Some(preferred_username) = preferred_username
        && current_user.preferred_username != Some(preferred_username.clone())
    {
        updated_user.preferred_username = Set(Some(preferred_username));
    }

    if let Some(name) = payload.name {
        updated_user.name = Set(sanitize_display_name(&name));
    }
    if let Some(description) = payload.description {
        updated_user.description = Set(Some(description));
    }
    if let Some(avatar_extension) = payload.avatar_extension {
        let avatar_extension = normalize_avatar_extension(&avatar_extension)?;
        let master_store = state.master_credentials().await?;
        let master_store = master_store.to_store(false).await?;

        if let Some(avatar) = &current_user.avatar {
            let path = flow_like_storage::Path::from("media")
                .child("users")
                .child(sub.clone())
                .child(avatar_file_name(avatar));
            if let Err(err) = master_store.as_generic().delete(&path).await {
                tracing::error!("Failed to delete existing avatar at {}: {:?}", path, err);
            }
        }

        // The upload is signed for the real extension; the media-transformer
        // rewrites it to `.webp`, which is what the row points at.
        let id = create_id();
        updated_user.avatar = Set(Some(id.clone()));

        let path = flow_like_storage::Path::from("media")
            .child("users")
            .child(sub.clone())
            .child(format!("{}.{}", id, avatar_extension));
        let signed_url = master_store
            .sign("PUT", &path, Duration::from_secs(60 * 5))
            .await?;
        response.signed_url = Some(signed_url.to_string());
    }

    if let Some(accepted_terms_version) = payload.accepted_terms_version {
        updated_user.accepted_terms_version = Set(Some(accepted_terms_version));
    }

    if let Some(tutorial_completed) = payload.tutorial_completed {
        updated_user.tutorial_completed = Set(tutorial_completed);
    }
    if let Some(dev_mode) = payload.dev_mode {
        updated_user.dev_mode = Set(dev_mode);
    }
    updated_user.updated_at = Set(chrono::Utc::now().naive_utc());
    updated_user.update(&state.db).await?;

    Ok(Json(response))
}

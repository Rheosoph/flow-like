use crate::{
    entity::user,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        profile::media::{cleanup_upload, finalize_upload, mutation_matches, prepare_upload},
        user::identity::sanitize_display_name,
    },
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct UpsertInfoBody {
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_extension: Option<String>,
    /// Complete an uploaded image. The existing avatar remains until validation succeeds.
    pub avatar_upload_id: Option<String>,
    pub accepted_terms_version: Option<String>,
    pub tutorial_completed: Option<bool>,
    pub dev_mode: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct UpsertInfoResponse {
    pub signed_url: Option<String>,
    pub avatar_upload_id: Option<String>,
    pub upload_pending: bool,
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

    if payload.avatar_extension.is_some() && payload.avatar_upload_id.is_some() {
        return Err(ApiError::bad_request(
            "Prepare and complete an avatar upload in separate requests",
        ));
    }
    let name = payload
        .name
        .as_ref()
        .map(|name| {
            let name = sanitize_display_name(name)
                .ok_or_else(|| ApiError::bad_request("Display name is required"))?;
            Ok::<_, ApiError>(name)
        })
        .transpose()?;
    let mut response = UpsertInfoResponse::default();
    let current_user = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let published_avatar = if let Some(upload_id) = &payload.avatar_upload_id {
        match finalize_upload(
            &state,
            &sub,
            upload_id,
            "avatar",
            current_user.avatar.as_deref(),
        )
        .await?
        {
            Some(image) => Some(image),
            None => {
                return Ok(Json(UpsertInfoResponse {
                    upload_pending: true,
                    ..Default::default()
                }));
            }
        }
    } else {
        None
    };

    let info = user.user_info(&state).await?;

    let email = info.email.clone();
    let preferred_username = info.preferred_username.clone();

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

    if let Some(name) = name {
        updated_user.name = Set(Some(name));
    }
    if let Some(description) = payload.description {
        updated_user.description = Set(Some(description));
    }
    if let Some(extension) = payload.avatar_extension {
        let (url, id) = prepare_upload(
            &state,
            &sub,
            &extension,
            "avatar",
            current_user.avatar.as_deref(),
        )
        .await?;
        response.signed_url = Some(url);
        response.avatar_upload_id = Some(id);
    }
    if let Some(image) = published_avatar {
        updated_user.avatar = Set(Some(image));
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
    updated_user.updated_at = Set(chrono::Utc::now().fixed_offset());
    if payload.avatar_upload_id.is_some() {
        let expected_avatar = match &current_user.avatar {
            Some(avatar) => user::Column::Avatar.eq(avatar),
            None => user::Column::Avatar.is_null(),
        };
        let intended = updated_user.clone();
        let result = user::Entity::update_many()
            .set(updated_user)
            .filter(user::Column::Id.eq(&sub))
            .filter(expected_avatar)
            .exec(&state.db)
            .await?;
        if result.rows_affected != 1 {
            let latest = user::Entity::find_by_id(&sub)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if !mutation_matches(&intended, &latest.into(), user::Column::UpdatedAt) {
                return Err(ApiError::conflict(
                    "Your photo changed during upload. Please try again",
                ));
            }
        }
        if let Some(upload_id) = &payload.avatar_upload_id {
            cleanup_upload(&state, &sub, upload_id, current_user.avatar.as_deref()).await;
        }
    } else {
        updated_user.update(&state.db).await?;
    }

    Ok(Json(response))
}

use std::time::Duration;

use crate::{
    entity::meta,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::meta::{MediaItem, MediaQuery, MetaMode},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, TransactionTrait};
use utoipa::ToSchema;

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PushMediaResponse {
    pub signed_url: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/meta/media",
    tag = "meta",
    description = "Get a signed upload URL for metadata media.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("template_id" = Option<String>, Query, description = "Template ID"),
        ("course_id" = Option<String>, Query, description = "Course ID"),
        ("widget_id" = Option<String>, Query, description = "Widget ID"),
        ("group_id" = Option<String>, Query, description = "Suite (app group) ID"),
        ("item" = String, Query, description = "Media item: icon, thumbnail, preview"),
        ("extension" = String, Query, description = "File extension (e.g. png, webp)")
    ),
    responses(
        (status = 200, description = "Signed upload URL", body = PushMediaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/meta/media", skip(state, user, query))]
pub async fn push_media(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<MediaQuery>,
) -> Result<Json<PushMediaResponse>, ApiError> {
    let mode = MetaMode::from_media_query(&query, &app_id);
    mode.ensure_write_permission(&user, &app_id, &state).await?;
    let language = query.language.as_deref().unwrap_or("en");
    let media_prefix = mode.media_prefix(&app_id);

    let txn = state.db.begin().await?;

    let existing_meta = mode
        .find_existing_meta(language, &txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut existing_preview = existing_meta.preview_media.clone().unwrap_or_default();

    let mut model: meta::ActiveModel = existing_meta.clone().into();
    model.updated_at = Set(chrono::Utc::now().naive_utc());
    let item_id = create_id();
    let item_name = format!("{}.{}", item_id, query.extension);

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;

    match &query.item {
        MediaItem::Icon => {
            if let Some(icon) = &existing_meta.icon {
                let file_name = format!("{}.webp", icon);
                let path = media_prefix.child(file_name);
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!("Failed to delete existing icon at {}: {:?}", path, err);
                }
            }
            model.icon = Set(Some(item_id));
        }
        MediaItem::Thumbnail => {
            if let Some(thumbnail) = &existing_meta.thumbnail {
                let file_name = format!("{}.webp", thumbnail);
                let path = media_prefix.child(file_name);
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!("Failed to delete existing thumbnail at {}: {:?}", path, err);
                }
            }
            model.thumbnail = Set(Some(item_id));
        }
        MediaItem::Preview => {
            existing_preview.push(item_id.clone());
            model.preview_media = Set(Some(existing_preview));
        }
    }

    model.update(&txn).await?;
    let path = media_prefix.child(item_name.clone());
    let signed_url = master_store
        .sign("PUT", &path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| {
            let id = create_id();
            tracing::error!(
                "[{}] Failed to sign URL for media item '{}' - {:?}",
                id,
                item_name,
                e
            );
            ApiError::internal_error(anyhow!("Failed to create signed URL, reference ID: {}", id))
        })?;

    txn.commit().await?;
    Ok(Json(PushMediaResponse {
        signed_url: signed_url.to_string(),
    }))
}

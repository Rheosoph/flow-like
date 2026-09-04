use std::sync::Arc;

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
use sea_orm::{ActiveModelTrait, ActiveValue::Set};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/meta/media/{media_id}",
    tag = "meta",
    description = "Remove metadata media and delete the underlying file.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("media_id" = String, Path, description = "Media ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("template_id" = Option<String>, Query, description = "Template ID"),
        ("course_id" = Option<String>, Query, description = "Course ID"),
        ("widget_id" = Option<String>, Query, description = "Widget ID"),
        ("group_id" = Option<String>, Query, description = "Suite (app group) ID"),
        ("item" = String, Query, description = "Media item: icon, thumbnail, preview"),
        ("extension" = String, Query, description = "File extension")
    ),
    responses(
        (status = 200, description = "Media removed", body = ()),
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
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/meta/media/{media_id}",
    skip(state, user, query)
)]
pub async fn remove_media(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, media_id)): Path<(String, String)>,
    Query(query): Query<MediaQuery>,
) -> Result<Json<()>, ApiError> {
    let mode = MetaMode::from_media_query(&query, &app_id);
    mode.ensure_write_permission(&user, &app_id, &state).await?;
    let language = query.language.clone().unwrap_or_else(|| "en".to_string());
    let media_path = mode
        .media_prefix(&app_id)
        .child(format!("{}.webp", media_id));
    let mode = Arc::new(mode);
    let query = Arc::new(query);

    // The reference is dropped first; the file goes once the row no longer
    // points at it, so the S3 call never runs inside the transaction.
    state
        .transaction(|txn| {
            let mode = mode.clone();
            let query = query.clone();
            let language = language.clone();
            let media_id = media_id.clone();
            Box::pin(async move {
                let existing_meta = mode
                    .find_existing_meta(&language, txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;

                let mut model: meta::ActiveModel = existing_meta.clone().into();
                model.updated_at = Set(chrono::Utc::now().naive_utc());

                match &query.item {
                    MediaItem::Icon => {
                        if existing_meta.icon == Some(media_id) {
                            model.icon = Set(None);
                        }
                    }
                    MediaItem::Thumbnail => {
                        if existing_meta.thumbnail == Some(media_id) {
                            model.thumbnail = Set(None);
                        }
                    }
                    MediaItem::Preview => {
                        let mut existing_preview = existing_meta.preview_media.unwrap_or_default();
                        existing_preview.retain(|id| id != &media_id);
                        model.preview_media = Set(Some(existing_preview));
                    }
                }

                model.update(txn).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    if let Err(e) = master_store.as_generic().delete(&media_path).await {
        tracing::error!("Failed to delete media file at {}: {:?}", media_path, e);
        return Err(ApiError::internal_error(anyhow!(
            "Failed to delete media file, reference ID: {}",
            create_id()
        )));
    }

    Ok(Json(()))
}

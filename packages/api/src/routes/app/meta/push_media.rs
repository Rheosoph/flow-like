use std::{sync::Arc, time::Duration};

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
    let language = query.language.clone().unwrap_or_else(|| "en".to_string());
    let media_prefix = mode.media_prefix(&app_id);
    let item_id = create_id();
    let item_name = format!("{}.{}", item_id, query.extension);
    let mode = Arc::new(mode);
    let query = Arc::new(query);

    // The row points at the new media id once this commits; the old file is
    // removed and the upload URL signed only afterwards, so no S3 round trip
    // ever runs inside the transaction.
    let replaced = state
        .transaction(|txn| {
            let mode = mode.clone();
            let query = query.clone();
            let language = language.clone();
            let item_id = item_id.clone();
            Box::pin(async move {
                let existing_meta = mode
                    .find_existing_meta(&language, txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;

                let mut model: meta::ActiveModel = existing_meta.clone().into();
                model.updated_at = Set(chrono::Utc::now().fixed_offset());

                let replaced = match &query.item {
                    MediaItem::Icon => {
                        model.icon = Set(Some(item_id));
                        existing_meta.icon
                    }
                    MediaItem::Thumbnail => {
                        model.thumbnail = Set(Some(item_id));
                        existing_meta.thumbnail
                    }
                    MediaItem::Preview => {
                        let mut existing_preview = existing_meta.preview_media.unwrap_or_default();
                        existing_preview.push(item_id);
                        model.preview_media = Set(Some(existing_preview));
                        None
                    }
                };

                model.update(txn).await?;
                Ok::<_, ApiError>(replaced)
            })
        })
        .await?;

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;

    if let Some(replaced) = replaced {
        let path = media_prefix.child(format!("{}.webp", replaced));
        if let Err(err) = master_store.as_generic().delete(&path).await {
            tracing::error!("Failed to delete replaced media at {}: {:?}", path, err);
        }
    }

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

    Ok(Json(PushMediaResponse {
        signed_url: signed_url.to_string(),
    }))
}

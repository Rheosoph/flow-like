use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::{
        ensure_app_publicly_visible,
        meta::{MetaMode, MetaQuery},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::Metadata;
use flow_like_storage::Path as FlowPath;
use sea_orm::TransactionTrait;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/meta",
    tag = "meta",
    description = "Get metadata for an app, template, course, or widget. Public apps are readable without membership.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("template_id" = Option<String>, Query, description = "Template ID"),
        ("course_id" = Option<String>, Query, description = "Course ID"),
        ("widget_id" = Option<String>, Query, description = "Widget ID")
    ),
    responses(
        (status = 200, description = "Metadata", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "GET /apps/{app_id}/meta", skip(state, user, query))]
pub async fn get_meta(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<MetaQuery>,
    Path(app_id): Path<String>,
) -> Result<Json<Metadata>, ApiError> {
    let mode = MetaMode::new(&query, &app_id);

    if mode
        .ensure_read_permission(&user, &app_id, &state)
        .await
        .is_err()
    {
        if !state.platform_config.features.unauthorized_read {
            user.sub()?;
        }
        match mode {
            MetaMode::App(_) => {
                ensure_app_publicly_visible(&app_id, &state).await?;
            }
            _ => return Err(ApiError::FORBIDDEN),
        }
    }

    let language = query.language.clone().unwrap_or_else(|| "en".to_string());
    let txn = state.db.begin().await?;

    let existing_meta = mode
        .find_existing_meta(&language, &txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut metadata = Metadata::from(existing_meta.clone());

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;
    let prefix = FlowPath::from("media").child("apps").child(app_id);
    metadata.presign(prefix, &store).await;
    Ok(Json(metadata))
}

use std::sync::Arc;

use crate::{
    audit_branch,
    entity::meta,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::meta::{MetaMode, MetaQuery},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::create_id;
use sea_orm::ActiveModelTrait;

#[utoipa::path(
    put,
    path = "/apps/{app_id}/meta",
    tag = "meta",
    description = "Create or update metadata for an app, template, course, or widget.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("template_id" = Option<String>, Query, description = "Template ID"),
        ("course_id" = Option<String>, Query, description = "Course ID"),
        ("widget_id" = Option<String>, Query, description = "Widget ID"),
        ("group_id" = Option<String>, Query, description = "Suite (app group) ID")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Metadata saved", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/meta", skip(state, user, query, meta))]
pub async fn upsert_meta(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<MetaQuery>,
    Json(meta): Json<flow_like::bit::Metadata>,
) -> Result<Json<()>, ApiError> {
    let mode = MetaMode::new(&query, &app_id);
    mode.ensure_write_permission(&user, &app_id, &state).await?;

    let language = query.language.clone().unwrap_or_else(|| "en".to_string());
    let mut model = meta::Model::from(meta.clone());

    model.lang = language.clone();
    model.updated_at = chrono::Utc::now().fixed_offset();

    model.template_id = None;
    model.bit_id = None;
    model.app_id = None;
    model.course_id = None;
    model.widget_id = None;
    model.group_id = None;

    match &mode {
        MetaMode::Template(id) => {
            model.template_id = Some(id.clone());
        }
        MetaMode::App(id) => {
            model.app_id = Some(id.clone());
        }
        MetaMode::Course(id) => {
            model.course_id = Some(id.clone());
        }
        MetaMode::Widget(id) => {
            model.widget_id = Some(id.clone());
        }
        MetaMode::Group(id) => {
            model.group_id = Some(id.clone());
        }
    }

    let mode = Arc::new(mode);
    let new_meta_id = create_id();

    let created = state
        .transaction(|txn| {
            let mode = mode.clone();
            let language = language.clone();
            let mut model = model.clone();
            let new_meta_id = new_meta_id.clone();
            Box::pin(async move {
                let existing_meta = mode.find_existing_meta(&language, txn).await?;

                if let Some(existing) = existing_meta {
                    model.created_at = existing.created_at;
                    model.id = existing.id;
                    model.icon = existing.icon;
                    model.thumbnail = existing.thumbnail;
                    let active_model: meta::ActiveModel = model.into();
                    active_model.reset_all().update(txn).await?;
                    return Ok(false);
                }

                model.id = new_meta_id;
                model.created_at = chrono::Utc::now().fixed_offset();
                let active_model: meta::ActiveModel = model.into();
                active_model.insert(txn).await?;
                Ok::<_, ApiError>(true)
            })
        })
        .await?;

    let summary = if created {
        format!("Metadata created (lang={})", language)
    } else {
        format!("Metadata updated (lang={})", language)
    };
    audit_branch!(state, user, app_id, "meta.upsert", "meta", app_id, summary);
    Ok(Json(()))
}

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
use sea_orm::{ActiveModelTrait, TransactionTrait};

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
        ("widget_id" = Option<String>, Query, description = "Widget ID")
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
#[tracing::instrument(name = "PUT /apps/{app_id}/meta", skip(state, user))]
pub async fn upsert_meta(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<MetaQuery>,
    Json(meta): Json<flow_like::bit::Metadata>,
) -> Result<Json<()>, ApiError> {
    let mode = MetaMode::new(&query, &app_id);
    mode.ensure_write_permission(&user, &app_id, &state).await?;

    let language = query.language.as_deref().unwrap_or("en");
    let mut model = meta::Model::from(meta.clone());

    model.lang = language.to_string();
    model.updated_at = chrono::Utc::now().naive_utc();

    model.template_id = None;
    model.bit_id = None;
    model.app_id = None;
    model.course_id = None;
    model.widget_id = None;

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
    }

    let txn = state.db.begin().await?;

    let existing_meta = mode.find_existing_meta(language, &txn).await?;

    if let Some(existing) = existing_meta {
        model.created_at = existing.created_at;
        model.id = existing.id;
        model.icon = existing.icon;
        model.thumbnail = existing.thumbnail;
        let mut active_model: meta::ActiveModel = model.into();
        active_model = active_model.reset_all();
        active_model.update(&txn).await?;
        txn.commit().await?;
        audit_branch!(
            state,
            user,
            app_id,
            "meta.upsert",
            "meta",
            app_id,
            format!("Metadata updated (lang={})", language)
        );
        return Ok(Json(()));
    }

    model.id = create_id();
    model.created_at = chrono::Utc::now().naive_utc();
    let active_model: meta::ActiveModel = model.into();
    active_model.insert(&txn).await?;
    txn.commit().await?;
    audit_branch!(
        state,
        user,
        app_id,
        "meta.upsert",
        "meta",
        app_id,
        format!("Metadata created (lang={})", language)
    );
    Ok(Json(()))
}

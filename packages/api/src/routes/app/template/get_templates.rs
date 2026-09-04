use crate::{
    deletion::{DeletionRoot, job::not_pending_deletion},
    ensure_permission,
    entity::{meta, template},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::Metadata;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Select};

/// A template whose deletion job has not finished has already lost its board
/// and page payloads from object storage, so listing it would hand the caller
/// an unopenable ghost.
fn templates_not_deleting(app_id: &str) -> Select<template::Entity> {
    template::Entity::find()
        .filter(template::Column::AppId.eq(app_id))
        .filter(not_pending_deletion(
            DeletionRoot::Template,
            (template::Entity, template::Column::Id),
        ))
}

/// One template per entry as (template id, board id, localized metadata).
pub type TemplateListing = Vec<(String, String, Metadata)>;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/templates",
    tag = "templates",
    description = "List templates for an app with localized metadata.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("limit" = Option<u64>, Query, description = "Max results"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "Template list", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/templates", skip(state, user, query))]
pub async fn get_templates(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<TemplateListing>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadTemplates);

    let language = query.language.as_deref().unwrap_or("en");

    let templates_with_meta = templates_not_deleting(&app_id)
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut templates = Vec::new();

    for (template_model, meta_models) in templates_with_meta {
        if let Some(meta) = meta_models
            .iter()
            .find(|meta| meta.lang == language)
            .or_else(|| meta_models.iter().find(|meta| &meta.lang == "en"))
        {
            let mut metadata = Metadata::from(meta.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(template_model.app_id.clone());
            metadata.presign(prefix, &store).await;
            templates.push((app_id.clone(), template_model.id.clone(), metadata));
        }
    }

    Ok(Json(templates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn template_listing_skips_roots_with_an_unfinished_deletion_job() {
        let sql = templates_not_deleting("app_1")
            .into_query()
            .to_string(PostgresQueryBuilder);

        assert!(
            sql.contains("NOT EXISTS(SELECT 1 FROM \"DeletionJob\""),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootId" = "Template"."id""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootKind" = 'Template'"#),
            "{sql}"
        );
        assert!(sql.contains(r#""DeletionJob"."status" <> 'DONE'"#), "{sql}");
    }
}

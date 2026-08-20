use crate::{
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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

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

    let templates_with_meta = template::Entity::find()
        .find_with_related(meta::Entity)
        .filter(template::Column::AppId.eq(&app_id))
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

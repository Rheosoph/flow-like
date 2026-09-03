use crate::{
    ensure_permission,
    entity::{meta, widget},
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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// One widget per entry as (widget id, board id, localized metadata).
pub type WidgetListing = Vec<(String, String, Metadata)>;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/widgets",
    tag = "widgets",
    description = "List widgets for an app with localized metadata.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("limit" = Option<u64>, Query, description = "Max results"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "Widget list", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/widgets", skip(state, user, query))]
pub async fn get_widgets(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<WidgetListing>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadWidgets);

    let language = query.language.as_deref().unwrap_or("en");

    let widgets_with_meta = widget::Entity::find()
        .find_with_related(meta::Entity)
        .filter(widget::Column::AppId.eq(&app_id))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut widgets = Vec::new();

    for (widget_model, meta_models) in widgets_with_meta {
        if let Some(meta) = meta_models
            .iter()
            .find(|meta| meta.lang == language)
            .or_else(|| meta_models.iter().find(|meta| &meta.lang == "en"))
        {
            let mut metadata = Metadata::from(meta.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(widget_model.app_id.clone());
            metadata.presign(prefix, &store).await;
            widgets.push((app_id.clone(), widget_model.id.clone(), metadata));
        }
    }

    Ok(Json(widgets))
}

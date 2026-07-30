use crate::{
    entity::{app, membership, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{app::App, bit::Metadata};
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};

#[utoipa::path(
    get,
    path = "/apps",
    tag = "apps",
    params(
        ("language" = Option<String>, Query, description = "Language code (default: en)"),
        ("limit" = Option<u64>, Query, description = "Maximum number of results (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "List of user applications with metadata", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps", skip_all)]
pub async fn get_apps(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<(App, Option<Metadata>)>>, ApiError> {
    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    let limit = query.limit.unwrap_or(100).min(100);

    let sub = user.sub()?;

    let apps_with_meta = app::Entity::find()
        .order_by_desc(app::Column::UpdatedAt)
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .filter(membership::Column::UserId.eq(sub))
        .limit(Some(limit.min(100)))
        .offset(query.offset)
        .all(&state.db)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut apps = Vec::new();

    for (app_model, meta_models) in apps_with_meta {
        let metadata = if let Some(meta) = meta_models
            .iter()
            .find(|meta| meta.lang == language)
            .or_else(|| meta_models.first())
        {
            let mut metadata = Metadata::from(meta.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(app_model.id.clone());
            metadata.presign(prefix, &store).await;
            Some(metadata)
        } else {
            None
        };

        apps.push((App::from(app_model), metadata));
    }

    Ok(Json(apps))
}

use crate::{
    entity::{
        app, meta,
        sea_orm_active_enums::{Category, Visibility},
        template,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{app::AppCategory, bit::Metadata};
use sea_orm::sea_query::{Expr, ExprTrait, extension::postgres::PgExpr};
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Store-wide template search. Templates carry no visibility of their own —
/// `entity::template` has no visibility/price/author column — so access is
/// inherited from the owning app, exactly as the app store does it.
#[derive(Deserialize, Debug, Clone)]
pub struct TemplateSearchQuery {
    /// Free-text match against template name and description
    pub query: Option<String>,
    pub language: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    /// Filter by the owning app's category
    pub category: Option<AppCategory>,
    pub tag: Option<String>,
    /// Restrict to templates in apps whose owner allows forking
    pub forkable_only: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct TemplateSearchHit {
    pub app_id: String,
    pub template_id: String,
    pub version: Option<String>,
    pub metadata: Option<Metadata>,
    /// The owning app's name, so a hit is attributable without a second call
    pub app_name: Option<String>,
    pub app_allow_forking: bool,
    pub app_price: i64,
    pub rating_sum: i64,
    pub rating_count: i64,
}

#[utoipa::path(
    get,
    path = "/apps/templates/search",
    tag = "templates",
    description = "Search templates published in publicly visible apps. Returns template metadata only — call the template preview endpoint for a structural summary.",
    params(
        ("query" = Option<String>, Query, description = "Search query string"),
        ("language" = Option<String>, Query, description = "Language code (default: en)"),
        ("limit" = Option<u64>, Query, description = "Maximum number of results (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination"),
        ("category" = Option<String>, Query, description = "Filter by the owning app's category"),
        ("tag" = Option<String>, Query, description = "Filter by tag"),
        ("forkable_only" = Option<bool>, Query, description = "Only templates in apps that allow forking")
    ),
    responses(
        (status = 200, description = "Matching templates with metadata", body = Vec<TemplateSearchHit>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/templates/search", skip(state, user))]
pub async fn search_templates(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<TemplateSearchQuery>,
) -> Result<Json<Vec<TemplateSearchHit>>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    // Keyed on query + language only. Safe because the result set is
    // visibility-only and identity-independent, exactly like `search_apps`.
    let cache_key = format!("search_templates:{:?}:{}", query, language);
    if let Some(cached) = state.get_cache(&cache_key) {
        return Ok(Json(cached));
    }

    let limit = std::cmp::min(query.limit.unwrap_or(50), 100);

    let mut qb = template::Entity::find()
        .join(JoinType::InnerJoin, template::Relation::App.def())
        .filter(
            app::Column::Visibility
                .eq(Visibility::Public)
                .or(app::Column::Visibility.eq(Visibility::PublicRequestAccess)),
        )
        .order_by_desc(template::Column::UpdatedAt)
        .limit(Some(limit))
        .offset(query.offset);

    if query.forkable_only.unwrap_or(false) {
        qb = qb.filter(app::Column::AllowForking.eq(true));
    }

    if let Some(category) = query.category.clone() {
        let category: Category = category.into();
        qb = qb.filter(
            app::Column::PrimaryCategory
                .eq(category.clone())
                .or(app::Column::SecondaryCategory.eq(category)),
        );
    }

    if let Some(search_str) = query.query.clone() {
        qb = qb.filter(
            meta::Column::Description
                .contains(&search_str)
                .or(meta::Column::Name.contains(&search_str)),
        );
    }

    if let Some(tag) = query.tag.clone() {
        qb = qb.filter(
            meta::Column::Tags
                .into_expr()
                .contains(Expr::val(serde_json::json!([tag]))),
        );
    }

    qb = qb.filter(
        meta::Column::Lang
            .is_null()
            .or(meta::Column::Lang.eq(&language))
            .or(meta::Column::Lang.eq("en")),
    );

    let models = qb
        .find_with_related(meta::Entity)
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let app_ids: Vec<String> = models
        .iter()
        .map(|(template, _)| template.app_id.clone())
        .collect();

    let apps = app::Entity::find()
        .find_with_related(meta::Entity)
        .filter(app::Column::Id.is_in(app_ids))
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let mut hits = Vec::with_capacity(models.len());

    for (template_model, meta_models) in models {
        let owning_app = apps
            .iter()
            .find(|(app_model, _)| app_model.id == template_model.app_id);

        let Some((app_model, app_meta)) = owning_app else {
            // The inner join guarantees a row; skip rather than fail if a
            // concurrent delete raced us.
            continue;
        };

        let metadata = match pick_metadata(&meta_models, &language) {
            Some(meta) => {
                let mut metadata = Metadata::from(meta.clone());
                let prefix = flow_like_storage::Path::from("media")
                    .child("apps")
                    .child(template_model.app_id.clone());
                metadata.presign(prefix, &store).await;
                Some(metadata)
            }
            None => None,
        };

        hits.push(TemplateSearchHit {
            app_id: template_model.app_id.clone(),
            template_id: template_model.id.clone(),
            version: template_model.version.clone(),
            metadata,
            app_name: pick_metadata(app_meta, &language).map(|meta| meta.name.clone()),
            app_allow_forking: app_model.allow_forking,
            app_price: app_model.price,
            rating_sum: template_model.rating_sum,
            rating_count: template_model.rating_count,
        });
    }

    state.set_cache(cache_key.to_string(), &hits);

    Ok(Json(hits))
}

fn pick_metadata<'a>(metadata: &'a [meta::Model], language: &str) -> Option<&'a meta::Model> {
    metadata
        .iter()
        .find(|meta| meta.lang == language)
        .or_else(|| metadata.iter().find(|meta| meta.lang == "en"))
        .or_else(|| metadata.first())
}

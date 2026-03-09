use std::{sync::Arc, time::Duration};

use crate::{
    entity::{bit, llm_model, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::{Bit, LlmModelEvaluation, Metadata};
use flow_like_storage::files::store::FlowLikeStore;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    get,
    path = "/bit/{bit_id}",
    tag = "bit",
    params(
        ("bit_id" = String, Path, description = "Unique identifier of the bit"),
        ("language" = Option<String>, Query, description = "Language code for metadata")
    ),
    responses(
        (status = 200, description = "Bit retrieved successfully", body = Bit),
        (status = 404, description = "Bit not found")
    )
)]
#[tracing::instrument(name = "GET /bit/{bit_id}", skip(state, user))]
pub async fn get_bit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bit_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Bit>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    let language = query.language.as_deref().unwrap_or("en");

    let cache_key = format!("get_bit:{}:{}", bit_id, language);

    if let Some(cached) = state.get_cache(&cache_key) {
        return Ok(Json(cached));
    }

    let bit: Vec<(bit::Model, Vec<meta::Model>)> = bit::Entity::find_by_id(&bit_id)
        .find_with_related(meta::Entity)
        .filter(
            bit::Column::Id.eq(&bit_id).and(
                meta::Column::Lang
                    .eq(language)
                    .or(meta::Column::Lang.eq("en").or(meta::Column::Lang.is_null())),
            ),
        )
        .all(&state.db)
        .await?;

    let metadata: Option<Vec<meta::Model>> = bit.first().map(|(_, meta)| meta.clone());

    let bit = match bit.into_iter().next() {
        Some((bit, _)) => bit,
        None => return Err(ApiError::NOT_FOUND),
    };

    let mut bit: Bit = bit.into();

    for meta in metadata.unwrap_or_default() {
        bit.meta.insert(meta.lang.clone(), Metadata::from(meta));
    }

    if let Some(ref slug) = bit.model_slug.clone() {
        if let Ok(Some(model)) = llm_model::Entity::find_by_id(slug).one(&state.db).await {
            bit.model_evaluation = Some(LlmModelEvaluation {
                slug: model.slug,
                name: model.name,
                release_date: model.release_date.map(|d| d.to_string()),
                creator_name: model.creator_name,
                creator_slug: model.creator_slug,
                evaluations: model.evaluations,
                pricing: model.pricing,
                median_output_tokens_per_second: model.median_output_tokens_per_second,
                median_time_to_first_token_seconds: model.median_time_to_first_token_seconds,
                median_time_to_first_answer_token: model.median_time_to_first_answer_token,
            });
        }
    }

    if !state.platform_config.features.unauthorized_read {
        bit = temporary_bit(bit, &state.cdn_bucket).await?;
    }

    state.set_cache(cache_key, &bit);

    Ok(Json(bit))
}

#[tracing::instrument(name = "sign_bit", skip(bit, store))]
pub async fn temporary_bit(bit: Bit, store: &Arc<FlowLikeStore>) -> flow_like_types::Result<Bit> {
    let mut bit = bit;

    let name = match bit.download_link {
        Some(ref link) => link.split("bits/").last().unwrap_or(&bit.hash),
        None => return Ok(bit),
    };

    let path = flow_like_storage::object_store::path::Path::from("bits").child(name.to_string());
    let url = store
        .sign("GET", &path, Duration::from_secs(60 * 60 * 24))
        .await?;
    bit.download_link = Some(url.to_string());

    Ok(bit)
}

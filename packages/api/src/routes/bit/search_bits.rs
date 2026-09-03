use crate::{
    entity::{bit, llm_model, meta, sea_orm_active_enums::BitType},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{bit::Bit, hub::BitSearchQuery};
use sea_orm::QueryTrait;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

use super::{get_bit::temporary_bit, llm_model_to_evaluation};

#[utoipa::path(
    post,
    path = "/bit/search",
    tag = "bit",
    params(
        ("language" = Option<String>, Query, description = "Language code for metadata")
    ),
    request_body = BitSearchQuery,
    responses(
        (status = 200, description = "Search results", body = Vec<Bit>)
    )
)]
#[tracing::instrument(name = "POST /bit", skip(state, user, bit_query, lang_query))]
pub async fn search_bits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(lang_query): Query<LanguageParams>,
    Json(bit_query): Json<BitSearchQuery>,
) -> Result<Json<Vec<Bit>>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    let language = lang_query.language.as_deref().unwrap_or("en");

    let cache_key = format!("search_bits:v2:{:?}:{:?}", bit_query, language);

    if let Some(cached) = state.get_cache(&cache_key) {
        return Ok(Json(cached));
    }

    let limit = std::cmp::min(bit_query.limit.unwrap_or(50), 100);
    let mut qb = bit::Entity::find()
        .limit(Some(limit))
        .offset(bit_query.offset);

    if let Some(types) = bit_query.bit_types {
        let types: Vec<BitType> = types.into_iter().map(Into::into).collect();
        qb = qb.filter(bit::Column::Type.is_in(types));
    }

    // qb = qb.left_join(meta::Entity);

    if let Some(search_str) = bit_query.search.as_ref() {
        qb = qb.filter(
            bit::Column::Id.in_subquery(
                meta::Entity::find()
                    .select_only()
                    .column(meta::Column::BitId)
                    .filter(
                        meta::Column::Description
                            .contains(search_str)
                            .or(meta::Column::Name.contains(search_str)),
                    )
                    .into_query(),
            ),
        );
    }

    let rows: Vec<(bit::Model, Option<llm_model::Model>, Option<meta::Model>)> = qb
        .find_also_related(llm_model::Entity)
        .find_also_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .is_null()
                .or(meta::Column::Lang.eq(language))
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await
        .map_err(ApiError::from)?;

    let mut bit_positions = std::collections::HashMap::<String, usize>::new();
    let mut grouped = Vec::<(Bit, Vec<meta::Model>)>::new();

    for (bit_model, llm_model, meta_model) in rows {
        let bit_id = bit_model.id.clone();
        let position = match bit_positions.get(&bit_id) {
            Some(position) => *position,
            None => {
                let mut bit: Bit = Bit::from(bit_model);
                if let Some(llm_model) = llm_model {
                    bit.model_evaluation = Some(llm_model_to_evaluation(llm_model));
                }
                grouped.push((bit, Vec::new()));
                let position = grouped.len() - 1;
                bit_positions.insert(bit_id, position);
                position
            }
        };

        if let Some(meta_model) = meta_model {
            grouped[position].1.push(meta_model);
        }
    }

    let mut bits = grouped
        .into_iter()
        .map(|(mut bit, meta_models)| {
            if meta_models.len() == 1 {
                bit.meta
                    .insert(language.to_string(), meta_models[0].clone().into());
            }

            let requested_lang_or_en = meta_models
                .iter()
                .find(|meta| meta.lang == language)
                .or_else(|| meta_models.first())
                .cloned();

            if let Some(requested_lang_or_en) = requested_lang_or_en {
                bit.meta.insert(
                    requested_lang_or_en.lang.clone(),
                    requested_lang_or_en.into(),
                );
            }

            bit
        })
        .collect::<Vec<_>>();

    if !state.platform_config.features.unauthorized_read {
        for bit in bits.iter_mut() {
            *bit = temporary_bit(bit.clone(), &state.cdn_bucket).await?;
        }
    }

    state.set_cache(cache_key, &bits);

    Ok(Json(bits))
}

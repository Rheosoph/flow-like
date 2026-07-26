use crate::{
    entity::{bit, meta, profile},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::bit::{Bit, Metadata};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::routes::bit::get_bit::temporary_bit;

const MAX_BITS: u64 = 100;

#[utoipa::path(
    get,
    path = "/profile/{profile_id}/bits",
    tag = "profile",
    params(
        ("profile_id" = String, Path, description = "Profile ID"),
        ("language" = Option<String>, Query, description = "Language code for metadata"),
        ("limit" = Option<u64>, Query, description = "Max items to return (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination"),
    ),
    responses(
        (status = 200, description = "Resolved bits in the profile", body = Vec<Bit>),
        (status = 404, description = "Profile not found")
    )
)]
#[tracing::instrument(name = "GET /profile/{profile_id}/bits", skip(state, user))]
pub async fn get_profile_bits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(profile_id): Path<String>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<Bit>>, ApiError> {
    let sub = user.sub()?;
    let language = query.language.as_deref().unwrap_or("en");
    let limit = std::cmp::min(query.limit.unwrap_or(MAX_BITS), MAX_BITS);
    let offset = query.offset.unwrap_or(0);

    let cache_key = format!(
        "profile_bits:{}:{}:{}:{}:{}",
        sub, profile_id, language, limit, offset
    );

    if let Some(cached) = state.get_cache::<Vec<Bit>>(&cache_key) {
        return Ok(Json(cached));
    }

    let profile = profile::Entity::find()
        .filter(
            profile::Column::Id
                .eq(&profile_id)
                .and(profile::Column::UserId.eq(&sub))
                .and(profile::Column::DeletedAt.is_null()),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let bit_ids = profile.bit_ids.unwrap_or_default();

    // Custom bits this profile activated (without provider secrets — this
    // response feeds UI lists). The full library lives at GET /user/bits.
    let custom_bits =
        crate::routes::user::bits::load_custom_bits_for_profile(&state, &sub, &bit_ids, false)
            .await
            .unwrap_or_default();

    if bit_ids.is_empty() {
        state.set_cache(cache_key, &custom_bits);
        return Ok(Json(custom_bits));
    }

    let raw_ids: Vec<&str> = bit_ids
        .iter()
        .map(|id| id.rsplit_once(':').map_or(id.as_str(), |(_, raw)| raw))
        .collect();

    let paginated: Vec<&str> = raw_ids
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .copied()
        .collect();

    if paginated.is_empty() {
        state.set_cache(cache_key, &custom_bits);
        return Ok(Json(custom_bits));
    }

    let models = bit::Entity::find()
        .filter(bit::Column::Id.is_in(paginated))
        .filter(
            meta::Column::Lang
                .is_null()
                .or(meta::Column::Lang.eq(language))
                .or(meta::Column::Lang.eq("en")),
        )
        .find_with_related(meta::Entity)
        .all(&state.db)
        .await?;

    let mut bits: Vec<Bit> = models
        .into_iter()
        .map(|(bit_model, meta_models)| {
            let mut bit: Bit = Bit::from(bit_model);
            let best = meta_models
                .iter()
                .find(|m| m.lang == language)
                .or_else(|| meta_models.first())
                .cloned();
            if let Some(m) = best {
                bit.meta.insert(m.lang.clone(), Metadata::from(m));
            }
            bit
        })
        .collect();

    if !state.platform_config.features.unauthorized_read {
        for bit in bits.iter_mut() {
            *bit = temporary_bit(bit.clone(), &state.cdn_bucket).await?;
        }
    }

    bits.extend(custom_bits);

    state.set_cache(cache_key, &bits);

    Ok(Json(bits))
}

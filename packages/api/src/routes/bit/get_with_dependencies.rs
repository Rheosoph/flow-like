use crate::{
    entity::{bit, bit_cache, bit_tree_cache},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{bit::Bit, utils::http::HTTPClient};
use flow_like_types::create_id;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, sea_query::OnConflict};
use serde_json::from_value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use super::get_bit::temporary_bit;

type ArtifactResolutionKey = (String, Option<String>, String, Option<String>);

fn artifact_resolution_key(bit: &Bit) -> ArtifactResolutionKey {
    let tree_identity = (!bit.dependencies.is_empty()).then(|| bit.dependency_tree_hash.clone());
    (
        bit.hash.clone(),
        bit.file_name.clone(),
        format!("{:?}", bit.bit_type),
        tree_identity,
    )
}

fn is_dependency_leaf(dependencies: Option<&[String]>) -> bool {
    dependencies.is_none_or(|dependencies| dependencies.is_empty())
}

#[utoipa::path(
    get,
    path = "/bit/{bit_id}/dependencies",
    tag = "bit",
    params(
        ("bit_id" = String, Path, description = "Unique identifier of the bit")
    ),
    responses(
        (status = 200, description = "Bit with all dependencies retrieved successfully", body = Vec<Bit>),
        (status = 404, description = "Bit not found")
    )
)]
#[tracing::instrument(name = "GET /bit/{bit_id}/dependencies", skip(state, user))]
pub async fn get_with_dependencies(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bit_id): Path<String>,
) -> Result<Json<Vec<Bit>>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    let cache_key = format!("get_with_dependencies:{}", bit_id);
    if let Some(cached) = state.get_cache(&cache_key) {
        return Ok(Json(cached));
    }

    let bit_model = bit::Entity::find_by_id(&bit_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Use the declared graph, not a hash equality heuristic. Legacy downloadable
    // leaves use `dependency_tree_hash == hash`, while path-aware identities do not.
    if is_dependency_leaf(bit_model.dependencies.as_deref()) {
        let mut bit: Bit = bit_model.into();
        if !state.platform_config.features.unauthorized_read {
            bit = temporary_bit(bit, &state.cdn_bucket).await?;
        }
        return Ok(Json(vec![bit]));
    }

    let cached_bits = bit_cache::Entity::find()
        .filter(bit_cache::Column::DependencyTreeHash.eq(bit_model.dependency_tree_hash.clone()))
        .find_also_related(bit::Entity)
        .all(&state.db)
        .await?;

    if !cached_bits.is_empty() {
        let mut bits: Vec<Bit> = Vec::with_capacity(cached_bits.len());

        for (cache, bit) in cached_bits {
            if let Some(bit) = bit {
                let mut bit: Bit = bit.into();
                if !state.platform_config.features.unauthorized_read {
                    bit = temporary_bit(bit, &state.cdn_bucket).await?;
                }
                bits.push(bit);
            } else if let Some(external_bit) = cache.external_bit {
                let bit = match from_value(external_bit) {
                    Ok(bit) => bit,
                    Err(e) => {
                        tracing::error!("Failed to deserialize external bit: {}", e);
                        continue;
                    }
                };

                bits.push(bit);
            }
        }

        return Ok(Json(bits));
    }

    // Capture original dependency_tree_hash before conversion (it might be None in DB)
    let original_dependency_tree_hash = bit_model.dependency_tree_hash.clone();
    let converted_bit: Bit = bit_model.into();
    let bits = fetch_dependencies(&converted_bit, &state).await?;

    // Only insert into cache if the bit has a real dependency_tree_hash in the database
    if let Some(ref dep_hash) = original_dependency_tree_hash {
        bit_tree_cache::Entity::insert(bit_tree_cache::ActiveModel {
            dependency_tree_hash: Set(dep_hash.clone()),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        })
        .on_conflict(
            OnConflict::column(bit_tree_cache::Column::DependencyTreeHash)
                .update_column(bit_tree_cache::Column::UpdatedAt)
                .to_owned(),
        )
        .exec_with_returning(&state.db)
        .await?;

        insert_bit_cache(&state, &bits, dep_hash).await?;
    }

    state.set_cache(cache_key, &bits);

    Ok(Json(bits))
}

#[tracing::instrument(name = "recursive_fetch_dependencies", skip(bit, state))]
async fn fetch_dependencies(bit: &Bit, state: &AppState) -> flow_like_types::Result<Vec<Bit>> {
    let mut bits = vec![bit.clone()];
    let self_domain = state.platform_config.domain.clone();
    let http_client = HTTPClient::new_without_refetch();
    let http_client = Arc::new(http_client);
    let mut hubs: HashMap<String, flow_like::hub::Hub> = HashMap::new();

    let mut seen_artifacts = HashSet::from([artifact_resolution_key(bit)]);

    let mut recursion_guard = HashSet::from([format!("{}:{}", bit.hub, bit.id)]);
    let mut new_dependencies = bit.dependencies.clone();

    while !new_dependencies.is_empty() {
        let mut next_dependencies = Vec::new();
        for dependency in &new_dependencies {
            let (hub, id) = dependency.split_once(':').ok_or_else(|| {
                flow_like_types::Error::msg(format!("Invalid dependency format: {}", dependency))
            })?;

            if !recursion_guard.insert(dependency.clone()) {
                continue;
            }

            if hub == self_domain {
                let own_bit = fetch_own_bit(id, state).await?;
                if !own_bit.dependencies.is_empty() {
                    next_dependencies.extend(own_bit.dependencies.clone());
                }
                if seen_artifacts.insert(artifact_resolution_key(&own_bit)) {
                    bits.push(own_bit);
                }
                continue;
            }

            let hub = match hubs.get(hub) {
                Some(hub) => hub.clone(),
                None => {
                    let new_hub = match flow_like::hub::Hub::new(hub, http_client.clone()).await {
                        Ok(hub) => hub,
                        Err(e) => {
                            tracing::error!("Failed to create hub for {}: {}", hub, e);
                            continue;
                        }
                    };
                    hubs.insert(hub.to_string(), new_hub.clone());
                    new_hub
                }
            };

            let bit = match hub.get_bit(id).await {
                Ok(bit) => bit,
                Err(e) => {
                    tracing::error!("Failed to fetch dependency {}: {}", dependency, e);
                    continue;
                }
            };

            if !bit.dependencies.is_empty() {
                next_dependencies.extend(bit.dependencies.clone());
            }

            if seen_artifacts.insert(artifact_resolution_key(&bit)) {
                bits.push(bit)
            }
        }
        new_dependencies = next_dependencies;
    }

    Ok(bits)
}

async fn fetch_own_bit(bit_id: &str, state: &AppState) -> flow_like_types::Result<Bit> {
    let bit = bit::Entity::find_by_id(bit_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| flow_like_types::Error::msg("Bit not found"))?;

    let mut bit: Bit = bit.into();
    if !state.platform_config.features.unauthorized_read {
        bit = temporary_bit(bit, &state.cdn_bucket).await?;
    }

    Ok(bit)
}

async fn insert_bit_cache(
    state: &AppState,
    bits: &[Bit],
    dependency_tree_hash: &str,
) -> flow_like_types::Result<()> {
    let domain = state.platform_config.domain.clone();
    let mut cache_entries = Vec::with_capacity(bits.len());
    for bit in bits {
        let external_bit = serde_json::to_value(bit).ok();

        let mut cache_entry = bit_cache::ActiveModel {
            dependency_tree_hash: Set(dependency_tree_hash.to_string()),
            external_bit: Set(external_bit),
            id: Set(create_id()),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            bit_id: Set(None),
            ..Default::default()
        };

        if bit.hub == domain {
            cache_entry.bit_id = Set(Some(bit.id.clone()));
            cache_entry.external_bit = Set(None);
        }

        cache_entries.push(cache_entry);
    }

    if !cache_entries.is_empty() {
        bit_cache::Entity::insert_many(cache_entries)
            .exec(&state.db)
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(hash: &str, file_name: &str) -> Bit {
        Bit {
            hash: hash.to_string(),
            file_name: Some(file_name.to_string()),
            ..Bit::default()
        }
    }

    #[test]
    fn dependency_resolution_preserves_equal_content_at_distinct_paths() {
        let root_config = artifact("same-content", "config.json");
        let nested_config = artifact("same-content", "nested/config.json");
        let mut seen = HashSet::new();

        assert!(seen.insert(artifact_resolution_key(&root_config)));
        assert!(seen.insert(artifact_resolution_key(&nested_config)));
    }

    #[test]
    fn dependency_resolution_still_deduplicates_the_same_artifact() {
        let first = artifact("same-content", "config.json");
        let duplicate = artifact("same-content", "config.json");
        let mut seen = HashSet::new();

        assert!(seen.insert(artifact_resolution_key(&first)));
        assert!(!seen.insert(artifact_resolution_key(&duplicate)));
    }

    #[test]
    fn dependency_resolution_preserves_distinct_composite_trees() {
        let mut first = artifact("same-content", "bundle.bin");
        first.dependencies = vec!["hub:first".to_string()];
        first.dependency_tree_hash = "first-tree".to_string();

        let mut second = artifact("same-content", "bundle.bin");
        second.dependencies = vec!["hub:second".to_string()];
        second.dependency_tree_hash = "second-tree".to_string();

        assert_ne!(
            artifact_resolution_key(&first),
            artifact_resolution_key(&second)
        );
    }

    #[test]
    fn dependency_resolution_preserves_distinct_bit_types() {
        let file = artifact("same-content", "config.json");
        let mut config = file.clone();
        config.bit_type = flow_like::bit::BitTypes::Config;

        assert_ne!(
            artifact_resolution_key(&file),
            artifact_resolution_key(&config)
        );
    }

    #[test]
    fn legacy_content_hash_leaves_remain_directly_resolvable() {
        let mut legacy = artifact("legacy-content-hash", "model.bin");
        legacy.dependency_tree_hash = legacy.hash.clone();

        assert!(is_dependency_leaf(Some(&legacy.dependencies)));

        legacy.dependency_tree_hash = "path-aware-v2-identity".to_string();
        assert!(is_dependency_leaf(Some(&legacy.dependencies)));
    }
}

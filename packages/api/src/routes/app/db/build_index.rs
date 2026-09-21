use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_write_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub enum IndexType {
    #[serde(
        alias = "full_text",
        alias = "FULL TEXT",
        alias = "FTS",
        alias = "INVERTED"
    )]
    FullText,
    #[serde(alias = "btree", alias = "BTREE")]
    BTree,
    #[serde(alias = "bitmap", alias = "BITMAP")]
    Bitmap,
    #[serde(alias = "label_list", alias = "LABEL LIST")]
    LabelList,
    #[serde(alias = "auto", alias = "AUTO")]
    Auto,
    #[serde(alias = "vector", alias = "VECTOR")]
    Vector,
    #[serde(alias = "fm", alias = "FM")]
    Fm,
    #[serde(alias = "ngram", alias = "NGRAM")]
    NGram,
    #[serde(alias = "zonemap", alias = "zone_map", alias = "ZONEMAP")]
    ZoneMap,
    #[serde(alias = "bloomfilter", alias = "bloom_filter", alias = "BLOOMFILTER")]
    BloomFilter,
    #[serde(alias = "rtree", alias = "r_tree", alias = "RTREE")]
    RTree,
    #[serde(alias = "ivf_flat", alias = "IVF_FLAT")]
    IvfFlat,
    #[serde(alias = "ivf_pq", alias = "IVF_PQ")]
    IvfPq,
    #[serde(alias = "ivf_sq", alias = "IVF_SQ")]
    IvfSq,
    #[serde(alias = "ivf_rq", alias = "IVF_RQ")]
    IvfRq,
    #[serde(alias = "ivf_hnsw_flat", alias = "IVF_HNSW_FLAT")]
    IvfHnswFlat,
    #[serde(alias = "ivf_hnsw_pq", alias = "IVF_HNSW_PQ")]
    IvfHnswPq,
    #[serde(alias = "ivf_hnsw_sq", alias = "IVF_HNSW_SQ")]
    IvfHnswSq,
}

impl std::fmt::Display for IndexType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            IndexType::FullText => "FULL TEXT",
            IndexType::BTree => "BTREE",
            IndexType::Bitmap => "BITMAP",
            IndexType::LabelList => "LABEL LIST",
            IndexType::Auto => "AUTO",
            IndexType::Vector => "VECTOR",
            IndexType::Fm => "FM",
            IndexType::NGram => "NGRAM",
            IndexType::ZoneMap => "ZONEMAP",
            IndexType::BloomFilter => "BLOOMFILTER",
            IndexType::RTree => "RTREE",
            IndexType::IvfFlat => "IVF_FLAT",
            IndexType::IvfPq => "IVF_PQ",
            IndexType::IvfSq => "IVF_SQ",
            IndexType::IvfRq => "IVF_RQ",
            IndexType::IvfHnswFlat => "IVF_HNSW_FLAT",
            IndexType::IvfHnswPq => "IVF_HNSW_PQ",
            IndexType::IvfHnswSq => "IVF_HNSW_SQ",
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct BuildIndexPayload {
    pub column: String,
    pub index_type: IndexType,
    pub optimize: bool,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/index",
    tag = "database",
    description = "Create an index for a table column.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = BuildIndexPayload,
    responses(
        (status = 200, description = "Index built", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/db/{table}/index",
    skip(state, user, scope, payload)
)]
pub async fn build_index(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<BuildIndexPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;
    super::validate_writable_selector(&selector)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;

    db.index(&payload.column, Some(&payload.index_type.to_string()))
        .await?;

    if payload.optimize {
        db.optimize(true).await?;
    }

    Ok(Json(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_value, json};

    #[test]
    fn existing_index_requests_keep_their_storage_behavior() {
        for (index_type, storage_type) in [
            ("FullText", "FULL TEXT"),
            ("BTree", "BTREE"),
            ("Bitmap", "BITMAP"),
            ("LabelList", "LABEL LIST"),
            ("Auto", "AUTO"),
        ] {
            let payload: BuildIndexPayload = from_value(json!({
                "column": "value",
                "index_type": index_type,
                "optimize": true
            }))
            .unwrap();

            assert_eq!(payload.column, "value");
            assert_eq!(payload.index_type.to_string(), storage_type);
            assert!(payload.optimize);
        }
    }

    #[test]
    fn explicit_index_types_and_workflow_labels_deserialize() {
        for (api_name, storage_type) in [
            ("Vector", "VECTOR"),
            ("Fm", "FM"),
            ("NGram", "NGRAM"),
            ("ZoneMap", "ZONEMAP"),
            ("BloomFilter", "BLOOMFILTER"),
            ("RTree", "RTREE"),
            ("IvfFlat", "IVF_FLAT"),
            ("IvfPq", "IVF_PQ"),
            ("IvfSq", "IVF_SQ"),
            ("IvfRq", "IVF_RQ"),
            ("IvfHnswFlat", "IVF_HNSW_FLAT"),
            ("IvfHnswPq", "IVF_HNSW_PQ"),
            ("IvfHnswSq", "IVF_HNSW_SQ"),
        ] {
            for name in [
                api_name.to_string(),
                storage_type.to_string(),
                storage_type.to_lowercase(),
            ] {
                let index_type: IndexType = from_value(json!(name)).unwrap();
                assert_eq!(index_type.to_string(), storage_type);
            }
        }
    }
}

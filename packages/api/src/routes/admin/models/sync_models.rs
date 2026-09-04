use crate::{
    db::insert_in_chunks, entity::llm_model, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ActiveValue::Set, sea_query::OnConflict};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct ModelCreator {
    pub name: String,
    pub slug: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct SyncModelEntry {
    pub slug: String,
    pub name: String,
    pub release_date: Option<String>,
    pub model_creator: ModelCreator,
    pub evaluations: Option<Value>,
    pub pricing: Option<Value>,
    pub median_output_tokens_per_second: Option<f64>,
    pub median_time_to_first_token_seconds: Option<f64>,
    pub median_time_to_first_answer_token: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct SyncModelsRequest {
    pub data: Vec<SyncModelEntry>,
}

/// Rows per upsert transaction; the evaluation and pricing JSON keep a chunk
/// well inside the per-transaction size budget.
const SYNC_CHUNK: usize = 500;

fn upsert_by_slug() -> OnConflict {
    OnConflict::column(llm_model::Column::Slug)
        .update_columns([
            llm_model::Column::Name,
            llm_model::Column::ReleaseDate,
            llm_model::Column::CreatorName,
            llm_model::Column::CreatorSlug,
            llm_model::Column::Evaluations,
            llm_model::Column::Pricing,
            llm_model::Column::MedianOutputTokensPerSecond,
            llm_model::Column::MedianTimeToFirstTokenSeconds,
            llm_model::Column::MedianTimeToFirstAnswerToken,
            llm_model::Column::UpdatedAt,
        ])
        .to_owned()
}

#[utoipa::path(
    post,
    path = "/admin/models/sync",
    tag = "admin",
    description = "Bulk upsert LLM models from an Artificial Analysis-compatible payload.",
    request_body = SyncModelsRequest,
    responses(
        (status = 200, description = "Models synced successfully"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "POST /admin/models/sync", skip(state, user, body))]
pub async fn sync_models(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<SyncModelsRequest>,
) -> Result<Json<usize>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteBits)
        .await?;

    let now = chrono::Utc::now().fixed_offset();

    let models: Vec<llm_model::ActiveModel> = body
        .data
        .into_iter()
        .map(|entry| {
            let release_date = entry
                .release_date
                .as_deref()
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().fixed_offset());

            llm_model::ActiveModel {
                slug: Set(entry.slug),
                name: Set(entry.name),
                release_date: Set(release_date),
                creator_name: Set(entry.model_creator.name),
                creator_slug: Set(entry.model_creator.slug),
                evaluations: Set(entry.evaluations),
                pricing: Set(entry.pricing),
                median_output_tokens_per_second: Set(entry.median_output_tokens_per_second),
                median_time_to_first_token_seconds: Set(entry.median_time_to_first_token_seconds),
                median_time_to_first_answer_token: Set(entry.median_time_to_first_answer_token),
                created_at: Set(now),
                updated_at: Set(now),
            }
        })
        .collect();

    let count = models.len();

    insert_in_chunks(
        &state.db,
        state.db_dialect,
        models,
        SYNC_CHUNK,
        Some(upsert_by_slug()),
    )
    .await?;

    Ok(Json(count))
}

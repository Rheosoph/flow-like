use crate::{
    audit, entity::llm_model, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveValue::Set, EntityTrait, sea_query::OnConflict};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct UpsertModelRequest {
    pub name: String,
    pub release_date: Option<String>,
    pub creator_name: String,
    pub creator_slug: String,
    pub evaluations: Option<Value>,
    pub pricing: Option<Value>,
    pub median_output_tokens_per_second: Option<f64>,
    pub median_time_to_first_token_seconds: Option<f64>,
    pub median_time_to_first_answer_token: Option<f64>,
}

#[utoipa::path(
    put,
    path = "/admin/models/{slug}",
    tag = "admin",
    description = "Upsert an LLM model entry by its unique slug.",
    params(("slug" = String, Path, description = "Unique model slug")),
    request_body = UpsertModelRequest,
    responses(
        (status = 200, description = "Model upserted successfully"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "PUT /admin/models/{slug}", skip(state, user, body))]
pub async fn upsert_model(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(slug): Path<String>,
    Json(body): Json<UpsertModelRequest>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteBits)
        .await?;

    let release_date = body
        .release_date
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().fixed_offset());

    let now = chrono::Utc::now().fixed_offset();
    let audit_slug = slug.clone();
    let audit_name = body.name.clone();

    llm_model::Entity::insert(llm_model::ActiveModel {
        slug: Set(slug),
        name: Set(body.name),
        release_date: Set(release_date),
        creator_name: Set(body.creator_name),
        creator_slug: Set(body.creator_slug),
        evaluations: Set(body.evaluations),
        pricing: Set(body.pricing),
        median_output_tokens_per_second: Set(body.median_output_tokens_per_second),
        median_time_to_first_token_seconds: Set(body.median_time_to_first_token_seconds),
        median_time_to_first_answer_token: Set(body.median_time_to_first_answer_token),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .on_conflict(
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
            .to_owned(),
    )
    .exec(&state.db)
    .await?;

    audit!(
        state,
        user,
        "admin.model.upsert",
        "llm_model",
        audit_slug,
        format!("LLM model upserted: {}", audit_name)
    );
    Ok(Json(()))
}

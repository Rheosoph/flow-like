use crate::{
    entity::{embedding_usage_tracking, execution_usage_tracking, llm_usage_tracking, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_page_size")]
    pub page_size: u64,
    pub app_id: Option<String>,
}

fn default_page() -> u64 {
    0
}
fn default_page_size() -> u64 {
    50
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

// -- LLM Usage History --

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LlmUsageRecord {
    pub id: String,
    pub model_id: String,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub token_in: i64,
    pub token_out: i64,
    pub latency: Option<f64>,
    pub app_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub price: i64,
    pub created_at: String,
}

#[utoipa::path(
    get,
    path = "/usage/llm",
    tag = "usage",
    params(PaginationParams),
    responses(
        (status = 200, description = "LLM usage history", body = PaginatedResponse<LlmUsageRecord>)
    )
)]
#[tracing::instrument(name = "GET /usage/llm", skip(state, user))]
pub async fn get_llm_history(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<PaginatedResponse<LlmUsageRecord>>, ApiError> {
    let sub = user.sub()?;
    let page_size = params.page_size.min(100);

    let mut query =
        llm_usage_tracking::Entity::find().filter(llm_usage_tracking::Column::UserId.eq(&sub));

    if let Some(ref app_id) = params.app_id {
        query = query.filter(llm_usage_tracking::Column::AppId.eq(app_id));
    }

    let total = query
        .clone()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let records = query
        .order_by_desc(llm_usage_tracking::Column::CreatedAt)
        .paginate(&state.db, page_size)
        .fetch_page(params.page)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let items: Vec<LlmUsageRecord> = records
        .into_iter()
        .map(|r| LlmUsageRecord {
            id: r.id,
            model_id: r.model_id,
            provider: r.provider,
            endpoint: r.endpoint,
            token_in: r.token_in,
            token_out: r.token_out,
            latency: r.latency,
            app_id: r.app_id,
            technical_user_id: r.technical_user_id,
            price: r.price,
            created_at: r.created_at.and_utc().to_rfc3339(),
        })
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        total,
        page: params.page,
        page_size,
    }))
}

// -- Embedding Usage History --

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EmbeddingUsageRecord {
    pub id: String,
    pub model_id: String,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub token_count: i64,
    pub latency: Option<f64>,
    pub app_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub price: i64,
    pub created_at: String,
}

#[utoipa::path(
    get,
    path = "/usage/embeddings",
    tag = "usage",
    params(PaginationParams),
    responses(
        (status = 200, description = "Embedding usage history", body = PaginatedResponse<EmbeddingUsageRecord>)
    )
)]
#[tracing::instrument(name = "GET /usage/embeddings", skip(state, user))]
pub async fn get_embedding_history(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<PaginatedResponse<EmbeddingUsageRecord>>, ApiError> {
    let sub = user.sub()?;
    let page_size = params.page_size.min(100);

    let mut query = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::UserId.eq(&sub));

    if let Some(ref app_id) = params.app_id {
        query = query.filter(embedding_usage_tracking::Column::AppId.eq(app_id));
    }

    let total = query
        .clone()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let records = query
        .order_by_desc(embedding_usage_tracking::Column::CreatedAt)
        .paginate(&state.db, page_size)
        .fetch_page(params.page)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let items: Vec<EmbeddingUsageRecord> = records
        .into_iter()
        .map(|r| EmbeddingUsageRecord {
            id: r.id,
            model_id: r.model_id,
            provider: r.provider,
            endpoint: r.endpoint,
            token_count: r.token_count,
            latency: r.latency,
            app_id: r.app_id,
            technical_user_id: r.technical_user_id,
            price: r.price,
            created_at: r.created_at.and_utc().to_rfc3339(),
        })
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        total,
        page: params.page,
        page_size,
    }))
}

// -- Execution Usage History --

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ExecutionUsageRecord {
    pub id: String,
    pub instance: Option<String>,
    pub board_id: String,
    pub node_id: String,
    pub version: String,
    pub microseconds: i64,
    pub status: String,
    pub app_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub created_at: String,
}

#[utoipa::path(
    get,
    path = "/usage/executions",
    tag = "usage",
    params(PaginationParams),
    responses(
        (status = 200, description = "Execution usage history", body = PaginatedResponse<ExecutionUsageRecord>)
    )
)]
#[tracing::instrument(name = "GET /usage/executions", skip(state, user))]
pub async fn get_execution_history(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<PaginatedResponse<ExecutionUsageRecord>>, ApiError> {
    let sub = user.sub()?;
    let page_size = params.page_size.min(100);

    let mut query = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::UserId.eq(&sub));

    if let Some(ref app_id) = params.app_id {
        query = query.filter(execution_usage_tracking::Column::AppId.eq(app_id));
    }

    let total = query
        .clone()
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let records = query
        .order_by_desc(execution_usage_tracking::Column::CreatedAt)
        .paginate(&state.db, page_size)
        .fetch_page(params.page)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let items: Vec<ExecutionUsageRecord> = records
        .into_iter()
        .map(|r| ExecutionUsageRecord {
            id: r.id,
            instance: r.instance,
            board_id: r.board_id,
            node_id: r.node_id,
            version: r.version,
            microseconds: r.microseconds,
            status: format!("{:?}", r.status),
            app_id: r.app_id,
            technical_user_id: r.technical_user_id,
            created_at: r.created_at.and_utc().to_rfc3339(),
        })
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        total,
        page: params.page,
        page_size,
    }))
}

// -- Usage Summary --

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UsageSummary {
    pub total_llm_price: i64,
    pub total_embedding_price: i64,
    pub total_llm_invocations: u64,
    pub total_embedding_invocations: u64,
    pub total_executions: u64,
}

#[utoipa::path(
    get,
    path = "/usage/summary",
    tag = "usage",
    responses(
        (status = 200, description = "Usage summary", body = UsageSummary)
    )
)]
#[tracing::instrument(name = "GET /usage/summary", skip(state, user))]
pub async fn get_usage_summary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<UsageSummary>, ApiError> {
    let sub = user.sub()?;

    let user_record = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    let llm_count = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::UserId.eq(&sub))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let embedding_count = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::UserId.eq(&sub))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    let execution_count = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::UserId.eq(&sub))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(e.into()))?;

    Ok(Json(UsageSummary {
        total_llm_price: user_record.total_llm_price,
        total_embedding_price: user_record.total_embedding_price,
        total_llm_invocations: llm_count,
        total_embedding_invocations: embedding_count,
        total_executions: execution_count,
    }))
}

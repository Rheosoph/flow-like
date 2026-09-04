//! List sink service tokens
//!
//! GET /admin/sinks
//!
//! Lists all registered sink tokens with their status.

use crate::{
    entity::sink_token, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListTokensQuery {
    /// Filter by sink type
    pub sink_type: Option<String>,
    /// Include revoked tokens (default: false)
    #[serde(default)]
    pub include_revoked: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SinkTokenInfo {
    pub jti: String,
    pub sink_type: String,
    pub name: Option<String>,
    pub revoked: bool,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListTokensResponse {
    pub tokens: Vec<SinkTokenInfo>,
    pub total: usize,
}

#[utoipa::path(
    get,
    path = "/admin/sinks",
    tag = "admin",
    params(ListTokensQuery),
    responses(
        (status = 200, description = "List of sink tokens", body = ListTokensResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/sinks", skip_all)]
pub async fn list_tokens(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListTokensQuery>,
) -> Result<Json<ListTokensResponse>, ApiError> {
    // Require admin permission
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let mut db_query = sink_token::Entity::find();

    // Filter by sink type if provided
    if let Some(ref sink_type) = query.sink_type {
        db_query = db_query.filter(sink_token::Column::SinkType.eq(sink_type));
    }

    // Filter out revoked unless requested
    if !query.include_revoked {
        db_query = db_query.filter(sink_token::Column::Revoked.eq(false));
    }

    let tokens = db_query
        .order_by_desc(sink_token::Column::CreatedAt)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let token_infos: Vec<SinkTokenInfo> = tokens
        .into_iter()
        .map(|t| SinkTokenInfo {
            jti: t.id,
            sink_type: t.sink_type,
            name: t.name,
            revoked: t.revoked,
            revoked_at: t.revoked_at.map(|dt| dt.to_rfc3339()),
            revoked_by: t.revoked_by,
            created_at: t.created_at.to_rfc3339(),
        })
        .collect();

    let total = token_infos.len();

    Ok(Json(ListTokensResponse {
        tokens: token_infos,
        total,
    }))
}

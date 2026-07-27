use crate::{
    entity::{app_purchase, profile, sea_orm_active_enums::PurchaseStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::overview::verify_sales_access;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PurchasesQuery {
    /// Filter by status
    pub status: Option<String>,
    /// Offset for pagination
    #[serde(default)]
    pub offset: u64,
    /// Limit for pagination (max 100)
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_limit() -> u64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseItem {
    pub id: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub user_avatar: Option<String>,
    pub price_paid: i64,
    pub original_price: i64,
    pub discount_amount: i64,
    pub discount_id: Option<String>,
    pub currency: String,
    pub status: String,
    pub completed_at: Option<String>,
    pub refunded_at: Option<String>,
    pub refund_reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PurchasesResponse {
    pub purchases: Vec<PurchaseItem>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// GET /apps/{app_id}/sales/purchases - List purchases for an app
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/purchases",
    tag = "sales",
    description = "List purchases for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("offset" = u64, Query, description = "Result offset"),
        ("limit" = u64, Query, description = "Max results (max 100)")
    ),
    responses(
        (status = 200, description = "Purchases list", body = PurchasesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/sales/purchases", skip(state, user, query))]
pub async fn list_purchases(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<PurchasesQuery>,
) -> Result<Json<PurchasesResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let limit = query.limit.min(100);

    let mut query_builder =
        app_purchase::Entity::find().filter(app_purchase::Column::AppId.eq(&app_id));

    // Filter by status if provided
    if let Some(status_str) = &query.status {
        let status = match status_str.to_uppercase().as_str() {
            "PENDING" => Some(PurchaseStatus::Pending),
            "COMPLETED" => Some(PurchaseStatus::Completed),
            "REFUNDED" => Some(PurchaseStatus::Refunded),
            "PARTIALLY_REFUNDED" => Some(PurchaseStatus::PartiallyRefunded),
            "FAILED" => Some(PurchaseStatus::Failed),
            _ => None,
        };
        if let Some(s) = status {
            query_builder = query_builder.filter(app_purchase::Column::Status.eq(s));
        }
    }

    let total = query_builder.clone().count(&state.db).await?;

    let purchases: Vec<app_purchase::Model> = query_builder
        .order_by_desc(app_purchase::Column::CreatedAt)
        .offset(query.offset)
        .limit(limit)
        .all(&state.db)
        .await?;

    // Fetch user profiles for display
    let user_ids: Vec<_> = purchases.iter().map(|p| p.user_id.clone()).collect();
    let profiles = if !user_ids.is_empty() {
        profile::Entity::find()
            .filter(
                profile::Column::UserId
                    .is_in(user_ids)
                    .and(profile::Column::DeletedAt.is_null()),
            )
            .all(&state.db)
            .await?
    } else {
        vec![]
    };

    let profile_map: std::collections::HashMap<_, _> = profiles
        .into_iter()
        .map(|p| (p.user_id.clone(), p))
        .collect();

    let purchase_items: Vec<PurchaseItem> = purchases
        .into_iter()
        .map(|p| {
            let profile = profile_map.get(&p.user_id);
            PurchaseItem {
                id: p.id,
                user_id: p.user_id.clone(),
                user_name: profile.map(|pr| pr.name.clone()),
                user_avatar: profile.and_then(|pr| pr.thumbnail.clone()),
                price_paid: p.price_paid,
                original_price: p.original_price,
                discount_amount: p.discount_amount,
                discount_id: p.discount_id,
                currency: p.currency,
                status: format!("{:?}", p.status),
                completed_at: p.completed_at.map(|d: chrono::NaiveDateTime| d.to_string()),
                refunded_at: p.refunded_at.map(|d: chrono::NaiveDateTime| d.to_string()),
                refund_reason: p.refund_reason,
                created_at: p.created_at.to_string(),
            }
        })
        .collect();

    Ok(Json(PurchasesResponse {
        purchases: purchase_items,
        total,
        offset: query.offset,
        limit,
    }))
}

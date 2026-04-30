use std::collections::HashMap;

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like::hub::UserTier;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TierInfo {
    pub name: String,
    pub product_id: Option<String>,
    pub max_non_visible_projects: i32,
    pub max_remote_executions: i32,
    pub execution_tier: String,
    pub max_total_size: i64,
    pub max_llm_cost: i32,
    pub max_llm_calls: Option<i32>,
    pub llm_tiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<PriceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PriceInfo {
    pub amount: i64,
    pub currency: String,
    pub interval: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PricingResponse {
    pub current_tier: String,
    pub tiers: HashMap<String, TierInfo>,
}

impl From<(&str, &UserTier)> for TierInfo {
    fn from((name, tier): (&str, &UserTier)) -> Self {
        Self {
            name: name.to_string(),
            product_id: tier.product_id.clone(),
            max_non_visible_projects: tier.max_non_visible_projects,
            max_remote_executions: tier.max_remote_executions,
            execution_tier: tier.execution_tier.clone(),
            max_total_size: tier.max_total_size,
            max_llm_cost: tier.max_llm_cost,
            max_llm_calls: tier.max_llm_calls,
            llm_tiers: tier.llm_tiers.clone(),
            price: None,
            contact_url: None,
        }
    }
}

const ENTERPRISE_TIER: &str = "ENTERPRISE";
const ENTERPRISE_CONTACT_URL: &str = "mailto:enterprise@flow-like.com";

#[utoipa::path(
    get,
    path = "/user/pricing",
    tag = "user",
    responses(
        (status = 200, description = "Returns pricing information for all tiers", body = PricingResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/pricing", skip(state, user))]
pub async fn get_pricing(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<PricingResponse>, ApiError> {
    let db_user = user.get_user(&state).await?;

    let current_tier = match db_user.tier {
        crate::entity::sea_orm_active_enums::UserTier::Free => "FREE",
        crate::entity::sea_orm_active_enums::UserTier::Premium => "PREMIUM",
        crate::entity::sea_orm_active_enums::UserTier::Pro => "PRO",
        crate::entity::sea_orm_active_enums::UserTier::Enterprise => "ENTERPRISE",
    };

    let mut tiers: HashMap<String, TierInfo> = HashMap::new();

    for (tier_name, tier_config) in &state.platform_config.tiers {
        let mut tier_info = TierInfo::from((tier_name.as_str(), tier_config));

        if tier_name.eq_ignore_ascii_case(ENTERPRISE_TIER) {
            tier_info.product_id = None;
            tier_info.price = None;
            tier_info.contact_url = Some(ENTERPRISE_CONTACT_URL.to_string());
            tiers.insert(tier_name.clone(), tier_info);
            continue;
        }

        if let (Some(stripe_client), Some(product_id)) =
            (&state.stripe_client, &tier_config.product_id)
            && let Ok(prices) = stripe::Price::list(
                stripe_client,
                &stripe::ListPrices {
                    product: Some(stripe::IdOrCreate::Id(product_id)),
                    active: Some(true),
                    limit: Some(1),
                    ..Default::default()
                },
            )
            .await
            && let Some(price) = prices.data.first()
        {
            tier_info.price = Some(PriceInfo {
                amount: price.unit_amount.unwrap_or(0),
                currency: price
                    .currency
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "usd".to_string()),
                interval: price.recurring.as_ref().map(|r| r.interval.to_string()),
            });
        }

        tiers.insert(tier_name.clone(), tier_info);
    }

    Ok(Json(PricingResponse {
        current_tier: current_tier.to_string(),
        tiers,
    }))
}

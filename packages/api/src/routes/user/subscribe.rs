use std::str::FromStr;

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like_types::anyhow;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscribeRequest {
    pub tier: String,
    pub success_url: String,
    pub cancel_url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubscribeResponse {
    pub checkout_url: String,
    pub session_id: String,
}

const ENTERPRISE_TIER: &str = "ENTERPRISE";

#[utoipa::path(
    post,
    path = "/user/subscribe",
    tag = "user",
    request_body = SubscribeRequest,
    responses(
        (status = 200, description = "Stripe checkout session created", body = SubscribeResponse),
        (status = 401, description = "Unauthorized"),
        (status = 400, description = "Invalid tier or premium not enabled")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /user/subscribe", skip_all)]
pub async fn create_subscription_checkout(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<SubscribeRequest>,
) -> Result<Json<SubscribeResponse>, ApiError> {
    if request.tier.eq_ignore_ascii_case(ENTERPRISE_TIER) {
        let conversion = &state.platform_config.conversion;
        let contact = conversion
            .contact
            .as_ref()
            .unwrap_or(&state.platform_config.contact);
        return Err(ApiError::bad_request(format!(
            "Enterprise plans require a custom agreement. Contact {}.",
            contact.preferred_reference()
        )));
    }

    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or_else(|| anyhow!("Premium features are not enabled"))?;

    let db_user = user.get_user(&state).await?;
    let stripe_id = db_user
        .stripe_id
        .ok_or_else(|| anyhow!("User does not have a Stripe customer ID"))?;

    let tier_config = state
        .platform_config
        .tiers
        .get(&request.tier.to_uppercase())
        .ok_or_else(|| anyhow!("Invalid tier: {}", request.tier))?;

    let product_id = tier_config
        .product_id
        .as_ref()
        .ok_or_else(|| anyhow!("Tier {} does not have a product configured", request.tier))?;

    let prices = stripe::Price::list(
        stripe_client,
        &stripe::ListPrices {
            product: Some(stripe::IdOrCreate::Id(product_id)),
            active: Some(true),
            limit: Some(1),
            ..Default::default()
        },
    )
    .await?;

    let price = prices
        .data
        .first()
        .ok_or_else(|| anyhow!("No price found for tier {}", request.tier))?;

    let mut params = stripe::CreateCheckoutSession::new();
    params.customer = Some(
        stripe::CustomerId::from_str(&stripe_id)
            .map_err(|_| anyhow!("Invalid Stripe customer ID"))?,
    );
    params.mode = Some(stripe::CheckoutSessionMode::Subscription);
    params.success_url = Some(&request.success_url);
    params.cancel_url = Some(&request.cancel_url);
    params.line_items = Some(vec![stripe::CreateCheckoutSessionLineItems {
        price: Some(price.id.to_string()),
        quantity: Some(1),
        ..Default::default()
    }]);
    params.allow_promotion_codes = Some(true);

    let session = stripe::CheckoutSession::create(stripe_client, params).await?;

    Ok(Json(SubscribeResponse {
        checkout_url: session.url.unwrap_or_default(),
        session_id: session.id.to_string(),
    }))
}

use std::str::FromStr;

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like_types::anyhow;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubscribeRequest {
    pub tier: String,
    #[serde(default)]
    pub price_id: Option<String>,
    #[serde(default)]
    pub interval: Option<String>,
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
            limit: Some(100),
            ..Default::default()
        },
    )
    .await?;

    let interval = request.interval.as_deref().unwrap_or("month");
    if !matches!(interval, "month" | "year") {
        return Err(ApiError::bad_request("Choose monthly or annual billing"));
    }
    // Resolve within the configured product so a caller cannot buy another product's price.
    let display = state
        .platform_config
        .conversion
        .tier_display
        .get(&request.tier.to_uppercase())
        .ok_or_else(|| ApiError::bad_request("This plan does not have published pricing"))?;
    let price =
        select_subscription_price(&prices.data, request.price_id.as_deref(), interval, display)
            .ok_or_else(|| {
                ApiError::bad_request(
                    "This billing option is unavailable. Refresh pricing and try again.",
                )
            })?;

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

fn select_subscription_price<'a>(
    prices: &'a [stripe::Price],
    price_id: Option<&str>,
    interval: &str,
    display: &flow_like::hub::TierDisplay,
) -> Option<&'a stripe::Price> {
    prices
        .iter()
        .filter(|price| {
            super::pricing::eligible_price(price, display, interval)
                && price_id.is_none_or(|id| price.id.to_string() == id)
        })
        .min_by(|left, right| left.id.as_str().cmp(right.id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(id: &str, interval: stripe::RecurringInterval) -> stripe::Price {
        stripe::Price {
            id: stripe::PriceId::from_str(id).unwrap(),
            active: Some(true),
            unit_amount: Some(1900),
            billing_scheme: Some(stripe::PriceBillingScheme::PerUnit),
            currency: Some(stripe::Currency::EUR),
            recurring: Some(stripe::Recurring {
                interval,
                interval_count: 1,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn display() -> flow_like::hub::TierDisplay {
        flow_like::hub::TierDisplay {
            monthly_price_cents: Some(1900),
            annual_price_cents: Some(1900),
            currency: Some("eur".into()),
            ..Default::default()
        }
    }

    #[test]
    fn explicit_price_must_match_product_list_and_billing_period() {
        let prices = vec![
            price("price_annual", stripe::RecurringInterval::Year),
            price("price_monthly", stripe::RecurringInterval::Month),
        ];
        assert_eq!(
            select_subscription_price(&prices, None, "month", &display())
                .unwrap()
                .id
                .to_string(),
            "price_monthly"
        );
        assert!(
            select_subscription_price(&prices, Some("price_other_product"), "month", &display())
                .is_none()
        );
        assert!(
            select_subscription_price(&prices, Some("price_annual"), "month", &display()).is_none()
        );
        assert!(
            select_subscription_price(&prices, Some("price_annual"), "year", &display()).is_some()
        );
    }

    #[test]
    fn archived_one_off_and_multi_year_prices_are_not_subscription_options() {
        let mut archived = price("price_archived", stripe::RecurringInterval::Month);
        archived.active = Some(false);
        let mut one_off = price("price_oneoff", stripe::RecurringInterval::Month);
        one_off.recurring = None;
        let mut multi = price("price_multiyear", stripe::RecurringInterval::Year);
        multi.recurring.as_mut().unwrap().interval_count = 2;
        assert!(
            select_subscription_price(&[archived, one_off], None, "month", &display()).is_none()
        );
        assert!(select_subscription_price(&[multi], None, "year", &display()).is_none());
    }

    #[test]
    fn advertised_amount_currency_and_licensed_billing_are_required() {
        let mut cheap = price("price_old", stripe::RecurringInterval::Month);
        cheap.unit_amount = Some(900);
        let mut usd = price("price_usd", stripe::RecurringInterval::Month);
        usd.currency = Some(stripe::Currency::USD);
        let mut metered = price("price_metered", stripe::RecurringInterval::Month);
        metered.recurring.as_mut().unwrap().usage_type = stripe::RecurringUsageType::Metered;
        for candidate in [cheap, usd, metered] {
            let id = candidate.id.to_string();
            assert!(
                select_subscription_price(&[candidate], Some(&id), "month", &display()).is_none()
            );
        }
        let prices = [
            price("price_z", stripe::RecurringInterval::Month),
            price("price_a", stripe::RecurringInterval::Month),
        ];
        assert_eq!(
            select_subscription_price(&prices, None, "month", &display())
                .unwrap()
                .id
                .to_string(),
            "price_a"
        );
    }
}

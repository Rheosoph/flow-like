use crate::{
    entity::{app, membership, meta, sea_orm_active_enums::Visibility, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PurchaseParams {
    /// Accepted for older clients. Return URLs are built by the server.
    pub success_url: Option<String>,
    /// Accepted for older clients. Return URLs are built by the server.
    pub cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseResponse {
    pub checkout_url: Option<String>,
    pub already_member: bool,
    pub app_id: String,
}

/// POST /apps/{app_id}/team/purchase
///
/// Initiates a Stripe checkout session for purchasing a paid app.
/// - If user is already a member, returns already_member=true with no checkout URL
/// - Reuses the persisted open checkout for this buyer and offer
/// - Returns the checkout URL for the frontend to redirect to
#[utoipa::path(
    post,
    path = "/apps/{app_id}/team/purchase",
    tag = "team",
    description = "Start a purchase flow for a paid app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = PurchaseParams,
    responses(
        (status = 200, description = "Purchase session", body = PurchaseResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/team/purchase", skip(state, user, _params))]
pub async fn purchase(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    _params: Option<Json<PurchaseParams>>,
) -> Result<Json<PurchaseResponse>, ApiError> {
    let sub = user.sub()?;

    // Check if user is already a member
    let existing_membership = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .one(&state.db)
        .await?;

    if existing_membership.is_some() {
        tracing::info!(
            user_id = %sub,
            app_id = %app_id,
            "User already has membership, no purchase needed"
        );
        return Ok(Json(PurchaseResponse {
            checkout_url: None,
            already_member: true,
            app_id: app_id.clone(),
        }));
    }

    // Get the app to check price and visibility
    let app = app::Entity::find_by_id(app_id.clone())
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Verify app has a price
    if app.price <= 0
        || !matches!(
            app.visibility,
            Visibility::Public | Visibility::PublicRequestAccess
        )
    {
        tracing::warn!(
            user_id = %sub,
            app_id = %app_id,
            "Attempted to purchase free app via purchase endpoint"
        );
        return Err(ApiError::bad_request(
            "This app is free. Use the join endpoint instead.".to_string(),
        ));
    }

    if app.visibility == Visibility::PublicRequestAccess {
        return Err(crate::payments::error(
            "APPROVAL_REQUIRED",
            "Paid access requests are not available for checkout yet.",
        ));
    }

    // Get Stripe client
    let _stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or(anyhow!("Stripe not configured"))?;

    let stripe_id = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .and_then(|u| u.stripe_id)
        .ok_or(anyhow!("User does not have a Stripe customer ID"))?;

    if !crate::stripe_connect::request::valid_id(&stripe_id, "cus_") {
        return Err(ApiError::internal("Invalid Stripe customer ID"));
    }

    // Get app metadata for display name (try to fetch from database)
    let app_name = meta::Entity::find()
        .filter(meta::Column::AppId.eq(Some(app_id.clone())))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
        .map(|m| m.name)
        .unwrap_or_else(|| format!("App {}", &app_id[..8.min(app_id.len())]));

    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let (success_url, cancel_url) =
        crate::payments::legacy_checkout::checkout_urls(&frontend_url, false, &app_id)?;
    let client_ref = format!("app_purchase:{sub}:{app_id}");
    let parameters = json!({
        "success_url": success_url, "cancel_url": cancel_url, "mode": "payment", "customer": stripe_id,
        "client_reference_id": client_ref, "payment_method_types": ["card"],
        "metadata": {"type": "app_purchase", "app_id": app_id, "user_id": sub, "price_cents": app.price.to_string()},
        "line_items": [{"quantity": 1, "price_data": {"currency": "eur", "unit_amount": app.price,
            "product_data": {"name": app_name, "description": format!("One-time purchase of {app_name}")}}}]
    });
    let checkout_url =
        crate::payments::legacy_checkout::start(&state, "app_purchase", &sub, &app_id, parameters)
            .await?;

    Ok(Json(PurchaseResponse {
        checkout_url,
        already_member: false,
        app_id,
    }))
}

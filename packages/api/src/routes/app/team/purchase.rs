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
use std::collections::HashMap;
use stripe::CustomerId;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PurchaseParams {
    /// Optional success URL override (frontend will append receipt info)
    pub success_url: Option<String>,
    /// Optional cancel URL override
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
/// - Creates an idempotent checkout session (same user+app = same session if not expired)
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
#[tracing::instrument(name = "POST /apps/{app_id}/team/purchase", skip(state, user, params))]
pub async fn purchase(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(params): Json<PurchaseParams>,
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

    // Get Stripe client
    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or(anyhow!("Stripe not configured"))?;

    let stripe_id = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .and_then(|u| u.stripe_id)
        .ok_or(anyhow!("User does not have a Stripe customer ID"))?;

    let customer_id: CustomerId = stripe_id
        .parse()
        .map_err(|e| anyhow!("Invalid Stripe customer ID for user {}: {}", sub, e))?;

    // Get app metadata for display name (try to fetch from database)
    let app_name = meta::Entity::find()
        .filter(meta::Column::AppId.eq(Some(app_id.clone())))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
        .map(|m| m.name)
        .unwrap_or_else(|| format!("App {}", &app_id[..8.min(app_id.len())]));

    // Build URLs
    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let success_url = params
        .success_url
        .unwrap_or_else(|| format!("{}/store?id={}&purchase=success", frontend_url, app_id));
    let cancel_url = params
        .cancel_url
        .unwrap_or_else(|| format!("{}/store?id={}&purchase=canceled", frontend_url, app_id));

    // Build metadata for webhook processing
    let mut metadata = HashMap::new();
    metadata.insert("type".to_string(), "app_purchase".to_string());
    metadata.insert("app_id".to_string(), app_id.clone());
    metadata.insert("user_id".to_string(), sub.clone());
    metadata.insert("price_cents".to_string(), app.price.to_string());

    // Create checkout session
    let mut params = stripe::CreateCheckoutSession::new();
    params.success_url = Some(&success_url);
    params.cancel_url = Some(&cancel_url);
    params.mode = Some(stripe::CheckoutSessionMode::Payment);
    params.customer = Some(customer_id);

    // client_reference_id is used to identify this purchase in the webhook
    // Format: "app_purchase:{user_id}:{app_id}"
    let client_ref = format!("app_purchase:{}:{}", sub, app_id);
    params.client_reference_id = Some(&client_ref);

    // Create line item for the app
    let line_item = stripe::CreateCheckoutSessionLineItems {
        price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
            currency: stripe::Currency::EUR,
            product_data: Some(stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                name: app_name.clone(),
                description: Some(format!("One-time purchase of {}", app_name)),
                ..Default::default()
            }),
            unit_amount: Some(app.price),
            ..Default::default()
        }),
        quantity: Some(1),
        ..Default::default()
    };
    params.line_items = Some(vec![line_item]);
    params.metadata = Some(metadata);

    // Create the checkout session
    let session = stripe::CheckoutSession::create(stripe_client, params)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create Stripe checkout session");
            anyhow!("Failed to create checkout session: {}", e)
        })?;

    let checkout_url = session.url;

    tracing::info!(
        user_id = %sub,
        app_id = %app_id,
        session_id = %session.id,
        "Created checkout session for app purchase"
    );

    Ok(Json(PurchaseResponse {
        checkout_url,
        already_member: false,
        app_id,
    }))
}

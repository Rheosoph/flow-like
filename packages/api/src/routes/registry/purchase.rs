//! WASM Package purchase endpoint.
//!
//! Creates a Stripe checkout session for purchasing a paid WASM package.
//! Mirrors the app purchase flow (packages/api/src/routes/app/team/purchase.rs).

use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
use crate::entity::{user, wasm_package, wasm_package_user};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use stripe::CustomerId;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WasmPurchaseParams {
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WasmPurchaseResponse {
    pub checkout_url: Option<String>,
    pub already_has_access: bool,
    pub package_id: String,
}

/// POST /registry/package/{package_id}/purchase
///
/// Initiate a Stripe checkout for a paid WASM package.
/// - If user already has access, returns already_has_access=true
/// - Package must be Public or PublicRequestAccess with price > 0
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/purchase",
    tag = "registry",
    description = "Start a purchase flow for a paid WASM package.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = WasmPurchaseParams,
    responses(
        (status = 200, description = "Purchase session", body = WasmPurchaseResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []))
)]
pub async fn purchase(
    State(state): State<AppState>,
    Extension(user_ext): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(params): Json<WasmPurchaseParams>,
) -> Result<Json<WasmPurchaseResponse>, ApiError> {
    let sub = user_ext.sub()?;

    let existing_access = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&sub))
        .one(&state.db)
        .await?;

    if existing_access.is_some() {
        return Ok(Json(WasmPurchaseResponse {
            checkout_url: None,
            already_has_access: true,
            package_id,
        }));
    }

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if package.price <= 0
        || !matches!(
            package.visibility,
            WasmPackageVisibility::Public | WasmPackageVisibility::PublicRequestAccess
        )
    {
        return Err(ApiError::bad_request(
            "This package is free or not available for purchase. Use the access endpoint instead.",
        ));
    }

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
        .map_err(|e| anyhow!("Invalid Stripe customer ID: {}", e))?;

    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let success_url = params
        .success_url
        .unwrap_or_else(|| format!("{}/nodes?id={}&purchase=success", frontend_url, package_id));
    let cancel_url = params
        .cancel_url
        .unwrap_or_else(|| format!("{}/nodes?id={}&purchase=canceled", frontend_url, package_id));

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("type".to_string(), "wasm_purchase".to_string());
    metadata.insert("package_id".to_string(), package_id.clone());
    metadata.insert("user_id".to_string(), sub.clone());
    metadata.insert("price_cents".to_string(), package.price.to_string());

    // client_reference_id format: "wasm_purchase:{user_id}:{package_id}"
    let client_ref = format!("wasm_purchase:{}:{}", sub, package_id);

    let mut checkout_params = stripe::CreateCheckoutSession::new();
    checkout_params.success_url = Some(&success_url);
    checkout_params.cancel_url = Some(&cancel_url);
    checkout_params.mode = Some(stripe::CheckoutSessionMode::Payment);
    checkout_params.customer = Some(customer_id);
    checkout_params.client_reference_id = Some(&client_ref);

    let line_item = stripe::CreateCheckoutSessionLineItems {
        price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
            currency: stripe::Currency::EUR,
            product_data: Some(stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                name: package.name.clone(),
                description: Some(format!("One-time purchase of WASM package: {}", package.name)),
                ..Default::default()
            }),
            unit_amount: Some(package.price),
            ..Default::default()
        }),
        quantity: Some(1),
        ..Default::default()
    };
    checkout_params.line_items = Some(vec![line_item]);
    checkout_params.metadata = Some(metadata);

    let session = stripe::CheckoutSession::create(stripe_client, checkout_params)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create Stripe checkout for WASM package");
            anyhow!("Failed to create checkout session: {}", e)
        })?;

    tracing::info!(
        user_id = %sub,
        package_id = %package_id,
        session_id = %session.id,
        "Created checkout session for WASM package purchase"
    );

    Ok(Json(WasmPurchaseResponse {
        checkout_url: session.url,
        already_has_access: false,
        package_id,
    }))
}

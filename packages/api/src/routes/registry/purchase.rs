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
use serde_json::json;
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
/// - Package must be Public with price > 0; paid approval checkout is unavailable
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
    _params: Option<Json<WasmPurchaseParams>>,
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

    if package.visibility == WasmPackageVisibility::PublicRequestAccess {
        return Err(crate::payments::error(
            "APPROVAL_REQUIRED",
            "Paid access requests are not available for checkout yet.",
        ));
    }

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

    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let (success_url, cancel_url) =
        crate::payments::legacy_checkout::checkout_urls(&frontend_url, true, &package_id)?;
    let client_ref = format!("wasm_purchase:{sub}:{package_id}");
    let parameters = json!({
        "success_url": success_url, "cancel_url": cancel_url, "mode": "payment", "customer": stripe_id,
        "client_reference_id": client_ref, "payment_method_types": ["card"],
        "metadata": {"type": "wasm_purchase", "package_id": package_id, "user_id": sub, "price_cents": package.price.to_string()},
        "line_items": [{"quantity": 1, "price_data": {"currency": "eur", "unit_amount": package.price,
            "product_data": {"name": package.name, "description": format!("One-time purchase of WASM package: {}", package.name)}}}]
    });
    let checkout_url = crate::payments::legacy_checkout::start(
        &state,
        "wasm_purchase",
        &sub,
        &package_id,
        parameters,
    )
    .await?;

    Ok(Json(WasmPurchaseResponse {
        checkout_url,
        already_has_access: false,
        package_id,
    }))
}

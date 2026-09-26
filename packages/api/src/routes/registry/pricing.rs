//! Package price, set by the package owner.

use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePackagePriceRequest {
    /// Price in EUR cents; 0 makes the package free.
    pub price: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PackagePriceResponse {
    pub price: i64,
}

/// PATCH /registry/package/{package_id}/price
#[utoipa::path(
    patch,
    path = "/registry/package/{package_id}/price",
    tag = "registry",
    description = "Set the price of a package in EUR cents. Set 0 to make it free. People who already have the package keep it. Only the package owner can change the price, and selling requires a ready payout account.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = UpdatePackagePriceRequest,
    responses(
        (status = 200, description = "Price updated", body = PackagePriceResponse),
        (status = 403, description = "Only the package owner can set the price"),
        (status = 404, description = "Package not found"),
        (status = 409, description = "The price is outside the allowed range, the package can't be sold, or the payout account isn't ready")
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_price(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(body): Json<UpdatePackagePriceRequest>,
) -> Result<Json<PackagePriceResponse>, ApiError> {
    let sub = user.sub()?;
    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Owner);

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Package not found"))?;

    let config = &state.platform_config.payments;
    crate::payments::domain::package_listing_price(
        body.price,
        &package.visibility,
        config.marketplace_min_amount,
        config.max_payment_amount,
    )?;
    if body.price > 0 && config.marketplace_enabled {
        crate::payments::accounts::require_can_sell(&state, &sub).await?;
    }

    let mut active: wasm_package::ActiveModel = package.into();
    active.price = Set(body.price);
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    let updated = active.update(&state.db).await?;

    Ok(Json(PackagePriceResponse {
        price: updated.price,
    }))
}

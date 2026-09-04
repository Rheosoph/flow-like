use crate::{entity::app, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::overview::verify_sales_access;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePriceRequest {
    /// New price in cents (must be >= 0)
    pub price: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PriceResponse {
    pub price: i64,
    pub updated: bool,
}

/// PATCH /apps/{app_id}/sales/price - Update the app price
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/sales/price",
    tag = "sales",
    description = "Update the app price.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdatePriceRequest,
    responses(
        (status = 200, description = "Price updated", body = PriceResponse),
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
#[tracing::instrument(name = "PATCH /apps/{app_id}/sales/price", skip(state, user, body))]
pub async fn update_price(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdatePriceRequest>,
) -> Result<Json<PriceResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    if body.price < 0 {
        return Err(ApiError::bad_request(
            "Price cannot be negative".to_string(),
        ));
    }

    let existing = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active: app::ActiveModel = existing.into();
    active.price = Set(body.price);
    active.updated_at = Set(chrono::Utc::now().fixed_offset());

    active.update(&state.db).await?;

    tracing::info!(
        app_id = %app_id,
        user_id = %sub,
        new_price = body.price,
        "App price updated"
    );

    Ok(Json(PriceResponse {
        price: body.price,
        updated: true,
    }))
}

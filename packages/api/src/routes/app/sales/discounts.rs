use crate::{
    entity::{app_discount, sea_orm_active_enums::DiscountType},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::overview::verify_sales_access;

/// Discount windows arrive as RFC3339 (including the API's own `to_rfc3339()`
/// output round-tripped back by the dashboard), as `…Z`, or with no zone at
/// all. Zone-less strings have always been read as UTC; anchoring the parse
/// keeps that, since a bare string is ambiguous once the column carries an
/// offset. Everything is normalised to UTC so the stored instant is stable.
fn parse_client_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.to_utc().fixed_offset());
    }
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .map(|stamp| stamp.and_utc().fixed_offset())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListDiscountsQuery {
    /// Filter to only active discounts
    #[serde(default)]
    pub active_only: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscountResponse {
    pub id: String,
    pub app_id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub discount_type: String,
    pub discount_value: i64,
    pub max_uses: Option<i64>,
    pub used_count: i64,
    pub min_purchase_amount: Option<i64>,
    pub starts_at: String,
    pub expires_at: Option<String>,
    pub is_active: bool,
    pub is_valid: bool,
    pub created_at: String,
}

impl From<app_discount::Model> for DiscountResponse {
    fn from(d: app_discount::Model) -> Self {
        let now = Utc::now().fixed_offset();
        let is_valid = d.is_active
            && d.starts_at <= now
            && d.expires_at.map(|e| e > now).unwrap_or(true)
            && d.max_uses.map(|m| d.used_count < m).unwrap_or(true);

        Self {
            id: d.id,
            app_id: d.app_id,
            code: d.code,
            name: d.name,
            description: d.description,
            discount_type: format!("{:?}", d.discount_type),
            discount_value: d.discount_value,
            max_uses: d.max_uses,
            used_count: d.used_count,
            min_purchase_amount: d.min_purchase_amount,
            starts_at: d.starts_at.to_rfc3339(),
            expires_at: d.expires_at.map(|e| e.to_rfc3339()),
            is_active: d.is_active,
            is_valid,
            created_at: d.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateDiscountRequest {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    /// "percentage" or "fixed_amount"
    pub discount_type: String,
    /// Percentage (0-100) or fixed amount in cents
    pub discount_value: i64,
    pub max_uses: Option<i64>,
    pub min_purchase_amount: Option<i64>,
    /// ISO 8601 datetime string
    pub starts_at: Option<String>,
    /// ISO 8601 datetime string
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDiscountRequest {
    pub code: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub discount_type: Option<String>,
    pub discount_value: Option<i64>,
    pub max_uses: Option<i64>,
    pub min_purchase_amount: Option<i64>,
    pub starts_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_active: Option<bool>,
}

/// GET /apps/{app_id}/sales/discounts - List all discounts for an app
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/discounts",
    tag = "sales",
    description = "List discounts for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("active_only" = bool, Query, description = "Only active discounts")
    ),
    responses(
        (status = 200, description = "Discount list", body = Vec<DiscountResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/sales/discounts", skip(state, user, query))]
pub async fn list_discounts(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ListDiscountsQuery>,
) -> Result<Json<Vec<DiscountResponse>>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let mut query_builder =
        app_discount::Entity::find().filter(app_discount::Column::AppId.eq(&app_id));

    if query.active_only {
        query_builder = query_builder.filter(app_discount::Column::IsActive.eq(true));
    }

    let discounts = query_builder
        .order_by_desc(app_discount::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let response: Vec<DiscountResponse> = discounts.into_iter().map(Into::into).collect();

    Ok(Json(response))
}

/// GET /apps/{app_id}/sales/discounts/{discount_id} - Get a specific discount
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/discounts/{discount_id}",
    tag = "sales",
    description = "Get a discount by ID.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("discount_id" = String, Path, description = "Discount ID")
    ),
    responses(
        (status = 200, description = "Discount", body = DiscountResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/sales/discounts/{discount_id}",
    skip(state, user)
)]
pub async fn get_discount(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, discount_id)): Path<(String, String)>,
) -> Result<Json<DiscountResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let discount = app_discount::Entity::find_by_id(&discount_id)
        .filter(app_discount::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    Ok(Json(discount.into()))
}

/// POST /apps/{app_id}/sales/discounts - Create a new discount
#[utoipa::path(
    post,
    path = "/apps/{app_id}/sales/discounts",
    tag = "sales",
    description = "Create a discount.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = CreateDiscountRequest,
    responses(
        (status = 200, description = "Discount created", body = DiscountResponse),
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
#[tracing::instrument(name = "POST /apps/{app_id}/sales/discounts", skip(state, user, body))]
pub async fn create_discount(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<CreateDiscountRequest>,
) -> Result<Json<DiscountResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    // Validate discount code uniqueness for this app
    let existing = app_discount::Entity::find()
        .filter(app_discount::Column::AppId.eq(&app_id))
        .filter(app_discount::Column::Code.eq(&body.code))
        .one(&state.db)
        .await?;

    if existing.is_some() {
        return Err(ApiError::bad_request(
            "A discount with this code already exists for this app".to_string(),
        ));
    }

    let discount_type = match body.discount_type.to_lowercase().as_str() {
        "percentage" => DiscountType::Percentage,
        "fixed_amount" | "fixed" => DiscountType::FixedAmount,
        _ => {
            return Err(ApiError::bad_request(
                "discount_type must be 'percentage' or 'fixed_amount'".to_string(),
            ));
        }
    };

    // Validate percentage is 0-100
    if discount_type == DiscountType::Percentage
        && (body.discount_value < 0 || body.discount_value > 100)
    {
        return Err(ApiError::bad_request(
            "Percentage discount must be between 0 and 100".to_string(),
        ));
    }

    if body.discount_value < 0 {
        return Err(ApiError::bad_request(
            "Discount value cannot be negative".to_string(),
        ));
    }

    let now = Utc::now().fixed_offset();

    let starts_at = body
        .starts_at
        .as_deref()
        .and_then(parse_client_timestamp)
        .unwrap_or(now);

    let expires_at = body.expires_at.as_deref().and_then(parse_client_timestamp);

    let id = create_id();

    let new_discount = app_discount::ActiveModel {
        id: Set(id.clone()),
        app_id: Set(app_id.clone()),
        code: Set(body.code.to_uppercase()),
        name: Set(body.name),
        description: Set(body.description),
        discount_type: Set(discount_type),
        discount_value: Set(body.discount_value),
        max_uses: Set(body.max_uses),
        used_count: Set(0),
        min_purchase_amount: Set(body.min_purchase_amount),
        starts_at: Set(starts_at),
        expires_at: Set(expires_at),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let created = new_discount.insert(&state.db).await?;

    tracing::info!(
        discount_id = %id,
        app_id = %app_id,
        code = %created.code,
        "Discount created"
    );

    Ok(Json(created.into()))
}

/// PATCH /apps/{app_id}/sales/discounts/{discount_id} - Update a discount
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/sales/discounts/{discount_id}",
    tag = "sales",
    description = "Update a discount.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("discount_id" = String, Path, description = "Discount ID")
    ),
    request_body = UpdateDiscountRequest,
    responses(
        (status = 200, description = "Discount updated", body = DiscountResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/sales/discounts/{discount_id}",
    skip(state, user, body)
)]
pub async fn update_discount(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, discount_id)): Path<(String, String)>,
    Json(body): Json<UpdateDiscountRequest>,
) -> Result<Json<DiscountResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let existing = app_discount::Entity::find_by_id(&discount_id)
        .filter(app_discount::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active: app_discount::ActiveModel = existing.into();

    if let Some(code) = body.code {
        // Check uniqueness if code is being changed
        let duplicate = app_discount::Entity::find()
            .filter(app_discount::Column::AppId.eq(&app_id))
            .filter(app_discount::Column::Code.eq(&code))
            .filter(app_discount::Column::Id.ne(&discount_id))
            .one(&state.db)
            .await?;

        if duplicate.is_some() {
            return Err(ApiError::bad_request(
                "A discount with this code already exists".to_string(),
            ));
        }

        active.code = Set(code.to_uppercase());
    }

    if let Some(name) = body.name {
        active.name = Set(name);
    }

    if let Some(description) = body.description {
        active.description = Set(Some(description));
    }

    if let Some(discount_type_str) = body.discount_type {
        let discount_type = match discount_type_str.to_lowercase().as_str() {
            "percentage" => DiscountType::Percentage,
            "fixed_amount" | "fixed" => DiscountType::FixedAmount,
            _ => {
                return Err(ApiError::bad_request(
                    "discount_type must be 'percentage' or 'fixed_amount'".to_string(),
                ));
            }
        };
        active.discount_type = Set(discount_type);
    }

    if let Some(value) = body.discount_value {
        if value < 0 {
            return Err(ApiError::bad_request(
                "Discount value cannot be negative".to_string(),
            ));
        }
        active.discount_value = Set(value);
    }

    if let Some(max_uses) = body.max_uses {
        active.max_uses = Set(Some(max_uses));
    }

    if let Some(min_amount) = body.min_purchase_amount {
        active.min_purchase_amount = Set(Some(min_amount));
    }

    if let Some(starts_at_str) = body.starts_at {
        let starts_at = parse_client_timestamp(&starts_at_str)
            .ok_or_else(|| ApiError::bad_request("Invalid starts_at date format".to_string()))?;
        active.starts_at = Set(starts_at);
    }

    if let Some(expires_at_str) = body.expires_at {
        let expires_at = parse_client_timestamp(&expires_at_str)
            .ok_or_else(|| ApiError::bad_request("Invalid expires_at date format".to_string()))?;
        active.expires_at = Set(Some(expires_at));
    }

    if let Some(is_active) = body.is_active {
        active.is_active = Set(is_active);
    }

    active.updated_at = Set(Utc::now().fixed_offset());

    let updated = active.update(&state.db).await?;

    tracing::info!(
        discount_id = %discount_id,
        app_id = %app_id,
        "Discount updated"
    );

    Ok(Json(updated.into()))
}

/// DELETE /apps/{app_id}/sales/discounts/{discount_id} - Delete a discount
#[utoipa::path(
    delete,
    path = "/apps/{app_id}/sales/discounts/{discount_id}",
    tag = "sales",
    description = "Delete a discount.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("discount_id" = String, Path, description = "Discount ID")
    ),
    responses(
        (status = 200, description = "Discount deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/sales/discounts/{discount_id}",
    skip(state, user)
)]
pub async fn delete_discount(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, discount_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let existing = app_discount::Entity::find_by_id(&discount_id)
        .filter(app_discount::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    app_discount::Entity::delete_by_id(&existing.id)
        .exec(&state.db)
        .await?;

    tracing::info!(
        discount_id = %discount_id,
        app_id = %app_id,
        "Discount deleted"
    );

    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// POST /apps/{app_id}/sales/discounts/{discount_id}/toggle - Toggle discount active state
#[utoipa::path(
    post,
    path = "/apps/{app_id}/sales/discounts/{discount_id}/toggle",
    tag = "sales",
    description = "Enable or disable a discount.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("discount_id" = String, Path, description = "Discount ID")
    ),
    responses(
        (status = 200, description = "Discount toggled", body = DiscountResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/sales/discounts/{discount_id}/toggle",
    skip(state, user)
)]
pub async fn toggle_discount(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, discount_id)): Path<(String, String)>,
) -> Result<Json<DiscountResponse>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    let existing = app_discount::Entity::find_by_id(&discount_id)
        .filter(app_discount::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let new_active = !existing.is_active;

    let mut active: app_discount::ActiveModel = existing.into();
    active.is_active = Set(new_active);
    active.updated_at = Set(Utc::now().fixed_offset());

    let updated = active.update(&state.db).await?;

    tracing::info!(
        discount_id = %discount_id,
        app_id = %app_id,
        is_active = new_active,
        "Discount toggled"
    );

    Ok(Json(updated.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dashboard's edit form seeds `startsAt` from the value this module
    /// serialised, so whatever `DiscountResponse` emits has to parse back here.
    /// It emits `to_rfc3339()`; before that arm existed the round-trip 400'd.
    #[test]
    fn accepts_the_string_the_api_itself_emits() {
        let stored = Utc::now().fixed_offset();
        let round_tripped =
            parse_client_timestamp(&stored.to_rfc3339()).expect("own output must parse back");
        assert_eq!(round_tripped, stored.to_utc().fixed_offset());
    }

    #[test]
    fn accepts_every_client_spelling_as_the_same_instant() {
        let expected = NaiveDateTime::parse_from_str("2026-09-04T08:18:44", "%Y-%m-%dT%H:%M:%S")
            .unwrap()
            .and_utc()
            .fixed_offset();

        for raw in [
            "2026-09-04T08:18:44",
            "2026-09-04T08:18:44Z",
            "2026-09-04T08:18:44.000Z",
            "2026-09-04T08:18:44+00:00",
            // An offset is honoured, not read as UTC.
            "2026-09-04T10:18:44+02:00",
        ] {
            assert_eq!(parse_client_timestamp(raw), Some(expected), "{raw}");
        }

        assert_eq!(parse_client_timestamp("not a date"), None);
    }
}

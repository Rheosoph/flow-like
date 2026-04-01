//! WASM Package access request (join queue) endpoints.
//!
//! Mirrors the app join queue flow:
//! - Public + free → instant access (wasm_package_user record)
//! - Public + paid → join queue entry (must go through purchase)
//! - PublicRequestAccess → join queue entry (must be approved by maintainer)
//! - Private → forbidden

use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
use crate::entity::{wasm_package, wasm_package_join_queue, wasm_package_user};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RequestAccessParams {
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestAccessResponse {
    pub granted: bool,
    pub queued: bool,
    pub requires_purchase: bool,
    pub package_id: String,
}

/// PUT /registry/package/{package_id}/access
///
/// Request access to a package.
/// - Public + free: instant access granted
/// - PublicRequestAccess: queued for maintainer approval
/// - Public + paid: queued (must use purchase endpoint)
/// - Private: forbidden
#[utoipa::path(
    put,
    path = "/registry/package/{package_id}/access",
    tag = "registry",
    description = "Request access to a WASM package.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = RequestAccessParams,
    responses(
        (status = 200, description = "Access result", body = RequestAccessResponse),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn request_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(params): Json<RequestAccessParams>,
) -> Result<Json<RequestAccessResponse>, ApiError> {
    let sub = user.sub()?;
    let txn = state.db.begin().await?;

    let existing = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&sub))
        .one(&txn)
        .await?;

    if existing.is_some() {
        return Ok(Json(RequestAccessResponse {
            granted: true,
            queued: false,
            requires_purchase: false,
            package_id,
        }));
    }

    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if package.visibility == WasmPackageVisibility::Private {
        return Err(ApiError::FORBIDDEN);
    }

    // Public + free → instant grant
    if package.visibility == WasmPackageVisibility::Public && package.price <= 0 {
        let now = chrono::Utc::now().naive_utc();
        wasm_package_user::ActiveModel {
            id: Set(create_id()),
            package_id: Set(package_id.clone()),
            user_id: Set(sub),
            permission: Set(WasmPackagePermission::User.bits()),
            granted_by: Set(None),
            granted_at: Set(now),
        }
        .insert(&txn)
        .await?;

        txn.commit().await?;
        state.invalidate_wasm_permission(&user.sub()?, &package_id);

        return Ok(Json(RequestAccessResponse {
            granted: true,
            queued: false,
            requires_purchase: false,
            package_id,
        }));
    }

    // Public + paid → needs purchase
    if package.visibility == WasmPackageVisibility::Public && package.price > 0 {
        return Ok(Json(RequestAccessResponse {
            granted: false,
            queued: false,
            requires_purchase: true,
            package_id,
        }));
    }

    // PublicRequestAccess → join queue
    let existing_request = wasm_package_join_queue::Entity::find()
        .filter(wasm_package_join_queue::Column::PackageId.eq(&package_id))
        .filter(wasm_package_join_queue::Column::UserId.eq(&sub))
        .one(&txn)
        .await?;

    if let Some(existing_request) = existing_request {
        let mut active: wasm_package_join_queue::ActiveModel = existing_request.into();
        active.comment = Set(params.comment);
        active.updated_at = Set(chrono::Utc::now().naive_utc());
        active.update(&txn).await?;
    } else {
        wasm_package_join_queue::ActiveModel {
            id: Set(create_id()),
            user_id: Set(sub),
            package_id: Set(package_id.clone()),
            comment: Set(params.comment),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        }
        .insert(&txn)
        .await?;
    }

    txn.commit().await?;

    Ok(Json(RequestAccessResponse {
        granted: false,
        queued: true,
        requires_purchase: false,
        package_id,
    }))
}

/// POST /registry/package/{package_id}/access/{request_id}
///
/// Accept a join request (maintainer+).
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/access/{request_id}",
    tag = "registry",
    description = "Accept an access request.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("request_id" = String, Path, description = "Join request ID")
    ),
    responses(
        (status = 200, description = "Request accepted"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn accept_access_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, request_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Maintainer);

    let txn = state.db.begin().await?;

    let request = wasm_package_join_queue::Entity::find()
        .filter(wasm_package_join_queue::Column::PackageId.eq(&package_id))
        .filter(wasm_package_join_queue::Column::Id.eq(&request_id))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let already = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&request.user_id))
        .one(&txn)
        .await?;

    if already.is_some() {
        let active: wasm_package_join_queue::ActiveModel = request.into();
        active.delete(&txn).await?;
        txn.commit().await?;
        return Ok(Json(()));
    }

    let now = chrono::Utc::now().naive_utc();
    wasm_package_user::ActiveModel {
        id: Set(create_id()),
        package_id: Set(package_id.clone()),
        user_id: Set(request.user_id.clone()),
        permission: Set(WasmPackagePermission::User.bits()),
        granted_by: Set(Some(sub)),
        granted_at: Set(now),
    }
    .insert(&txn)
    .await?;

    let active: wasm_package_join_queue::ActiveModel = request.into();
    active.delete(&txn).await?;
    txn.commit().await?;

    Ok(Json(()))
}

/// DELETE /registry/package/{package_id}/access/{request_id}
///
/// Reject a join request (maintainer+).
#[utoipa::path(
    delete,
    path = "/registry/package/{package_id}/access/{request_id}",
    tag = "registry",
    description = "Reject an access request.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("request_id" = String, Path, description = "Join request ID")
    ),
    responses(
        (status = 200, description = "Request rejected"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn reject_access_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, request_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Maintainer);

    let txn = state.db.begin().await?;

    let request = wasm_package_join_queue::Entity::find()
        .filter(wasm_package_join_queue::Column::PackageId.eq(&package_id))
        .filter(wasm_package_join_queue::Column::Id.eq(&request_id))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let active: wasm_package_join_queue::ActiveModel = request.into();
    active.delete(&txn).await?;
    txn.commit().await?;

    Ok(Json(()))
}

/// GET /registry/package/{package_id}/access
///
/// List pending access requests for a package (maintainer+).
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/access",
    tag = "registry",
    description = "List pending access requests.",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Pending requests", body = Vec<AccessRequest>),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_access_requests(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
) -> Result<Json<Vec<AccessRequest>>, ApiError> {
    let sub = user.sub()?;
    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Maintainer);

    let requests = wasm_package_join_queue::Entity::find()
        .filter(wasm_package_join_queue::Column::PackageId.eq(&package_id))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let result: Vec<AccessRequest> = requests
        .into_iter()
        .map(|r| AccessRequest {
            id: r.id,
            user_id: r.user_id,
            package_id: r.package_id,
            comment: r.comment,
            created_at: chrono::DateTime::from_naive_utc_and_offset(r.created_at, chrono::Utc),
        })
        .collect();

    Ok(Json(result))
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccessRequest {
    pub id: String,
    pub user_id: String,
    pub package_id: String,
    pub comment: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

use crate::{devices, error::ApiError, instances, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use flow_like_device_protocol::*;
use std::result::Result;
use utoipa::OpenApi;

#[derive(OpenApi)]
pub(crate) struct CloudApprovalsApi;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{id}/resource-grants",
            get(resource_grants).post(create_resource_grant),
        )
        .route(
            "/{id}/resource-grants/{grant}",
            get(resource_grant).delete(revoke_resource_grant),
        )
        .route(
            "/{id}/resource-grants/{grant}/billing",
            get(grant_billing).post(approve_billing_grant),
        )
        .route("/{id}/billing-grants", get(billing_grants))
        .route(
            "/{id}/billing-grants/{billing}",
            get(billing_grant).delete(revoke_billing_grant),
        )
        .route(
            "/{id}/instances",
            get(list_instances).post(register_instance),
        )
        .route("/{id}/instances/{instance}", delete(retire_instance))
}

async fn create_resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<CreateResourceGrantRequest>,
) -> Result<Json<ResourceGrantResponse>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::create_grant(&devices::context(&state), &owner, &id, request).await?,
    ))
}
async fn resource_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ResourceGrantResponse>>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::grants(&devices::context(&state), &owner, &id).await?,
    ))
}
async fn resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<ResourceGrantResponse>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::get_grant(&devices::context(&state), &owner, &id, &grant).await?,
    ))
}
async fn revoke_resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    instances::revoke_grant(&devices::context(&state), &owner, &id, &grant).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn approve_billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
    Json(request): Json<ApproveBillingGrantRequest>,
) -> Result<Json<BillingGrantResponse>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::approve_billing(&devices::context(&state), &owner, &id, &grant, request).await?,
    ))
}
async fn grant_billing(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<BillingGrantResponse>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::get_billing(&devices::context(&state), &owner, &id, &grant).await?,
    ))
}
async fn billing_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<BillingGrantResponse>>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::billing_grants(&devices::context(&state), &owner, &id).await?,
    ))
}
async fn billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, billing)): Path<(String, String)>,
) -> Result<Json<BillingGrantResponse>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::billing_grant(&devices::context(&state), &owner, &id, &billing).await?,
    ))
}
async fn revoke_billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, billing)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    instances::revoke_billing(&devices::context(&state), &owner, &id, &billing).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_instances(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<InstanceReceipt>>, ApiError> {
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        instances::instances(&devices::context(&state), &owner, &id).await?,
    ))
}
async fn register_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<InstanceRegistrationRequest>,
) -> Result<Json<InstanceReceipt>, ApiError> {
    Ok(Json(
        instances::register(&devices::context(&state), &id, request).await?,
    ))
}
async fn retire_instance(
    State(state): State<AppState>,
    Path((id, instance)): Path<(String, String)>,
    Json(request): Json<ReceiptRequest>,
) -> Result<StatusCode, ApiError> {
    instances::retire(&devices::context(&state), &id, &instance, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

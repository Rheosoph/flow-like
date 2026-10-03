use crate::{devices, error::ApiError, instances, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use flow_like_device_protocol::*;
use serde::Serialize;
use std::result::Result;
use utoipa::{OpenApi, ToSchema};

#[derive(OpenApi)]
#[openapi(paths(
    resource_grants,
    resource_grant,
    grant_billing,
    billing_grants,
    billing_grant,
    billing_eligibility,
    billing_usage,
    resource_summary,
    super::app_placements::device_placements,
    super::schedules::release,
    super::schedules::give_back
))]
pub(crate) struct CloudApprovalsApi;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/resource-summary", get(resource_summary))
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
        .route(
            "/{id}/resource-grants/{grant}/billing/eligibility",
            get(billing_eligibility),
        )
        .route("/{id}/billing-grants", get(billing_grants))
        .route(
            "/{id}/billing-grants/{billing}",
            get(billing_grant).delete(revoke_billing_grant),
        )
        .route("/{id}/billing-grants/{billing}/usage", get(billing_usage))
        .route(
            "/{id}/instances",
            get(list_instances).post(register_instance),
        )
        .route("/{id}/instances/{instance}", delete(retire_instance))
}

/// A cloud approval: what one service on a device may use of a project and of hosted models.
#[derive(Serialize, ToSchema)]
pub(crate) struct DeviceResourceGrant {
    grant_id: String,
    device_id: String,
    placement_id: String,
    deployment_id: String,
    project_id: String,
    app_id: Option<String>,
    delegating_user_id: String,
    authz_version: u64,
    model_ids: Vec<String>,
    max_instances: u32,
    /// Unix seconds.
    expires_at: i64,
    /// `active` or `revoked`.
    status: String,
    /// `read_only`, `read_write`, or `null` when the service gets no project files.
    #[schema(value_type = Option<String>)]
    online_access: Option<OnlineProjectAccess>,
    /// Unix seconds at which the approval stops working. At or before now once it has
    /// ended. Only on active approvals.
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_expires_at: Option<i64>,
    /// What ends the approval first: `approval` (its own expiry), `sharing_grant` (the
    /// approver's Deploy permission on the device) or `access_rules` (the device's access
    /// rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    effective_limit: Option<EffectiveLimit>,
    /// `storage_full` while the project's storage is full and the service can only read.
    /// Only shown to the approver and to people who can read the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    online_write_blocked: Option<OnlineWriteBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approved_by_user_id: Option<String>,
    /// Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<i64>,
}

impl From<ResourceGrantResponse> for DeviceResourceGrant {
    fn from(grant: ResourceGrantResponse) -> Self {
        let ResourceGrantResponse {
            grant_id,
            device_id,
            placement_id,
            deployment_id,
            project_id,
            app_id,
            delegating_user_id,
            authz_version,
            model_ids,
            max_instances,
            expires_at,
            status,
            online_access,
            effective_expires_at,
            effective_limit,
            online_write_blocked,
            approved_by_user_id,
            created_at,
        } = grant;
        Self {
            grant_id,
            device_id,
            placement_id,
            deployment_id,
            project_id,
            app_id,
            delegating_user_id,
            authz_version,
            model_ids,
            max_instances,
            expires_at,
            status,
            online_access,
            effective_expires_at,
            effective_limit,
            online_write_blocked,
            approved_by_user_id,
            created_at,
        }
    }
}

/// A spending limit: what one person pays for the hosted models a cloud approval allows.
#[derive(Serialize, ToSchema)]
pub(crate) struct DeviceBillingGrant {
    billing_grant_id: String,
    grant_id: String,
    payer_id: String,
    authz_version: u64,
    /// Micro EUR across every instance of the service; it does not renew.
    limit_micros: i64,
    used_micros: i64,
    reserved_micros: i64,
    /// Unix seconds.
    expires_at: i64,
    /// `active` or `revoked`.
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    approved_by_user_id: Option<String>,
    /// Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<i64>,
}

impl From<BillingGrantResponse> for DeviceBillingGrant {
    fn from(billing: BillingGrantResponse) -> Self {
        let BillingGrantResponse {
            billing_grant_id,
            grant_id,
            payer_id,
            authz_version,
            limit_micros,
            used_micros,
            reserved_micros,
            expires_at,
            status,
            approved_by_user_id,
            created_at,
        } = billing;
        Self {
            billing_grant_id,
            grant_id,
            payer_id,
            authz_version,
            limit_micros,
            used_micros,
            reserved_micros,
            expires_at,
            status,
            approved_by_user_id,
            created_at,
        }
    }
}

/// The person calling, once this hub manages devices at all.
pub(super) async fn caller(state: &AppState, user: &AppUser) -> Result<String, ApiError> {
    devices::enabled(&devices::context(state))?;
    devices::human_owner(state, user).await
}

#[utoipa::path(
    get,
    path = "/devices/resource-summary",
    tag = "devices",
    description = "See the cloud approvals and spending limits on all your devices at once: every approval on a device you own, and on other devices the ones you gave or pay for. Active approvals say when they really stop working and what ends them first.",
    responses(
        (status = 200, description = "Cloud approvals and spending limits by device", body = instances::ResourceSummary),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn resource_summary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<instances::ResourceSummary>, ApiError> {
    let viewer = caller(&state, &user).await?;
    Ok(Json(instances::resource_summary(&state, &viewer).await?))
}

async fn create_resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<CreateResourceGrantRequest>,
) -> Result<Json<DeviceResourceGrant>, ApiError> {
    let owner = caller(&state, &user).await?;
    let grant = instances::create_grant(&devices::context(&state), &owner, &id, request).await?;
    instances::audit_grant(&state, &user, instances::GRANT_CREATE, &grant).await;
    Ok(Json(grant.into()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/resource-grants",
    tag = "devices",
    description = "List the cloud approvals on a device, active ones first. The device owner sees every approval, everyone else the ones they gave. Active approvals say when they really stop working and what ends them first.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Cloud approvals on the device", body = Vec<DeviceResourceGrant>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "Device not found"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn resource_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<DeviceResourceGrant>>, ApiError> {
    let owner = caller(&state, &user).await?;
    let mut grants = instances::grants(&devices::context(&state), &owner, &id).await?;
    instances::project::mark_blocked_writes(&state, &owner, &mut grants).await?;
    Ok(Json(grants.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/resource-grants/{grant}",
    tag = "devices",
    description = "Read one cloud approval. Only the device owner and the person who gave the approval can read it.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("grant" = String, Path, description = "Cloud approval ID")
    ),
    responses(
        (status = 200, description = "The cloud approval", body = DeviceResourceGrant),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "No such approval, or it is not yours to see"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<DeviceResourceGrant>, ApiError> {
    let owner = caller(&state, &user).await?;
    let mut grant = instances::get_grant(&devices::context(&state), &owner, &id, &grant).await?;
    instances::project::mark_blocked_writes(&state, &owner, std::slice::from_mut(&mut grant))
        .await?;
    Ok(Json(grant.into()))
}

async fn revoke_resource_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let owner = caller(&state, &user).await?;
    let revoked = instances::revoke_grant(&devices::context(&state), &owner, &id, &grant).await?;
    instances::audit_grant(&state, &user, instances::GRANT_REVOKE, &revoked).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn approve_billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
    Json(request): Json<ApproveBillingGrantRequest>,
) -> Result<Json<DeviceBillingGrant>, ApiError> {
    let owner = caller(&state, &user).await?;
    let billing =
        instances::approve_billing(&devices::context(&state), &owner, &id, &grant, request).await?;
    instances::audit_billing(&state, &user, instances::BILLING_APPROVE, &billing).await;
    Ok(Json(billing.into()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/resource-grants/{grant}/billing",
    tag = "devices",
    description = "Read the spending limit of a cloud approval: the active one, or the most recent one when none is active.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("grant" = String, Path, description = "Cloud approval ID")
    ),
    responses(
        (status = 200, description = "The spending limit", body = DeviceBillingGrant),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "No spending limit, or the approval is not yours to see"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn grant_billing(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<DeviceBillingGrant>, ApiError> {
    let owner = caller(&state, &user).await?;
    let billing = instances::get_billing(&devices::context(&state), &owner, &id, &grant).await?;
    Ok(Json(billing.into()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/resource-grants/{grant}/billing/eligibility",
    tag = "devices",
    description = "Check, before you set a spending limit, whether your plan includes each hosted model the cloud approval allows. Nothing is reserved or changed.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("grant" = String, Path, description = "Cloud approval ID")
    ),
    responses(
        (status = 200, description = "Your plan and what it includes of the approved models", body = instances::BillingEligibility),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "No such approval, or it is not yours to see"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn billing_eligibility(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<instances::BillingEligibility>, ApiError> {
    let payer = caller(&state, &user).await?;
    Ok(Json(
        instances::billing_eligibility(
            &devices::context(&state),
            &state.platform_config.tiers,
            &payer,
            &id,
            &grant,
        )
        .await?,
    ))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/billing-grants",
    tag = "devices",
    description = "List the spending limits on a device, active ones first. The device owner sees every limit, everyone else those on the approvals they gave.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Spending limits on the device", body = Vec<DeviceBillingGrant>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "Device not found"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn billing_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<DeviceBillingGrant>>, ApiError> {
    let owner = caller(&state, &user).await?;
    let billing = instances::billing_grants(&devices::context(&state), &owner, &id).await?;
    Ok(Json(billing.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/billing-grants/{billing}",
    tag = "devices",
    description = "Read one spending limit. Only the device owner and the person who gave the cloud approval can read it.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("billing" = String, Path, description = "Spending limit ID")
    ),
    responses(
        (status = 200, description = "The spending limit", body = DeviceBillingGrant),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "No such spending limit, or it is not yours to see"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, billing)): Path<(String, String)>,
) -> Result<Json<DeviceBillingGrant>, ApiError> {
    let owner = caller(&state, &user).await?;
    let billing =
        instances::billing_grant(&devices::context(&state), &owner, &id, &billing).await?;
    Ok(Json(billing.into()))
}

#[utoipa::path(
    get,
    path = "/devices/{id}/billing-grants/{billing}/usage",
    tag = "devices",
    description = "See what was spent under one spending limit: the totals, and the spend of the 100 instances that used it most recently.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("billing" = String, Path, description = "Spending limit ID")
    ),
    responses(
        (status = 200, description = "Spend under the spending limit", body = instances::BillingGrantUsage),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The account or token cannot manage devices"),
        (status = 404, description = "No such spending limit, or it is not yours to see"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn billing_usage(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, billing)): Path<(String, String)>,
) -> Result<Json<instances::BillingGrantUsage>, ApiError> {
    let owner = caller(&state, &user).await?;
    Ok(Json(
        instances::billing_usage(&devices::context(&state), &owner, &id, &billing).await?,
    ))
}

async fn revoke_billing_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, billing)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let owner = caller(&state, &user).await?;
    let revoked =
        instances::revoke_billing(&devices::context(&state), &owner, &id, &billing).await?;
    instances::audit_billing(&state, &user, instances::BILLING_REVOKE, &revoked).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_instances(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<InstanceReceipt>>, ApiError> {
    let owner = caller(&state, &user).await?;
    Ok(Json(
        instances::instances(&devices::context(&state), &owner, &id).await?,
    ))
}

async fn register_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<InstanceRegistrationRequest>,
) -> Result<Json<InstanceReceipt>, ApiError> {
    let receipt = instances::register(&devices::context(&state), &id, request).await?;
    instances::audit_registration(&state, &receipt).await;
    Ok(Json(receipt))
}

async fn retire_instance(
    State(state): State<AppState>,
    Path((id, instance)): Path<(String, String)>,
    Json(request): Json<ReceiptRequest>,
) -> Result<StatusCode, ApiError> {
    instances::retire(&devices::context(&state), &id, &instance, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::ApiDoc;

    fn grant() -> ResourceGrantResponse {
        ResourceGrantResponse {
            grant_id: "grant".into(),
            device_id: "device".into(),
            placement_id: "placement".into(),
            deployment_id: "deployment".into(),
            project_id: "project".into(),
            app_id: None,
            delegating_user_id: "delegator".into(),
            authz_version: 1,
            model_ids: vec!["model".into()],
            max_instances: 2,
            expires_at: 1_900_000_000,
            status: "active".into(),
            online_access: None,
            effective_expires_at: None,
            effective_limit: None,
            online_write_blocked: None,
            approved_by_user_id: None,
            created_at: None,
        }
    }

    fn billing() -> BillingGrantResponse {
        BillingGrantResponse {
            billing_grant_id: "billing".into(),
            grant_id: "grant".into(),
            payer_id: "payer".into(),
            authz_version: 1,
            limit_micros: 5_000,
            used_micros: 10,
            reserved_micros: 20,
            expires_at: 1_800_000_000,
            status: "active".into(),
            approved_by_user_id: None,
            created_at: None,
        }
    }

    /// The documented bodies are what is sent: the same JSON as the protocol types, with
    /// and without the facts an older hub lacked.
    #[test]
    fn documented_bodies_are_the_protocol_wire_format() {
        let full = ResourceGrantResponse {
            app_id: Some("project".into()),
            online_access: Some(OnlineProjectAccess::ReadWrite),
            effective_expires_at: Some(1_800_000_000),
            effective_limit: Some(EffectiveLimit::AccessRules),
            online_write_blocked: Some(OnlineWriteBlock::StorageFull),
            approved_by_user_id: Some("delegator".into()),
            created_at: Some(1_700_000_000),
            ..grant()
        };
        for grant in [grant(), full] {
            assert_eq!(
                serde_json::to_value(DeviceResourceGrant::from(grant.clone())).unwrap(),
                serde_json::to_value(&grant).unwrap()
            );
        }
        let paid = BillingGrantResponse {
            approved_by_user_id: Some("payer".into()),
            created_at: Some(1_700_000_000),
            ..billing()
        };
        for billing in [billing(), paid] {
            assert_eq!(
                serde_json::to_value(DeviceBillingGrant::from(billing.clone())).unwrap(),
                serde_json::to_value(&billing).unwrap()
            );
        }
    }

    const DOCUMENTED_PATHS: [&str; 10] = [
        "/devices/{id}/resource-grants",
        "/devices/{id}/resource-grants/{grant}",
        "/devices/{id}/resource-grants/{grant}/billing",
        "/devices/{id}/resource-grants/{grant}/billing/eligibility",
        "/devices/{id}/billing-grants",
        "/devices/{id}/billing-grants/{billing}",
        "/devices/{id}/billing-grants/{billing}/usage",
        "/devices/resource-summary",
        "/apps/{app_id}/device-placements",
        "/apps/{app_id}/device-metadata",
    ];

    /// Moving a schedule between the hub and a service: one path, two operations.
    const SCHEDULE_PATH: &str = "/apps/{app_id}/device-schedules/{event_id}";

    const DOCUMENTED_FIELDS: [(&str, &[&str]); 19] = [
        ("ResourceSummary", &["server_time", "devices"]),
        (
            "DeviceResourceSummary",
            &["device_id", "approvals", "billing"],
        ),
        (
            "ApprovalSummary",
            &[
                "grant_id",
                "placement_id",
                "app_id",
                "status",
                "expires_at",
                "effective_expires_at",
                "effective_limit",
                "online_access",
                "online_write_blocked",
                "payer_is_me",
                "approver_is_me",
            ],
        ),
        (
            "SpendingLimitSummary",
            &[
                "billing_grant_id",
                "grant_id",
                "limit_micros",
                "used_micros",
                "reserved_micros",
                "expires_at",
                "payer_is_me",
            ],
        ),
        (
            "AppDevicePlacements",
            &["server_time", "placements", "schedules", "event_types"],
        ),
        (
            "AppDeviceSchedule",
            &[
                "event_id",
                "state",
                "since",
                "seen_at",
                "hub_resumes_at",
                "grant_id",
                "device_id",
                "placement_id",
            ],
        ),
        ("ReleaseScheduleRequest", &["device_id", "placement_id"]),
        ("ReleasedSchedule", &["state", "since"]),
        ("GivenBackSchedule", &["hub_resumes_at"]),
        (
            "AppDevicePlacement",
            &[
                "device_id",
                "placement_id",
                "deployment_id",
                "relationship",
                "grant",
                "billing",
                "instances",
            ],
        ),
        (
            "PlacementApproval",
            &[
                "grant_id",
                "status",
                "expires_at",
                "effective_expires_at",
                "effective_limit",
                "online_access",
                "model_ids",
                "max_instances",
                "approved_by_user_id",
                "created_at",
            ],
        ),
        ("PlacementInstances", &["active", "newest_lease_expires_at"]),
        (
            "DeviceResourceGrant",
            &[
                "effective_expires_at",
                "effective_limit",
                "online_write_blocked",
                "approved_by_user_id",
                "created_at",
            ],
        ),
        ("DeviceBillingGrant", &["approved_by_user_id", "created_at"]),
        (
            "BillingEligibility",
            &["payer_id", "plan", "eligible", "models"],
        ),
        ("ModelEligibility", &["model_id", "tier", "allowed"]),
        (
            "BillingGrantUsage",
            &["billing_grant_id", "totals", "instances"],
        ),
        (
            "BillingGrantUsageTotals",
            &["used_micros", "reserved_micros", "operations"],
        ),
        (
            "BillingGrantInstanceUsage",
            &["instance_id", "first_at", "last_at"],
        ),
    ];

    #[test]
    fn cloud_paths_are_documented() {
        let spec = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
        let operations = DOCUMENTED_PATHS
            .into_iter()
            .map(|path| ("get", path))
            .chain([("put", SCHEDULE_PATH), ("delete", SCHEDULE_PATH)]);
        for (method, path) in operations {
            let operation = &spec["paths"][path][method];
            assert!(
                operation.is_object(),
                "no OpenAPI operation {method} {path}"
            );
            assert_eq!(operation["tags"], serde_json::json!(["devices"]), "{path}");
            assert!(
                operation["description"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()),
                "{path}"
            );
            assert!(operation["security"].is_array(), "{path}");
        }
        for (schema, fields) in DOCUMENTED_FIELDS {
            let properties = &spec["components"]["schemas"][schema]["properties"];
            for field in fields {
                assert!(
                    properties[field].is_object(),
                    "schema {schema} lacks {field}"
                );
            }
        }
    }

    /// A route that overlaps another one panics only when the router is built, which is
    /// when the API starts.
    #[test]
    fn the_fleet_reads_fit_beside_the_routes_they_are_mounted_with() {
        let _devices = crate::routes::devices::routes();
        let _apps = crate::routes::app::routes();
    }

    fn handler<'a>(source: &'a str, name: &str) -> &'a str {
        let body = source
            .split_once(&format!("\nasync fn {name}("))
            .unwrap_or_else(|| panic!("handler {name} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    /// The revoke paths return what was withdrawn so that it is recorded; a handler that
    /// drops it loses the only trace of who delegated credentials to a device.
    #[test]
    fn every_delegation_change_is_recorded_once_under_its_own_action() {
        let source = include_str!("routes.rs");
        for (name, change, record) in [
            (
                "create_resource_grant",
                "instances::create_grant(",
                "instances::audit_grant(&state, &user, instances::GRANT_CREATE, &grant)",
            ),
            (
                "revoke_resource_grant",
                "instances::revoke_grant(",
                "instances::audit_grant(&state, &user, instances::GRANT_REVOKE, &revoked)",
            ),
            (
                "approve_billing_grant",
                "instances::approve_billing(",
                "instances::audit_billing(&state, &user, instances::BILLING_APPROVE, &billing)",
            ),
            (
                "revoke_billing_grant",
                "instances::revoke_billing(",
                "instances::audit_billing(&state, &user, instances::BILLING_REVOKE, &revoked)",
            ),
            (
                "register_instance",
                "instances::register(",
                "instances::audit_registration(&state, &receipt)",
            ),
        ] {
            let body = handler(source, name);
            assert_eq!(body.matches("instances::audit_").count(), 1, "{name}");
            let (committed, recorded) = (body.find(change), body.find(record));
            assert!(recorded.is_some(), "{name} must record {record}");
            assert!(
                committed.is_some() && committed < recorded,
                "{name} records only a change that was made"
            );
        }
        for name in [
            "resource_grants",
            "resource_grant",
            "grant_billing",
            "billing_eligibility",
            "billing_grants",
            "billing_grant",
            "billing_usage",
            "resource_summary",
            "list_instances",
            "retire_instance",
        ] {
            assert!(
                !handler(source, name).contains("instances::audit_"),
                "{name} changes no delegation"
            );
        }
    }
}

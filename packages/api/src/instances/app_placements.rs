//! `GET /apps/{app_id}/device-placements`: where a project holds cloud approvals, as far
//! as the hub knows and the caller may see.

use super::{
    now,
    repository::{FleetApprovals, effective_ends, fleet_approvals},
    resource_summary::SpendingLimitSummary,
    routes::caller,
};
use crate::{
    devices::{self, DeviceContext, view::DeviceRelationship},
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderName, header},
};
use flow_like_device_protocol::{EffectiveLimit, OnlineProjectAccess, ResourceGrantResponse};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// The device routes answer under these headers; this route is mounted beside the app's.
const NO_STORE: [(HeaderName, &str); 2] = [
    (header::CACHE_CONTROL, "no-store"),
    (header::PRAGMA, "no-cache"),
];

/// Cloud approvals of one project on the devices in your device list.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct AppDevicePlacements {
    /// The hub's clock (Unix seconds) when it judged which approvals still run.
    pub server_time: i64,
    /// One entry per service: its approval that still runs, else its latest one.
    pub placements: Vec<AppDevicePlacement>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct AppDevicePlacement {
    pub device_id: String,
    pub placement_id: String,
    pub deployment_id: String,
    pub relationship: DeviceRelationship,
    pub grant: PlacementApproval,
    /// The spending limit in force, or `null` when there is none or it was withdrawn.
    pub billing: Option<SpendingLimitSummary>,
    pub instances: PlacementInstances,
}

/// The cloud approval of one service.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct PlacementApproval {
    pub grant_id: String,
    /// `active` or `revoked`.
    pub status: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// Unix seconds at which the approval stops working. At or before `server_time` once
    /// it has ended. Only on active approvals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_expires_at: Option<i64>,
    /// What ends the approval first: `approval`, `sharing_grant` or `access_rules`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub effective_limit: Option<EffectiveLimit>,
    /// `read_only`, `read_write`, or `null` when the service gets no project files.
    #[schema(value_type = Option<String>)]
    pub online_access: Option<OnlineProjectAccess>,
    pub model_ids: Vec<String>,
    pub max_instances: u32,
    pub approved_by_user_id: Option<String>,
    /// Unix seconds.
    pub created_at: Option<i64>,
}

/// Instances of the service that hold a lease right now.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct PlacementInstances {
    pub active: i64,
    /// Unix seconds, or `null` when no instance holds a lease.
    pub newest_lease_expires_at: Option<i64>,
}

type VisibleDevices = HashMap<String, DeviceRelationship>;

/// Keeps one approval per service on the devices in the caller's list. Approvals arrive
/// with the one that still runs first, so that is the one kept.
fn keep_visible_services(grants: &mut Vec<ResourceGrantResponse>, visible: &VisibleDevices) {
    let mut services = HashSet::new();
    grants.retain(|grant| {
        visible.contains_key(&grant.device_id)
            && services.insert((grant.device_id.clone(), grant.placement_id.clone()))
    });
}

fn listed(
    server_time: i64,
    found: FleetApprovals,
    visible: &VisibleDevices,
    viewer: &str,
) -> AppDevicePlacements {
    let FleetApprovals {
        grants,
        billing,
        leases,
        ..
    } = found;
    let placement = |grant: ResourceGrantResponse| {
        let relationship = *visible.get(&grant.device_id)?;
        let leases = leases.get(&grant.grant_id).copied().unwrap_or_default();
        Some(AppDevicePlacement {
            relationship,
            billing: billing
                .get(&grant.grant_id)
                .map(|billing| SpendingLimitSummary::new(billing, viewer)),
            instances: PlacementInstances {
                active: leases.active,
                newest_lease_expires_at: leases.newest_expires_at,
            },
            device_id: grant.device_id,
            placement_id: grant.placement_id,
            deployment_id: grant.deployment_id,
            grant: PlacementApproval {
                grant_id: grant.grant_id,
                status: grant.status,
                expires_at: grant.expires_at,
                effective_expires_at: grant.effective_expires_at,
                effective_limit: grant.effective_limit,
                online_access: grant.online_access,
                model_ids: grant.model_ids,
                max_instances: grant.max_instances,
                approved_by_user_id: grant.approved_by_user_id,
                created_at: grant.created_at,
            },
        })
    };
    AppDevicePlacements {
        server_time,
        placements: grants.into_iter().filter_map(placement).collect(),
    }
}

/// The approvals of `app_id` that `viewer` sees: every one on a device they own, and
/// their own on devices that are still in their device list.
pub(super) async fn placements(
    state: &DeviceContext<'_>,
    viewer: &str,
    app_id: &str,
) -> Result<AppDevicePlacements, ApiError> {
    devices::enabled(state)?;
    let server_time = now();
    let mut found = fleet_approvals(state.db, viewer, Some(app_id), server_time).await?;
    let mut visible = VisibleDevices::new();
    if !found.grants.is_empty() {
        visible = devices::view::visible_devices(state, viewer)
            .await?
            .into_iter()
            .map(|device| (device.device_id, device.relationship))
            .collect();
        keep_visible_services(&mut found.grants, &visible);
        effective_ends(state.db, &found.owners, &mut found.grants).await?;
    }
    Ok(listed(server_time, found, &visible, viewer))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/device-placements",
    tag = "devices",
    description = "See where a project holds cloud approvals: one entry per service on the devices in your device list, with its approval, spending limit and running instances. On a device you do not own you see only the approvals you gave.",
    params(("app_id" = String, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Cloud approvals of the project on your devices", body = AppDevicePlacements),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read this project's flows, or the account or token cannot manage devices"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
pub(crate) async fn device_placements(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<([(HeaderName, &'static str); 2], Json<AppDevicePlacements>), ApiError> {
    let viewer = caller(&state, &user).await?;
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let found = placements(&devices::context(&state), &viewer, &app_id).await?;
    Ok((NO_STORE, Json(found)))
}

#[cfg(test)]
mod tests {
    use super::super::repository::Leases;
    use super::*;
    use flow_like_device_protocol::BillingGrantResponse;

    fn approval(device: &str, placement: &str, grant: &str) -> ResourceGrantResponse {
        ResourceGrantResponse {
            grant_id: grant.into(),
            device_id: device.into(),
            placement_id: placement.into(),
            deployment_id: format!("deployment-{placement}"),
            project_id: "project".into(),
            app_id: Some("project".into()),
            delegating_user_id: "me".into(),
            authz_version: 1,
            model_ids: vec!["model".into()],
            max_instances: 2,
            expires_at: 1_900_000_000,
            status: "active".into(),
            online_access: Some(OnlineProjectAccess::ReadOnly),
            effective_expires_at: Some(1_800_000_000),
            effective_limit: Some(EffectiveLimit::AccessRules),
            online_write_blocked: None,
            approved_by_user_id: Some("me".into()),
            created_at: Some(1_700_000_000),
        }
    }

    fn visible() -> VisibleDevices {
        HashMap::from([
            ("edge".to_owned(), DeviceRelationship::Owner),
            ("lab".to_owned(), DeviceRelationship::CloudApproval),
        ])
    }

    #[test]
    fn each_service_on_a_listed_device_keeps_its_first_approval() {
        let mut grants = vec![
            approval("edge", "bot", "running"),
            approval("gone", "bot", "on-a-device-no-longer-listed"),
            approval("lab", "bot", "same-service-name-on-another-device"),
            approval("edge", "bot", "replaced"),
            approval("edge", "reports", "another-service"),
        ];
        keep_visible_services(&mut grants, &visible());
        assert_eq!(
            grants
                .iter()
                .map(|grant| grant.grant_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "running",
                "same-service-name-on-another-device",
                "another-service"
            ]
        );
    }

    /// The device console reads exactly these keys; `billing` is `null` rather than
    /// absent, and an end that is not known is left out.
    #[test]
    fn placements_are_sent_in_the_shape_the_console_reads() {
        let revoked = ResourceGrantResponse {
            status: "revoked".into(),
            effective_expires_at: None,
            effective_limit: None,
            online_access: None,
            ..approval("lab", "reports", "withdrawn")
        };
        let found = FleetApprovals {
            grants: vec![approval("edge", "bot", "running"), revoked],
            billing: HashMap::from([(
                "running".to_owned(),
                BillingGrantResponse {
                    billing_grant_id: "billing".into(),
                    grant_id: "running".into(),
                    payer_id: "other".into(),
                    authz_version: 1,
                    limit_micros: 5_000,
                    used_micros: 10,
                    reserved_micros: 20,
                    expires_at: 1_800_000_000,
                    status: "active".into(),
                    approved_by_user_id: Some("other".into()),
                    created_at: Some(1_700_000_000),
                },
            )]),
            leases: HashMap::from([(
                "running".to_owned(),
                Leases {
                    active: 2,
                    newest_expires_at: Some(1_750_000_600),
                },
            )]),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(listed(1_750_000_000, found, &visible(), "me")).unwrap(),
            serde_json::json!({
                "server_time": 1_750_000_000,
                "placements": [{
                    "device_id": "edge",
                    "placement_id": "bot",
                    "deployment_id": "deployment-bot",
                    "relationship": "owner",
                    "grant": {
                        "grant_id": "running",
                        "status": "active",
                        "expires_at": 1_900_000_000,
                        "effective_expires_at": 1_800_000_000,
                        "effective_limit": "access_rules",
                        "online_access": "read_only",
                        "model_ids": ["model"],
                        "max_instances": 2,
                        "approved_by_user_id": "me",
                        "created_at": 1_700_000_000
                    },
                    "billing": {
                        "billing_grant_id": "billing",
                        "grant_id": "running",
                        "limit_micros": 5_000,
                        "used_micros": 10,
                        "reserved_micros": 20,
                        "expires_at": 1_800_000_000,
                        "payer_is_me": false
                    },
                    "instances": { "active": 2, "newest_lease_expires_at": 1_750_000_600 }
                }, {
                    "device_id": "lab",
                    "placement_id": "reports",
                    "deployment_id": "deployment-reports",
                    "relationship": "cloud_approval",
                    "grant": {
                        "grant_id": "withdrawn",
                        "status": "revoked",
                        "expires_at": 1_900_000_000,
                        "online_access": null,
                        "model_ids": ["model"],
                        "max_instances": 2,
                        "approved_by_user_id": "me",
                        "created_at": 1_700_000_000
                    },
                    "billing": null,
                    "instances": { "active": 0, "newest_lease_expires_at": null }
                }]
            })
        );
    }

    /// The project permission is checked before anything about the project's devices is
    /// read, and the answer is never cached.
    #[test]
    fn placements_are_read_only_after_the_project_permission_check() {
        let source = include_str!("app_placements.rs");
        let handler = source
            .split_once("\npub(crate) async fn device_placements(")
            .expect("handler exists")
            .1;
        let handler = handler
            .split_once("\n}\n")
            .map_or(handler, |(body, _)| body);
        let position = |needle: &str| {
            handler
                .find(needle)
                .unwrap_or_else(|| panic!("the handler calls {needle}"))
        };
        let account = position("caller(&state, &user)");
        let permission =
            position("ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards)");
        let read = position("placements(&devices::context(&state), &viewer, &app_id)");
        assert!(account < permission && permission < read);
        assert!(handler.contains("Ok((NO_STORE, Json(found)))"));
    }
}

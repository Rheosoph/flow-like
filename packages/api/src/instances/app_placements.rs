//! `GET /apps/{app_id}/device-placements`: where a project holds cloud approvals, as far
//! as the hub knows and the caller may see.

use super::{
    now,
    repository::{FleetApprovals, effective_ends, fleet_approvals},
    resource_summary::SpendingLimitSummary,
    routes::caller,
    schedules::{self, AppDeviceSchedule, ScheduleRow},
};
use crate::{
    devices::{self, DeviceContext, view::DeviceRelationship},
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::device_metadata::DEVICE_EVENT_TYPES,
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
    /// The event types of this project you can deploy to a device from this hub. An event
    /// with a default Page goes to a device whatever its type. Schedules (`cron`) and bots
    /// (`telegram`, `discord`) run in one place at a time and are listed only while this hub
    /// can record where.
    pub event_types: Vec<String>,
    /// The project's schedules and bots that a device runs, is about to run, or just ran, at
    /// most 512. One the hub has and nobody released is not listed. Missing on a hub that
    /// cannot hand schedules or bots to devices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedules: Option<Vec<AppDeviceSchedule>>,
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

/// The types this hub hands to devices. Without the claim table it cannot record where a
/// schedule or a bot runs, so it hands over neither.
fn event_types(claims: bool) -> Vec<String> {
    DEVICE_EVENT_TYPES
        .into_iter()
        .filter(|event_type| claims || !schedules::CLAIMED_EVENT_TYPES.contains(event_type))
        .map(str::to_owned)
        .collect()
}

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
    schedule_rows: Option<Vec<ScheduleRow>>,
) -> AppDevicePlacements {
    let placement = |grant: ResourceGrantResponse| {
        let relationship = *visible.get(&grant.device_id)?;
        let leases = found
            .leases
            .get(&grant.grant_id)
            .copied()
            .unwrap_or_default();
        Some(AppDevicePlacement {
            relationship,
            billing: found
                .billing
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
        placements: found.grants.into_iter().filter_map(placement).collect(),
        event_types: event_types(schedule_rows.is_some()),
        schedules: schedule_rows.map(|rows| {
            schedules::listed(rows, |device| visible.contains_key(device), server_time)
        }),
    }
}

/// The approvals of `app_id` that `viewer` sees: every one on a device they own, and
/// their own on devices that are still in their device list. With them, where the project's
/// schedules run; which device runs one is said only for a device in that list.
pub(super) async fn placements(
    state: &DeviceContext<'_>,
    viewer: &str,
    app_id: &str,
) -> Result<AppDevicePlacements, ApiError> {
    devices::enabled(state)?;
    let server_time = now();
    let mut found = fleet_approvals(state.db, viewer, Some(app_id), server_time).await?;
    let schedule_rows = schedules::listed_rows(state.db, app_id, server_time).await?;
    let names_devices = schedule_rows
        .iter()
        .flatten()
        .any(|row| row.device_id().is_some());
    let mut visible = VisibleDevices::new();
    if !found.grants.is_empty() || names_devices {
        visible = devices::view::visible_devices(state, viewer)
            .await?
            .into_iter()
            .map(|device| (device.device_id, device.relationship))
            .collect();
        keep_visible_services(&mut found.grants, &visible);
        effective_ends(state.db, &found.owners, &mut found.grants).await?;
    }
    Ok(listed(server_time, found, &visible, viewer, schedule_rows))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/device-placements",
    tag = "devices",
    description = "See where a project holds cloud approvals: one entry per service on the devices in your device list, with its approval, spending limit and running instances. On a device you do not own you see only the approvals you gave. The answer also says which event types you can deploy to a device and where the project's schedules and bots run.",
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
        assert!(
            serde_json::to_value(listed(
                1_750_000_000,
                FleetApprovals::default(),
                &visible(),
                "me",
                None
            ))
            .unwrap()
            .get("schedules")
            .is_none(),
            "a hub without the schedules table reads like one that cannot hand schedules over"
        );
        assert_eq!(
            serde_json::to_value(listed(
                1_750_000_000,
                found,
                &visible(),
                "me",
                Some(Vec::new())
            ))
            .unwrap(),
            serde_json::json!({
                "server_time": 1_750_000_000,
                "event_types": ["http","simple_chat","rest","mcp","daemon","cron","api","quick_action","generic_form","telegram","discord"],
                "schedules": [],
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

    /// Without the claim table no claim can be recorded, so neither a schedule nor a bot is
    /// offered for a device.
    #[test]
    fn a_hub_without_the_claim_table_offers_no_schedule_or_bot() {
        let answer = serde_json::to_value(listed(
            1_750_000_000,
            FleetApprovals::default(),
            &visible(),
            "me",
            None,
        ))
        .unwrap();
        assert_eq!(
            answer["event_types"],
            serde_json::json!([
                "http",
                "simple_chat",
                "rest",
                "mcp",
                "daemon",
                "api",
                "quick_action",
                "generic_form"
            ])
        );
    }

    /// Its presence tells a client that this hub hands the types of round two to devices.
    #[test]
    fn the_event_types_are_documented_for_the_apps_author() {
        use utoipa::OpenApi;
        let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
        let schema = &spec["components"]["schemas"]["AppDevicePlacements"];
        let event_types = &schema["properties"]["event_types"];
        assert_eq!(event_types["type"], "array");
        assert_eq!(event_types["items"]["type"], "string");
        assert!(
            event_types["description"]
                .as_str()
                .is_some_and(|text| text.contains("deploy to a device"))
        );
        assert!(
            schema["required"]
                .as_array()
                .is_some_and(|required| required.contains(&"event_types".into()))
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

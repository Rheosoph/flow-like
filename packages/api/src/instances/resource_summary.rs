//! `GET /devices/resource-summary`: the approvals and spending limits of every device in
//! one read, so no screen has to ask each device for its lists.

use super::{
    now, project,
    repository::{FleetApprovals, effective_ends, fleet_approvals},
};
use crate::{
    devices::{self, DeviceContext},
    error::ApiError,
    state::AppState,
};
use flow_like_device_protocol::{
    BillingGrantResponse, EffectiveLimit, OnlineProjectAccess, OnlineWriteBlock,
    ResourceGrantResponse,
};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

/// Cloud approvals and spending limits on every device where you can see them.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct ResourceSummary {
    /// The hub's clock (Unix seconds) when it judged which approvals still run.
    pub server_time: i64,
    /// Devices with at least one approval you can see.
    pub devices: Vec<DeviceResourceSummary>,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct DeviceResourceSummary {
    pub device_id: String,
    /// Approvals that still run first, then the newest.
    pub approvals: Vec<ApprovalSummary>,
    /// The spending limit in force on each approval that has one. Withdrawn limits are
    /// not listed.
    pub billing: Vec<SpendingLimitSummary>,
}

/// One cloud approval, without what only its own page needs.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct ApprovalSummary {
    pub grant_id: String,
    pub placement_id: String,
    pub app_id: Option<String>,
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
    /// `storage_full` while the project's storage is full and the service can only read.
    /// Only shown to the approver and to people who can read the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub online_write_blocked: Option<OnlineWriteBlock>,
    /// Whether the spending limit in force on this approval is yours.
    pub payer_is_me: bool,
    /// Whether you gave this approval.
    pub approver_is_me: bool,
}

/// The spending limit in force on a cloud approval.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct SpendingLimitSummary {
    pub billing_grant_id: String,
    pub grant_id: String,
    /// Micro EUR across every instance of the service; it does not renew.
    pub limit_micros: i64,
    pub used_micros: i64,
    pub reserved_micros: i64,
    /// Unix seconds. In the past once the limit has run out.
    pub expires_at: i64,
    pub payer_is_me: bool,
}

impl SpendingLimitSummary {
    pub(super) fn new(billing: &BillingGrantResponse, viewer: &str) -> Self {
        Self {
            billing_grant_id: billing.billing_grant_id.clone(),
            grant_id: billing.grant_id.clone(),
            limit_micros: billing.limit_micros,
            used_micros: billing.used_micros,
            reserved_micros: billing.reserved_micros,
            expires_at: billing.expires_at,
            payer_is_me: billing.payer_id == viewer,
        }
    }
}

impl ApprovalSummary {
    fn new(
        grant: ResourceGrantResponse,
        limit: Option<&SpendingLimitSummary>,
        viewer: &str,
    ) -> Self {
        Self {
            approver_is_me: grant.delegating_user_id == viewer,
            payer_is_me: limit.is_some_and(|limit| limit.payer_is_me),
            grant_id: grant.grant_id,
            placement_id: grant.placement_id,
            app_id: grant.app_id,
            status: grant.status,
            expires_at: grant.expires_at,
            effective_expires_at: grant.effective_expires_at,
            effective_limit: grant.effective_limit,
            online_access: grant.online_access,
            online_write_blocked: grant.online_write_blocked,
        }
    }
}

/// Groups what a read found by device, in the order of each device's first approval.
pub(super) fn summarize(server_time: i64, found: FleetApprovals, viewer: &str) -> ResourceSummary {
    let mut devices: Vec<DeviceResourceSummary> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    for grant in found.grants {
        let limit = found
            .billing
            .get(&grant.grant_id)
            .map(|billing| SpendingLimitSummary::new(billing, viewer));
        let position = *positions.entry(grant.device_id.clone()).or_insert_with(|| {
            devices.push(DeviceResourceSummary {
                device_id: grant.device_id.clone(),
                approvals: Vec::new(),
                billing: Vec::new(),
            });
            devices.len() - 1
        });
        let device = &mut devices[position];
        device
            .approvals
            .push(ApprovalSummary::new(grant, limit.as_ref(), viewer));
        device.billing.extend(limit);
    }
    ResourceSummary {
        server_time,
        devices,
    }
}

/// Every approval `viewer` sees across the fleet with its real end, and the hub time it
/// was judged at.
pub(super) async fn visible_approvals(
    state: &DeviceContext<'_>,
    viewer: &str,
) -> Result<(i64, FleetApprovals), ApiError> {
    devices::enabled(state)?;
    let server_time = now();
    let mut found = fleet_approvals(state.db, viewer, None, server_time).await?;
    effective_ends(state.db, &found.owners, &mut found.grants).await?;
    Ok((server_time, found))
}

pub(crate) async fn resource_summary(
    state: &AppState,
    viewer: &str,
) -> Result<ResourceSummary, ApiError> {
    let (server_time, mut found) = visible_approvals(&devices::context(state), viewer).await?;
    project::mark_blocked_writes(state, viewer, &mut found.grants).await?;
    Ok(summarize(server_time, found, viewer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(device: &str, placement: &str, delegator: &str) -> ResourceGrantResponse {
        ResourceGrantResponse {
            grant_id: format!("grant-{placement}"),
            device_id: device.into(),
            placement_id: placement.into(),
            deployment_id: "deployment".into(),
            project_id: "project".into(),
            app_id: None,
            delegating_user_id: delegator.into(),
            authz_version: 1,
            model_ids: vec!["model".into()],
            max_instances: 1,
            expires_at: 1_900_000_000,
            status: "active".into(),
            online_access: None,
            effective_expires_at: Some(1_900_000_000),
            effective_limit: Some(EffectiveLimit::Approval),
            online_write_blocked: None,
            approved_by_user_id: Some(delegator.into()),
            created_at: Some(1_700_000_000),
        }
    }

    fn limit(placement: &str, payer: &str) -> (String, BillingGrantResponse) {
        (
            format!("grant-{placement}"),
            BillingGrantResponse {
                billing_grant_id: format!("billing-{placement}"),
                grant_id: format!("grant-{placement}"),
                payer_id: payer.into(),
                authz_version: 1,
                limit_micros: 5_000,
                used_micros: 10,
                reserved_micros: 20,
                expires_at: 1_800_000_000,
                status: "active".into(),
                approved_by_user_id: Some(payer.into()),
                created_at: Some(1_700_000_000),
            },
        )
    }

    fn revoked(device: &str, placement: &str, delegator: &str) -> ResourceGrantResponse {
        ResourceGrantResponse {
            status: "revoked".into(),
            effective_expires_at: None,
            effective_limit: None,
            ..approval(device, placement, delegator)
        }
    }

    /// A read-and-write approval that a full project storage narrows to reads.
    fn blocked(device: &str, placement: &str, delegator: &str) -> ResourceGrantResponse {
        ResourceGrantResponse {
            app_id: Some("project".into()),
            online_access: Some(OnlineProjectAccess::ReadWrite),
            effective_expires_at: Some(1_800_000_000),
            effective_limit: Some(EffectiveLimit::SharingGrant),
            online_write_blocked: Some(OnlineWriteBlock::StorageFull),
            ..approval(device, placement, delegator)
        }
    }

    #[test]
    fn approvals_are_grouped_by_device_and_say_what_is_the_viewers() {
        let found = FleetApprovals {
            grants: vec![
                approval("edge", "mine", "me"),
                approval("lab", "theirs", "other"),
                revoked("edge", "old", "me"),
            ],
            billing: HashMap::from([limit("mine", "other"), limit("theirs", "me")]),
            ..Default::default()
        };
        let summary = summarize(1_750_000_000, found, "me");
        assert_eq!(summary.server_time, 1_750_000_000);
        let listed = |device: usize| {
            let device = &summary.devices[device];
            let approvals = device.approvals.iter().map(|approval| {
                (
                    approval.placement_id.as_str(),
                    approval.approver_is_me,
                    approval.payer_is_me,
                )
            });
            let limits = device
                .billing
                .iter()
                .map(|limit| (limit.billing_grant_id.as_str(), limit.payer_is_me));
            (
                device.device_id.as_str(),
                approvals.collect::<Vec<_>>(),
                limits.collect::<Vec<_>>(),
            )
        };
        assert_eq!(summary.devices.len(), 2);
        assert_eq!(
            listed(0),
            (
                "edge",
                vec![("mine", true, false), ("old", true, false)],
                vec![("billing-mine", false)]
            ),
            "approved by me; the limit in force is someone else's, or there is none"
        );
        assert_eq!(
            listed(1),
            (
                "lab",
                vec![("theirs", false, true)],
                vec![("billing-theirs", true)]
            )
        );
    }

    /// The device console reads exactly these keys; an end that is not known is left out,
    /// never sent as `null`.
    #[test]
    fn the_summary_is_sent_in_the_shape_the_console_reads() {
        let found = FleetApprovals {
            grants: vec![
                blocked("edge", "online", "me"),
                revoked("edge", "old", "other"),
            ],
            billing: HashMap::from([limit("online", "me")]),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(summarize(1_750_000_000, found, "me")).unwrap(),
            serde_json::json!({
                "server_time": 1_750_000_000,
                "devices": [{
                    "device_id": "edge",
                    "approvals": [{
                        "grant_id": "grant-online",
                        "placement_id": "online",
                        "app_id": "project",
                        "status": "active",
                        "expires_at": 1_900_000_000,
                        "effective_expires_at": 1_800_000_000,
                        "effective_limit": "sharing_grant",
                        "online_access": "read_write",
                        "online_write_blocked": "storage_full",
                        "payer_is_me": true,
                        "approver_is_me": true
                    }, {
                        "grant_id": "grant-old",
                        "placement_id": "old",
                        "app_id": null,
                        "status": "revoked",
                        "expires_at": 1_900_000_000,
                        "online_access": null,
                        "payer_is_me": false,
                        "approver_is_me": false
                    }],
                    "billing": [{
                        "billing_grant_id": "billing-online",
                        "grant_id": "grant-online",
                        "limit_micros": 5_000,
                        "used_micros": 10,
                        "reserved_micros": 20,
                        "expires_at": 1_800_000_000,
                        "payer_is_me": true
                    }]
                }]
            })
        );
    }
}

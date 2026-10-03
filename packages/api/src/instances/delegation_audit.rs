//! Delegating project storage, models or billing to a device, and everything an instance
//! does with that delegation, is recorded on the delegated project's chain (the platform
//! chain for model-only grants), so its owner can trace which device held credentials.

use super::*;
use crate::{audit::AuditRecordInput, middleware::jwt::AppUser};

pub(crate) const GRANT_CREATE: &str = "device.delegation.grant.create";
pub(crate) const GRANT_REVOKE: &str = "device.delegation.grant.revoke";
pub(crate) const BILLING_APPROVE: &str = "device.delegation.billing.approve";
pub(crate) const BILLING_REVOKE: &str = "device.delegation.billing.revoke";
const REGISTER_ACTION: &str = "instance.register";

/// Instances act for their delegating user; their records name the instance as the actor.
pub(super) fn instance_record(
    instance_id: &str,
    scope: Option<&str>,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    details: serde_json::Value,
) -> AuditRecordInput {
    let mut input = AuditRecordInput::system(
        &format!("instance:{instance_id}"),
        action,
        resource_type,
        resource_id,
    )
    .with_details(details);
    input.scope = scope.map(str::to_owned);
    input.actor_ip = crate::audit::request::actor_ip();
    input
}

pub(crate) async fn audit_grant(
    state: &AppState,
    user: &AppUser,
    action: &str,
    grant: &ResourceGrantResponse,
) {
    if !crate::audit::records(&state.platform_config.audit, action) {
        return;
    }
    crate::audit::record_for_user(
        state,
        user,
        grant.app_id.clone(),
        action.into(),
        "PlacementResourceGrant".into(),
        grant.grant_id.clone(),
        Some(grant_details(grant)),
    )
    .await;
}

pub(crate) async fn audit_billing(
    state: &AppState,
    user: &AppUser,
    action: &str,
    billing: &BillingGrantResponse,
) {
    if !crate::audit::records(&state.platform_config.audit, action) {
        return;
    }
    let Some(grant) = delegation(state, &billing.grant_id, action).await else {
        return;
    };
    crate::audit::record_for_user(
        state,
        user,
        grant.app_id.clone(),
        action.into(),
        "PlacementBillingGrant".into(),
        billing.billing_grant_id.clone(),
        Some(billing_details(&grant, billing)),
    )
    .await;
}

/// Opens the trail that the instance's storage leases and replayed writes continue.
pub(crate) async fn audit_registration(state: &AppState, receipt: &InstanceReceipt) {
    if !crate::audit::records(&state.platform_config.audit, REGISTER_ACTION) {
        return;
    }
    let Some(grant) = delegation(state, &receipt.grant_id, REGISTER_ACTION).await else {
        return;
    };
    crate::audit::record_entry(state, registration_record(receipt, &grant)).await;
}

async fn delegation(
    state: &AppState,
    grant_id: &str,
    action: &str,
) -> Option<ResourceGrantResponse> {
    match read_grant(&state.db, grant_id).await {
        Ok(grant) => Some(grant.info),
        Err(error) => {
            crate::audit::request::record_failure();
            tracing::error!(%error, action, grant_id, "AUDIT FAILURE: delegation could not be read");
            None
        }
    }
}

fn grant_details(grant: &ResourceGrantResponse) -> serde_json::Value {
    serde_json::json!({
        "device_id": grant.device_id,
        "delegated_user": grant.delegating_user_id,
        "project_id": grant.project_id,
        "placement_id": grant.placement_id,
        "deployment_id": grant.deployment_id,
        "access": grant.online_access,
        "models": grant.model_ids.len(),
        "max_instances": grant.max_instances,
        "expires_at": grant.expires_at,
        "status": grant.status,
    })
}

fn billing_details(
    grant: &ResourceGrantResponse,
    billing: &BillingGrantResponse,
) -> serde_json::Value {
    serde_json::json!({
        "device_id": grant.device_id,
        "grant_id": billing.grant_id,
        "delegated_user": grant.delegating_user_id,
        "payer_id": billing.payer_id,
        "limit_micros": billing.limit_micros,
        "expires_at": billing.expires_at,
        "status": billing.status,
    })
}

fn registration_record(
    receipt: &InstanceReceipt,
    grant: &ResourceGrantResponse,
) -> AuditRecordInput {
    instance_record(
        &receipt.instance_id,
        grant.app_id.as_deref(),
        REGISTER_ACTION,
        "WorkloadInstance",
        &receipt.instance_id,
        serde_json::json!({
            "device_id": receipt.device_id,
            "grant_id": receipt.grant_id,
            "billing_grant_id": receipt.billing_grant_id,
            "delegated_user": grant.delegating_user_id,
            "purpose": receipt.purpose,
            "access": grant.online_access,
            "lease_expires_at": receipt.lease_expires_at,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::AuditActorType;

    fn grant(app_id: Option<&str>) -> ResourceGrantResponse {
        ResourceGrantResponse {
            grant_id: "grant".into(),
            device_id: "device".into(),
            placement_id: "placement".into(),
            deployment_id: "deployment".into(),
            project_id: "project".into(),
            app_id: app_id.map(str::to_owned),
            delegating_user_id: "delegator".into(),
            authz_version: 2,
            model_ids: vec!["model-a".into(), "model-b".into()],
            max_instances: 3,
            expires_at: 1_900_000_000,
            status: "revoked".into(),
            online_access: app_id.map(|_| OnlineProjectAccess::ReadWrite),
            effective_expires_at: None,
            effective_limit: None,
            online_write_blocked: None,
            approved_by_user_id: Some("delegator".into()),
            created_at: Some(1_800_000_000),
        }
    }

    #[test]
    fn delegation_changes_are_security_evidence_at_every_audit_level() {
        for action in [GRANT_CREATE, GRANT_REVOKE, BILLING_APPROVE, BILLING_REVOKE] {
            assert_eq!(
                crate::audit::required_level(action),
                crate::audit::AuditLevel::Minimal,
                "{action}"
            );
            assert_eq!(
                crate::audit::RetentionClass::of(action),
                crate::audit::RetentionClass::Evidence,
                "{action}"
            );
        }
        assert_eq!(
            crate::audit::required_level(REGISTER_ACTION),
            crate::audit::AuditLevel::Standard
        );
    }

    #[test]
    fn delegation_details_name_the_device_delegate_and_access() {
        let details = grant_details(&grant(Some("project")));
        assert_eq!(details["device_id"], "device");
        assert_eq!(details["delegated_user"], "delegator");
        assert_eq!(
            details["access"],
            serde_json::json!(OnlineProjectAccess::ReadWrite)
        );
        assert_eq!(details["models"], 2);
        assert_eq!(details["expires_at"], 1_900_000_000);
        assert_eq!(details["status"], "revoked");
        let billing = BillingGrantResponse {
            billing_grant_id: "billing".into(),
            grant_id: "grant".into(),
            payer_id: "payer".into(),
            authz_version: 1,
            limit_micros: 5_000,
            used_micros: 0,
            reserved_micros: 0,
            expires_at: 1_800_000_000,
            status: "active".into(),
            approved_by_user_id: Some("payer".into()),
            created_at: Some(1_700_000_000),
        };
        let details = billing_details(&grant(None), &billing);
        assert_eq!(details["payer_id"], "payer");
        assert_eq!(details["delegated_user"], "delegator");
        assert_eq!(details["device_id"], "device");
        assert_eq!(details["limit_micros"], 5_000);
    }

    #[test]
    fn instance_records_name_the_instance_and_the_delegated_project() {
        let receipt = InstanceReceipt {
            instance_id: "instance".into(),
            purpose: InstancePurpose::Workload,
            device_id: "device".into(),
            grant_id: "grant".into(),
            billing_grant_id: None,
            workload_key: SigningKey::generate().public_key(),
            key_epoch: 1,
            registered_at: 1_700_000_000,
            lease_expires_at: 1_700_000_300,
            registration_jws: "jws".into(),
        };
        let online = registration_record(&receipt, &grant(Some("project")));
        assert_eq!(online.actor_id, "instance:instance");
        assert!(matches!(online.actor_type, AuditActorType::System));
        assert_eq!(online.scope.as_deref(), Some("project"));
        assert_eq!(online.resource_id, "instance");
        let details = online.details.unwrap();
        assert_eq!(details["delegated_user"], "delegator");
        assert_eq!(details["device_id"], "device");
        let models_only = registration_record(&receipt, &grant(None));
        assert_eq!(models_only.scope, None);
    }
}

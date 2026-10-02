//! OpenAPI document for the device registry and access routes, merged into `ApiDoc`.

use super::{
    management::{AccessGrant, AccessRole, MyAccess},
    recovery::{AccountBackupList, AccountBackupView},
    repository::AuthRejectionCode,
    view::{
        AuthRejection, CloudApprovals, DeviceEnrollmentState, DeviceEnrollmentView, DeviceLimits,
        DeviceRelationship, DeviceUsage, DeviceUsageView, DeviceView,
    },
};
use crate::routes::devices::RenameDeviceRequest;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::devices::list,
        crate::routes::devices::status,
        crate::routes::devices::rename,
        crate::routes::devices::list_enrollments,
        crate::routes::devices::usage,
        super::recovery::list,
        super::management::my_access,
    ),
    components(schemas(
        DeviceView,
        DeviceRelationship,
        CloudApprovals,
        AuthRejection,
        AuthRejectionCode,
        DeviceEnrollmentView,
        DeviceEnrollmentState,
        RenameDeviceRequest,
        DeviceUsageView,
        DeviceLimits,
        DeviceUsage,
        AccountBackupList,
        AccountBackupView,
        MyAccess,
        AccessGrant,
        AccessRole,
    ))
)]
pub(crate) struct RegistryApi;

#[cfg(test)]
mod tests {
    use crate::openapi::ApiDoc;
    use serde_json::Value;
    use std::collections::BTreeSet;
    use utoipa::OpenApi;

    #[test]
    fn registry_paths_are_documented() {
        let spec: Value = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
        for (path, method) in [
            ("/devices", "get"),
            ("/devices/{id}", "get"),
            ("/devices/{id}", "patch"),
            ("/devices/enrollments", "get"),
            ("/devices/usage", "get"),
            ("/devices/controller-vaults", "get"),
            ("/devices/{id}/management/my-access", "get"),
        ] {
            let operation = spec
                .pointer(&format!("/paths/{}/{method}", path.replace('/', "~1")))
                .unwrap_or_else(|| panic!("OpenAPI path '{path}' has no '{method}' operation"));
            assert_eq!(
                operation.pointer("/tags/0").and_then(Value::as_str),
                Some("devices"),
                "{method} {path} must be tagged 'devices'"
            );
            assert!(
                operation
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|description| !description.is_empty()),
                "{method} {path} needs a user-facing description"
            );
        }
        assert_eq!(
            spec.pointer(
                "/paths/~1devices/get/responses/200/content/application~1json/schema/items/$ref"
            )
            .and_then(Value::as_str),
            Some("#/components/schemas/DeviceView")
        );
        assert_eq!(
            spec.pointer("/paths/~1devices~1enrollments/get/parameters/0/name")
                .and_then(Value::as_str),
            Some("state")
        );
    }

    #[test]
    fn usage_backup_and_access_answers_name_their_schemas() {
        let spec: Value = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
        for (path, schema) in [
            ("/devices/usage", "DeviceUsageView"),
            ("/devices/controller-vaults", "AccountBackupList"),
            ("/devices/{id}/management/my-access", "DeviceMyAccess"),
        ] {
            assert_eq!(
                spec.pointer(&format!(
                    "/paths/{}/get/responses/200/content/application~1json/schema/$ref",
                    path.replace('/', "~1")
                ))
                .and_then(Value::as_str),
                Some(format!("#/components/schemas/{schema}").as_str()),
                "GET {path} must answer with {schema}"
            );
        }
        assert_eq!(
            spec.pointer("/components/schemas/DeviceAccessRole/enum"),
            Some(&serde_json::json!(["owner", "grantee"]))
        );
    }

    /// A listed backup never documents the encrypted keys, and access never documents
    /// the signed rules, another account or a key.
    const DOCUMENTED_FIELDS: [(&str, &[&str]); 5] = [
        ("DeviceUsageView", &["server_time", "limits", "usage"]),
        ("AccountBackupList", &["vaults", "used", "max"]),
        (
            "AccountBackupView",
            &["key_id", "revision", "updated_at", "public_key_thumbprint"],
        ),
        (
            "DeviceMyAccess",
            &[
                "device_id",
                "role",
                "owner_id",
                "policy_version",
                "policy_expires_at",
                "applied_version",
                "applied",
                "grants",
            ],
        ),
        (
            "DeviceAccessGrant",
            &[
                "grant_id",
                "scope",
                "capabilities",
                "expires_at",
                "controller_key_thumbprint",
                "group_id",
            ],
        ),
    ];

    #[test]
    fn usage_backup_and_access_schemas_carry_no_secrets() {
        let spec: Value = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
        for (schema, fields) in DOCUMENTED_FIELDS {
            let documented: BTreeSet<&str> = spec
                .pointer(&format!("/components/schemas/{schema}/properties"))
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{schema} is not a registered schema"))
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                documented,
                fields.iter().copied().collect(),
                "{schema} documents other fields than it sends"
            );
        }
    }

    #[test]
    fn registry_schemas_describe_what_people_receive_and_send() {
        let spec: Value = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
        let view = spec
            .pointer("/components/schemas/DeviceView/properties")
            .and_then(Value::as_object)
            .expect("DeviceView is a registered schema");
        for field in [
            "display_name",
            "revoked_at",
            "relationship",
            "access_expires_at",
            "access_rules_expire_at",
            "cloud_approvals",
            "auth_rejection",
        ] {
            assert!(
                view.contains_key(field),
                "DeviceView.{field} is undocumented"
            );
        }
        assert_eq!(
            view.get("status").and_then(|status| status.get("enum")),
            Some(&serde_json::json!(["active", "revoked"]))
        );
        assert_eq!(
            spec.pointer("/components/schemas/RenameDeviceRequest/required"),
            Some(&serde_json::json!(["display_name"])),
            "a rename must state the display name, even to clear it"
        );
    }
}

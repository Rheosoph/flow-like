//! Human-facing device rows (`DeviceView`) for the registry routes. The heartbeat and
//! the device-principal branch of `GET /devices/{id}` keep returning plain `DeviceStatus`.

use super::{
    DeviceContext, recovery,
    repository::{self, AuthRejectionCode, EnrollmentRecord, count},
};
use crate::error::ApiError;
use flow_like::hub::StandaloneConfig;
use flow_like_device_protocol::{DeviceIdentity, DeviceRegistrationStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::{
    ToSchema,
    openapi::schema::{Object, ObjectBuilder, Type},
};

/// Each row source is bounded on its own; active enrollment policy caps a fleet at 1000.
const SOURCE_LIMIT: usize = 1000;

/// Recipients of the latest access rules of an active device whose owner is active.
/// `$1` is the caller and `$2` the current time.
pub(super) const SHARED: &str = r#"d.status='active' AND d."ownerId"<>$1 AND EXISTS(SELECT 1 FROM "User" u WHERE u.id=d."ownerId" AND u.status='ACTIVE') AND EXISTS(SELECT 1 FROM "DeviceManagementRecipient" r WHERE r."deviceId"=d.id AND r."userId"=$1 AND r."expiresAt">$2 AND r.version=(SELECT MAX(p.version) FROM "DeviceManagementPolicy" p WHERE p."deviceId"=d.id))"#;

/// Cloud access the caller delegated to placements on device `d` and can still revoke.
const DELEGATED: &str = r#""PlacementResourceGrant" g WHERE g."deviceId"=d.id AND g."delegatingUserId"=$1 AND g.status='active' AND g."expiresAt">$2"#;
/// Spending the caller pays for on device `d` and can still revoke.
const SPONSORED: &str = r#""PlacementBillingGrant" b JOIN "PlacementResourceGrant" g ON g.id=b."grantId" WHERE g."deviceId"=d.id AND b."payerId"=$1 AND b.status='active' AND b."expiresAt">$2"#;

const COLUMNS: &str = r#"d.id,d."ownerId",d.name,d.status,d."authEpoch",d.identity,d."registeredAt",d."lastSeenAt",d."revokedAt",d."displayName""#;
const RULES_EXPIRY: &str = r#"(SELECT p."expiresAt" FROM "DeviceManagementPolicy" p WHERE p."deviceId"=d.id AND p.version=(SELECT MAX(q.version) FROM "DeviceManagementPolicy" q WHERE q."deviceId"=d.id)) AS "rulesExpireAt""#;
const GRANT_EXPIRY: &str = r#"(SELECT MAX(r."expiresAt") FROM "DeviceManagementRecipient" r WHERE r."deviceId"=d.id AND r."userId"=$1 AND r."expiresAt">$2 AND r.version=(SELECT MAX(p.version) FROM "DeviceManagementPolicy" p WHERE p."deviceId"=d.id)) AS "grantExpiresAt""#;
/// A rejection stops being reported once the device has checked in again.
const REJECTION: &str = r#"LEFT JOIN "DeviceAuthRejection" j ON j."deviceId"=d.id AND j."lastAt">COALESCE(d."lastSeenAt",0)"#;
const REJECTION_COLUMNS: &str = r#"j.code AS "rejectionCode",j."skewSeconds" AS "rejectionSkew",j.count AS "rejectionCount",j."firstAt" AS "rejectionFirstAt",j."lastAt" AS "rejectionLastAt""#;

/// Revoked history never pushes an active device out of the bounded response.
const ACTIVE_FIRST: &str =
    r#"CASE WHEN d.status='active' THEN 0 ELSE 1 END,d."registeredAt" DESC,d.id"#;
const NEWEST_FIRST: &str = r#"d."registeredAt" DESC,d.id"#;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

/// How the caller relates to a device. A device visible for several reasons carries
/// the strongest one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeviceRelationship {
    /// The caller registered the device.
    Owner,
    /// The owner's current access rules grant the caller access.
    Shared,
    /// The caller only approved cloud access or spending for a service on it.
    CloudApproval,
}

impl DeviceRelationship {
    const PRECEDENCE: [Self; 3] = [Self::Owner, Self::Shared, Self::CloudApproval];

    /// `projection` over this row source of `"ManagedDevice" d`, ordered and bounded.
    fn select(self, projection: &str, join: &str, scope: &str) -> String {
        let (predicate, order) = match self {
            Self::Owner => (r#"d."ownerId"=$1"#.to_owned(), ACTIVE_FIRST),
            Self::Shared => (SHARED.to_owned(), NEWEST_FIRST),
            // Approvals stay reachable after sharing ends or the device is revoked, so
            // the person who gave them can still withdraw them.
            Self::CloudApproval => (
                format!(
                    r#"d."ownerId"<>$1 AND (EXISTS(SELECT 1 FROM {DELEGATED}) OR EXISTS(SELECT 1 FROM {SPONSORED}))"#
                ),
                NEWEST_FIRST,
            ),
        };
        format!(
            r#"SELECT {projection} FROM "ManagedDevice" d {join} WHERE {predicate}{scope} ORDER BY {order} LIMIT {SOURCE_LIMIT}"#
        )
    }

    /// Ids only: the owner source needs no clock.
    fn ids(self, user_id: &str, now: i64) -> Statement {
        let mut values = vec![Value::from(user_id)];
        if self != Self::Owner {
            values.push(now.into());
        }
        sql(&self.select("d.id", "", ""), values)
    }

    fn rows(self, user_id: &str, now: i64, device_id: Option<&str>) -> Statement {
        let (facts, join) = match self {
            Self::Owner => (format!(",{RULES_EXPIRY},{REJECTION_COLUMNS}"), REJECTION),
            Self::Shared => (format!(",{RULES_EXPIRY},{GRANT_EXPIRY}"), ""),
            Self::CloudApproval => (String::new(), ""),
        };
        let mut values = vec![Value::from(user_id), now.into()];
        let scope = device_id.map_or("", |id| {
            values.push(id.into());
            " AND d.id=$3"
        });
        let projection = format!(
            r#"{COLUMNS},(SELECT COUNT(*) FROM {DELEGATED}) AS "resourceGrants",(SELECT MAX(g."expiresAt") FROM {DELEGATED}) AS "resourceGrantsExpireAt",(SELECT COUNT(*) FROM {SPONSORED}) AS "billingGrants",(SELECT MAX(b."expiresAt") FROM {SPONSORED}) AS "billingGrantsExpireAt"{facts}"#
        );
        sql(&self.select(&projection, join, scope), values)
    }
}

/// A device the caller may see, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisibleDevice {
    pub device_id: String,
    pub relationship: DeviceRelationship,
}

/// Active cloud approvals the caller gave for services on a device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = DeviceCloudApprovals)]
pub(crate) struct CloudApprovals {
    /// Cloud access approvals the caller delegated.
    pub resource_grants: u64,
    /// Spending approvals the caller pays for.
    pub billing_grants: u64,
    /// When the last of them ends (Unix seconds).
    pub expires_at: i64,
}

/// Why the hub is refusing the device since it last checked in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = DeviceAuthRejection)]
pub(crate) struct AuthRejection {
    pub code: AuthRejectionCode,
    /// Device clock minus hub clock in seconds, for `clock_skew`.
    pub skew_seconds: Option<i64>,
    /// Refusals for this reason since the device last checked in.
    pub count: u64,
    pub first_at: i64,
    pub last_at: i64,
}

fn registration_status_schema() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(["active", "revoked"]))
        .build()
}

/// A registered device as a person sees it. Device agents never receive this shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DeviceView {
    pub device_id: String,
    pub owner_id: String,
    /// Name from the signed setup package; it never changes.
    pub name: String,
    /// The owner's label for the device, shown instead of `name`.
    pub display_name: Option<String>,
    /// Public device keys: `auth_key`, `management_key` and `telemetry_key`.
    #[schema(value_type = Object)]
    pub identity: DeviceIdentity,
    #[schema(schema_with = registration_status_schema)]
    pub status: DeviceRegistrationStatus,
    pub registered_at: i64,
    pub last_seen_at: Option<i64>,
    pub auth_epoch: u64,
    /// Unset for devices revoked before the hub recorded the time.
    pub revoked_at: Option<i64>,
    pub relationship: DeviceRelationship,
    /// When the caller's shared access ends; unset for the owner.
    pub access_expires_at: Option<i64>,
    /// When the owner's current access rules expire, for the owner and people they share with.
    pub access_rules_expire_at: Option<i64>,
    pub cloud_approvals: Option<CloudApprovals>,
    /// Owner only.
    pub auth_rejection: Option<AuthRejection>,
}

/// Shared access ends with the grant or with the access rules that carry it,
/// whichever comes first.
fn access_expiry(grant: Option<i64>, rules: Option<i64>) -> Option<i64> {
    match (grant, rules) {
        (Some(grant), Some(rules)) => Some(grant.min(rules)),
        (grant, _) => grant,
    }
}

fn cloud_approvals(row: &QueryResult) -> Result<Option<CloudApprovals>, ApiError> {
    let expires_at = [
        row.try_get::<Option<i64>>("", "resourceGrantsExpireAt")?,
        row.try_get::<Option<i64>>("", "billingGrantsExpireAt")?,
    ]
    .into_iter()
    .flatten()
    .max();
    let Some(expires_at) = expires_at else {
        return Ok(None);
    };
    Ok(Some(CloudApprovals {
        resource_grants: count(row, "resourceGrants")?,
        billing_grants: count(row, "billingGrants")?,
        expires_at,
    }))
}

/// A reason written by a newer hub reads as no reason instead of failing the row.
fn auth_rejection(row: &QueryResult) -> Result<Option<AuthRejection>, ApiError> {
    let Some(code) = row
        .try_get::<Option<String>>("", "rejectionCode")?
        .as_deref()
        .and_then(AuthRejectionCode::parse)
    else {
        return Ok(None);
    };
    Ok(Some(AuthRejection {
        code,
        skew_seconds: row.try_get("", "rejectionSkew")?,
        count: count(row, "rejectionCount")?,
        first_at: row.try_get("", "rejectionFirstAt")?,
        last_at: row.try_get("", "rejectionLastAt")?,
    }))
}

/// What a row tells the caller beyond the registration depends on why they see it.
#[derive(Default)]
struct RelationshipFacts {
    access_expires_at: Option<i64>,
    access_rules_expire_at: Option<i64>,
    auth_rejection: Option<AuthRejection>,
}

fn relationship_facts(
    row: &QueryResult,
    relationship: DeviceRelationship,
) -> Result<RelationshipFacts, ApiError> {
    Ok(match relationship {
        DeviceRelationship::Owner => RelationshipFacts {
            access_expires_at: None,
            access_rules_expire_at: row.try_get("", "rulesExpireAt")?,
            auth_rejection: auth_rejection(row)?,
        },
        DeviceRelationship::Shared => {
            let access_rules_expire_at = row.try_get("", "rulesExpireAt")?;
            RelationshipFacts {
                access_expires_at: access_expiry(
                    row.try_get("", "grantExpiresAt")?,
                    access_rules_expire_at,
                ),
                access_rules_expire_at,
                auth_rejection: None,
            }
        }
        DeviceRelationship::CloudApproval => RelationshipFacts::default(),
    })
}

fn view(row: &QueryResult, relationship: DeviceRelationship) -> Result<DeviceView, ApiError> {
    let status = repository::status(row)?;
    let facts = relationship_facts(row, relationship)?;
    Ok(DeviceView {
        device_id: status.device_id,
        owner_id: status.owner_id,
        name: status.name,
        display_name: row.try_get("", "displayName")?,
        identity: status.identity,
        status: status.status,
        registered_at: status.registered_at,
        last_seen_at: status.last_seen_at,
        auth_epoch: status.auth_epoch,
        revoked_at: row.try_get("", "revokedAt")?,
        relationship,
        access_expires_at: facts.access_expires_at,
        access_rules_expire_at: facts.access_rules_expire_at,
        cloud_approvals: cloud_approvals(row)?,
        auth_rejection: facts.auth_rejection,
    })
}

/// Keeps the first row per device: sources arrive strongest relationship first.
fn strongest<T>(
    sources: impl IntoIterator<Item = Vec<T>>,
    device_id: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut seen = HashSet::new();
    sources
        .into_iter()
        .flatten()
        .filter(|row| seen.insert(device_id(row).to_owned()))
        .collect()
}

/// Every device the caller owns, has shared access to, or approved cloud access for.
pub(crate) async fn visible_devices(
    state: &DeviceContext<'_>,
    user_id: &str,
) -> Result<Vec<VisibleDevice>, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let mut sources = Vec::with_capacity(DeviceRelationship::PRECEDENCE.len());
    for relationship in DeviceRelationship::PRECEDENCE {
        sources.push(
            state
                .db
                .query_all_raw(relationship.ids(user_id, now))
                .await?
                .into_iter()
                .map(|row| {
                    Ok(VisibleDevice {
                        device_id: row.try_get("", "id")?,
                        relationship,
                    })
                })
                .collect::<Result<Vec<_>, ApiError>>()?,
        );
    }
    Ok(strongest(sources, |device| &device.device_id))
}

async fn source(
    state: &DeviceContext<'_>,
    relationship: DeviceRelationship,
    user_id: &str,
    now: i64,
    device_id: Option<&str>,
) -> Result<Vec<DeviceView>, ApiError> {
    state
        .db
        .query_all_raw(relationship.rows(user_id, now, device_id))
        .await?
        .iter()
        .map(|row| view(row, relationship))
        .collect()
}

pub(crate) async fn list(
    state: &DeviceContext<'_>,
    user_id: &str,
) -> Result<Vec<DeviceView>, ApiError> {
    super::enabled(state)?;
    let now = chrono::Utc::now().timestamp();
    let mut sources = Vec::with_capacity(DeviceRelationship::PRECEDENCE.len());
    for relationship in DeviceRelationship::PRECEDENCE {
        sources.push(source(state, relationship, user_id, now, None).await?);
    }
    Ok(strongest(sources, |device| &device.device_id))
}

/// A device the caller cannot see answers like one that does not exist.
pub(crate) async fn get(
    state: &DeviceContext<'_>,
    user_id: &str,
    device_id: &str,
) -> Result<DeviceView, ApiError> {
    super::enabled(state)?;
    let now = chrono::Utc::now().timestamp();
    for relationship in DeviceRelationship::PRECEDENCE {
        if let Some(device) = source(state, relationship, user_id, now, Some(device_id))
            .await?
            .pop()
        {
            return Ok(device);
        }
    }
    Err(ApiError::NOT_FOUND)
}

/// Which setup packages `GET /devices/enrollments` lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EnrollmentFilter {
    /// Packages still waiting to be started, including ones that expired unused.
    #[default]
    Open,
    /// Also packages the owner cancelled.
    Recent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeviceEnrollmentState {
    Pending,
    Expired,
    Cancelled,
}

/// A setup package that has not become a device. It carries no secret: the bootstrap
/// key, the token id and the signed manifest stay on the hub and the creating computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DeviceEnrollmentView {
    pub enrollment_id: String,
    /// The device id the package will register.
    pub device_id: String,
    pub name: String,
    pub state: DeviceEnrollmentState,
    pub created_at: i64,
    pub expires_at: i64,
    /// Identifies the computer that created the package.
    pub controller_key_thumbprint: String,
}

fn enrollment_state(cancelled: bool, expires_at: i64, now: i64) -> DeviceEnrollmentState {
    if cancelled {
        DeviceEnrollmentState::Cancelled
    } else if expires_at <= now {
        DeviceEnrollmentState::Expired
    } else {
        DeviceEnrollmentState::Pending
    }
}

fn enrollment(record: EnrollmentRecord, now: i64) -> Result<DeviceEnrollmentView, ApiError> {
    Ok(DeviceEnrollmentView {
        state: enrollment_state(record.cancelled, record.expires_at, now),
        controller_key_thumbprint: record
            .controller_key
            .thumbprint()
            .map_err(|error| ApiError::internal(error.to_string()))?,
        enrollment_id: record.enrollment_id,
        device_id: record.device_id,
        name: record.name,
        created_at: record.created_at,
        expires_at: record.expires_at,
    })
}

pub(crate) async fn enrollments(
    state: &DeviceContext<'_>,
    owner: &str,
    filter: EnrollmentFilter,
) -> Result<Vec<DeviceEnrollmentView>, ApiError> {
    super::enabled(state)?;
    let now = chrono::Utc::now().timestamp();
    super::repository(state)
        .enrollments(owner, filter == EnrollmentFilter::Recent)
        .await?
        .into_iter()
        .map(|record| enrollment(record, now))
        .collect()
}

/// What this hub allows one account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DeviceLimits {
    /// Active devices plus setup packages that can still be started.
    pub max_devices: u32,
    /// Setup packages that may wait to be started at the same time.
    pub max_pending_enrollments: u32,
    /// How long a new setup package can be started, in seconds.
    pub enrollment_ttl_seconds: u64,
    /// Setup packages that may be created within 24 hours.
    pub max_enrollments_per_day: u64,
    /// Account backups of device keys.
    pub max_account_backups: u64,
}

/// What the account uses of `DeviceLimits`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DeviceUsage {
    pub active_devices: u64,
    /// Revoked devices stay listed and count toward no limit.
    pub revoked_devices: u64,
    /// Setup packages that can still be started.
    pub pending_enrollments: u64,
    /// Setup packages created in the last 24 hours, whatever became of them.
    pub enrollments_last_24h: u64,
    /// Account backups that count toward the limit: those of active devices and of
    /// setup packages that can still be started.
    pub account_backups: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DeviceUsageView {
    /// The hub's clock when it counted (Unix seconds).
    pub server_time: i64,
    pub limits: DeviceLimits,
    pub usage: DeviceUsage,
}

fn limits(config: &StandaloneConfig) -> DeviceLimits {
    DeviceLimits {
        max_devices: config.max_devices_per_user,
        max_pending_enrollments: config.max_pending_enrollments_per_user,
        enrollment_ttl_seconds: config.enrollment_ttl_seconds,
        max_enrollments_per_day: repository::daily_enrollment_limit(
            config.max_devices_per_user,
            config.max_pending_enrollments_per_user,
        ),
        max_account_backups: recovery::MAX_ACCOUNT_BACKUPS,
    }
}

/// The limits setup enforces for the caller and how much of them is in use.
pub(crate) async fn usage(
    state: &DeviceContext<'_>,
    owner: &str,
) -> Result<DeviceUsageView, ApiError> {
    super::enabled(state)?;
    let now = chrono::Utc::now().timestamp();
    let enrollment = repository::enrollment_counts(state.db, owner, now).await?;
    let row = state
        .db
        .query_one_raw(sql(
            &format!(
                r#"SELECT (SELECT COUNT(*) FROM "ManagedDevice" WHERE "ownerId"=$1 AND status='revoked') AS "revokedDevices",(SELECT COUNT(*) FROM "DeviceControllerVault" v WHERE v."userId"=$1 AND {}) AS "accountBackups""#,
                recovery::HOLDS_SLOT
            ),
            [owner.into(), now.into()],
        ))
        .await?
        .ok_or_else(|| ApiError::internal(format!("Missing device usage counts for owner {owner}")))?;
    Ok(DeviceUsageView {
        server_time: now,
        limits: limits(state.config),
        usage: DeviceUsage {
            active_devices: enrollment.active_devices,
            revoked_devices: count(&row, "revokedDevices")?,
            pending_enrollments: enrollment.pending,
            enrollments_last_24h: enrollment.last_day,
            account_backups: count(&row, "accountBackups")?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{DeviceStatus, SigningKey};

    const STATUS_FIELDS: [&str; 8] = [
        "device_id",
        "owner_id",
        "name",
        "identity",
        "status",
        "registered_at",
        "last_seen_at",
        "auth_epoch",
    ];

    fn owned(device_id: &str) -> DeviceView {
        DeviceView {
            device_id: device_id.into(),
            owner_id: "owner".into(),
            name: "lab-gpu-02".into(),
            display_name: None,
            identity: DeviceIdentity {
                auth_key: SigningKey::generate().public_key(),
                management_key: [42; 32],
                telemetry_key: SigningKey::generate().public_key(),
            },
            status: DeviceRegistrationStatus::Active,
            registered_at: 1_727_000_000,
            last_seen_at: None,
            auth_epoch: 1,
            revoked_at: None,
            relationship: DeviceRelationship::Owner,
            access_expires_at: None,
            access_rules_expire_at: None,
            cloud_approvals: None,
            auth_rejection: None,
        }
    }

    #[test]
    fn a_view_can_never_be_fed_to_an_agent() {
        let mut json = serde_json::to_value(owned("device")).unwrap();
        // Even a row without any extra fact is refused by the agent's strict parser.
        assert!(serde_json::from_value::<DeviceStatus>(json.clone()).is_err());
        assert_eq!(json["relationship"], "owner");
        assert!(json["auth_rejection"].is_null() && json["revoked_at"].is_null());

        // The registration fields themselves keep the agent's encoding.
        json.as_object_mut()
            .unwrap()
            .retain(|field, _| STATUS_FIELDS.contains(&field.as_str()));
        let status: DeviceStatus = serde_json::from_value(json).unwrap();
        assert_eq!(status.device_id, "device");
        assert_eq!(status.status, DeviceRegistrationStatus::Active);
    }

    #[test]
    fn shared_access_ends_with_the_grant_or_its_rules_whichever_is_first() {
        assert_eq!(access_expiry(Some(300), Some(600)), Some(300));
        assert_eq!(access_expiry(Some(300), Some(100)), Some(100));
        assert_eq!(access_expiry(Some(300), None), Some(300));
        assert_eq!(access_expiry(None, Some(600)), None);
    }

    #[test]
    fn a_lapsed_package_is_reported_as_expired_and_a_cancelled_one_stays_cancelled() {
        assert_eq!(
            enrollment_state(false, 101, 100),
            DeviceEnrollmentState::Pending
        );
        assert_eq!(
            enrollment_state(false, 100, 100),
            DeviceEnrollmentState::Expired
        );
        assert_eq!(
            enrollment_state(true, 101, 100),
            DeviceEnrollmentState::Cancelled
        );
        assert_eq!(
            enrollment_state(true, 1, 100),
            DeviceEnrollmentState::Cancelled
        );
    }

    #[test]
    fn a_device_keeps_its_strongest_relationship() {
        let row = |device_id: &str, relationship| VisibleDevice {
            device_id: device_id.into(),
            relationship,
        };
        let merged = strongest(
            [
                vec![row("a", DeviceRelationship::Owner)],
                vec![
                    row("a", DeviceRelationship::Shared),
                    row("b", DeviceRelationship::Shared),
                ],
                vec![
                    row("b", DeviceRelationship::CloudApproval),
                    row("c", DeviceRelationship::CloudApproval),
                ],
            ],
            |device| &device.device_id,
        );
        assert_eq!(
            merged,
            [
                row("a", DeviceRelationship::Owner),
                row("b", DeviceRelationship::Shared),
                row("c", DeviceRelationship::CloudApproval),
            ]
        );
    }

    #[test]
    fn reported_limits_are_the_ones_setup_enforces() {
        let config = StandaloneConfig {
            max_devices_per_user: 100,
            max_pending_enrollments_per_user: 10,
            enrollment_ttl_seconds: 86_400,
            ..StandaloneConfig::default()
        };
        let view = DeviceUsageView {
            server_time: 1_727_770_000,
            limits: limits(&config),
            usage: DeviceUsage {
                active_devices: 6,
                revoked_devices: 1,
                pending_enrollments: 1,
                enrollments_last_24h: 2,
                account_backups: 4,
            },
        };
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({
                "server_time": 1_727_770_000,
                "limits": {
                    "max_devices": 100,
                    "max_pending_enrollments": 10,
                    "enrollment_ttl_seconds": 86_400,
                    "max_enrollments_per_day": 220,
                    "max_account_backups": 256,
                },
                "usage": {
                    "active_devices": 6,
                    "revoked_devices": 1,
                    "pending_enrollments": 1,
                    "enrollments_last_24h": 2,
                    "account_backups": 4,
                },
            })
        );
    }

    fn bound(statement: &Statement) -> usize {
        statement.values.as_ref().map_or(0, |values| values.0.len())
    }

    /// A placeholder without a value, or a value without a placeholder, fails only on a
    /// real server; the database tests are ignored by default, so check the pairing here.
    #[test]
    fn every_row_source_is_bounded_and_binds_exactly_what_it_references() {
        let highest = |sql: &str| {
            (1..=4)
                .rev()
                .find(|index| sql.contains(&format!("${index}")))
        };
        for relationship in DeviceRelationship::PRECEDENCE {
            for statement in [
                relationship.ids("user", 100),
                relationship.rows("user", 100, None),
                relationship.rows("user", 100, Some("device")),
            ] {
                assert_eq!(
                    highest(&statement.sql),
                    Some(bound(&statement)),
                    "{relationship:?}: {}",
                    statement.sql
                );
                assert!(statement.sql.ends_with(&format!("LIMIT {SOURCE_LIMIT}")));
            }
            assert!(
                relationship
                    .rows("user", 100, Some("device"))
                    .sql
                    .contains(" AND d.id=$3 ORDER BY ")
            );
        }
        assert_eq!(bound(&DeviceRelationship::Owner.ids("user", 100)), 1);
        // Only the owner's rows carry the hub's refusal reason.
        for (relationship, reported) in [
            (DeviceRelationship::Owner, true),
            (DeviceRelationship::Shared, false),
            (DeviceRelationship::CloudApproval, false),
        ] {
            assert_eq!(
                relationship
                    .rows("user", 100, None)
                    .sql
                    .contains("DeviceAuthRejection"),
                reported
            );
        }
    }
}

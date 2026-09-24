//! Project licences for registry packages.
//!
//! A package that is not free and public must be licensed for every project
//! that pins it: `AppPackage.membershipId` names an admin or the owner who
//! holds the package (any `WasmPackageUser` row). When nobody eligible holds
//! it the pin lapses (`stale`, `staleSince`): it takes no updates, keeps
//! running for [`GRACE_DAYS`] and is then disabled for cloud runs, the app
//! catalog and project downloads. [`reconcile_app`] re-derives the holder of
//! every pin and is called whenever members, roles or package access change.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, JoinType, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::entity::sea_orm_active_enums::{NotificationType, WasmPackageVisibility};
use crate::entity::{
    app_package, membership, meta, notification, role, wasm_package, wasm_package_user,
};
use crate::error::ApiError;
use crate::permission::role_permission::RolePermissions;
use crate::push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent};
use crate::state::AppState;

pub const GRACE_DAYS: i64 = 30;
const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
const SWEEP_APP_LIMIT: u64 = 200;
static LAST_SWEEP_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LicenseStatus {
    Active,
    Lapsed,
    Expired,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageLicense {
    pub required: bool,
    pub status: LicenseStatus,
    pub holder_user_id: Option<String>,
    pub lapsed_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub grace_days: i64,
}

/// Free public packages are available to everyone; every other package needs
/// a holder in each project that pins it.
pub fn requires_license(package: &wasm_package::Model) -> bool {
    !(package.visibility == WasmPackageVisibility::Public && package.price <= 0)
}

fn grace() -> chrono::Duration {
    chrono::Duration::days(GRACE_DAYS)
}

pub fn status(pin: &app_package::Model, now: DateTime<Utc>) -> LicenseStatus {
    if !pin.stale {
        return LicenseStatus::Active;
    }
    match pin.stale_since {
        Some(since) if since.to_utc() + grace() <= now => LicenseStatus::Expired,
        _ => LicenseStatus::Lapsed,
    }
}

pub fn license(
    pin: &app_package::Model,
    package: Option<&wasm_package::Model>,
    holder_user_id: Option<String>,
    now: DateTime<Utc>,
) -> PackageLicense {
    let lapsed_at = pin
        .stale
        .then_some(pin.stale_since)
        .flatten()
        .map(|at| at.to_utc());
    PackageLicense {
        required: package.is_none_or(requires_license),
        status: status(pin, now),
        holder_user_id: (!pin.stale).then_some(holder_user_id).flatten(),
        lapsed_at,
        expires_at: lapsed_at.map(|at| at + grace()),
        grace_days: GRACE_DAYS,
    }
}

/// Pins that may still run: licensed, or lapsed within the grace period. A stale
/// pin without `staleSince` is mid-reconcile and counts as freshly lapsed.
pub fn usable_pins(now: DateTime<Utc>) -> Condition {
    Condition::any()
        .add(app_package::Column::Stale.eq(false))
        .add(app_package::Column::StaleSince.is_null())
        .add(app_package::Column::StaleSince.gt((now - grace()).fixed_offset()))
}

/// Whether `user_id` holds `package`: free public packages are held by
/// everyone, everything else needs an access row.
pub async fn user_holds<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    package: &wasm_package::Model,
) -> Result<bool, ApiError> {
    if !requires_license(package) {
        return Ok(true);
    }
    Ok(wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package.id))
        .filter(wasm_package_user::Column::UserId.eq(user_id))
        .filter(wasm_package_user::Column::Permission.ne(0))
        .one(db)
        .await?
        .is_some())
}

/// Admins and the owner may only pin or re-license a package they hold.
pub async fn ensure_holds<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    package: &wasm_package::Model,
) -> Result<(), ApiError> {
    if user_holds(db, user_id, package).await? {
        return Ok(());
    }
    if package.visibility == WasmPackageVisibility::Public {
        return Err(ApiError::coded(
            axum::http::StatusCode::PAYMENT_REQUIRED,
            "PACKAGE_LICENSE_REQUIRED",
            format!(
                "Get '{}' before adding it to a project. Admins and the owner license the packages a project uses.",
                package.name
            ),
        ));
    }
    Err(ApiError::coded(
        axum::http::StatusCode::FORBIDDEN,
        "PACKAGE_ACCESS_REQUIRED",
        format!(
            "You need access to '{}' before adding it to a project. Admins and the owner license the packages a project uses.",
            package.name
        ),
    ))
}

#[derive(Clone, Debug)]
pub struct Lapse {
    pub pin_id: String,
    pub app_id: String,
    pub package_id: String,
    pub package_name: String,
    pub stale_since: DateTime<FixedOffset>,
}

struct Holder {
    membership_id: String,
    user_id: String,
    owner: bool,
}

/// Admin and owner memberships of the app, owner first, then oldest first.
async fn eligible_members<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<Vec<Holder>, ApiError> {
    let permissions = || Expr::col((role::Entity, role::Column::Permissions));
    let rows: Vec<(String, String, i64)> = membership::Entity::find()
        .select_only()
        .column(membership::Column::Id)
        .column(membership::Column::UserId)
        .column_as(permissions(), "permissions")
        .join(JoinType::InnerJoin, membership::Relation::Role.def())
        .filter(membership::Column::AppId.eq(app_id))
        .filter(
            permissions()
                .bit_and((RolePermissions::Admin | RolePermissions::Owner).bits())
                .ne(0),
        )
        .filter(permissions().bit_and(!RolePermissions::all().bits()).eq(0))
        .order_by_asc(membership::Column::CreatedAt)
        .order_by_asc(membership::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let mut holders: Vec<Holder> = rows
        .into_iter()
        .map(|(membership_id, user_id, permissions)| Holder {
            membership_id,
            user_id,
            owner: permissions & RolePermissions::Owner.bits() != 0,
        })
        .collect();
    holders.sort_by_key(|holder| !holder.owner);
    Ok(holders)
}

/// Re-derive the licence holder of every pin of `app_id`. Returns the pins
/// that lapsed in this call so the caller can notify the project's admins.
pub async fn reconcile_app<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<Vec<Lapse>, ApiError> {
    let pins = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .all(db)
        .await?;
    if pins.is_empty() {
        return Ok(Vec::new());
    }
    let package_ids: Vec<String> = pins.iter().map(|pin| pin.package_id.clone()).collect();
    let packages: HashMap<String, wasm_package::Model> = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(package_ids.clone()))
        .all(db)
        .await?
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect();
    let holders = eligible_members(db, app_id).await?;
    let held: HashSet<(String, String)> = if holders.is_empty() {
        HashSet::new()
    } else {
        wasm_package_user::Entity::find()
            .select_only()
            .column(wasm_package_user::Column::UserId)
            .column(wasm_package_user::Column::PackageId)
            .filter(wasm_package_user::Column::PackageId.is_in(package_ids))
            .filter(
                wasm_package_user::Column::UserId
                    .is_in(holders.iter().map(|holder| holder.user_id.clone())),
            )
            .filter(wasm_package_user::Column::Permission.ne(0))
            .into_tuple()
            .all(db)
            .await?
            .into_iter()
            .collect()
    };

    let now = Utc::now().fixed_offset();
    let mut lapses = Vec::new();
    for pin in pins {
        // A package gone from the registry is handled by its own status checks.
        let Some(package) = packages.get(&pin.package_id) else {
            continue;
        };
        let required = requires_license(package);
        let eligible: Vec<&Holder> = holders
            .iter()
            .filter(|holder| {
                !required || held.contains(&(holder.user_id.clone(), pin.package_id.clone()))
            })
            .collect();
        let current = pin
            .membership_id
            .as_deref()
            .filter(|id| eligible.iter().any(|holder| holder.membership_id == *id));
        let holder =
            current.or_else(|| eligible.first().map(|holder| holder.membership_id.as_str()));
        match holder {
            Some(holder) => {
                if pin.stale
                    || pin.stale_since.is_some()
                    || pin.membership_id.as_deref() != Some(holder)
                {
                    app_package::Entity::update_many()
                        .col_expr(app_package::Column::MembershipId, Expr::value(holder))
                        .col_expr(app_package::Column::Stale, Expr::value(false))
                        .col_expr(
                            app_package::Column::StaleSince,
                            Expr::value(Option::<DateTime<FixedOffset>>::None),
                        )
                        .filter(app_package::Column::Id.eq(&pin.id))
                        .exec(db)
                        .await?;
                }
            }
            None if pin.stale_since.is_none() => {
                let lapsed = app_package::Entity::update_many()
                    .col_expr(app_package::Column::Stale, Expr::value(true))
                    .col_expr(
                        app_package::Column::MembershipId,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(app_package::Column::StaleSince, Expr::value(now))
                    .filter(app_package::Column::Id.eq(&pin.id))
                    .filter(app_package::Column::StaleSince.is_null())
                    .exec(db)
                    .await?;
                if lapsed.rows_affected == 1 {
                    lapses.push(Lapse {
                        pin_id: pin.id.clone(),
                        app_id: pin.app_id.clone(),
                        package_id: pin.package_id.clone(),
                        package_name: package.name.clone(),
                        stale_since: now,
                    });
                }
            }
            None if !pin.stale || pin.membership_id.is_some() => {
                app_package::Entity::update_many()
                    .col_expr(app_package::Column::Stale, Expr::value(true))
                    .col_expr(
                        app_package::Column::MembershipId,
                        Expr::value(Option::<String>::None),
                    )
                    .filter(app_package::Column::Id.eq(&pin.id))
                    .exec(db)
                    .await?;
            }
            None => {}
        }
    }
    Ok(lapses)
}

/// Reconcile and notify, for call sites where the triggering change already
/// committed: a failure is logged, never surfaced.
pub async fn refresh_app(state: &AppState, app_id: &str) {
    match reconcile_app(&state.db, app_id).await {
        Ok(lapses) => notify_lapses(state, &lapses).await,
        Err(error) => {
            tracing::warn!(error = %error, app_id, "Package licence reconcile failed");
        }
    }
}

/// A user's access to a package changed: re-license every project where the
/// user is a member and the package is pinned.
pub async fn refresh_package_access(state: &AppState, user_id: &str, package_id: &str) {
    match apps_pinning_for_member(&state.db, user_id, package_id).await {
        Ok(app_ids) => {
            for app_id in app_ids {
                refresh_app(state, &app_id).await;
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, user_id, package_id, "Package licence lookup failed");
        }
    }
}

async fn apps_pinning_for_member<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    package_id: &str,
) -> Result<Vec<String>, ApiError> {
    let member_of: Vec<String> = membership::Entity::find()
        .select_only()
        .column(membership::Column::AppId)
        .filter(membership::Column::UserId.eq(user_id))
        .into_tuple()
        .all(db)
        .await?;
    if member_of.is_empty() {
        return Ok(Vec::new());
    }
    Ok(app_package::Entity::find()
        .select_only()
        .column(app_package::Column::AppId)
        .filter(app_package::Column::PackageId.eq(package_id))
        .filter(app_package::Column::AppId.is_in(member_of))
        .into_tuple()
        .all(db)
        .await?)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Lapsed,
    Week,
    Day,
    Expired,
}

impl Stage {
    fn key(self) -> &'static str {
        match self {
            Self::Lapsed => "lapsed",
            Self::Week => "week",
            Self::Day => "day",
            Self::Expired => "expired",
        }
    }

    fn at(stale_since: DateTime<FixedOffset>, now: DateTime<Utc>) -> Self {
        let left = stale_since.to_utc() + grace() - now;
        if left <= chrono::Duration::zero() {
            Self::Expired
        } else if left <= chrono::Duration::days(1) {
            Self::Day
        } else if left <= chrono::Duration::days(7) {
            Self::Week
        } else {
            Self::Lapsed
        }
    }
}

async fn app_name(state: &AppState, app_id: &str) -> String {
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    metas
        .iter()
        .find(|meta| meta.lang == "en")
        .or(metas.first())
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "your project".to_string())
}

fn notice(stage: Stage, package: &str, app: &str, deadline: DateTime<Utc>) -> (String, String) {
    let date = deadline.format("%B %-d, %Y");
    match stage {
        Stage::Lapsed => (
            format!("{package} needs a license in {app}"),
            format!(
                "No admin or owner of {app} has {package} anymore. Updates are paused and the package stops working on {date}. One of you needs to get the package."
            ),
        ),
        Stage::Week => (
            format!("{package} stops working in {app} within a week"),
            format!(
                "No admin or owner of {app} has {package}. It is disabled on {date} unless one of you gets the package."
            ),
        ),
        Stage::Day => (
            format!("{package} stops working in {app} tomorrow"),
            format!(
                "No admin or owner of {app} has {package}. It is disabled on {date} unless one of you gets the package."
            ),
        ),
        Stage::Expired => (
            format!("{package} was disabled in {app}"),
            format!(
                "Nobody in {app} licensed {package} within {GRACE_DAYS} days, so runs no longer load it. An admin or the owner can get the package to restore it."
            ),
        ),
    }
}

/// One notification per admin and owner for each stage of a lapse. Existing
/// ids are skipped so repeated sweeps do not push again.
async fn notify_stage(
    state: &AppState,
    app_id: &str,
    pin_id: &str,
    package_name: &str,
    stale_since: DateTime<FixedOffset>,
    stage: Stage,
) -> Result<(), ApiError> {
    let recipients = crate::routes::app::connection::admin_user_ids(state, app_id).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    let app = app_name(state, app_id).await;
    let (title, description) = notice(stage, package_name, &app, stale_since.to_utc() + grace());
    for user_id in recipients {
        let id = format!(
            "package-license:{pin_id}:{}:{}:{user_id}",
            stale_since.timestamp_millis(),
            stage.key()
        );
        if notification::Entity::find_by_id(&id)
            .one(&state.db)
            .await?
            .is_some()
        {
            continue;
        }
        dispatch_notification_idempotent(
            state,
            &id,
            DispatchNotificationInput {
                user_id,
                app_id: Some(app_id.to_string()),
                title: title.clone(),
                description: Some(description.clone()),
                icon: Some("package".to_string()),
                image: None,
                link: Some(format!("/library/config/packages?id={app_id}")),
                notification_type: NotificationType::System,
                source_run_id: None,
                source_node_id: None,
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn notify_lapses(state: &AppState, lapses: &[Lapse]) {
    for lapse in lapses {
        if let Err(error) = notify_stage(
            state,
            &lapse.app_id,
            &lapse.pin_id,
            &lapse.package_name,
            lapse.stale_since,
            Stage::Lapsed,
        )
        .await
        {
            tracing::warn!(error = %error, app_id = %lapse.app_id, package_id = %lapse.package_id, "Package licence notification failed");
        }
    }
}

/// Reconcile projects a hook may have missed (a holder removed by a raw
/// delete leaves `membershipId` null) and send the week, day and expiry
/// reminders for lapsed pins. Bounded per call; runs at most hourly per process.
pub async fn sweep(state: &AppState) -> Result<u64, ApiError> {
    let now = Utc::now();
    let last = LAST_SWEEP_MS.load(Ordering::Relaxed);
    if now.timestamp_millis() - last < SWEEP_INTERVAL.as_millis() as i64
        || LAST_SWEEP_MS
            .compare_exchange(
                last,
                now.timestamp_millis(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
    {
        return Ok(0);
    }
    let app_ids: Vec<String> = app_package::Entity::find()
        .select_only()
        .column(app_package::Column::AppId)
        .filter(
            Condition::any()
                .add(app_package::Column::Stale.eq(true))
                .add(app_package::Column::MembershipId.is_null()),
        )
        .distinct()
        .order_by_asc(app_package::Column::AppId)
        .limit(SWEEP_APP_LIMIT)
        .into_tuple()
        .all(&state.db)
        .await?;
    let mut reminded = 0;
    for app_id in &app_ids {
        refresh_app(state, app_id).await;
        let lapsed = app_package::Entity::find()
            .filter(app_package::Column::AppId.eq(app_id))
            .filter(app_package::Column::Stale.eq(true))
            .filter(app_package::Column::StaleSince.is_not_null())
            .all(&state.db)
            .await?;
        for pin in lapsed {
            let Some(since) = pin.stale_since else {
                continue;
            };
            let stage = Stage::at(since, now);
            if stage == Stage::Lapsed {
                continue;
            }
            let name = wasm_package::Entity::find_by_id(&pin.package_id)
                .one(&state.db)
                .await?
                .map(|package| package.name)
                .unwrap_or_else(|| pin.package_id.clone());
            if let Err(error) = notify_stage(state, app_id, &pin.id, &name, since, stage).await {
                tracing::warn!(error = %error, app_id, pin_id = %pin.id, "Package licence reminder failed");
            } else {
                reminded += 1;
            }
        }
    }
    Ok(reminded)
}

pub fn spawn_sweeper(state: AppState) -> Option<JoinHandle<()>> {
    if std::env::var("PACKAGE_LICENSE_SWEEPER_DISABLED")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Package licence sweeper disabled via PACKAGE_LICENSE_SWEEPER_DISABLED");
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = sweep(&state).await {
                tracing::error!(error = %error, "Package licence sweep failed");
            }
        }
    }))
}

/// The version a project member may download through the project's licence:
/// the pinned one, while the pin is not expired.
pub async fn project_download_version<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    package_id: &str,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(pin) = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::PackageId.eq(package_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if status(&pin, Utc::now()) == LicenseStatus::Expired {
        return Ok(None);
    }
    if requested.is_some_and(|version| version != pin.version) {
        return Ok(None);
    }
    Ok(Some(pin.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(stale: bool, since: Option<DateTime<Utc>>) -> app_package::Model {
        app_package::Model {
            id: "pin".into(),
            app_id: "app".into(),
            membership_id: None,
            package_id: "pkg".into(),
            version: "1.0.0".into(),
            added_at: Utc::now().fixed_offset(),
            auto_update: false,
            stale,
            stale_since: since.map(|at| at.fixed_offset()),
        }
    }

    #[test]
    fn grace_runs_thirty_days_from_the_lapse() {
        let now = Utc::now();
        assert_eq!(status(&pin(false, None), now), LicenseStatus::Active);
        assert_eq!(status(&pin(true, None), now), LicenseStatus::Lapsed);
        let lapsed = now - chrono::Duration::days(GRACE_DAYS) + chrono::Duration::minutes(1);
        assert_eq!(status(&pin(true, Some(lapsed)), now), LicenseStatus::Lapsed);
        let expired = now - chrono::Duration::days(GRACE_DAYS);
        assert_eq!(
            status(&pin(true, Some(expired)), now),
            LicenseStatus::Expired
        );
    }

    #[test]
    fn reminders_follow_the_remaining_grace() {
        let now = Utc::now();
        let since = |left_hours: i64| {
            (now - chrono::Duration::days(GRACE_DAYS) + chrono::Duration::hours(left_hours))
                .fixed_offset()
        };
        assert!(Stage::at(since(24 * 20), now) == Stage::Lapsed);
        assert!(Stage::at(since(24 * 7), now) == Stage::Week);
        assert!(Stage::at(since(24), now) == Stage::Day);
        assert!(Stage::at(since(0), now) == Stage::Expired);
    }

    async fn load_pin(db: &sea_orm::DatabaseConnection, package: &str) -> app_package::Model {
        app_package::Entity::find()
            .filter(app_package::Column::AppId.eq("app"))
            .filter(app_package::Column::PackageId.eq(package))
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires PACKAGE_LICENSE_TEST_DATABASE_URL pointing at an empty database with the full PostgreSQL schema"]
    async fn holders_hand_over_lapse_and_heal_against_a_real_schema() {
        use sea_orm::ConnectionTrait;
        let url = std::env::var("PACKAGE_LICENSE_TEST_DATABASE_URL")
            .expect("PACKAGE_LICENSE_TEST_DATABASE_URL must point at a disposable database");
        let db = sea_orm::Database::connect(url).await.unwrap();
        db.execute_unprepared(r#"
INSERT INTO "User" (id,"updatedAt") VALUES ('owner',now()),('admin-a',now()),('admin-b',now()),('member',now());
INSERT INTO "App" (id,"updatedAt") VALUES ('app',now());
INSERT INTO "Role" (id,"appId",name,permissions,"updatedAt") VALUES ('owner-role','app','Owner',1,now()),('admin-role','app','Admin',2,now()),('user-role','app','User',4,now());
INSERT INTO "Membership" (id,"userId","appId","roleId","createdAt","updatedAt") VALUES
 ('m-owner','owner','app','owner-role',now()-interval '3 days',now()),
 ('m-a','admin-a','app','admin-role',now()-interval '2 days',now()),
 ('m-b','admin-b','app','admin-role',now()-interval '1 day',now()),
 ('m-member','member','app','user-role',now(),now());
INSERT INTO "WasmPackage" (id,name,description,version,"wasmPath","wasmHash","wasmSize",nodes,permissions,visibility,status,price,"updatedAt") VALUES
 ('paid','Paid','',  '1.0.0','p','h',1,'[]','{}','PUBLIC','ACTIVE',500,now()),
 ('free','Free','',  '1.0.0','p','h',1,'[]','{}','PUBLIC','ACTIVE',0,now()),
 ('priv','Private','','1.0.0','p','h',1,'[]','{}','PRIVATE','ACTIVE',0,now());
INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission) VALUES ('u-paid','paid','admin-a',8),('u-priv','priv','admin-b',4),('u-member','paid','member',8);
INSERT INTO "AppPackage" (id,"appId","membershipId","packageId",version) VALUES
 ('pin-paid','app','m-a','paid','1.0.0'),('pin-free','app','m-a','free','1.0.0'),('pin-priv','app','m-b','priv','1.0.0');
"#).await.expect("database must be empty and carry the full schema");

        assert!(reconcile_app(&db, "app").await.unwrap().is_empty());
        assert_eq!(
            load_pin(&db, "paid").await.membership_id.as_deref(),
            Some("m-a")
        );

        // The holder leaves: a paying member who is not an admin does not count,
        // the free package passes to the owner, the paid one lapses once.
        db.execute_unprepared(r#"DELETE FROM "Membership" WHERE id='m-a'"#)
            .await
            .unwrap();
        let lapses = reconcile_app(&db, "app").await.unwrap();
        assert_eq!(
            lapses
                .iter()
                .map(|lapse| lapse.package_id.as_str())
                .collect::<Vec<_>>(),
            vec!["paid"]
        );
        let paid = load_pin(&db, "paid").await;
        assert!(paid.stale && paid.stale_since.is_some() && paid.membership_id.is_none());
        assert_eq!(
            load_pin(&db, "free").await.membership_id.as_deref(),
            Some("m-owner")
        );
        assert!(reconcile_app(&db, "app").await.unwrap().is_empty());
        assert_eq!(load_pin(&db, "paid").await.stale_since, paid.stale_since);
        assert_eq!(
            project_download_version(&db, "app", "paid", Some("1.0.0"))
                .await
                .unwrap()
                .as_deref(),
            Some("1.0.0")
        );

        // An admin getting the package heals the pin without a click.
        db.execute_unprepared(r#"INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission) VALUES ('u-b-paid','paid','admin-b',8)"#).await.unwrap();
        assert!(reconcile_app(&db, "app").await.unwrap().is_empty());
        let healed = load_pin(&db, "paid").await;
        assert!(!healed.stale && healed.stale_since.is_none());
        assert_eq!(healed.membership_id.as_deref(), Some("m-b"));

        // Demoting the only holder of the private package lapses it; after the
        // grace period it leaves the usable set and project downloads stop.
        db.execute_unprepared(r#"UPDATE "Membership" SET "roleId"='user-role' WHERE id='m-b'"#)
            .await
            .unwrap();
        let lapses = reconcile_app(&db, "app").await.unwrap();
        let mut lapsed: Vec<&str> = lapses
            .iter()
            .map(|lapse| lapse.package_id.as_str())
            .collect();
        lapsed.sort_unstable();
        assert_eq!(lapsed, vec!["paid", "priv"]);
        db.execute_unprepared(
            r#"UPDATE "AppPackage" SET "staleSince"=now()-interval '31 days' WHERE id='pin-priv'"#,
        )
        .await
        .unwrap();
        let now = Utc::now();
        assert_eq!(
            status(&load_pin(&db, "priv").await, now),
            LicenseStatus::Expired
        );
        let usable: Vec<String> = app_package::Entity::find()
            .filter(app_package::Column::AppId.eq("app"))
            .filter(usable_pins(now))
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|pin| pin.package_id)
            .collect();
        assert!(!usable.contains(&"priv".to_string()) && usable.contains(&"paid".to_string()));
        assert_eq!(
            project_download_version(&db, "app", "priv", None)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            project_download_version(&db, "app", "paid", Some("2.0.0"))
                .await
                .unwrap(),
            None
        );
    }

    #[test]
    fn license_hides_the_holder_while_lapsed() {
        let now = Utc::now();
        let since = now - chrono::Duration::days(2);
        let lapsed = license(&pin(true, Some(since)), None, Some("user".into()), now);
        assert_eq!(lapsed.status, LicenseStatus::Lapsed);
        assert!(lapsed.holder_user_id.is_none());
        assert_eq!(
            lapsed.expires_at,
            Some(since + chrono::Duration::days(GRACE_DAYS))
        );
        let active = license(&pin(false, None), None, Some("user".into()), now);
        assert_eq!(active.holder_user_id.as_deref(), Some("user"));
        assert!(active.lapsed_at.is_none());
    }
}

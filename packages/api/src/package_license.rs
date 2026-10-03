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

use crate::cache::{PlatformCache, Reservation};
use crate::entity::sea_orm_active_enums::{NotificationType, WasmPackageVisibility};
use crate::entity::{
    app_package, membership, meta, notification, role, wasm_package, wasm_package_user,
    wasm_package_version,
};
use crate::error::ApiError;
use crate::middleware::jwt::{AppPermissionResponse, AppUser};
use crate::permission::role_permission::RolePermissions;
use crate::push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent};
use crate::state::AppState;

pub const GRACE_DAYS: i64 = 30;
const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
const SWEEP_PAGE: u64 = 200;
const SWEEP_MAX_PAGES: usize = 25;
static LAST_SWEEP_MS: AtomicI64 = AtomicI64::new(0);
/// A pin that expired longer ago than this has had every reminder.
const REMINDER_DAYS_AFTER_EXPIRY: i64 = 7;

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

fn usable_app_pins_at(app_id: &str, now: DateTime<Utc>) -> sea_orm::Select<app_package::Entity> {
    app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(usable_pins(now))
}

/// The pins of `app_id` that may still be used: licensed, or lapsed within the
/// grace period.
pub async fn usable_app_pins<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<Vec<app_package::Model>, sea_orm::DbErr> {
    usable_app_pins_at(app_id, Utc::now()).all(db).await
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
    let cache = state.cache.platform().await.ok();
    let in_inbox = |id: String| async move {
        notification::Entity::find_by_id(id)
            .one(&state.db)
            .await
            .map(|row| row.is_some())
    };
    for user_id in recipients {
        let id = format!(
            "package-license:{pin_id}:{}:{}:{user_id}",
            stale_since.timestamp_millis(),
            stage.key()
        );
        let arrived = in_inbox(id.clone()).await?;
        if !reminder_due(cache.as_ref(), &id, arrived).await {
            continue;
        }
        let dispatched = dispatch_notification_idempotent(
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
        .await;
        // The inbox row is written before the push, so it tells whether the
        // reminder arrived when the dispatch reports an error.
        let delivered = dispatched.is_ok() || in_inbox(id.clone()).await.unwrap_or(false);
        settle_reminder(cache.as_ref(), &id, delivered).await;
        dispatched?;
    }
    Ok(())
}

const REMINDER_NAMESPACE: &str = "package-license-reminder";
/// How long a reminder is held while it is being sent. Shorter than the pause
/// between two sweeps, so one that was cut off mid-send is tried by the next.
const REMINDER_LEASE: Duration = Duration::from_secs(10 * 60);
/// How long a delivered reminder stays on record: longer than any stage lasts,
/// the 7 days after expiry being the longest.
const REMINDER_RECORD_TTL: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// Whether the reminder `id` went out before or is being sent right now;
/// otherwise this call holds it for `REMINDER_LEASE`. The inbox row is the
/// recipient's to delete, so it cannot be the only record of a reminder:
/// without this one, every sweep after a delete sends it again. A cache that
/// cannot be reached answers "no", so no reminder is lost to it.
async fn reminder_taken(cache: Option<&PlatformCache>, id: &str) -> bool {
    let Some(cache) = cache else {
        return false;
    };
    matches!(
        cache
            .try_insert(REMINDER_NAMESPACE, id, &false, REMINDER_LEASE)
            .await,
        Ok(Reservation::Held(_))
    )
}

/// Whether the reminder `id` still has to be sent; the caller then holds it.
/// One that is in the inbox without a record gets its record here: the send
/// was cut off after the row was written, or the record could not be stored,
/// and nothing else would write it before the recipient deletes the row.
async fn reminder_due(cache: Option<&PlatformCache>, id: &str, in_inbox: bool) -> bool {
    let taken = reminder_taken(cache, id).await;
    if in_inbox && !taken {
        settle_reminder(cache, id, true).await;
    }
    !in_inbox && !taken
}

/// Keeps a delivered reminder on record for its stage, or frees one that did
/// not arrive so the next sweep sends it.
async fn settle_reminder(cache: Option<&PlatformCache>, id: &str, delivered: bool) {
    let Some(cache) = cache else {
        return;
    };
    let settled = if delivered {
        cache
            .set(REMINDER_NAMESPACE, id, &true, REMINDER_RECORD_TTL)
            .await
    } else {
        cache.delete(REMINDER_NAMESPACE, id).await.map(|_| ())
    };
    if let Err(error) = settled {
        tracing::warn!(error = %error, id, delivered, "Package licence reminder record was not updated");
    }
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
/// reminders for lapsed pins. One call pages through every such project by app
/// id, so none waits behind the others; runs at most hourly per process.
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
    let mut reminded = 0;
    let mut after: Option<String> = None;
    for _ in 0..SWEEP_MAX_PAGES {
        let mut page = app_package::Entity::find()
            .select_only()
            .column(app_package::Column::AppId)
            .filter(
                Condition::any()
                    .add(app_package::Column::Stale.eq(true))
                    .add(app_package::Column::MembershipId.is_null()),
            )
            .distinct()
            .order_by_asc(app_package::Column::AppId)
            .limit(SWEEP_PAGE);
        if let Some(after) = after.as_deref() {
            page = page.filter(app_package::Column::AppId.gt(after));
        }
        let app_ids: Vec<String> = page.into_tuple().all(&state.db).await?;
        for app_id in &app_ids {
            refresh_app(state, app_id).await;
            reminded += remind_app(state, app_id, now).await?;
        }
        if (app_ids.len() as u64) < SWEEP_PAGE {
            return Ok(reminded);
        }
        after = app_ids.last().cloned();
    }
    tracing::warn!(
        pages = SWEEP_MAX_PAGES,
        after = ?after,
        "Package licence sweep reached its page limit; projects after this id were not swept"
    );
    Ok(reminded)
}

/// Send the reminders that are due for the lapsed pins of one project.
async fn remind_app(state: &AppState, app_id: &str, now: DateTime<Utc>) -> Result<u64, ApiError> {
    let reminders_due_since =
        (now - grace() - chrono::Duration::days(REMINDER_DAYS_AFTER_EXPIRY)).fixed_offset();
    let lapsed = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::Stale.eq(true))
        .filter(app_package::Column::StaleSince.gt(reminders_due_since))
        .all(&state.db)
        .await?;
    let mut reminded = 0;
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

/// The pin of `package_id` in `app_id` while it may still be used: licensed,
/// or lapsed within the grace period.
pub async fn usable_pin<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    package_id: &str,
) -> Result<Option<app_package::Model>, ApiError> {
    Ok(app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::PackageId.eq(package_id))
        .one(db)
        .await?
        .filter(|pin| status(pin, Utc::now()) != LicenseStatus::Expired))
}

/// The version a project member may download through the project's licence:
/// the pinned one, while the pin is not expired.
pub async fn project_download_version<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    package_id: &str,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    Ok(usable_pin(db, app_id, package_id)
        .await?
        .map(|pin| pin.version)
        .filter(|pinned| requested.is_none_or(|version| version == pinned)))
}

/// The usable pin of `package_id` in `app_id` for `user_id`. Every member of
/// the project uses the packages it licenses, whatever their role, so page
/// viewers see the widgets on its pages.
pub async fn member_pin<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    app_id: &str,
    package_id: &str,
) -> Result<Option<app_package::Model>, ApiError> {
    let member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::UserId.eq(user_id))
        .one(db)
        .await?
        .is_some();
    if !member {
        return Ok(None);
    }
    usable_pin(db, app_id, package_id).await
}

/// The pin of `package_id` the caller uses through `app_id`. API keys and app
/// connections act for a project rather than as one of its members, so only
/// signed-in members qualify.
pub async fn caller_pin(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    package_id: &str,
) -> Result<Option<app_package::Model>, ApiError> {
    let Ok(user_id) = user.sub() else {
        return Ok(None);
    };
    member_pin(&state.db, &user_id, app_id, package_id).await
}

/// Whether the caller can read the project's boards, and so run them on
/// their own device. That is what a package's node binary and node list are
/// for; members without it only load the package's widgets on the project's
/// pages. A "no" from the cached role is confirmed against the database: it
/// answers with the widgets-only view instead of an error, so nothing else
/// would tell a just-promoted member to ask again.
pub async fn reads_project_boards(state: &AppState, user: &AppUser, app_id: &str) -> bool {
    let reads = |permission: Result<AppPermissionResponse, ApiError>| {
        permission.is_ok_and(|permission| permission.has_permission(RolePermissions::ReadBoards))
    };
    reads(user.app_permission(app_id, state).await)
        || reads(user.app_permission_fresh(app_id, state).await)
}

/// The version of `package_id` the caller may use through `app_id`.
pub async fn member_pinned_version(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    package_id: &str,
) -> Result<Option<String>, ApiError> {
    Ok(caller_pin(state, user, app_id, package_id)
        .await?
        .map(|pin| pin.version))
}

/// Whether `pin` opens the widget files of `version`: the pinned version, or
/// one published before it, so widgets placed before the pin moved keep
/// loading for members until an editor reloads them.
pub async fn pin_covers_widget_version<C: ConnectionTrait>(
    db: &C,
    pin: &app_package::Model,
    version: &str,
) -> Result<bool, ApiError> {
    if pin.version == version {
        return Ok(true);
    }
    let published: HashMap<String, DateTime<FixedOffset>> = wasm_package_version::Entity::find()
        .select_only()
        .column(wasm_package_version::Column::Version)
        .column(wasm_package_version::Column::PublishedAt)
        .filter(wasm_package_version::Column::PackageId.eq(&pin.package_id))
        .filter(wasm_package_version::Column::Version.is_in([version, pin.version.as_str()]))
        .into_tuple::<(String, DateTime<FixedOffset>)>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    Ok(matches!(
        (published.get(version), published.get(&pin.version)),
        (Some(requested), Some(pinned)) if requested <= pinned
    ))
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
    fn usable_pins_are_asked_for_one_app_and_leave_out_expired_licences() {
        use sea_orm::{DatabaseBackend, QueryTrait};
        let now = DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
            .unwrap()
            .to_utc();
        let sql = usable_app_pins_at("app-1", now)
            .build(DatabaseBackend::Postgres)
            .to_string();

        let (_, filter) = sql.split_once(" WHERE ").expect("the query is filtered");
        assert_eq!(
            filter,
            r#""AppPackage"."appId" = 'app-1' AND ("AppPackage"."stale" = FALSE OR "AppPackage"."staleSince" IS NULL OR "AppPackage"."staleSince" > '2026-09-02 00:00:00.000000 +00:00')"#
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

    #[tokio::test]
    async fn a_reminder_is_leased_while_it_is_sent_and_recorded_once_it_arrived() {
        let (store, cache) = crate::cache::memory_platform_cache();
        // Entries expire on the wall clock; the store's own clock starts at 0.
        store.advance(Utc::now().timestamp_millis());
        let cache = Some(&cache);
        let past_the_lease = REMINDER_LEASE.as_millis() as i64 + 60_000;

        // Before the store's clock moves on: a lease taken later is already
        // behind it and would be free whether or not it was released.
        assert!(!reminder_taken(cache, "failed").await);
        settle_reminder(cache, "failed", false).await;
        assert!(
            !reminder_taken(cache, "failed").await,
            "a reminder that did not arrive is free for the next sweep"
        );

        assert!(!reminder_taken(cache, "cut-off").await);
        assert!(reminder_taken(cache, "cut-off").await);
        store.advance(past_the_lease);
        assert!(
            !reminder_taken(cache, "cut-off").await,
            "a send that was cut off is tried again by the next sweep"
        );

        assert!(!reminder_taken(cache, "arrived").await);
        settle_reminder(cache, "arrived", true).await;
        store.advance(past_the_lease);
        assert!(
            reminder_taken(cache, "arrived").await,
            "a delivered reminder stays on record after its inbox row is deleted"
        );
        store.advance(REMINDER_RECORD_TTL.as_millis() as i64);
        assert!(!reminder_taken(cache, "arrived").await);

        assert!(!reminder_taken(None, "no cache").await);
        assert!(!reminder_taken(None, "no cache").await);
    }

    #[tokio::test]
    async fn a_reminder_in_the_inbox_without_a_record_gets_one() {
        let (store, cache) = crate::cache::memory_platform_cache();
        store.advance(Utc::now().timestamp_millis());
        let cache = Some(&cache);
        let past_the_lease = REMINDER_LEASE.as_millis() as i64 + 60_000;

        assert!(reminder_due(cache, "fresh", false).await);
        assert!(
            !reminder_due(cache, "fresh", false).await,
            "another sweep is sending it"
        );

        // The send was cut off after the inbox row was written: only the row is left.
        assert!(!reminder_due(cache, "cut-off", true).await);
        store.advance(past_the_lease);
        assert!(
            !reminder_due(cache, "cut-off", false).await,
            "deleting the row later must not send it again"
        );
        assert!(!reminder_due(cache, "cut-off", true).await);

        assert!(!reminder_due(None, "no cache", true).await);
        assert!(reminder_due(None, "no cache", false).await);
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

    #[tokio::test]
    #[ignore = "requires PACKAGE_LICENSE_TEST_DATABASE_URL pointing at a database with the full PostgreSQL schema"]
    async fn members_use_the_project_pin_against_a_real_schema() {
        use sea_orm::ConnectionTrait;
        let url = std::env::var("PACKAGE_LICENSE_TEST_DATABASE_URL")
            .expect("PACKAGE_LICENSE_TEST_DATABASE_URL must point at a disposable database");
        let db = sea_orm::Database::connect(url).await.unwrap();
        let page_viewer = (RolePermissions::ReadTemplates
            | RolePermissions::ExecuteEvents
            | RolePermissions::ListEvents)
            .bits();
        db.execute_unprepared(&format!(r#"
INSERT INTO "User" (id,"updatedAt") VALUES ('m-admin',now()),('m-viewer',now()),('m-stranger',now());
INSERT INTO "App" (id,"updatedAt") VALUES ('m-app',now());
INSERT INTO "Role" (id,"appId",name,permissions,"updatedAt") VALUES ('m-admin-role','m-app','Admin',2,now()),('m-user-role','m-app','User',{page_viewer},now());
INSERT INTO "Membership" (id,"userId","appId","roleId","createdAt","updatedAt") VALUES
 ('m-m-admin','m-admin','m-app','m-admin-role',now(),now()),
 ('m-m-viewer','m-viewer','m-app','m-user-role',now(),now());
INSERT INTO "WasmPackage" (id,name,description,version,"wasmPath","wasmHash","wasmSize",nodes,permissions,visibility,status,price,"updatedAt") VALUES
 ('m-priv','Private','','2.0.0','p','h',1,'[]','{{}}','PRIVATE','ACTIVE',0,now());
INSERT INTO "WasmPackageVersion" (id,"packageId",version,"wasmPath","wasmHash","wasmSize","publishedAt") VALUES
 ('m-v1','m-priv','1.0.0','p','h',1,now()-interval '2 days'),
 ('m-v2','m-priv','1.1.0','p','h',1,now()-interval '1 day'),
 ('m-v3','m-priv','2.0.0','p','h',1,now());
INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission) VALUES ('m-u-admin','m-priv','m-admin',4);
INSERT INTO "AppPackage" (id,"appId","membershipId","packageId",version) VALUES ('m-pin','m-app','m-m-admin','m-priv','1.1.0');
"#)).await.expect("database must carry the full schema and none of this test's rows");

        // A page viewer has no ReadBoards and no access to the package of their own.
        let pin = member_pin(&db, "m-viewer", "m-app", "m-priv")
            .await
            .unwrap()
            .expect("every member uses the packages the project licenses");
        assert_eq!(pin.version, "1.1.0");
        assert!(
            member_pin(&db, "m-stranger", "m-app", "m-priv")
                .await
                .unwrap()
                .is_none()
        );

        // Widgets placed before the pin moved keep loading; newer versions stay closed.
        for (version, covered) in [
            ("1.1.0", true),
            ("1.0.0", true),
            ("2.0.0", false),
            ("9.9.9", false),
        ] {
            assert_eq!(
                pin_covers_widget_version(&db, &pin, version).await.unwrap(),
                covered,
                "{version}"
            );
        }
        // Downloads stay on the pinned version.
        assert_eq!(
            project_download_version(&db, "m-app", "m-priv", Some("1.0.0"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            project_download_version(&db, "m-app", "m-priv", None)
                .await
                .unwrap()
                .as_deref(),
            Some("1.1.0")
        );

        db.execute_unprepared(
            r#"UPDATE "AppPackage" SET stale=true,"staleSince"=now()-interval '31 days',"membershipId"=NULL WHERE id='m-pin'"#,
        )
        .await
        .unwrap();
        assert!(
            member_pin(&db, "m-viewer", "m-app", "m-priv")
                .await
                .unwrap()
                .is_none(),
            "an expired pin opens nothing"
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

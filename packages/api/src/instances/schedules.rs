//! Where an app's schedule or bot runs when it is not the hub: at most one row per (app, event).
//!
//! A person who can edit the app's events releases a schedule or a bot to one service. That
//! service's device claims it with its workload identity once the service runs, and confirms the
//! claim while it does. Nothing here looks at the event's type. The hub's trigger funnel skips a
//! schedule while a working approval holds it, and for a grace period after it was handed back,
//! so a scheduled time is missed rather than run twice; the hub's doors for a bot answer without
//! a run for as long.

use super::{routes::caller, *};
use crate::{
    audit_branch,
    db::{AsDbConflict, DbConflict, DbDialect},
    ensure_permission,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderName, StatusCode, header},
};
use sea_orm::{
    DatabaseConnection, DatabaseTransaction, DbErr, FromQueryResult, QueryResult, RuntimeErr, Value,
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
};
use utoipa::ToSchema;

/// Signed by the device, so the route and the proof must name the same path.
pub(crate) const CLAIM_PATH: &str = "/instances/project/schedules";
/// The claim body is read before the device proof is checked.
pub(crate) const CLAIM_BODY_LIMIT_BYTES: usize = 16 * 1024;
const MAX_CLAIMED_EVENTS: usize = 64;
const MAX_LISTED_SCHEDULES: usize = 512;
/// A hand-back the device confirmed, or one made after it retired every process.
const CONFIRMED_GRACE_SECONDS: i64 = 300;
/// A hand-back while a process may still run: it can hold a storage lease this long.
const UNCONFIRMED_GRACE_SECONDS: i64 = MAX_INSTANCE_STORAGE_LEASE_SECONDS + CONFIRMED_GRACE_SECONDS;
const RUNS_ELSEWHERE: &str = "SCHEDULE_RUNS_ELSEWHERE";
const RETURNING: &str = "SCHEDULE_RETURNING";
const RELEASE_ACTION: &str = "event.schedule.release";
const GIVE_BACK_ACTION: &str = "event.schedule.give_back";
const CLAIM_ACTION: &str = "instance.schedules.claim";

/// The bots: a device holds one the way it holds a schedule.
pub(crate) const BOT_EVENT_TYPES: [&str; 2] = ["telegram", "discord"];
/// The event types a device runs in one place only, so the hub hands them over with a claim.
pub(crate) const CLAIMED_EVENT_TYPES: [&str; 3] = ["cron", BOT_EVENT_TYPES[0], BOT_EVENT_TYPES[1]];

const NO_STORE: [(HeaderName, &str); 2] = [
    (header::CACHE_CONTROL, "no-store"),
    (header::PRAGMA, "no-cache"),
];

const COLUMNS: &str =
    r#""eventId","deviceId","placementId","releasedAt","grantId","claimedAt","seenAt","resumeAt""#;
/// Ends a claim and, with `RELEASE_CLEARED`, the release it was made under.
const CLAIM_CLEARED: &str = r#""grantId"=NULL,"claimedAt"=NULL,"seenAt"=NULL"#;
const RELEASE_CLEARED: &str =
    r#""deviceId"=NULL,"placementId"=NULL,"releasedBy"=NULL,"releasedAt"=NULL"#;

#[derive(Clone, Debug, PartialEq, Eq, FromQueryResult)]
pub(super) struct ScheduleRow {
    #[sea_orm(from_alias = "eventId")]
    event_id: String,
    #[sea_orm(from_alias = "deviceId")]
    device_id: Option<String>,
    #[sea_orm(from_alias = "placementId")]
    placement_id: Option<String>,
    #[sea_orm(from_alias = "releasedAt")]
    released_at: Option<i64>,
    #[sea_orm(from_alias = "grantId")]
    grant_id: Option<String>,
    #[sea_orm(from_alias = "claimedAt")]
    claimed_at: Option<i64>,
    #[sea_orm(from_alias = "seenAt")]
    seen_at: Option<i64>,
    #[sea_orm(from_alias = "resumeAt")]
    resume_at: Option<i64>,
}

impl ScheduleRow {
    fn read(row: &QueryResult) -> Result<Self, ApiError> {
        Ok(Self::from_query_result(row, "")?)
    }

    /// The service a person released the schedule to.
    fn service(&self) -> Option<(&str, &str)> {
        Some((self.device_id.as_deref()?, self.placement_id.as_deref()?))
    }

    /// The approval whose device runs it, and since when.
    fn claim(&self) -> Option<(&str, i64)> {
        Some((self.grant_id.as_deref()?, self.claimed_at?))
    }

    /// When the hub runs it again, while that is still ahead.
    fn returning(&self, now: i64) -> Option<i64> {
        self.resume_at.filter(|resume_at| now <= *resume_at)
    }

    pub(super) fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }
}

fn row_statement(app_id: &str, event_id: &str) -> sea_orm::Statement {
    sql(
        &format!(
            r#"SELECT {COLUMNS} FROM "DeviceScheduleClaim" WHERE "appId"=$1 AND "eventId"=$2"#
        ),
        [app_id.into(), event_id.into()],
    )
}

async fn read_row<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    event_id: &str,
) -> Result<Option<ScheduleRow>, ApiError> {
    db.query_one_raw(row_statement(app_id, event_id))
        .await?
        .as_ref()
        .map(ScheduleRow::read)
        .transpose()
}

/// SQLSTATE 42P01: the claim table is not there, so its migration has not run and nothing
/// was ever handed to a device.
fn table_missing(error: &DbErr) -> bool {
    let (DbErr::Exec(RuntimeErr::SqlxError(error)) | DbErr::Query(RuntimeErr::SqlxError(error))) =
        error
    else {
        return false;
    };
    let missing = error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "42P01");
    static REPORTED: AtomicBool = AtomicBool::new(false);
    if missing && !REPORTED.swap(true, Ordering::Relaxed) {
        tracing::error!(
            "The DeviceScheduleClaim table is missing: apply the device schedules migration. Until then the hub runs every schedule itself."
        );
    }
    missing
}

/// Whether the claim table exists. Code can reach a hub before its migration does.
pub(super) async fn table_exists<C: ConnectionTrait>(db: &C) -> Result<bool, ApiError> {
    let probe = sql(r#"SELECT "grantId" FROM "DeviceScheduleClaim" LIMIT 0"#, []);
    match db.query_one_raw(probe).await {
        Ok(_) => Ok(true),
        Err(error) if table_missing(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// A row that another writer changed between this transaction's read and its write.
fn changed_meanwhile() -> ApiError {
    ApiError::conflict("The schedule or bot changed at the same moment; retry the request")
        .with_conflict(DbConflict::Serialization)
}

/// The checks every instance call passes, without the instance: the device is registered,
/// the approval is active, unexpired and still backed by its project, and its approver may
/// still deploy to the device. A refusal means "no"; anything else is an error.
async fn approval_works(tx: &DatabaseTransaction, grant_id: &str) -> Result<bool, ApiError> {
    let checked = async {
        let device_id = read_grant(tx, grant_id).await?.info.device_id;
        let device = lock_device(tx, &device_id).await?;
        let grant = lock_grant(tx, grant_id).await?;
        device_deployment_authority(tx, &device, &grant.info).await
    };
    match checked.await {
        Ok(()) => Ok(true),
        Err(error) if error.status().is_client_error() && error.db_conflict().is_none() => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Whether a device was removed from the hub, and when that is recorded. A device without a
/// row is gone too.
async fn device_removal(
    tx: &DatabaseTransaction,
    device_id: Option<&str>,
) -> Result<(bool, Option<i64>), ApiError> {
    let Some(device_id) = device_id else {
        return Ok((true, None));
    };
    let device = tx
        .query_one_raw(sql(
            r#"SELECT status,"revokedAt" FROM "ManagedDevice" WHERE id=$1"#,
            [device_id.into()],
        ))
        .await?;
    let Some(device) = device else {
        return Ok((true, None));
    };
    Ok((
        device.try_get::<String>("", "status")? != "active",
        device.try_get("", "revokedAt")?,
    ))
}

/// When an approval that no longer works stopped working, as far as the hub can read it:
/// its expiry when that has passed, else the removal of its device, else now. Also says
/// whether the device is gone.
async fn approval_end(
    tx: &DatabaseTransaction,
    grant_id: &str,
    device_id: Option<&str>,
    now: i64,
) -> Result<(i64, bool), ApiError> {
    let expired = match read_grant(tx, grant_id).await {
        Ok(grant) => Some(grant.info.expires_at).filter(|expires_at| *expires_at <= now),
        Err(error) if error.status() == StatusCode::NOT_FOUND => None,
        Err(error) => return Err(error),
    };
    let (removed, removed_at) = device_removal(tx, device_id).await?;
    let end = expired
        .or(removed.then(|| removed_at.unwrap_or(now)))
        .unwrap_or(now);
    Ok((end.min(now), removed))
}

/// What became of a claim once its approval was looked at.
#[derive(Debug, PartialEq, Eq)]
enum Settled {
    /// The approval works: its device runs the schedule.
    Runs,
    /// Nobody holds it; the hub runs it again after this time, when one is set.
    HubResumes(Option<i64>),
}

/// Decides a claimed row. An approval that stopped working without being revoked (expired,
/// device removed, approver lost the right) changes no row, so the hub notices here and hands
/// the schedule back itself. The release stays, unless the device is gone: a renewed approval
/// of the same service may claim again. The first observer wins.
async fn settle_claim(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    row: &ScheduleRow,
    grant_id: &str,
    now: i64,
) -> Result<Settled, ApiError> {
    let app_id = app_id.to_owned();
    let event_id = row.event_id.clone();
    let device_id = row.device_id.clone();
    let grant_id = grant_id.to_owned();
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |tx| {
        let app_id = app_id.clone();
        let event_id = event_id.clone();
        let device_id = device_id.clone();
        let grant_id = grant_id.clone();
        Box::pin(async move {
            if approval_works(tx, &grant_id).await? {
                return Ok(Settled::Runs);
            }
            let (end, device_removed) =
                approval_end(tx, &grant_id, device_id.as_deref(), now).await?;
            let resume_at = end + UNCONFIRMED_GRACE_SECONDS;
            hand_back_for_ended_approval(
                tx,
                [&app_id, &event_id, &grant_id],
                resume_at,
                device_removed,
            )
            .await
        })
    })
    .await
}

/// Ends the claim `[app, event, approval]` names, when the row still names that approval.
/// Otherwise another observer settled it first, and the row says what it found.
async fn hand_back_for_ended_approval(
    tx: &DatabaseTransaction,
    [app_id, event_id, grant_id]: [&str; 3],
    resume_at: i64,
    device_removed: bool,
) -> Result<Settled, ApiError> {
    let release = if device_removed {
        format!(",{RELEASE_CLEARED}")
    } else {
        String::new()
    };
    let handed_back = tx
        .execute_raw(sql(
            &format!(
                r#"UPDATE "DeviceScheduleClaim" SET {CLAIM_CLEARED},"resumeAt"=$4{release} WHERE "appId"=$1 AND "eventId"=$2 AND "grantId"=$3"#
            ),
            [
                app_id.into(),
                event_id.into(),
                grant_id.into(),
                resume_at.into(),
            ],
        ))
        .await?;
    if handed_back.rows_affected() == 1 {
        return Ok(Settled::HubResumes(Some(resume_at)));
    }
    Ok(match read_row(tx, app_id, event_id).await? {
        Some(row) if row.claim().is_some() => Settled::Runs,
        Some(row) => Settled::HubResumes(row.resume_at),
        None => Settled::HubResumes(None),
    })
}

/// Whether the hub must skip this scheduled time because a device runs the schedule, or ran
/// it until a moment ago. `scheduled` is the occurrence the trigger names, when it names one:
/// a scheduler that replays a late occurrence after a hand-back must not run what the device
/// already ran. Independent of the hub's devices switch: a hub that turned devices off still
/// has devices running claimed schedules from their cache.
pub(crate) async fn runs_on_device(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    event_id: &str,
    scheduled: Option<i64>,
    now: i64,
) -> Result<bool, ApiError> {
    let row = match db.query_one_raw(row_statement(app_id, event_id)).await {
        Ok(row) => row.as_ref().map(ScheduleRow::read).transpose()?,
        Err(error) if table_missing(&error) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let Some(row) = row else {
        return Ok(false);
    };
    let resume_at = match row.claim() {
        Some((grant_id, _)) => {
            match settle_claim(db, dialect, app_id, &row, grant_id, now).await? {
                Settled::Runs => return Ok(true),
                Settled::HubResumes(resume_at) => resume_at,
            }
        }
        None => row.resume_at,
    };
    Ok(resume_at.is_some_and(|resume_at| scheduled.unwrap_or(now) <= resume_at))
}

/// Which of `events` (app, event) a device runs, or whose grace period runs, as the rows say
/// now: one statement for all of them. Unlike `runs_on_device` it settles nothing, so a claim
/// whose approval stopped working counts until a door, a release elsewhere or a person ends
/// it. A hub without the claim table handed nothing over.
pub(crate) async fn claimed_events<C: ConnectionTrait>(
    db: &C,
    events: &[(String, String)],
    now: i64,
) -> Result<HashSet<(String, String)>, ApiError> {
    if events.is_empty() {
        return Ok(HashSet::new());
    }
    let mut values: Vec<Value> = vec![now.into()];
    let pairs = events
        .iter()
        .map(|(app_id, event_id)| {
            values.extend([app_id.clone().into(), event_id.clone().into()]);
            format!("(${},${})", values.len() - 1, values.len())
        })
        .collect::<Vec<_>>()
        .join(",");
    let claimed = sql(
        &format!(
            r#"SELECT "appId","eventId" FROM "DeviceScheduleClaim" WHERE (("grantId" IS NOT NULL AND "claimedAt" IS NOT NULL) OR "resumeAt">=$1) AND ("appId","eventId") IN ({pairs})"#
        ),
        values,
    );
    match db.query_all_raw(claimed).await {
        Ok(rows) => rows
            .iter()
            .map(|row| -> Result<(String, String), ApiError> {
                Ok((row.try_get("", "appId")?, row.try_get("", "eventId")?))
            })
            .collect(),
        Err(error) if table_missing(&error) => Ok(HashSet::new()),
        Err(error) => Err(error.into()),
    }
}

/// When the hub runs a schedule again that this approval's device ran: soon when the device
/// retired every process of the service, else after anything it still runs has lost its lease.
async fn resume_after(tx: &DatabaseTransaction, grant_id: &str, now: i64) -> Result<i64, ApiError> {
    let may_still_run = tx
        .query_one_raw(sql(
            r#"SELECT id FROM "WorkloadInstance" WHERE "grantId"=$1 AND status='active' AND (purpose IS NULL OR purpose='workload') LIMIT 1"#,
            [grant_id.into()],
        ))
        .await?
        .is_some();
    Ok(now
        + if may_still_run {
            UNCONFIRMED_GRACE_SECONDS
        } else {
            CONFIRMED_GRACE_SECONDS
        })
}

/// Hands every schedule a revoked approval ran back to the hub. The release stays, so a new
/// approval of the same service can claim again; schedules that were released to the service
/// but never claimed are left alone.
pub(super) async fn hand_back_grant(
    tx: &DatabaseTransaction,
    grant: &ResourceGrantResponse,
    now: i64,
) -> Result<(), ApiError> {
    let Some(app_id) = &grant.app_id else {
        return Ok(());
    };
    let resume_at = resume_after(tx, &grant.grant_id, now).await?;
    tx.execute_raw(sql(
        &format!(
            r#"UPDATE "DeviceScheduleClaim" SET {CLAIM_CLEARED},"resumeAt"=$3 WHERE "appId"=$1 AND "grantId"=$2"#
        ),
        [
            app_id.clone().into(),
            grant.grant_id.clone().into(),
            resume_at.into(),
        ],
    ))
    .await?;
    Ok(())
}

/// The complete set of schedules a service runs now.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleClaimRequest {
    event_ids: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ScheduleClaimResponse {
    /// The hub's clock (Unix seconds) inside the transaction.
    server_time: i64,
    claimed: Vec<ClaimedSchedule>,
    held: Vec<HeldSchedule>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ClaimedSchedule {
    event_id: String,
    /// The hub's clock when this approval got the schedule: the hub may have fired every
    /// scheduled time up to it. A confirmation returns the stored value.
    since: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct HeldSchedule {
    event_id: String,
    reason: HoldReason,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HoldReason {
    NotReleased,
    RunsElsewhere,
}

fn claim_request(body: &[u8]) -> Result<Vec<String>, ApiError> {
    let request: ScheduleClaimRequest = serde_json::from_slice(body).map_err(invalid)?;
    let mut seen = std::collections::HashSet::new();
    if request.event_ids.len() > MAX_CLAIMED_EVENTS
        || !request
            .event_ids
            .iter()
            .all(|id| validate_instance_identifier(id).is_ok() && seen.insert(id.as_str()))
    {
        return Err(invalid("schedule claim"));
    }
    Ok(request.event_ids)
}

/// The service an approval belongs to and the app whose schedules it may run.
#[derive(Clone)]
struct Claimant {
    app_id: String,
    device_id: String,
    placement_id: String,
    grant_id: String,
}

/// `$from, …` scoped to the requested events, with their bound values appended to `values`.
fn requested(values: &mut Vec<Value>, event_ids: &[String]) -> String {
    let list = placeholders(values.len() + 1, event_ids.len());
    values.extend(event_ids.iter().map(|id| id.clone().into()));
    list
}

impl Claimant {
    /// `$1` the app, `$2` the approval.
    fn approval(&self) -> Vec<Value> {
        vec![self.app_id.clone().into(), self.grant_id.clone().into()]
    }

    /// Ends release and claim of what the approval ran and its device no longer lists: a
    /// hand-back the device confirmed. Returns how many it handed back.
    async fn hand_back_unlisted(
        &self,
        tx: &DatabaseTransaction,
        event_ids: &[String],
        now: i64,
    ) -> Result<u64, ApiError> {
        let mut values = self.approval();
        values.push((now + CONFIRMED_GRACE_SECONDS).into());
        let listed = match event_ids {
            [] => String::new(),
            listed => format!(
                r#" AND "eventId" NOT IN ({})"#,
                requested(&mut values, listed)
            ),
        };
        let handed_back = tx
            .execute_raw(sql(
                &format!(
                    r#"UPDATE "DeviceScheduleClaim" SET {CLAIM_CLEARED},{RELEASE_CLEARED},"resumeAt"=$3 WHERE "appId"=$1 AND "grantId"=$2{listed}"#
                ),
                values,
            ))
            .await?;
        Ok(handed_back.rows_affected())
    }

    /// Takes what is released to the service and the approval does not hold: unclaimed, or
    /// claimed by an earlier approval of the same service, whose process is gone. Then stamps
    /// the confirmation on everything it holds of `event_ids`. Returns how many it took.
    async fn take_released(
        &self,
        tx: &DatabaseTransaction,
        event_ids: &[String],
        now: i64,
    ) -> Result<u64, ApiError> {
        let mut values = self.approval();
        values.extend([
            self.device_id.clone().into(),
            self.placement_id.clone().into(),
            now.into(),
        ]);
        let events = requested(&mut values, event_ids);
        let taken = tx
            .execute_raw(sql(
                &format!(
                    r#"UPDATE "DeviceScheduleClaim" SET "grantId"=$2,"claimedAt"=$5,"seenAt"=$5,"resumeAt"=NULL WHERE "appId"=$1 AND "deviceId"=$3 AND "placementId"=$4 AND ("grantId" IS NULL OR "grantId"<>$2) AND "eventId" IN ({events})"#
                ),
                values,
            ))
            .await?;
        let mut values = self.approval();
        values.push(now.into());
        let events = requested(&mut values, event_ids);
        tx.execute_raw(sql(
            &format!(
                r#"UPDATE "DeviceScheduleClaim" SET "seenAt"=$3 WHERE "appId"=$1 AND "grantId"=$2 AND "eventId" IN ({events})"#
            ),
            values,
        ))
        .await?;
        Ok(taken.rows_affected())
    }

    /// What became of one requested event: the time the approval got it, or why it is held.
    fn outcome(&self, row: Option<&ScheduleRow>) -> Result<Result<i64, HoldReason>, ApiError> {
        let service = (self.device_id.as_str(), self.placement_id.as_str());
        Ok(
            match (
                row.and_then(ScheduleRow::service),
                row.and_then(ScheduleRow::claim),
            ) {
                (_, Some((grant_id, since))) if grant_id == self.grant_id => Ok(since),
                (None, _) => Err(HoldReason::NotReleased),
                // Released to this service after `take_released` looked.
                (Some(released_to), _) if released_to == service => {
                    return Err(changed_meanwhile());
                }
                (Some(_), _) => Err(HoldReason::RunsElsewhere),
            },
        )
    }
}

/// The rows of the requested events as they are now.
async fn requested_rows(
    tx: &DatabaseTransaction,
    app_id: &str,
    event_ids: &[String],
) -> Result<HashMap<String, ScheduleRow>, ApiError> {
    let mut values = vec![app_id.into()];
    let events = requested(&mut values, event_ids);
    tx.query_all_raw(sql(
        &format!(
            r#"SELECT {COLUMNS} FROM "DeviceScheduleClaim" WHERE "appId"=$1 AND "eventId" IN ({events})"#
        ),
        values,
    ))
    .await?
    .iter()
    .map(|row| ScheduleRow::read(row).map(|row| (row.event_id.clone(), row)))
    .collect()
}

/// Replaces the set of schedules the claimant's approval runs with `event_ids`, as far as a
/// person released them to its service. Returns the answer and whether the set changed.
async fn claim_in(
    tx: &DatabaseTransaction,
    claimant: &Claimant,
    event_ids: &[String],
    now: i64,
) -> Result<(ScheduleClaimResponse, bool), ApiError> {
    let handed_back = claimant.hand_back_unlisted(tx, event_ids, now).await?;
    let mut response = ScheduleClaimResponse {
        server_time: now,
        claimed: Vec::new(),
        held: Vec::new(),
    };
    if event_ids.is_empty() {
        return Ok((response, handed_back > 0));
    }
    let taken = claimant.take_released(tx, event_ids, now).await?;
    let rows = requested_rows(tx, &claimant.app_id, event_ids).await?;
    for event_id in event_ids {
        let event_id = event_id.clone();
        match claimant.outcome(rows.get(&event_id))? {
            Ok(since) => response.claimed.push(ClaimedSchedule { event_id, since }),
            Err(reason) => response.held.push(HeldSchedule { event_id, reason }),
        }
    }
    Ok((response, handed_back > 0 || taken > 0))
}

/// The claim of an authenticated workload instance. The service is the approval's own; a
/// device can only claim what a person released to that service.
pub(super) async fn claim_for(
    state: &DeviceContext<'_>,
    authorization: &project::AuthorizedProject,
    event_ids: Vec<String>,
) -> Result<(ScheduleClaimResponse, bool), ApiError> {
    let claims = &authorization.claims;
    if claims.purpose != InstancePurpose::Workload {
        return Err(ApiError::FORBIDDEN);
    }
    let claimant = Claimant {
        app_id: claims.project_id.clone(),
        device_id: claims.device_id.clone(),
        placement_id: claims.placement_id.clone(),
        grant_id: claims.grant_id.clone(),
    };
    let authorization = authorization.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let authorization = authorization.clone();
            let claimant = claimant.clone();
            let event_ids = event_ids.clone();
            Box::pin(async move {
                project::recheck_in(tx, &authorization).await?;
                claim_in(tx, &claimant, &event_ids, now()).await
            })
        },
    )
    .await
}

/// `POST /instances/project/schedules`: a service tells the hub which schedules it runs now.
pub(crate) async fn claim(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<ScheduleClaimResponse, ApiError> {
    let context = devices::context(state);
    let authorization = project::authenticate(&context, headers, "POST", CLAIM_PATH).await?;
    let event_ids = claim_request(body)?;
    let (response, changed) = claim_for(&context, &authorization, event_ids).await?;
    if changed {
        project::audit(
            state,
            &authorization.claims,
            CLAIM_ACTION,
            "PlacementResourceGrant",
            &authorization.claims.grant_id,
            serde_json::json!({
                "claimed": response.claimed.len(),
                "held": response.held.len(),
            }),
        )
        .await;
    }
    Ok(response)
}

/// The service a schedule or bot is released to.
#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseScheduleRequest {
    device_id: String,
    /// The service on that device.
    placement_id: String,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct ReleasedSchedule {
    /// `released` while the hub still runs it, `device` once the service runs it.
    #[schema(value_type = String)]
    state: ScheduleState,
    /// Unix seconds: when it was released, or when the service took it over.
    since: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct GivenBackSchedule {
    /// Unix seconds from which the hub takes it back, or `null` when no service had taken it
    /// over.
    hub_resumes_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ScheduleState {
    Device,
    Released,
    Returning,
}

fn released(row: &ScheduleRow) -> ReleasedSchedule {
    match row.claim() {
        Some((_, since)) => ReleasedSchedule {
            state: ScheduleState::Device,
            since,
        },
        None => ReleasedSchedule {
            state: ScheduleState::Released,
            since: row.released_at.unwrap_or_default(),
        },
    }
}

/// Releases a schedule to one service. Returns what the row says now and whether this call
/// changed it.
pub(super) async fn release_to(
    state: &DeviceContext<'_>,
    person: &str,
    app_id: &str,
    event_id: &str,
    request: &ReleaseScheduleRequest,
) -> Result<(ReleasedSchedule, bool), ApiError> {
    for id in [event_id, &request.device_id, &request.placement_id] {
        validate_instance_identifier(id).map_err(invalid)?;
    }
    let target = (request.device_id.as_str(), request.placement_id.as_str());
    // A claim by another service only ends here when its approval stopped working. That
    // hand-back is committed by itself: it stands even when this release is then refused.
    if let Some(row) = read_row(state.db, app_id, event_id).await?
        && row.service() != Some(target)
        && let Some((grant_id, _)) = row.claim()
        && settle_claim(state.db, state.dialect, app_id, &row, grant_id, now()).await?
            == Settled::Runs
    {
        return Err(runs_elsewhere());
    }
    let person = person.to_owned();
    let app_id = app_id.to_owned();
    let event_id = event_id.to_owned();
    let request = request.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let person = person.clone();
            let app_id = app_id.clone();
            let event_id = event_id.clone();
            let request = request.clone();
            Box::pin(async move { release_in(tx, &person, &app_id, &event_id, &request).await })
        },
    )
    .await
}

/// Only a schedule nobody runs and no grace period protects moves; one that is already
/// released to this service stays as it is.
const RELEASE: &str = r#"INSERT INTO "DeviceScheduleClaim" ("appId","eventId","deviceId","placementId","releasedBy","releasedAt") VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT ("appId","eventId") DO UPDATE SET "deviceId"=EXCLUDED."deviceId","placementId"=EXCLUDED."placementId","releasedBy"=EXCLUDED."releasedBy","releasedAt"=EXCLUDED."releasedAt","seenAt"=NULL,"resumeAt"=NULL WHERE "DeviceScheduleClaim"."grantId" IS NULL AND ("DeviceScheduleClaim"."resumeAt" IS NULL OR "DeviceScheduleClaim"."resumeAt"<$6) AND ("DeviceScheduleClaim"."deviceId" IS NULL OR "DeviceScheduleClaim"."deviceId"<>EXCLUDED."deviceId" OR "DeviceScheduleClaim"."placementId"<>EXCLUDED."placementId")"#;

async fn release_in(
    tx: &DatabaseTransaction,
    person: &str,
    app_id: &str,
    event_id: &str,
    request: &ReleaseScheduleRequest,
) -> Result<(ReleasedSchedule, bool), ApiError> {
    let now = now();
    let written = tx
        .execute_raw(sql(
            RELEASE,
            [
                app_id.into(),
                event_id.into(),
                request.device_id.clone().into(),
                request.placement_id.clone().into(),
                person.into(),
                now.into(),
            ],
        ))
        .await?;
    let row = read_row(tx, app_id, event_id)
        .await?
        .ok_or_else(changed_meanwhile)?;
    let target = (request.device_id.as_str(), request.placement_id.as_str());
    if row.service() == Some(target) {
        return Ok((released(&row), written.rows_affected() == 1));
    }
    if row.claim().is_some() {
        return Err(runs_elsewhere());
    }
    if row.returning(now).is_some() {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            RETURNING,
            "This schedule or bot is still returning to the hub from where it ran. Release it again once its grace period is over.",
        ));
    }
    Err(changed_meanwhile())
}

fn runs_elsewhere() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        RUNS_ELSEWHERE,
        "Another service runs this schedule or bot. It runs in one place: remove it there, or give it back to the hub first.",
    )
}

/// Gives a schedule back to the hub. Returns when the hub runs it again (`None` when it ran
/// it all along) and whether a release ended.
pub(super) async fn give_back_from(
    state: &DeviceContext<'_>,
    app_id: &str,
    event_id: &str,
) -> Result<(GivenBackSchedule, bool), ApiError> {
    validate_instance_identifier(event_id).map_err(invalid)?;
    let app_id = app_id.to_owned();
    let event_id = event_id.to_owned();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let app_id = app_id.clone();
            let event_id = event_id.clone();
            Box::pin(async move { give_back_in(tx, &app_id, &event_id).await })
        },
    )
    .await
}

async fn give_back_in(
    tx: &DatabaseTransaction,
    app_id: &str,
    event_id: &str,
) -> Result<(GivenBackSchedule, bool), ApiError> {
    let now = now();
    let Some(row) = read_row(tx, app_id, event_id).await? else {
        let hub_ran_it = GivenBackSchedule {
            hub_resumes_at: None,
        };
        return Ok((hub_ran_it, false));
    };
    let mut values: Vec<Value> = vec![app_id.into(), event_id.into()];
    let (hub_resumes_at, held_by) = match row.claim() {
        Some((grant_id, _)) => {
            values.push(grant_id.to_owned().into());
            let resume_at = resume_after(tx, grant_id, now).await?;
            (Some(resume_at), r#""grantId"=$3"#)
        }
        None => (row.returning(now), r#""grantId" IS NULL"#),
    };
    let statement = match hub_resumes_at {
        Some(resume_at) => {
            values.push(resume_at.into());
            format!(
                r#"UPDATE "DeviceScheduleClaim" SET {CLAIM_CLEARED},{RELEASE_CLEARED},"resumeAt"=${} WHERE "appId"=$1 AND "eventId"=$2 AND {held_by}"#,
                values.len()
            )
        }
        // Nothing ran it and no grace period is left: the hub's again, as with no row.
        None => format!(
            r#"DELETE FROM "DeviceScheduleClaim" WHERE "appId"=$1 AND "eventId"=$2 AND {held_by}"#
        ),
    };
    if tx
        .execute_raw(sql(&statement, values))
        .await?
        .rows_affected()
        != 1
    {
        return Err(changed_meanwhile());
    }
    let given_back = GivenBackSchedule { hub_resumes_at };
    Ok((given_back, row.service().is_some()))
}

/// A schedule or bot of an app that a device runs, is about to run, or just ran.
#[derive(Debug, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct AppDeviceSchedule {
    event_id: String,
    /// `device`: a service runs it. `released`: released to a service that has not taken it
    /// over, so the hub still has it. `returning`: handed back; the hub takes it back at
    /// `hub_resumes_at`.
    #[schema(value_type = String)]
    state: ScheduleState,
    /// Unix seconds: when the service took it over, or when it was released.
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<i64>,
    /// Unix seconds of the device's last confirmation. A running service confirms every 30
    /// minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    seen_at: Option<i64>,
    /// Unix seconds until which the hub does not run it.
    #[serde(skip_serializing_if = "Option::is_none")]
    hub_resumes_at: Option<i64>,
    /// The cloud approval that runs it. Only on a device in your device list.
    #[serde(skip_serializing_if = "Option::is_none")]
    grant_id: Option<String>,
    /// Only on a device in your device list.
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    /// The service on that device. Only on a device in your device list.
    #[serde(skip_serializing_if = "Option::is_none")]
    placement_id: Option<String>,
}

/// The rows of one app that say more than "the hub runs it", or `None` when the claim table
/// does not exist yet: a hub that cannot hand schedules to devices lists none.
pub(super) async fn listed_rows<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    now: i64,
) -> Result<Option<Vec<ScheduleRow>>, ApiError> {
    let listed = sql(
        &format!(
            r#"SELECT {COLUMNS} FROM "DeviceScheduleClaim" WHERE "appId"=$1 AND ("deviceId" IS NOT NULL OR "resumeAt">=$2) ORDER BY "eventId" LIMIT {MAX_LISTED_SCHEDULES}"#
        ),
        [app_id.into(), now.into()],
    );
    match db.query_all_raw(listed).await {
        Ok(rows) => rows
            .iter()
            .map(ScheduleRow::read)
            .collect::<Result<_, _>>()
            .map(Some),
        Err(error) if table_missing(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Who runs which schedule, from the rows as they are. Which device and service is said only
/// for a device the caller sees.
pub(super) fn listed(
    rows: Vec<ScheduleRow>,
    sees_device: impl Fn(&str) -> bool,
    now: i64,
) -> Vec<AppDeviceSchedule> {
    rows.into_iter()
        .filter_map(|row| {
            let hub_resumes_at = row.returning(now);
            let visible = row.device_id().is_some_and(&sees_device);
            let (state, since) = match (row.claim(), row.service()) {
                (Some((_, since)), _) => (ScheduleState::Device, Some(since)),
                (None, Some(_)) => (ScheduleState::Released, row.released_at),
                (None, None) => (ScheduleState::Returning, None),
            };
            if state == ScheduleState::Returning && hub_resumes_at.is_none() {
                return None;
            }
            let claimed = state == ScheduleState::Device;
            Some(AppDeviceSchedule {
                state,
                since,
                seen_at: row.seen_at.filter(|_| claimed),
                hub_resumes_at,
                grant_id: row.grant_id.filter(|_| visible),
                device_id: row.device_id.filter(|_| visible),
                placement_id: row.placement_id.filter(|_| visible),
                event_id: row.event_id,
            })
        })
        .collect()
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/device-schedules/{event_id}",
    tag = "devices",
    description = "Let one service on a device run a schedule or a bot of this app instead of the hub. The hub keeps running a schedule until that service is running and takes it over; a bot connects from the device once the service has taken it over. A schedule or a bot runs in one place: while another service runs it, or it is still returning to the hub, this is refused. Deploying a schedule or a bot to a device does this for you.",
    params(
        ("app_id" = String, Path, description = "Project ID"),
        ("event_id" = String, Path, description = "The event ID of the schedule or bot")
    ),
    request_body = ReleaseScheduleRequest,
    responses(
        (status = 200, description = "Where the schedule or bot runs now", body = ReleasedSchedule),
        (status = 400, description = "An ID is not valid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot edit this project's events, or the account or token cannot manage devices"),
        (status = 409, description = "Another service runs it (code SCHEDULE_RUNS_ELSEWHERE), or it is still returning to the hub from where it ran (code SCHEDULE_RETURNING)"),
        (status = 503, description = "This hub does not manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
pub(crate) async fn release(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(request): Json<ReleaseScheduleRequest>,
) -> Result<([(HeaderName, &'static str); 2], Json<ReleasedSchedule>), ApiError> {
    let person = caller(&state, &user).await?;
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let (released, changed) = release_to(
        &devices::context(&state),
        &person,
        &app_id,
        &event_id,
        &request,
    )
    .await?;
    if changed {
        audit_branch!(
            state,
            user,
            app_id,
            RELEASE_ACTION,
            "Event",
            event_id,
            serde_json::json!({
                "device_id": request.device_id,
                "placement_id": request.placement_id,
            })
        );
    }
    Ok((NO_STORE, Json(released)))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/device-schedules/{event_id}",
    tag = "devices",
    description = "Give a schedule or a bot of this app back to the hub. The hub runs a schedule again a few minutes later, or about an hour later while the device may still be running it: a scheduled time is missed rather than run twice. The device stops running it, or disconnects the bot, at its next check with the hub. The hub does not connect a bot by itself.",
    params(
        ("app_id" = String, Path, description = "Project ID"),
        ("event_id" = String, Path, description = "The event ID of the schedule or bot")
    ),
    responses(
        (status = 200, description = "From when the hub runs it again", body = GivenBackSchedule),
        (status = 400, description = "The event ID is not valid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot edit this project's events, or the account or token cannot manage devices")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
pub(crate) async fn give_back(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<([(HeaderName, &'static str); 2], Json<GivenBackSchedule>), ApiError> {
    // Not behind the devices switch: on a hub that turned devices off this is the way to
    // get a schedule back.
    devices::human_owner(&state, &user).await?;
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let (given_back, changed) =
        give_back_from(&devices::context(&state), &app_id, &event_id).await?;
    if changed {
        audit_branch!(
            state,
            user,
            app_id,
            GIVE_BACK_ACTION,
            "Event",
            event_id,
            serde_json::json!({ "hub_resumes_at": given_back.hub_resumes_at })
        );
    }
    Ok((NO_STORE, Json(given_back)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(event_id: &str) -> ScheduleRow {
        ScheduleRow {
            event_id: event_id.into(),
            device_id: None,
            placement_id: None,
            released_at: None,
            grant_id: None,
            claimed_at: None,
            seen_at: None,
            resume_at: None,
        }
    }

    fn released_row(event_id: &str) -> ScheduleRow {
        ScheduleRow {
            device_id: Some("dev-1".into()),
            placement_id: Some("reports".into()),
            released_at: Some(1_790_000_000),
            ..row(event_id)
        }
    }

    fn rows() -> Vec<ScheduleRow> {
        vec![
            ScheduleRow {
                grant_id: Some("g-1".into()),
                claimed_at: Some(1_790_000_000),
                seen_at: Some(1_790_001_800),
                released_at: Some(1_789_990_000),
                ..released_row("evt_report")
            },
            released_row("evt_mail"),
            ScheduleRow {
                resume_at: Some(1_790_000_300),
                ..row("evt_sync")
            },
            ScheduleRow {
                resume_at: Some(1_789_999_999),
                ..row("evt_back_on_the_hub")
            },
        ]
    }

    /// The literal of the design's contract with the device console.
    #[test]
    fn the_listing_says_who_runs_which_schedule() {
        assert_eq!(
            serde_json::to_value(listed(rows(), |device| device == "dev-1", 1_790_000_000))
                .unwrap(),
            serde_json::json!([
                {"event_id":"evt_report","state":"device","since":1790000000_i64,"seen_at":1790001800_i64,"grant_id":"g-1","device_id":"dev-1","placement_id":"reports"},
                {"event_id":"evt_mail","state":"released","since":1790000000_i64,"device_id":"dev-1","placement_id":"reports"},
                {"event_id":"evt_sync","state":"returning","hub_resumes_at":1790000300_i64}
            ])
        );
    }

    #[test]
    fn a_device_the_caller_cannot_see_is_not_named() {
        assert_eq!(
            serde_json::to_value(listed(rows(), |_| false, 1_790_000_000)).unwrap(),
            serde_json::json!([
                {"event_id":"evt_report","state":"device","since":1790000000_i64,"seen_at":1790001800_i64},
                {"event_id":"evt_mail","state":"released","since":1790000000_i64},
                {"event_id":"evt_sync","state":"returning","hub_resumes_at":1790000300_i64}
            ])
        );
    }

    #[test]
    fn a_release_during_a_grace_period_says_when_the_hub_runs_it_again() {
        let waiting = ScheduleRow {
            resume_at: Some(1_790_003_900),
            ..released_row("evt_report")
        };
        assert_eq!(
            serde_json::to_value(listed(vec![waiting], |_| true, 1_790_000_000)).unwrap(),
            serde_json::json!([
                {"event_id":"evt_report","state":"released","since":1790000000_i64,"hub_resumes_at":1790003900_i64,"device_id":"dev-1","placement_id":"reports"}
            ])
        );
    }

    #[test]
    fn a_claim_names_at_most_64_distinct_valid_events() {
        assert_eq!(
            claim_request(br#"{"event_ids":["evt_report","evt_sync"]}"#).unwrap(),
            ["evt_report", "evt_sync"]
        );
        assert!(claim_request(br#"{"event_ids":[]}"#).unwrap().is_empty());
        let ids = |count: usize| {
            let ids = (0..count)
                .map(|index| format!("event-{index}"))
                .collect::<Vec<_>>();
            serde_json::to_vec(&serde_json::json!({ "event_ids": ids })).unwrap()
        };
        assert_eq!(claim_request(&ids(64)).unwrap().len(), 64);
        for refused in [
            ids(65),
            br#"{"event_ids":["evt_a","evt_a"]}"#.to_vec(),
            br#"{"event_ids":["../other"]}"#.to_vec(),
            br#"{"event_ids":["evt_a"],"placement_id":"reports"}"#.to_vec(),
            br#"{}"#.to_vec(),
            b"not json".to_vec(),
        ] {
            assert_eq!(
                claim_request(&refused).unwrap_err().status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn the_claim_answer_is_what_the_agent_reads() {
        let response = ScheduleClaimResponse {
            server_time: 1_790_000_000,
            claimed: vec![ClaimedSchedule {
                event_id: "evt_report".into(),
                since: 1_789_990_000,
            }],
            held: vec![HeldSchedule {
                event_id: "evt_sync".into(),
                reason: HoldReason::NotReleased,
            }],
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"server_time":1790000000,"claimed":[{"event_id":"evt_report","since":1789990000}],"held":[{"event_id":"evt_sync","reason":"not_released"}]}"#
        );
        assert_eq!(
            serde_json::to_value(HoldReason::RunsElsewhere).unwrap(),
            "runs_elsewhere"
        );
    }

    #[test]
    fn release_and_give_back_answer_in_their_documented_shape() {
        assert_eq!(
            serde_json::to_value(released(&released_row("evt_mail"))).unwrap(),
            serde_json::json!({"state":"released","since":1790000000_i64})
        );
        assert_eq!(
            serde_json::to_value(GivenBackSchedule {
                hub_resumes_at: None
            })
            .unwrap(),
            serde_json::json!({"hub_resumes_at":null})
        );
        assert!(
            serde_json::from_str::<ReleaseScheduleRequest>(
                r#"{"device_id":"dev-1","placement_id":"reports","extra":true}"#
            )
            .is_err()
        );
        assert_eq!(UNCONFIRMED_GRACE_SECONDS, 3_900);
    }

    fn handler<'a>(source: &'a str, name: &str) -> &'a str {
        let body = source
            .split_once(&format!("\npub(crate) async fn {name}("))
            .unwrap_or_else(|| panic!("handler {name} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    /// Moving a schedule needs the right to edit the app's events, checked before the row is
    /// touched, and each move is recorded once under its own action.
    #[test]
    fn both_moves_need_the_right_to_edit_events_and_are_recorded() {
        let source = include_str!("schedules.rs");
        let permission = "ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents)";
        for (name, account, change, action) in [
            (
                "release",
                "caller(&state, &user)",
                "release_to(",
                "RELEASE_ACTION",
            ),
            (
                "give_back",
                "devices::human_owner(&state, &user)",
                "give_back_from(",
                "GIVE_BACK_ACTION",
            ),
        ] {
            let body = handler(source, name);
            let position = |needle: &str| {
                body.find(needle)
                    .unwrap_or_else(|| panic!("{name} calls {needle}"))
            };
            assert!(position(account) < position(permission), "{name}");
            assert!(position(permission) < position(change), "{name}");
            assert!(position(change) < position(action), "{name}");
            assert_eq!(body.matches("audit_branch!(").count(), 1, "{name}");
            assert!(body.contains("Ok((NO_STORE, Json("), "{name}");
        }
        assert!(
            !handler(source, "give_back").contains("caller(&state, &user)"),
            "giving a schedule back works on a hub that turned devices off"
        );
    }
}

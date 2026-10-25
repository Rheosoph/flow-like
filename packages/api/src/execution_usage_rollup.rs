//! Executions per UTC day, so period reads never re-count a finished day.
//!
//! Maintenance rolls a day up once it has closed: [`maintain`] rides the
//! `run_sweep` job on serverless deployments and [`spawn`] runs it everywhere
//! else. Readers use only days whose marker says they are rolled up and count
//! the rest from raw executions, so a lagging or failed pass costs speed, never
//! correctness.
//!
//! A pass reads a day once, in one snapshot: the raw executions grouped per
//! rollup row, and the rollup rows as they stood. Each row then gains the
//! difference between the two, added to whatever it holds when the write lands.
//! A late desktop report ([`count_late_execution`], the only writer that can
//! land on a closed day) adds itself to the same row at any time, so the day
//! counts it exactly once whether it commits before the snapshot, during the
//! pass or after it, and a pass that stops half way can simply run again. The
//! writes go in parts that fit DSQL's transaction cap, and each part re-checks
//! the day's claim, so a runner that takes over a stalled day never writes
//! beside the one it replaced.

use std::{
    collections::{HashMap, HashSet},
    time::Duration as StdDuration,
};

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use flow_like_types::tokio::{self, task::JoinHandle, time::Instant};
use sea_orm::{
    ActiveEnum, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    FromQueryResult, IsolationLevel, QueryFilter, QueryOrder, QuerySelect, Select, Set,
    sea_query::{Alias, Expr, ExprTrait, OnConflict},
};

use crate::{
    db::{
        DSQL_MAX_ROWS_PER_TRANSACTION, DbDialect, RetryPolicy, delete_in_batches_by_tuple,
        retry_transaction,
    },
    entity::{
        execution_usage_daily, execution_usage_day, execution_usage_tracking,
        sea_orm_active_enums::ExecutionStatus,
    },
    state::AppState,
    utils::time::utc_midnight,
};

/// How long a day stays open after midnight UTC, so an execution stamped just
/// before midnight but committed just after still lands in its roll-up.
const GRACE_MINUTES: i64 = 15;
const RETENTION_ENV: &str = "EXECUTION_USAGE_ROLLUP_RETENTION_DAYS";
const DEFAULT_RETENTION_DAYS: i64 = 400;
/// Home activity reads up to 90 days back.
const MIN_RETENTION_DAYS: i64 = 90;
const DISABLED_ENV: &str = "EXECUTION_USAGE_ROLLUP_DISABLED";
const SPAWN_INTERVAL: StdDuration = StdDuration::from_secs(5 * 60);
const PASS_BUDGET: StdDuration = StdDuration::from_secs(20);
/// A claim no part has refreshed for this long belongs to a runner that stopped.
const CLAIM_TTL_SECONDS: i64 = 120;
const INSERT_CHUNK: usize = 500;
const DELETE_CHUNK: usize = 1_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RollupSettings {
    pub(crate) retention_days: i64,
    pub(crate) budget: StdDuration,
    /// Rows one part writes. Every part also refreshes the claim, and DSQL
    /// caps a transaction at 3,000 rows.
    pub(crate) part_rows: usize,
}

impl RollupSettings {
    fn from_env() -> Self {
        Self {
            retention_days: retention_days(),
            budget: PASS_BUDGET,
            part_rows: DSQL_MAX_ROWS_PER_TRANSACTION - 1,
        }
    }
}

fn retention_days() -> i64 {
    let days = std::env::var(RETENTION_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS);
    Ord::max(days, MIN_RETENTION_DAYS)
}

/// The oldest day the rollup keeps at `now`.
fn first_kept_day(now: DateTime<FixedOffset>, retention_days: i64) -> NaiveDate {
    now.naive_utc().date() - Duration::days(retention_days)
}

/// The latest UTC day that has closed at `now`.
pub(crate) fn last_closed_day(now: DateTime<FixedOffset>) -> NaiveDate {
    (now.naive_utc() - Duration::minutes(GRACE_MINUTES)).date() - Duration::days(1)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RollupPass {
    pub rolled_days: u64,
    pub pending_days: u64,
    pub deleted_rows: u64,
}

/// Runs [`maintain`] every five minutes for deployments without a scheduler.
pub fn spawn(state: AppState) -> Option<JoinHandle<()>> {
    if std::env::var(DISABLED_ENV)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Execution usage rollup disabled via {DISABLED_ENV}");
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SPAWN_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            run_pass(&state).await;
        }
    }))
}

/// [`maintain`] for a scheduled caller: a pass is budgeted and a day it leaves
/// behind is counted live until a later pass finishes it, so a failure is
/// logged rather than returned.
pub async fn run_pass(state: &AppState) {
    match maintain(state).await {
        Ok(pass) if pass == RollupPass::default() => {}
        Ok(pass) => tracing::info!(
            rolled_days = pass.rolled_days,
            pending_days = pass.pending_days,
            deleted_rows = pass.deleted_rows,
            "Execution usage rollup pass completed"
        ),
        Err(error) => tracing::error!(%error, "Execution usage rollup pass failed"),
    }
}

/// Drops days past the retention, then rolls up closed days, newest first,
/// until the pass budget is spent. The next pass continues where this one
/// stopped.
pub async fn maintain(state: &AppState) -> Result<RollupPass, DbErr> {
    maintain_with(
        &state.db,
        state.db_dialect,
        Utc::now().fixed_offset(),
        &RollupSettings::from_env(),
    )
    .await
}

pub(crate) async fn maintain_with(
    db: &DatabaseConnection,
    dialect: DbDialect,
    now: DateTime<FixedOffset>,
    settings: &RollupSettings,
) -> Result<RollupPass, DbErr> {
    let deadline = Instant::now() + settings.budget;
    let first_kept = first_kept_day(now, settings.retention_days);
    let mut pass = RollupPass {
        deleted_rows: expire(db, dialect, first_kept, deadline).await?,
        ..RollupPass::default()
    };
    let pending = pending_days(db, first_kept, last_closed_day(now)).await?;
    for day in &pending {
        if Instant::now() >= deadline {
            break;
        }
        match roll_up_day(db, dialect, *day, settings.part_rows, deadline).await {
            Ok(DayOutcome::Rolled) => pass.rolled_days += 1,
            Ok(DayOutcome::Busy) => {}
            Ok(DayOutcome::OutOfTime) => break,
            Err(error) => tracing::warn!(%error, %day, "Rolling up execution usage failed"),
        }
    }
    pass.pending_days = pending.len() as u64 - pass.rolled_days;
    Ok(pass)
}

/// The first day in `from..=through` the rollup does not cover yet. Callers
/// count raw executions from that day on.
pub(crate) async fn first_unrolled_day(
    db: &DatabaseConnection,
    from: NaiveDate,
    through: NaiveDate,
) -> NaiveDate {
    if from > through {
        return from;
    }
    match rolled_days(db, from, through).await {
        Ok(rolled) => from
            .iter_days()
            .take_while(|day| *day <= through)
            .find(|day| !rolled.contains(day))
            .unwrap_or(through + Duration::days(1)),
        Err(error) => {
            tracing::warn!(%error, %from, %through, "Reading rolled-up execution days failed");
            from
        }
    }
}

/// Counts `execution` in its day's rollup row when that day is before `now`'s
/// and still kept. Executions are stamped with the time they ran, so only a
/// late report can land on a day a pass may already have counted. Call it in
/// the transaction that inserts the execution.
pub(crate) async fn count_late_execution<C: ConnectionTrait>(
    conn: &C,
    execution: &execution_usage_tracking::Model,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    count_late_execution_kept(conn, execution, now, retention_days()).await
}

async fn count_late_execution_kept<C: ConnectionTrait>(
    conn: &C,
    execution: &execution_usage_tracking::Model,
    now: DateTime<FixedOffset>,
    retention_days: i64,
) -> Result<(), DbErr> {
    let day = execution.created_at.naive_utc().date();
    if day >= now.naive_utc().date() || day < first_kept_day(now, retention_days) {
        return Ok(());
    }
    let group = DayGroup {
        user_id: execution.user_id.clone(),
        technical_user_id: execution.technical_user_id.clone(),
        app_id: execution.app_id.clone(),
        status: execution.status.clone(),
        executions: 1,
        microseconds: execution.microseconds,
    };
    add_rows(conn, vec![group.row(day, 1, execution.microseconds)]).await
}

#[derive(Debug, FromQueryResult)]
struct DayGroup {
    user_id: Option<String>,
    technical_user_id: Option<String>,
    app_id: Option<String>,
    status: ExecutionStatus,
    executions: i64,
    microseconds: i64,
}

impl DayGroup {
    fn id(&self, day: NaiveDate) -> String {
        row_id(
            day,
            self.user_id.as_deref(),
            self.technical_user_id.as_deref(),
            self.app_id.as_deref(),
            &self.status,
        )
    }

    fn row(
        self,
        day: NaiveDate,
        executions: i64,
        microseconds: i64,
    ) -> execution_usage_daily::ActiveModel {
        execution_usage_daily::ActiveModel {
            day: Set(day),
            id: Set(self.id(day)),
            user_id: Set(self.user_id),
            technical_user_id: Set(self.technical_user_id),
            app_id: Set(self.app_id),
            status: Set(self.status),
            executions: Set(executions),
            microseconds: Set(microseconds),
        }
    }
}

/// The id of the rollup row a day's executions with these keys count toward.
fn row_id(
    day: NaiveDate,
    user_id: Option<&str>,
    technical_user_id: Option<&str>,
    app_id: Option<&str>,
    status: &ExecutionStatus,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(day.format("%Y-%m-%d").to_string().as_bytes());
    for part in [user_id, technical_user_id, app_id] {
        match part {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(&(value.len() as u64).to_be_bytes());
                hasher.update(value.as_bytes());
            }
            None => {
                hasher.update(&[0]);
            }
        }
    }
    hasher.update(status.to_value().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Every execution of `day`, counted per account, technical user, app and
/// severity. Plain group keys keep the planner's estimate honest, and the
/// `createdAt` covering index answers it without touching the table.
fn day_groups(day: NaiveDate) -> Select<execution_usage_tracking::Entity> {
    use execution_usage_tracking::Column;

    execution_usage_tracking::Entity::find()
        .select_only()
        .column_as(Column::UserId, "user_id")
        .column_as(Column::TechnicalUserId, "technical_user_id")
        .column_as(Column::AppId, "app_id")
        .column(Column::Status)
        .expr_as(Expr::cust("COUNT(*)"), "executions")
        .expr_as(
            Expr::col(Column::Microseconds)
                .sum()
                .cast_as(Alias::new("BIGINT")),
            "microseconds",
        )
        .filter(Column::CreatedAt.gte(utc_midnight(day)))
        .filter(Column::CreatedAt.lt(utc_midnight(day + Duration::days(1))))
        .group_by(Column::UserId)
        .group_by(Column::TechnicalUserId)
        .group_by(Column::AppId)
        .group_by(Column::Status)
}

/// What each of the day's rows must gain to hold the executions the snapshot
/// saw. Rows without a group any more (written for an account deleted since)
/// go back to zero. Rows that already match are left alone, so a pass that
/// stopped half way resumes where it ended.
fn differences(
    day: NaiveDate,
    groups: Vec<DayGroup>,
    rows: Vec<execution_usage_daily::Model>,
) -> Vec<execution_usage_daily::ActiveModel> {
    let mut held: HashMap<String, (i64, i64)> = rows
        .iter()
        .map(|row| (row.id.clone(), (row.executions, row.microseconds)))
        .collect();
    let mut changes = Vec::new();
    for group in groups {
        let (executions, microseconds) = held.remove(&group.id(day)).unwrap_or((0, 0));
        let gained = (
            group.executions - executions,
            group.microseconds - microseconds,
        );
        if gained != (0, 0) {
            changes.push(group.row(day, gained.0, gained.1));
        }
    }
    for row in rows {
        if let Some((executions, microseconds)) = held.remove(&row.id)
            && (executions, microseconds) != (0, 0)
        {
            changes.push(execution_usage_daily::ActiveModel {
                day: Set(row.day),
                id: Set(row.id),
                user_id: Set(row.user_id),
                technical_user_id: Set(row.technical_user_id),
                app_id: Set(row.app_id),
                status: Set(row.status),
                executions: Set(-executions),
                microseconds: Set(-microseconds),
            });
        }
    }
    changes
}

/// Adds each row's counts to the stored row, creating it when missing.
async fn add_rows<C: ConnectionTrait>(
    conn: &C,
    rows: Vec<execution_usage_daily::ActiveModel>,
) -> Result<(), DbErr> {
    use execution_usage_daily::Column;

    let add = |column: Column| {
        Expr::col((execution_usage_daily::Entity, column))
            .add(Expr::col((Alias::new("excluded"), column)))
    };
    for chunk in rows.chunks(INSERT_CHUNK) {
        execution_usage_daily::Entity::insert_many(chunk.to_vec())
            .on_conflict(
                OnConflict::columns([Column::Day, Column::Id])
                    .value(Column::Executions, add(Column::Executions))
                    .value(Column::Microseconds, add(Column::Microseconds))
                    .to_owned(),
            )
            .exec_without_returning(conn)
            .await?;
    }
    Ok(())
}

async fn rolled_days<C: ConnectionTrait>(
    conn: &C,
    from: NaiveDate,
    through: NaiveDate,
) -> Result<HashSet<NaiveDate>, DbErr> {
    use execution_usage_day::Column;

    Ok(execution_usage_day::Entity::find()
        .select_only()
        .column(Column::Day)
        .filter(Column::Day.between(from, through))
        .filter(Column::RolledUpAt.is_not_null())
        .into_tuple::<NaiveDate>()
        .all(conn)
        .await?
        .into_iter()
        .collect())
}

/// Closed days in `from..=through` without a finished roll-up, newest first,
/// so the days home activity reads most catch up first after an outage.
async fn pending_days(
    db: &DatabaseConnection,
    from: NaiveDate,
    through: NaiveDate,
) -> Result<Vec<NaiveDate>, DbErr> {
    if from > through {
        return Ok(Vec::new());
    }
    let rolled = rolled_days(db, from, through).await?;
    let mut days: Vec<NaiveDate> = from
        .iter_days()
        .take_while(|day| *day <= through)
        .filter(|day| !rolled.contains(day))
        .collect();
    days.reverse();
    Ok(days)
}

/// Deletes the days before `first_kept`, one key range per day, then their
/// markers.
async fn expire(
    db: &DatabaseConnection,
    dialect: DbDialect,
    first_kept: NaiveDate,
    deadline: Instant,
) -> Result<u64, DbErr> {
    use execution_usage_day::Column;

    let days: Vec<NaiveDate> = execution_usage_day::Entity::find()
        .select_only()
        .column(Column::Day)
        .filter(Column::Day.lt(first_kept))
        .order_by_asc(Column::Day)
        .into_tuple()
        .all(db)
        .await?;
    let mut deleted = 0;
    for day in days {
        if Instant::now() >= deadline {
            break;
        }
        deleted += delete_in_batches_by_tuple::<execution_usage_daily::Entity>(
            db,
            dialect,
            Condition::all().add(execution_usage_daily::Column::Day.eq(day)),
            DELETE_CHUNK,
            None,
        )
        .await?
        .rows;
        execution_usage_day::Entity::delete_by_id(day)
            .exec(db)
            .await?;
    }
    Ok(deleted)
}

enum DayOutcome {
    Rolled,
    /// Rolled up or claimed by another runner, or this runner lost its claim.
    Busy,
    OutOfTime,
}

async fn roll_up_day(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
    part_rows: usize,
    deadline: Instant,
) -> Result<DayOutcome, DbErr> {
    let claim = flow_like_types::create_id();
    if !claim_day(db, dialect, day, &claim).await? {
        return Ok(DayOutcome::Busy);
    }
    let outcome = write_day(db, dialect, day, &claim, part_rows, deadline).await;
    if !matches!(outcome, Ok(DayOutcome::Rolled))
        && let Err(error) = settle_claim(db, dialect, day, &claim, false).await
    {
        tracing::warn!(%error, %day, "Releasing an execution usage rollup claim failed");
    }
    outcome
}

async fn write_day(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
    claim: &str,
    part_rows: usize,
    deadline: Instant,
) -> Result<DayOutcome, DbErr> {
    let (groups, rows) = snapshot(db, dialect, day).await?;
    for part in differences(day, groups, rows).chunks(part_rows.max(1)) {
        if Instant::now() >= deadline {
            return Ok(DayOutcome::OutOfTime);
        }
        if !write_part(db, dialect, day, claim, part.to_vec()).await? {
            return Ok(DayOutcome::Busy);
        }
    }
    Ok(if settle_claim(db, dialect, day, claim, true).await? {
        DayOutcome::Rolled
    } else {
        DayOutcome::Busy
    })
}

/// Claims a day nobody has rolled up and no live runner holds.
async fn claim_day(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
    claim: &str,
) -> Result<bool, DbErr> {
    use execution_usage_day::Column;

    let claim = claim.to_owned();
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |txn| {
        let claim = claim.clone();
        Box::pin(async move {
            let now = Utc::now().fixed_offset();
            let inserted = execution_usage_day::Entity::insert(execution_usage_day::ActiveModel {
                day: Set(day),
                rolled_up_at: Set(None),
                claim: Set(Some(claim.clone())),
                claimed_at: Set(Some(now)),
            })
            .on_conflict(OnConflict::column(Column::Day).do_nothing().to_owned())
            .exec_without_returning(txn)
            .await?;
            if inserted > 0 {
                return Ok(true);
            }
            let stale = now - Duration::seconds(CLAIM_TTL_SECONDS);
            let taken = execution_usage_day::Entity::update_many()
                .col_expr(Column::Claim, Expr::value(claim))
                .col_expr(Column::ClaimedAt, Expr::value(now))
                .filter(Column::Day.eq(day))
                .filter(Column::RolledUpAt.is_null())
                .filter(
                    Condition::any()
                        .add(Column::Claim.is_null())
                        .add(Column::ClaimedAt.lt(stale)),
                )
                .exec(txn)
                .await?;
            Ok(taken.rows_affected > 0)
        })
    })
    .await
}

/// The raw groups and the rollup rows of `day`, read in one snapshot.
async fn snapshot(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
) -> Result<(Vec<DayGroup>, Vec<execution_usage_daily::Model>), DbErr> {
    retry_transaction(
        db,
        dialect,
        Some(IsolationLevel::RepeatableRead),
        &RetryPolicy::idempotent(),
        move |txn| {
            Box::pin(async move {
                let groups = day_groups(day).into_model::<DayGroup>().all(txn).await?;
                let rows = execution_usage_daily::Entity::find()
                    .filter(execution_usage_daily::Column::Day.eq(day))
                    .all(txn)
                    .await?;
                Ok((groups, rows))
            })
        },
    )
    .await
}

/// Adds one part while the claim still holds. The additions are not
/// idempotent, so a commit with an unknown outcome is not retried; the next
/// pass reads a fresh snapshot instead.
async fn write_part(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
    claim: &str,
    rows: Vec<execution_usage_daily::ActiveModel>,
) -> Result<bool, DbErr> {
    use execution_usage_day::Column;

    let claim = claim.to_owned();
    retry_transaction(db, dialect, None, &RetryPolicy::default(), move |txn| {
        let claim = claim.clone();
        let rows = rows.clone();
        Box::pin(async move {
            let held = execution_usage_day::Entity::update_many()
                .col_expr(Column::ClaimedAt, Expr::value(Utc::now().fixed_offset()))
                .filter(Column::Day.eq(day))
                .filter(Column::Claim.eq(claim))
                .exec(txn)
                .await?;
            if held.rows_affected == 0 {
                return Ok(false);
            }
            add_rows(txn, rows).await?;
            Ok(true)
        })
    })
    .await
}

/// Clears this runner's claim, marking the day rolled up when `rolled`.
async fn settle_claim(
    db: &DatabaseConnection,
    dialect: DbDialect,
    day: NaiveDate,
    claim: &str,
    rolled: bool,
) -> Result<bool, DbErr> {
    use execution_usage_day::Column;

    let claim = claim.to_owned();
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        let claim = claim.clone();
        Box::pin(async move {
            let mut update = execution_usage_day::Entity::update_many()
                .col_expr(Column::Claim, Expr::value(Option::<String>::None))
                .col_expr(
                    Column::ClaimedAt,
                    Expr::value(Option::<DateTime<FixedOffset>>::None),
                );
            if rolled {
                update =
                    update.col_expr(Column::RolledUpAt, Expr::value(Utc::now().fixed_offset()));
            }
            let settled = update
                .filter(Column::Day.eq(day))
                .filter(Column::Claim.eq(claim))
                .exec(txn)
                .await?;
            Ok(settled.rows_affected > 0)
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::usage::test_db::{database, insert};
    use sea_orm::{DbBackend, QueryTrait};

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn at(stamp: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(stamp).expect("valid timestamp")
    }

    fn settings(part_rows: usize) -> RollupSettings {
        RollupSettings {
            retention_days: 5,
            budget: StdDuration::from_secs(60),
            part_rows,
        }
    }

    async fn rolled<C: ConnectionTrait>(db: &C, on: NaiveDate) -> Vec<(i64, i64)> {
        let mut rows: Vec<_> = execution_usage_daily::Entity::find()
            .filter(execution_usage_daily::Column::Day.eq(on))
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.executions, row.microseconds))
            .collect();
        rows.sort();
        rows
    }

    async fn execution(db: &DatabaseConnection, id: &str) -> execution_usage_tracking::Model {
        execution_usage_tracking::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_day_closes_a_grace_period_after_midnight() {
        assert_eq!(
            last_closed_day(at("2026-10-21T00:14:59Z")),
            day(2026, 10, 19)
        );
        assert_eq!(
            last_closed_day(at("2026-10-21T00:15:00Z")),
            day(2026, 10, 20)
        );
        assert_eq!(
            last_closed_day(at("2026-10-21T23:59:59Z")),
            day(2026, 10, 20)
        );
    }

    #[test]
    fn row_ids_separate_every_key_and_a_missing_value_from_an_empty_one() {
        let id = |date: u32, user, technical_user, app, status| {
            row_id(day(2026, 10, date), user, technical_user, app, &status)
        };
        let base = id(20, Some("user"), None, Some("app"), ExecutionStatus::Info);
        assert_eq!(
            base,
            id(20, Some("user"), None, Some("app"), ExecutionStatus::Info)
        );
        for other in [
            id(21, Some("user"), None, Some("app"), ExecutionStatus::Info),
            id(
                20,
                Some("user"),
                Some(""),
                Some("app"),
                ExecutionStatus::Info,
            ),
            id(20, Some("userapp"), None, None, ExecutionStatus::Info),
            id(20, Some("user"), None, Some("app"), ExecutionStatus::Error),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn a_day_is_grouped_by_plain_columns_over_half_open_bounds() {
        let sql = day_groups(day(2026, 10, 20))
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            sql.contains(r#""createdAt" >= '2026-10-20 00:00:00.000000 +00:00'"#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""createdAt" < '2026-10-21 00:00:00.000000 +00:00'"#),
            "{sql}"
        );
        assert!(
            sql.ends_with(
                r#"GROUP BY "ExecutionUsageTracking"."userId", "ExecutionUsageTracking"."technicalUserId", "ExecutionUsageTracking"."appId", "ExecutionUsageTracking"."status""#
            ),
            "{sql}"
        );
    }

    #[test]
    fn differences_skip_settled_rows_and_zero_rows_without_a_group() {
        let on = day(2026, 10, 20);
        let group = |user: &str, executions| DayGroup {
            user_id: Some(user.into()),
            technical_user_id: None,
            app_id: None,
            status: ExecutionStatus::Info,
            executions,
            microseconds: executions * 10,
        };
        let stored = |group: DayGroup, executions: i64| execution_usage_daily::Model {
            day: on,
            id: group.id(on),
            user_id: group.user_id,
            technical_user_id: None,
            app_id: None,
            status: ExecutionStatus::Info,
            executions,
            microseconds: executions * 10,
        };
        let changes = differences(
            on,
            vec![group("settled", 4), group("behind", 5), group("new", 2)],
            vec![
                stored(group("settled", 0), 4),
                stored(group("behind", 0), 3),
                stored(group("deleted", 0), 7),
            ],
        );
        let gained: Vec<_> = changes
            .into_iter()
            .map(|row| (row.user_id.unwrap(), row.executions.unwrap()))
            .collect();
        assert_eq!(
            gained,
            [
                (Some("behind".into()), 2),
                (Some("new".into()), 2),
                (Some("deleted".into()), -7)
            ]
        );
    }

    #[tokio::test]
    async fn a_pass_rolls_closed_days_in_parts_and_leaves_today_live() {
        let db = database().await;
        let now = at("2026-10-22T12:00:00Z");
        for (id, user, app) in [
            ("a1", "alice", Some("a")),
            ("a2", "alice", Some("a")),
            ("b1", "bob", Some("a")),
            ("c1", "carol", None),
        ] {
            insert(
                &db,
                id,
                user,
                app,
                at("2026-10-20T08:00:00Z"),
                ExecutionStatus::Info,
                10,
            )
            .await;
        }
        insert(
            &db,
            "today",
            "alice",
            Some("a"),
            now,
            ExecutionStatus::Info,
            10,
        )
        .await;

        let pass = maintain_with(&db, DbDialect::Postgres, now, &settings(1))
            .await
            .unwrap();
        assert_eq!((pass.rolled_days, pass.pending_days), (5, 0));
        assert_eq!(
            rolled(&db, day(2026, 10, 20)).await,
            [(1, 10), (1, 10), (2, 20)]
        );
        assert_eq!(
            first_unrolled_day(&db, day(2026, 10, 17), day(2026, 10, 22)).await,
            day(2026, 10, 22)
        );

        let again = maintain_with(&db, DbDialect::Postgres, now, &settings(1))
            .await
            .unwrap();
        assert_eq!(again, RollupPass::default());
    }

    #[tokio::test]
    async fn late_reports_count_once_before_and_after_the_pass() {
        let db = database().await;
        let now = at("2026-10-22T12:00:00Z");
        let closed = day(2026, 10, 20);
        let stamp = at("2026-10-20T08:00:00Z");
        insert(
            &db,
            "ran",
            "alice",
            Some("a"),
            stamp,
            ExecutionStatus::Info,
            10,
        )
        .await;
        insert(
            &db,
            "early",
            "alice",
            Some("a"),
            stamp,
            ExecutionStatus::Info,
            10,
        )
        .await;
        count_late_execution(&db, &execution(&db, "early").await, now)
            .await
            .unwrap();
        assert_eq!(rolled(&db, closed).await, [(1, 10)]);

        maintain_with(&db, DbDialect::Postgres, now, &settings(10))
            .await
            .unwrap();
        assert_eq!(rolled(&db, closed).await, [(2, 20)]);

        insert(
            &db,
            "late",
            "alice",
            Some("a"),
            stamp,
            ExecutionStatus::Info,
            10,
        )
        .await;
        count_late_execution(&db, &execution(&db, "late").await, now)
            .await
            .unwrap();
        assert_eq!(rolled(&db, closed).await, [(3, 30)]);
    }

    #[tokio::test]
    async fn a_live_claim_keeps_other_runners_off_until_it_goes_stale() {
        use execution_usage_day::Column;

        let db = database().await;
        let closed = day(2026, 10, 21);
        let claim = |token| claim_day(&db, DbDialect::Postgres, closed, token);
        let settle = |token, rolled| settle_claim(&db, DbDialect::Postgres, closed, token, rolled);
        assert!(claim("first").await.unwrap());
        assert!(!claim("second").await.unwrap());

        execution_usage_day::Entity::update_many()
            .col_expr(Column::ClaimedAt, Expr::value(at("2000-01-01T00:00:00Z")))
            .filter(Column::Day.eq(closed))
            .exec(&db)
            .await
            .unwrap();
        assert!(claim("second").await.unwrap());

        let stalled = write_part(&db, DbDialect::Postgres, closed, "first", Vec::new());
        assert!(!stalled.await.unwrap());
        assert!(!settle("first", true).await.unwrap());
        assert!(settle("second", false).await.unwrap());
        assert_eq!(
            first_unrolled_day(&db, closed, closed).await,
            closed,
            "a released claim leaves the day for the next pass"
        );
    }

    #[tokio::test]
    async fn retention_drops_whole_days_and_late_reports_for_them() {
        let db = database().await;
        let stamp = at("2026-10-20T08:00:00Z");
        insert(
            &db,
            "old",
            "alice",
            Some("a"),
            stamp,
            ExecutionStatus::Info,
            10,
        )
        .await;
        maintain_with(
            &db,
            DbDialect::Postgres,
            at("2026-10-22T12:00:00Z"),
            &settings(10),
        )
        .await
        .unwrap();
        assert_eq!(rolled(&db, day(2026, 10, 20)).await, [(1, 10)]);

        let later = at("2026-10-26T12:00:00Z");
        let pass = maintain_with(&db, DbDialect::Postgres, later, &settings(10))
            .await
            .unwrap();
        assert_eq!(pass.deleted_rows, 1);
        assert!(rolled(&db, day(2026, 10, 20)).await.is_empty());
        assert_eq!(
            execution_usage_day::Entity::find_by_id(day(2026, 10, 20))
                .one(&db)
                .await
                .unwrap(),
            None
        );

        insert(
            &db,
            "late",
            "alice",
            Some("a"),
            stamp,
            ExecutionStatus::Info,
            10,
        )
        .await;
        count_late_execution_kept(&db, &execution(&db, "late").await, later, 5)
            .await
            .unwrap();
        assert!(rolled(&db, day(2026, 10, 20)).await.is_empty());
    }

    /// A late report that commits between a pass's snapshot and its write
    /// keeps its own addition; the pass adds only what its snapshot saw.
    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_ROLLUP_TEST_DATABASE_URL"]
    async fn postgres_counts_a_late_report_racing_a_pass_once() {
        use sea_orm::{ActiveModelTrait, TransactionTrait};

        let db = postgres::database().await;
        let closed = day(2026, 10, 20);
        let now = at("2026-10-22T12:00:00Z");
        for id in ["first", "second"] {
            postgres::execution(id, "alice", at("2026-10-20T08:00:00Z"))
                .insert(&db)
                .await
                .unwrap();
        }

        assert!(
            claim_day(&db, DbDialect::Postgres, closed, "pass")
                .await
                .unwrap()
        );
        let (groups, rows) = snapshot(&db, DbDialect::Postgres, closed).await.unwrap();

        let report = db.begin().await.unwrap();
        let late = postgres::execution("late", "alice", at("2026-10-20T09:00:00Z"))
            .insert(&report)
            .await
            .unwrap();
        count_late_execution(&report, &late, now).await.unwrap();
        report.commit().await.unwrap();

        let changes = differences(closed, groups, rows);
        assert!(
            write_part(&db, DbDialect::Postgres, closed, "pass", changes)
                .await
                .unwrap()
        );
        assert!(
            settle_claim(&db, DbDialect::Postgres, closed, "pass", true)
                .await
                .unwrap()
        );
        assert_eq!(rolled(&db, closed).await, [(3, 30)]);

        let pass = maintain_with(&db, DbDialect::Postgres, now, &settings(10))
            .await
            .unwrap();
        assert_eq!(rolled(&db, closed).await, [(3, 30)]);
        assert_eq!(pass.pending_days, 0);
    }
}

/// A fresh PostgreSQL database holding the tables the rollup touches, created
/// on the server `FLOW_LIKE_ROLLUP_TEST_DATABASE_URL` points at (the role needs
/// CREATEDB), with the rollup migration applied as committed.
#[cfg(test)]
pub(crate) mod postgres {
    use chrono::{DateTime, FixedOffset};
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Set};

    use crate::entity::{execution_usage_tracking, sea_orm_active_enums::ExecutionStatus};

    const TABLES: &str = r#"
        CREATE TABLE "User" ("id" TEXT PRIMARY KEY);
        CREATE TABLE "App" ("id" TEXT PRIMARY KEY);
        CREATE TABLE "TechnicalUser" ("id" TEXT PRIMARY KEY);
        CREATE TABLE "ExecutionUsageTracking" (
            "id" TEXT PRIMARY KEY, "instance" TEXT, "boardId" TEXT NOT NULL,
            "nodeId" TEXT NOT NULL, "version" TEXT NOT NULL,
            "microseconds" BIGINT NOT NULL, "status" TEXT NOT NULL,
            "userId" TEXT REFERENCES "User"("id") ON DELETE SET NULL,
            "appId" TEXT REFERENCES "App"("id") ON DELETE CASCADE,
            "technicalUserId" TEXT REFERENCES "TechnicalUser"("id") ON DELETE SET NULL,
            "createdAt" TIMESTAMPTZ(3) NOT NULL, "updatedAt" TIMESTAMPTZ(3) NOT NULL
        );
        CREATE INDEX "ExecutionUsageTracking_createdAt_idx" ON "ExecutionUsageTracking"("createdAt");
        INSERT INTO "User" VALUES ('alice'), ('bob');
        INSERT INTO "App" VALUES ('a'), ('b');
    "#;

    pub(crate) async fn database() -> DatabaseConnection {
        let url = std::env::var("FLOW_LIKE_ROLLUP_TEST_DATABASE_URL")
            .expect("Set FLOW_LIKE_ROLLUP_TEST_DATABASE_URL to a PostgreSQL server");
        let name = format!("rollup_{}", uuid::Uuid::new_v4().simple());
        let admin = Database::connect(&url).await.unwrap();
        admin
            .execute_unprepared(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        admin.close().await.unwrap();
        let (server, _) = url.rsplit_once('/').expect("URL names a database");
        let db = Database::connect(format!("{server}/{name}")).await.unwrap();
        let migration: String =
            include_str!("../prisma/migrations/20261021150000_execution_usage_daily/migration.sql")
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .map(|line| format!("{line}\n"))
                .collect();
        for statement in TABLES.split(';').chain(migration.split(';')) {
            if !statement.trim().is_empty() {
                db.execute_unprepared(statement).await.unwrap();
            }
        }
        db
    }

    pub(crate) fn execution(
        id: &str,
        user: &str,
        at: DateTime<FixedOffset>,
    ) -> execution_usage_tracking::ActiveModel {
        execution_usage_tracking::ActiveModel {
            id: Set(id.to_owned()),
            instance: Set(None),
            board_id: Set("board".into()),
            node_id: Set("entry".into()),
            version: Set(id.to_owned()),
            microseconds: Set(10),
            user_id: Set(Some(user.to_owned())),
            app_id: Set(Some("a".into())),
            created_at: Set(at),
            updated_at: Set(at),
            technical_user_id: Set(None),
            status: Set(ExecutionStatus::Info),
        }
    }
}

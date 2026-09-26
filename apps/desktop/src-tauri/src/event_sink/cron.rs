use anyhow::Result;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::{EventRegistration, EventSink, failure_log::FailureLog, manager::DbConnection};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledLocal {
    pub date: String, // "YYYY-MM-DD"
    pub time: String, // "HH:mm"
}

impl ScheduledLocal {
    fn to_utc_timestamp(&self, tz: Tz) -> Option<i64> {
        let date = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()?;
        let time = NaiveTime::parse_from_str(&self.time, "%H:%M").ok()?;
        let naive_dt = date.and_time(time);
        let tz_dt = tz.from_local_datetime(&naive_dt).single()?;
        let utc_dt = tz_dt.with_timezone(&Utc);
        Some(utc_dt.timestamp())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CronSchedule {
    // Only "expression" is allowed in this branch
    Expression { expression: String },

    // Only "scheduled_for" is allowed in this branch
    Scheduled { scheduled_for: ScheduledLocal },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSink {
    #[serde(flatten)]
    pub schedule: CronSchedule,
    pub last_fired: Option<String>,
    pub timezone: Option<String>,
    /// Where this sink should execute: "LOCAL", "REMOTE", or "HYBRID"
    #[serde(default)]
    pub sink_execution: Option<String>,
}

/// A row from `cron_jobs` that is ready to fire: (event_id, expression, scheduled_for, timezone).
type DueJob = (String, Option<String>, Option<i64>, String);

/// What the worker does with a due job after one attempt to fire it.
enum FireOutcome {
    Fired,
    /// The bus could not take the event right now; the job is retried shortly.
    Retry(String),
    /// The registration cannot fire until it is saved again; the job moves on.
    Defer(String),
}

/// Delay before a job whose event could not be handed to the bus is retried.
const TRANSIENT_RETRY_SECS: i64 = 5;

/// Retry delay for a one-off job that could not fire. An expression job skips
/// to its next occurrence instead.
const DEFERRED_ONE_SHOT_RETRY_SECS: i64 = 5 * 60;

fn format_timestamp(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

impl CronSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                event_id      TEXT PRIMARY KEY,
                expression    TEXT,
                scheduled_for INTEGER,
                timezone      TEXT NOT NULL,
                last_fired    INTEGER,
                next_run      INTEGER,
                created_at    INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_cron_next_run ON cron_jobs(next_run)",
            [],
        )?;

        Ok(())
    }

    fn parse_tz(tz: Option<&str>) -> Tz {
        tz.and_then(|s| s.parse::<Tz>().ok())
            .unwrap_or(chrono_tz::UTC)
    }

    fn compute_next_from_cron(expr: &str, tz: Tz) -> Option<i64> {
        let expr_with_seconds = if expr.split_whitespace().count() == 5 {
            format!("0 {}", expr)
        } else {
            expr.to_string()
        };

        match Schedule::from_str(&expr_with_seconds) {
            Ok(schedule) => {
                let mut upcoming = schedule.upcoming(tz);
                let next = upcoming.next();

                match next {
                    Some(dt) => {
                        let utc_dt = dt.with_timezone(&Utc);
                        let timestamp = utc_dt.timestamp();
                        Some(timestamp)
                    }
                    None => None,
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to parse cron expression '{}': {}",
                    expr_with_seconds,
                    e
                );
                None
            }
        }
    }

    fn add_job(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &CronSink,
    ) -> Result<()> {
        tracing::info!("Adding cron job for event_id: {}", registration.event_id);

        let conn = db.lock().unwrap();
        let now = Utc::now().timestamp();
        let tz = Self::parse_tz(config.timezone.as_deref());

        let (expression, scheduled_for_ts) = match &config.schedule {
            CronSchedule::Expression { expression } => {
                tracing::debug!(
                    "Config: expression='{}', timezone={:?}",
                    expression,
                    config.timezone
                );
                let expr = expression.trim();
                if expr.is_empty() {
                    return Err(anyhow::anyhow!("Cron expression cannot be empty"));
                }
                (Some(expr.to_string()), None)
            }
            CronSchedule::Scheduled { scheduled_for } => {
                tracing::debug!(
                    "Config: scheduled_for='{} {}', timezone={:?}",
                    scheduled_for.date,
                    scheduled_for.time,
                    config.timezone
                );
                let ts = scheduled_for
                    .to_utc_timestamp(tz)
                    .ok_or_else(|| anyhow::anyhow!("Invalid scheduled_for date/time"))?;
                (None, Some(ts))
            }
        };

        let last_fired_ts = config
            .last_fired
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let next_run = if let Some(ref expr) = expression {
            Self::compute_next_from_cron(expr.trim(), tz)
        } else {
            scheduled_for_ts
        };

        tracing::info!(
            "Calculated next_run: {:?} for event_id: {}",
            next_run,
            registration.event_id
        );

        conn.execute(
            "INSERT OR REPLACE INTO cron_jobs
             (event_id, expression, scheduled_for, timezone, last_fired, next_run, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                registration.event_id,
                expression,
                scheduled_for_ts,
                config.timezone.as_deref().unwrap_or("UTC"),
                last_fired_ts,
                next_run,
                now,
            ],
        )?;

        tracing::info!(
            "Successfully inserted cron job for event_id: {}",
            registration.event_id
        );
        Ok(())
    }

    fn remove_job(db: &DbConnection, event_id: &str) -> Result<()> {
        // Startup cleanup unregisters before the worker has created the table.
        Self::init_tables(db)?;
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM cron_jobs WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }

    fn calculate_missing_next_runs(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id, expression, scheduled_for, timezone
               FROM cron_jobs
              WHERE next_run IS NULL",
        )?;

        let jobs: Vec<(String, Option<String>, Option<i64>, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        drop(stmt);

        tracing::debug!("Found {} jobs with NULL next_run", jobs.len());

        for (event_id, expression, scheduled_for, tz_str) in jobs {
            let tz = Self::parse_tz(Some(&tz_str));

            let next_run = if let Some(expr) = expression.as_ref().filter(|e| !e.trim().is_empty())
            {
                Self::compute_next_from_cron(expr.trim(), tz)
            } else {
                scheduled_for
            };

            if let Some(ts) = next_run {
                tracing::debug!("Updating event_id {} with next_run: {}", event_id, ts);
                conn.execute(
                    "UPDATE cron_jobs SET next_run = ?1 WHERE event_id = ?2",
                    params![ts, event_id],
                )?;
            } else {
                tracing::warn!(
                    "Deleting event_id {} - no valid next_run could be calculated",
                    event_id
                );
                conn.execute(
                    "DELETE FROM cron_jobs WHERE event_id = ?1",
                    params![event_id],
                )?;
            }
        }

        Ok(())
    }

    fn get_due_jobs(db: &DbConnection, now: i64) -> Result<Vec<DueJob>> {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id, expression, scheduled_for, timezone
               FROM cron_jobs
              WHERE next_run IS NOT NULL AND next_run <= ?1
           ORDER BY next_run ASC
              LIMIT 64",
        )?;

        stmt.query_map(params![now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    fn fire_event(app_handle: &AppHandle, event_id: &str) -> FireOutcome {
        use crate::state::TauriEventSinkManagerState;

        let Some(manager_state) = app_handle.try_state::<TauriEventSinkManagerState>() else {
            return FireOutcome::Retry("event sink manager not available".to_owned());
        };
        let manager = match manager_state.0.try_lock() {
            Ok(manager) => manager,
            Err(_) => return FireOutcome::Retry("event sink manager busy".to_owned()),
        };
        match manager.fire_event_for_retry(app_handle, event_id, None, None) {
            Ok(Ok(())) => FireOutcome::Fired,
            Ok(Err(reason)) => FireOutcome::Retry(reason),
            Err(err) => FireOutcome::Defer(err.to_string()),
        }
    }

    fn handle_executed_job(
        db: &DbConnection,
        event_id: &str,
        expression: Option<String>,
        tz: Tz,
        now: i64,
    ) -> Result<()> {
        let conn = db.lock().unwrap();

        if let Some(expr) = expression.filter(|e| !e.trim().is_empty()) {
            if let Some(next_ts) = Self::compute_next_from_cron(expr.trim(), tz) {
                conn.execute(
                    "UPDATE cron_jobs SET last_fired = ?1, next_run = ?2 WHERE event_id = ?3",
                    params![now, next_ts, event_id],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM cron_jobs WHERE event_id = ?1",
                    params![event_id],
                )?;
            }
        } else {
            conn.execute(
                "DELETE FROM cron_jobs WHERE event_id = ?1",
                params![event_id],
            )?;
        }

        Ok(())
    }

    /// Stores `next_run`, or drops the job when there is none.
    fn reschedule(db: &DbConnection, event_id: &str, next_run: Option<i64>) -> Result<()> {
        let conn = db.lock().unwrap();
        match next_run {
            Some(ts) => conn.execute(
                "UPDATE cron_jobs SET next_run = ?1 WHERE event_id = ?2",
                params![ts, event_id],
            )?,
            None => conn.execute(
                "DELETE FROM cron_jobs WHERE event_id = ?1",
                params![event_id],
            )?,
        };
        Ok(())
    }

    /// Moves a job that cannot fire past `now`, so the worker stops retrying it
    /// every tick. Returns the new `next_run`, or `None` when the job was
    /// dropped because its expression never yields another time.
    fn defer_job(
        db: &DbConnection,
        event_id: &str,
        expression: Option<String>,
        tz: Tz,
        now: i64,
    ) -> Result<Option<i64>> {
        let next_run = match expression.filter(|e| !e.trim().is_empty()) {
            Some(expr) => Self::compute_next_from_cron(expr.trim(), tz),
            None => Some(now + DEFERRED_ONE_SHOT_RETRY_SECS),
        };
        Self::reschedule(db, event_id, next_run)?;
        Ok(next_run)
    }

    fn get_next_upcoming(db: &DbConnection) -> Option<i64> {
        let conn = db.lock().unwrap();
        conn.query_row(
            "SELECT MIN(next_run) FROM cron_jobs WHERE next_run IS NOT NULL",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None)
    }

    /// Fires every due job and returns the earliest `next_run` left in the table.
    async fn process_jobs(
        db: &DbConnection,
        app_handle: &AppHandle,
        failures: &mut FailureLog,
    ) -> Result<Option<i64>> {
        Self::calculate_missing_next_runs(db)?;

        let now = Utc::now().timestamp();
        let due_jobs = Self::get_due_jobs(db, now)?;

        tracing::debug!("Found {} due jobs at timestamp {}", due_jobs.len(), now);

        for (event_id, expression, _scheduled_for, tz_str) in due_jobs {
            let tz = Self::parse_tz(Some(&tz_str));

            tracing::debug!("Firing event: {}", event_id);

            // A failure is logged at full level only when it changes.
            match Self::fire_event(app_handle, &event_id) {
                FireOutcome::Fired => {
                    match failures.record_success(&event_id) {
                        Some(attempts) => tracing::info!(
                            "Event {} fired successfully after {} failed attempts",
                            event_id,
                            attempts
                        ),
                        None => tracing::info!("Event {} fired successfully", event_id),
                    }
                    Self::handle_executed_job(db, &event_id, expression, tz, now)?;
                }
                FireOutcome::Retry(reason) => {
                    Self::reschedule(db, &event_id, Some(now + TRANSIENT_RETRY_SECS))?;
                    if failures.record_failure(&event_id, &reason) {
                        tracing::warn!(
                            "Event {} could not be fired ({}), retrying in {}s",
                            event_id,
                            reason,
                            TRANSIENT_RETRY_SECS
                        );
                    } else {
                        tracing::debug!(
                            "Event {} could not be fired ({}), retrying in {}s",
                            event_id,
                            reason,
                            TRANSIENT_RETRY_SECS
                        );
                    }
                }
                FireOutcome::Defer(reason) => {
                    let next_run = Self::defer_job(db, &event_id, expression, tz, now)?;
                    let outcome = match next_run {
                        Some(ts) => format!("next attempt at {}", format_timestamp(ts)),
                        None => "dropped, its schedule has no further occurrence".to_owned(),
                    };
                    // The schedule misses its runs until the registration is fixed.
                    if failures.record_failure(&event_id, &reason) {
                        tracing::error!("Event {} cannot fire: {}; {}", event_id, reason, outcome);
                    } else {
                        tracing::debug!("Event {} cannot fire: {}; {}", event_id, reason, outcome);
                    }
                }
            }
        }

        let next = Self::get_next_upcoming(db);
        tracing::debug!("Next upcoming job at: {:?}", next);
        Ok(next)
    }
}

#[async_trait::async_trait]
impl EventSink for CronSink {
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        let app_handle = app_handle.clone();
        let worker_db = db.clone();

        flow_like_types::tokio::spawn(async move {
            tracing::info!("🚀 Cron worker started");

            const MIN_TICK: Duration = Duration::from_millis(250);
            const MAX_TICK: Duration = Duration::from_secs(10);

            let mut failures = FailureLog::default();

            loop {
                let next_upcoming =
                    match Self::process_jobs(&worker_db, &app_handle, &mut failures).await {
                        Ok(ts) => ts,
                        Err(e) => {
                            tracing::error!("Cron processing error: {}", e);
                            None
                        }
                    };

                let now = Utc::now().timestamp();
                let sleep_dur = if let Some(ts) = next_upcoming {
                    if ts <= now {
                        MIN_TICK
                    } else {
                        let d = Duration::from_secs((ts - now) as u64);
                        d.min(MAX_TICK).max(MIN_TICK)
                    }
                } else {
                    MAX_TICK
                };

                flow_like_types::tokio::time::sleep(sleep_dur).await;
            }
        });

        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::info!("Cron sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "CronSink::on_register called for event_id: {}",
            registration.event_id
        );

        Self::add_job(&db, registration, self)?;

        tracing::info!(
            "CronSink::on_register completed for event_id: {}",
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::remove_job(&db, &registration.event_id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    fn db_with_job(
        event_id: &str,
        expression: Option<&str>,
        scheduled_for: Option<i64>,
        next_run: i64,
    ) -> DbConnection {
        let db: DbConnection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        CronSink::init_tables(&db).unwrap();
        db.lock()
            .unwrap()
            .execute(
                "INSERT INTO cron_jobs
                 (event_id, expression, scheduled_for, timezone, last_fired, next_run, created_at)
                 VALUES (?1, ?2, ?3, 'UTC', NULL, ?4, 0)",
                params![event_id, expression, scheduled_for, next_run],
            )
            .unwrap();
        db
    }

    /// `None` when the job row is gone.
    fn stored_next_run(db: &DbConnection, event_id: &str) -> Option<Option<i64>> {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT next_run FROM cron_jobs WHERE event_id = ?1",
                params![event_id],
                |row| row.get(0),
            )
            .ok()
    }

    #[test]
    fn defer_job_skips_an_expression_job_to_its_next_occurrence() {
        let now = Utc::now().timestamp();
        let db = db_with_job("minutely", Some("* * * * *"), None, now - 3600);

        let deferred = CronSink::defer_job(
            &db,
            "minutely",
            Some("* * * * *".to_owned()),
            chrono_tz::UTC,
            now,
        )
        .unwrap();

        let next_run = deferred.expect("expression job keeps a next_run");
        assert!(next_run > now, "{next_run} <= {now}");
        assert!(next_run <= now + 61, "{next_run} > {now} + 61");
        assert_eq!(stored_next_run(&db, "minutely"), Some(deferred));
    }

    #[test]
    fn defer_job_retries_a_one_shot_job_later() {
        let now = Utc::now().timestamp();
        let db = db_with_job("once", None, Some(now - 10), now - 10);

        let deferred = CronSink::defer_job(&db, "once", None, chrono_tz::UTC, now).unwrap();

        assert_eq!(deferred, Some(now + DEFERRED_ONE_SHOT_RETRY_SECS));
        assert_eq!(stored_next_run(&db, "once"), Some(deferred));
    }

    #[test]
    fn defer_job_drops_a_job_whose_expression_never_recurs() {
        let now = Utc::now().timestamp();
        let db = db_with_job("broken", Some("not a cron"), None, now - 10);

        let deferred = CronSink::defer_job(
            &db,
            "broken",
            Some("not a cron".to_owned()),
            chrono_tz::UTC,
            now,
        )
        .unwrap();

        assert_eq!(deferred, None);
        assert_eq!(stored_next_run(&db, "broken"), None);
    }

    #[test]
    fn remove_job_works_before_the_worker_created_the_table() {
        let db: DbConnection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));

        CronSink::remove_job(&db, "missing").unwrap();

        db.lock()
            .unwrap()
            .execute(
                "INSERT INTO cron_jobs (event_id, timezone, created_at) VALUES ('later', 'UTC', 0)",
                [],
            )
            .expect("remove_job created the table");
    }
}

//! Periodic reconciliation for stuck workflow runs.
//!
//! When the executor crashes, the SSE connection drops, or any other
//! infrastructure failure prevents the `completed` event from reaching
//! the API, the run row stays at `Pending`/`Running` forever. Long-running
//! API processes run the sweeper on a fixed interval. Stateless deployments
//! invoke the same bounded pass through the maintenance endpoint. Each pass
//! finds runs that have been non-terminal for longer than the configured grace
//! period and marks them as `Timeout` so operators can see them in the UI.
//!
//! `Local` runs are skipped — they have no executor lifecycle and the
//! caller controls completion explicitly.

use std::time::Duration;

use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};

use crate::audit::{ExecutionAuditContext, record_execution_result};
use crate::entity::execution_run;
use crate::entity::prelude::ExecutionRun;
use crate::entity::sea_orm_active_enums::{AuditActorType, RunMode, RunStatus};

const DEFAULT_INTERVAL_SECS: u64 = 300;
const DEFAULT_GRACE_SECS: u64 = 3600;
const DEFAULT_BATCH_SIZE: u64 = 500;
// Keep the generated `WHERE id IN (...)` statement below conservative bind
// parameter limits on every supported SQL backend.
const MAX_BATCH_SIZE: u64 = 900;

/// Configuration for the run sweeper.
#[derive(Clone, Debug)]
pub struct RunSweeperConfig {
    pub interval: Duration,
    pub grace: Duration,
    pub batch_size: u64,
}

impl RunSweeperConfig {
    /// Build config from environment variables.
    /// - `RUN_SWEEPER_INTERVAL_SECS`: how often to sweep (default 300)
    /// - `RUN_SWEEPER_GRACE_SECS`: how long a run can stay Pending/Running before being marked Timeout (default 3600)
    /// - `RUN_SWEEPER_BATCH_SIZE`: maximum rows reconciled per pass (default 500, maximum 900)
    pub fn from_env() -> Self {
        let interval = std::env::var("RUN_SWEEPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        let grace = std::env::var("RUN_SWEEPER_GRACE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_GRACE_SECS);
        let batch_size =
            normalized_batch_size(std::env::var("RUN_SWEEPER_BATCH_SIZE").ok().as_deref());
        Self {
            interval: Duration::from_secs(interval),
            grace: Duration::from_secs(grace),
            batch_size,
        }
    }
}

fn normalized_batch_size(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_BATCH_SIZE)
        .min(MAX_BATCH_SIZE)
}

/// Spawn the run sweeper as a background task.
///
/// Returns `None` if `RUN_SWEEPER_DISABLED=1` is set, otherwise returns
/// the join handle of the spawned task. The task runs forever and is
/// expected to be aborted on process shutdown.
pub fn spawn_run_sweeper(
    context: ExecutionAuditContext,
    config: RunSweeperConfig,
) -> Option<JoinHandle<()>> {
    if std::env::var("RUN_SWEEPER_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Run sweeper disabled via RUN_SWEEPER_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        grace_secs = config.grace.as_secs(),
        batch_size = config.batch_size,
        "Spawning run sweeper"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match sweep_once(&context, config.grace, config.batch_size).await {
                Ok(0) => {}
                Ok(n) => tracing::warn!(swept = n, "Run sweeper marked stale runs as Timeout"),
                Err(e) => tracing::error!(error = %e, "Run sweeper iteration failed"),
            }
        }
    });

    Some(handle)
}

/// Run one bounded sweeper iteration. The oldest stale runs are reconciled
/// first. Returns the number of rows updated.
///
/// Exposed for tests and ad-hoc reconciliation; the spawned task calls
/// this on each interval tick.
pub async fn sweep_once(
    context: &ExecutionAuditContext,
    grace: Duration,
    batch_size: u64,
) -> Result<u64, sea_orm::DbErr> {
    let db = context.db.as_ref();
    // ExecutionRun timestamps use millisecond precision. Reuse that exact value
    // for the update and the query identifying rows changed by this sweep.
    let now = chrono::DateTime::from_timestamp_millis(chrono::Utc::now().timestamp_millis())
        .expect("current timestamp is representable in milliseconds")
        .fixed_offset();
    let threshold =
        now - chrono::Duration::from_std(grace).unwrap_or_else(|_| chrono::Duration::seconds(3600));
    let batch_size = batch_size.clamp(1, MAX_BATCH_SIZE);

    let stale = ExecutionRun::find()
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
        .filter(execution_run::Column::Mode.ne(RunMode::Local))
        .filter(execution_run::Column::UpdatedAt.lt(threshold))
        .order_by_asc(execution_run::Column::UpdatedAt)
        .order_by_asc(execution_run::Column::Id)
        .limit(batch_size)
        .all(db)
        .await?;

    if stale.is_empty() {
        return Ok(0);
    }

    let ids: Vec<String> = stale.iter().map(|r| r.id.clone()).collect();
    for run in stale.iter().take(20) {
        tracing::info!(
            run_id = %run.id,
            app_id = %run.app_id,
            mode = ?run.mode,
            status = ?run.status,
            "Marking stuck run as Timeout"
        );
    }

    let result = ExecutionRun::update_many()
        .set(execution_run::ActiveModel {
            status: Set(RunStatus::Timeout),
            completed_at: Set(Some(now)),
            updated_at: Set(now),
            error_message: Set(Some(
                "Run exceeded grace period without completion event".to_string(),
            )),
            ..Default::default()
        })
        .filter(execution_run::Column::Id.is_in(ids.clone()))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
        // A callback may refresh a run after the selection query. Keep the
        // age predicate in the conditional update so the sweep cannot time out
        // a run that became active in that window.
        .filter(execution_run::Column::UpdatedAt.lt(threshold))
        .exec(db)
        .await?;

    if context.enabled && result.rows_affected > 0 {
        let timed_out = ExecutionRun::find()
            .filter(execution_run::Column::Id.is_in(ids))
            .filter(execution_run::Column::Status.eq(RunStatus::Timeout))
            .filter(execution_run::Column::CompletedAt.eq(now))
            .all(db)
            .await?;
        for run in timed_out {
            record_execution_result(context, &run, "run-sweeper", AuditActorType::System)
                .await
                .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
        }
    }
    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_size_defaults_and_is_bounded() {
        assert_eq!(normalized_batch_size(None), DEFAULT_BATCH_SIZE);
        assert_eq!(normalized_batch_size(Some("")), DEFAULT_BATCH_SIZE);
        assert_eq!(normalized_batch_size(Some("0")), DEFAULT_BATCH_SIZE);
        assert_eq!(normalized_batch_size(Some("25")), 25);
        assert_eq!(normalized_batch_size(Some("50000")), MAX_BATCH_SIZE);
    }
}

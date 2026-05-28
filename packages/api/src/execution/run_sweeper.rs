//! Periodic reconciliation for stuck workflow runs.
//!
//! When the executor crashes, the SSE connection drops, or any other
//! infrastructure failure prevents the `completed` event from reaching
//! the API, the run row stays at `Pending`/`Running` forever. The
//! sweeper runs on a fixed interval, finds runs that have been
//! non-terminal for longer than the configured grace period, and marks
//! them as `Timeout` so operators can see them in the UI.
//!
//! `Local` runs are skipped — they have no executor lifecycle and the
//! caller controls completion explicitly.

use std::sync::Arc;
use std::time::Duration;

use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entity::execution_run;
use crate::entity::prelude::ExecutionRun;
use crate::entity::sea_orm_active_enums::{RunMode, RunStatus};

const DEFAULT_INTERVAL_SECS: u64 = 300;
const DEFAULT_GRACE_SECS: u64 = 3600;

/// Configuration for the run sweeper.
#[derive(Clone, Debug)]
pub struct RunSweeperConfig {
    pub interval: Duration,
    pub grace: Duration,
}

impl RunSweeperConfig {
    /// Build config from environment variables.
    /// - `RUN_SWEEPER_INTERVAL_SECS`: how often to sweep (default 300)
    /// - `RUN_SWEEPER_GRACE_SECS`: how long a run can stay Pending/Running before being marked Timeout (default 3600)
    pub fn from_env() -> Self {
        let interval = std::env::var("RUN_SWEEPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        let grace = std::env::var("RUN_SWEEPER_GRACE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_GRACE_SECS);
        Self {
            interval: Duration::from_secs(interval),
            grace: Duration::from_secs(grace),
        }
    }
}

/// Spawn the run sweeper as a background task.
///
/// Returns `None` if `RUN_SWEEPER_DISABLED=1` is set, otherwise returns
/// the join handle of the spawned task. The task runs forever and is
/// expected to be aborted on process shutdown.
pub fn spawn_run_sweeper(
    db: Arc<DatabaseConnection>,
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
        "Spawning run sweeper"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match sweep_once(db.as_ref(), config.grace).await {
                Ok(0) => {}
                Ok(n) => tracing::warn!(swept = n, "Run sweeper marked stale runs as Timeout"),
                Err(e) => tracing::error!(error = %e, "Run sweeper iteration failed"),
            }
        }
    });

    Some(handle)
}

/// Run one sweeper iteration. Returns the number of rows updated.
///
/// Exposed for tests and ad-hoc reconciliation; the spawned task calls
/// this on each interval tick.
pub async fn sweep_once(db: &DatabaseConnection, grace: Duration) -> Result<u64, sea_orm::DbErr> {
    let now = chrono::Utc::now().naive_utc();
    let threshold =
        now - chrono::Duration::from_std(grace).unwrap_or_else(|_| chrono::Duration::seconds(3600));

    let stale = ExecutionRun::find()
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
        .filter(execution_run::Column::Mode.ne(RunMode::Local))
        .filter(execution_run::Column::UpdatedAt.lt(threshold))
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
        .filter(execution_run::Column::Id.is_in(ids))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
        .exec(db)
        .await?;

    Ok(result.rows_affected)
}

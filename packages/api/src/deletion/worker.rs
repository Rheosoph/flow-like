//! In-process driver for the deletion queue.
//!
//! Long-running deployments spawn it next to the run sweeper; stateless
//! deployments call the same pass through `POST /maintenance/run` with
//! `{"job": "deletion_queue"}`.

use std::time::Duration;

use flow_like_types::tokio::{self, task::JoinHandle};

use super::drain::PassBudget;
use super::run_queue;
use crate::state::AppState;

const DEFAULT_INTERVAL_SECS: u64 = 30;

#[derive(Clone, Debug)]
pub struct DeletionWorkerConfig {
    pub interval: Duration,
    pub budget: PassBudget,
}

impl DeletionWorkerConfig {
    /// `DELETION_WORKER_INTERVAL_SECS` (default 30) plus the pass budget from
    /// [`PassBudget::from_env`].
    pub fn from_env() -> Self {
        let interval = std::env::var("DELETION_WORKER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        Self {
            interval: Duration::from_secs(interval),
            budget: PassBudget::from_env(),
        }
    }
}

/// Spawn the queue driver. Returns `None` when `DELETION_WORKER_DISABLED=1`.
pub fn spawn_deletion_worker(
    state: AppState,
    config: DeletionWorkerConfig,
) -> Option<JoinHandle<()>> {
    if std::env::var("DELETION_WORKER_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Deletion worker disabled via DELETION_WORKER_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        max_chunks = config.budget.max_chunks,
        max_secs = config.budget.max_duration.as_secs(),
        "Spawning deletion worker"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match run_queue(&state, config.budget).await {
                Ok(report) if report.claimed == 0 => {}
                Ok(report) => tracing::info!(
                    claimed = report.claimed,
                    completed = report.completed,
                    suspended = report.suspended,
                    failed = report.failed,
                    "Deletion worker pass completed"
                ),
                Err(error) => tracing::error!(error = %error, "Deletion worker pass failed"),
            }
        }
    });

    Some(handle)
}

//! Removal of channel rows whose waiter never cleaned up (crashed executor, dropped chat
//! stream). Long-lived hosts run the ticker; serverless deployments drive [`sweep_expired`]
//! through `POST /maintenance/run` (`cache_cleanup` job).

use std::sync::Arc;
use std::time::Duration;

use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::entity::{channel, prelude::Channel};

const DEFAULT_INTERVAL_SECS: u64 = 900;

#[derive(Clone, Debug)]
pub struct ChannelSweeperConfig {
    pub interval: Duration,
}

impl ChannelSweeperConfig {
    /// - `CHANNEL_SWEEPER_INTERVAL_SECS`: how often to sweep (default 900)
    pub fn from_env() -> Self {
        let interval = std::env::var("CHANNEL_SWEEPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        Self {
            interval: Duration::from_secs(interval),
        }
    }
}

/// Returns `None` when `CHANNEL_SWEEPER_DISABLED=1` is set.
pub fn spawn_channel_sweeper(
    db: Arc<DatabaseConnection>,
    config: ChannelSweeperConfig,
) -> Option<JoinHandle<()>> {
    if std::env::var("CHANNEL_SWEEPER_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Channel sweeper disabled via CHANNEL_SWEEPER_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        "Spawning channel sweeper"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match sweep_expired(db.as_ref()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "Channel sweeper removed expired rows"),
                Err(e) => tracing::error!(error = %e, "Channel sweeper iteration failed"),
            }
        }
    });

    Some(handle)
}

/// `DELETE FROM "Channel" WHERE "expiresAt" < now`. Returns the number of rows removed.
pub async fn sweep_expired(db: &DatabaseConnection) -> Result<u64, sea_orm::DbErr> {
    let now = chrono::Utc::now().timestamp();
    let result = Channel::delete_many()
        .filter(channel::Column::ExpiresAt.lt(now))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn sweep_deletes_strictly_expired_rows() {
        let sql = Channel::delete_many()
            .filter(channel::Column::ExpiresAt.lt(1_700_000_000i64))
            .build(DatabaseBackend::Postgres)
            .to_string();
        assert_eq!(
            sql,
            r#"DELETE FROM "public"."Channel" WHERE "Channel"."expiresAt" < 1700000000"#
        );
    }

    #[test]
    fn interval_env_falls_back_to_default() {
        let config = ChannelSweeperConfig::from_env();
        assert!(config.interval.as_secs() > 0);
    }
}

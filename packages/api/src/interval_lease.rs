//! One replica per interval for background passes that are idempotent but expensive.

use std::time::Duration;

use crate::{cache::Reservation, state::AppState};

const NAMESPACE: &str = "background-lease";

/// Whether this replica runs the current pass of `name`: the first to claim it holds it for
/// `lease`. Without a reachable cache every replica runs, which only duplicates the work.
pub(crate) async fn claim(state: &AppState, name: &str, lease: Duration) -> bool {
    let cache = match state.cache.platform().await {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!(%error, pass = name, "No cache for the background lease; running anyway");
            return true;
        }
    };
    match cache
        .try_insert(NAMESPACE, name, &flow_like_types::create_id(), lease)
        .await
    {
        Ok(Reservation::Acquired) => true,
        Ok(Reservation::Held(_)) => false,
        Err(error) => {
            tracing::warn!(%error, pass = name, "Background lease unavailable; running anyway");
            true
        }
    }
}

/// `env` as whole seconds; `None` when it is `0`, `default` when unset or unreadable.
pub(crate) fn interval_from_env(env: &str, default: Duration) -> Option<Duration> {
    match std::env::var(env)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        Some(0) => None,
        Some(seconds) => Some(Duration::from_secs(seconds)),
        None => Some(default),
    }
}

//! Cache-family probes: the flow key/value cache and the execution state store.
//!
//! Both are resolved lazily by the process, so probing them doubles as the only
//! reachability check the deployment has for a backend no request has touched yet.

use std::time::Instant;

use super::types::{MetricFreshness, ResourceKind, ResourceMetric, ResourceStatus};
use crate::{cache::CacheStoreStats, state::AppState};

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// Probe the flow key/value cache.
pub async fn probe(state: &AppState) -> ResourceStatus {
    let started = Instant::now();

    let store = match state.cache.store().await {
        Ok(store) => store,
        Err(error) => {
            return ResourceStatus::new("cache", ResourceKind::Cache, "Cache", "unknown")
                .latency_ms(elapsed_ms(started))
                .failed(format!("Cache backend unavailable: {error}"));
        }
    };

    let stats = store.stats().await;
    let status = ResourceStatus::new("cache", ResourceKind::Cache, "Cache", store.backend_name())
        .latency_ms(elapsed_ms(started));

    match stats {
        Ok(Some(stats)) => with_stats(status, stats),
        Ok(None) => status.unsupported("This cache backend exposes no cheap statistics"),
        Err(error) => status.failed(format!("Cache statistics unavailable: {error}")),
    }
}

/// Probe the execution state store.
///
/// Reachability and latency only: the trait deliberately exposes no statistics, because
/// counting run state costs a scan on every backend that implements it.
pub async fn probe_state_store(state: &AppState) -> ResourceStatus {
    let started = Instant::now();

    match crate::routes::execution::progress::get_state_store(state).await {
        Ok(store) => ResourceStatus::new(
            "state-store",
            ResourceKind::StateStore,
            "Execution state",
            store.backend_name(),
        )
        .latency_ms(elapsed_ms(started)),
        Err(error) => ResourceStatus::new(
            "state-store",
            ResourceKind::StateStore,
            "Execution state",
            "unknown",
        )
        .latency_ms(elapsed_ms(started))
        .failed(format!("Execution state store unavailable: {error}")),
    }
}

fn with_stats(status: ResourceStatus, stats: CacheStoreStats) -> ResourceStatus {
    let observed_at = stats.observed_at.map(|at| at.to_rfc3339());
    let mut metrics = Vec::new();

    if let Some(entries) = stats.entries {
        metrics.push(ResourceMetric::count("entries", "Entries", entries));
    }
    if let Some(size_bytes) = stats.size_bytes {
        metrics.push(ResourceMetric::bytes("size_bytes", "Size", size_bytes));
    }
    if let Some(max_size_bytes) = stats.max_size_bytes {
        metrics.push(ResourceMetric::bytes(
            "max_size_bytes",
            "Memory limit",
            max_size_bytes,
        ));
    }
    if let Some(expired_pending) = stats.expired_pending {
        let metric = ResourceMetric::count(
            "expired_pending",
            "Expired, awaiting sweep",
            expired_pending,
        );
        metrics.push(if stats.expired_pending_capped {
            metric.note("Counting stopped at the scan cap — the backlog is at least this large")
        } else {
            metric
        });
    }
    if let Some(hits) = stats.hits {
        metrics.push(ResourceMetric::count("hits", "Hits", hits));
    }
    if let Some(misses) = stats.misses {
        metrics.push(ResourceMetric::count("misses", "Misses", misses));
    }
    if let Some(evictions) = stats.evictions {
        metrics.push(ResourceMetric::count("evictions", "Evictions", evictions));
    }
    if let (Some(hits), Some(misses)) = (stats.hits, stats.misses) {
        let lookups = hits + misses;
        if lookups > 0 {
            let hit_rate = hits as f64 / lookups as f64;
            metrics.push(ResourceMetric::ratio("hit_rate", "Hit rate", hit_rate));
        }
    }

    // A backend that dates its numbers is telling us they lag its writes — DynamoDB's
    // item count is hours old. Rendering that as a live reading would send an operator
    // hunting for entries that were already evicted.
    if let Some(observed_at) = observed_at.as_deref() {
        metrics = metrics
            .into_iter()
            .map(|metric| {
                metric
                    .freshness(MetricFreshness::Estimate)
                    .observed_at(observed_at)
            })
            .collect();
    }

    let status = status.metrics(metrics);
    match stats.note {
        Some(note) => status.message(note),
        None => status,
    }
}

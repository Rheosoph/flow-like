//! Wire contract for the admin resource-status endpoint.
//!
//! Every probe answers with the same [`ResourceStatus`] shape so the dashboard can
//! render a backend it was never taught about. The typed extras a single family needs
//! — Postgres table sizes, connection states — hang off the response instead of being
//! smuggled through an untyped bag on the status.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Family a resource belongs to. The dashboard groups its cards by this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    Database,
    Cache,
    Storage,
    StateStore,
}

/// Outcome of one probe.
///
/// `Unsupported` and `NotConfigured` are deliberately distinct from `Unavailable`:
/// a backend with no cheap statistics API is healthy, and an unconfigured optional
/// backend is not a fault. Painting either red would train operators to ignore red.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResourceHealth {
    /// Probed successfully.
    Ok,
    /// Reachable, but a metric sits outside its healthy band.
    Degraded,
    /// The probe failed; `message` carries the reason.
    Unavailable,
    /// Configured and reachable, but exposes no cheap statistics.
    Unsupported,
    /// Not configured on this deployment.
    NotConfigured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MetricUnit {
    Bytes,
    Count,
    Milliseconds,
    Seconds,
    /// Between 0 and 1; rendered as a percentage.
    Ratio,
    PerSecond,
}

/// How current a value is.
///
/// A cloud provider's daily storage rollup and a live `SELECT` must not render
/// identically — an operator reading a 24-hour-old bucket size as "now" will chase
/// a deletion that already happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MetricFreshness {
    /// Read from the resource during this request.
    Live,
    /// A backend-maintained statistic that lags writes (DynamoDB item counts,
    /// Postgres `n_live_tup`).
    Estimate,
    /// Produced by a provider metrics pipeline; `observed_at` carries its timestamp.
    Provider,
    /// Derived from the delta against a previous sample.
    Rate,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetric {
    /// Stable identifier, e.g. `size_bytes`. The frontend keys off this, never the label.
    pub key: String,
    pub label: String,
    pub value: f64,
    pub unit: MetricUnit,
    pub freshness: MetricFreshness,
    /// When the value was measured, for anything that is not `Live`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    /// Caveat shown next to the value, e.g. that a count is capped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ResourceMetric {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        unit: MetricUnit,
        freshness: MetricFreshness,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value,
            unit,
            freshness,
            observed_at: None,
            note: None,
        }
    }

    pub fn bytes(key: impl Into<String>, label: impl Into<String>, value: i64) -> Self {
        Self::new(
            key,
            label,
            value as f64,
            MetricUnit::Bytes,
            MetricFreshness::Live,
        )
    }

    pub fn count(key: impl Into<String>, label: impl Into<String>, value: i64) -> Self {
        Self::new(
            key,
            label,
            value as f64,
            MetricUnit::Count,
            MetricFreshness::Live,
        )
    }

    pub fn ratio(key: impl Into<String>, label: impl Into<String>, value: f64) -> Self {
        Self::new(key, label, value, MetricUnit::Ratio, MetricFreshness::Live)
    }

    pub fn millis(key: impl Into<String>, label: impl Into<String>, value: f64) -> Self {
        Self::new(
            key,
            label,
            value,
            MetricUnit::Milliseconds,
            MetricFreshness::Live,
        )
    }

    pub fn per_second(key: impl Into<String>, label: impl Into<String>, value: f64) -> Self {
        Self::new(
            key,
            label,
            value,
            MetricUnit::PerSecond,
            MetricFreshness::Rate,
        )
    }

    pub fn freshness(mut self, freshness: MetricFreshness) -> Self {
        self.freshness = freshness;
        self
    }

    pub fn observed_at(mut self, observed_at: impl Into<String>) -> Self {
        self.observed_at = Some(observed_at.into());
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// One probed resource.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    /// Stable id: `database`, `cache`, `state-store`, `storage:meta`,
    /// `storage:content`, `storage:cdn`.
    pub id: String,
    pub kind: ResourceKind,
    pub label: String,
    /// Implementation actually in use: `postgres`, `redis`, `dynamodb`, `s3`, …
    pub backend: String,
    /// Which instance this is — bucket name, region, account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub status: ResourceHealth,
    /// Why the status is not `Ok`, or a caveat about the numbers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Round trip of the probe itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub metrics: Vec<ResourceMetric>,
}

impl ResourceStatus {
    pub fn new(
        id: impl Into<String>,
        kind: ResourceKind,
        label: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            label: label.into(),
            backend: backend.into(),
            detail: None,
            status: ResourceHealth::Ok,
            message: None,
            latency_ms: None,
            metrics: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn detail_opt(mut self, detail: Option<String>) -> Self {
        self.detail = detail.filter(|value| !value.is_empty());
        self
    }

    pub fn health(mut self, status: ResourceHealth) -> Self {
        self.status = status;
        self
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn latency_ms(mut self, latency_ms: u64) -> Self {
        self.latency_ms = Some(latency_ms);
        self
    }

    pub fn metrics(mut self, metrics: impl IntoIterator<Item = ResourceMetric>) -> Self {
        self.metrics.extend(metrics);
        self
    }

    /// Mark the probe failed, keeping whatever identity was already filled in.
    pub fn failed(self, message: impl Into<String>) -> Self {
        self.health(ResourceHealth::Unavailable).message(message)
    }

    /// Mark the backend healthy but statistics-free.
    pub fn unsupported(self, message: impl Into<String>) -> Self {
        self.health(ResourceHealth::Unsupported).message(message)
    }
}

/// One relation's on-disk footprint.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableUsage {
    pub name: String,
    /// Heap plus indexes plus TOAST.
    pub total_bytes: i64,
    pub table_bytes: i64,
    pub index_bytes: i64,
    /// Planner estimate, not a `COUNT(*)`.
    pub estimated_rows: i64,
    /// Rows awaiting vacuum; a high share against `estimated_rows` means bloat.
    pub dead_rows: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStateCount {
    /// `active`, `idle`, `idle in transaction`, or `unknown`.
    pub state: String,
    pub count: i64,
}

/// Cumulative `pg_stat_database` counters since the last statistics reset.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCounters {
    pub commits: i64,
    pub rollbacks: i64,
    pub tuples_returned: i64,
    pub tuples_fetched: i64,
    pub tuples_inserted: i64,
    pub tuples_updated: i64,
    pub tuples_deleted: i64,
    pub blocks_hit: i64,
    pub blocks_read: i64,
    pub deadlocks: i64,
    pub temp_files: i64,
    pub temp_bytes: i64,
}

/// Per-second rates derived from the delta between two counter samples.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRates {
    /// Seconds between the two samples the rates were derived from.
    pub window_seconds: f64,
    pub commits: f64,
    pub rollbacks: f64,
    pub tuples_read: f64,
    pub tuples_written: f64,
    pub blocks_read: f64,
}

/// Asynchronous schema jobs on engines that build indexes out of band (Aurora DSQL
/// `sys.jobs`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseJobs {
    /// Submitted or still processing.
    pub pending: i64,
    pub failed: i64,
    pub completed: i64,
}

/// An index the planner ignores because its build failed or has not finished.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvalidIndex {
    pub table: String,
    pub name: String,
}

/// Relational extras, rendered on the detail page rather than the dashboard card.
///
/// Engines without the `pg_stat_*` catalog leave `connections`, `counters` and `rates`
/// empty and name the missing sections in `unsupported`, so a bare section reads as a
/// known limitation rather than a failed query. `largest_tables` then carries row
/// estimates with zero byte sizes.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_name: Option<String>,
    pub largest_tables: Vec<TableUsage>,
    pub connections: Vec<ConnectionStateCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counters: Option<DatabaseCounters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rates: Option<DatabaseRates>,
    /// When the counters were last zeroed; rates before a second sample are absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_reset_at: Option<String>,
    /// Statistics this engine cannot provide at all, e.g. `size on disk`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<DatabaseJobs>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalid_indexes: Vec<InvalidIndex>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminResourcesResponse {
    /// RFC 3339 timestamp of the probe run behind this payload.
    pub generated_at: String,
    /// True when served from the short-lived response cache rather than freshly probed.
    pub cached: bool,
    pub resources: Vec<ResourceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_detail: Option<DatabaseDetail>,
}

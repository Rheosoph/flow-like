//! [`crate::TickStore`] implementations, one per cloud, each behind its feature
//! so the Azure image never links the GCP client and vice versa.
//!
//! Both keep the same document shape:
//! `{ id: <event_id>, app_id, cron_expression, last_fired_at, updated_at }`.
//! `id`/`app_id` address the document (Cosmos partition key `/app_id`),
//! `last_fired_at` is the watermark the tick advances, `updated_at` is the wall
//! clock of the write for operators reading the container by hand.

#[cfg(feature = "azure")]
pub mod cosmos;
#[cfg(feature = "gcp")]
pub mod firestore;

pub const DEFAULT_SCHEDULER_CONTAINER: &str = "scheduler";

#[cfg(any(feature = "azure", feature = "gcp"))]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ScheduleDocument {
    pub id: String,
    pub app_id: String,
    pub cron_expression: String,
    pub last_fired_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(any(feature = "azure", feature = "gcp"))]
impl ScheduleDocument {
    pub(crate) fn new(
        event_id: &str,
        app_id: &str,
        cron_expression: &str,
        last_fired_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            id: event_id.to_string(),
            app_id: app_id.to_string(),
            cron_expression: cron_expression.to_string(),
            last_fired_at,
            updated_at: chrono::Utc::now(),
        }
    }
}

#[cfg(any(feature = "azure", feature = "gcp"))]
pub(crate) fn container_from_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SCHEDULER_CONTAINER.to_string())
}

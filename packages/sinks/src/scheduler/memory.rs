//! In-memory scheduler using tokio-cron-scheduler
//!
//! This is used for Docker Compose and local development where we don't have
//! access to cloud-native scheduling services.

use super::{ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult};
use crate::CronSinkConfig;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

fn parse_scheduled_local(
    scheduled: &crate::ScheduledLocal,
    timezone: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    let date_parts: Vec<u32> = scheduled
        .date
        .split('-')
        .filter_map(|s| s.parse().ok())
        .collect();
    let time_parts: Vec<u32> = scheduled
        .time
        .split(':')
        .filter_map(|s| s.parse().ok())
        .collect();
    if date_parts.len() < 3 || time_parts.len() < 2 {
        return None;
    }
    let tz: chrono_tz::Tz = timezone.parse().ok()?;
    tz.with_ymd_and_hms(
        date_parts[0] as i32,
        date_parts[1],
        date_parts[2],
        time_parts[0],
        time_parts[1],
        0,
    )
    .single()
    .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Normalize a Unix-style cron expression (5 or 6 fields) into the 7-field
/// form the `cron` crate requires (`sec min hour dom mon dow year`).
///
/// - 5-field (`min hour dom mon dow`)   -> prepend `0 ` for seconds, append ` *` for year.
/// - 6-field (`sec min hour dom mon dow`) -> append ` *` for year.
/// - 7-field is passed through unchanged.
fn normalize_cron_for_crate(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    match parts.len() {
        5 => format!("0 {} *", parts.join(" ")),
        6 => format!("{} *", parts.join(" ")),
        _ => expr.to_string(),
    }
}

/// In-memory scheduler state for a single schedule
#[derive(Debug, Clone)]
struct ScheduleState {
    event_id: String,
    cron_expression: String,
    active: bool,
    config: CronSinkConfig,
    last_triggered: Option<chrono::DateTime<chrono::Utc>>,
}

/// In-memory scheduler implementation
///
/// This scheduler stores schedule definitions and is typically used with
/// a polling mechanism to sync with the database and trigger events.
/// The actual cron execution is handled by a separate service that calls
/// `trigger_due_schedules()`.
pub struct InMemoryScheduler {
    schedules: Arc<RwLock<HashMap<String, ScheduleState>>>,
}

impl InMemoryScheduler {
    /// Create a new in-memory scheduler
    pub fn new() -> Self {
        Self {
            schedules: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get all schedules that are due to be triggered
    ///
    /// This is called by the cron service to determine which events need triggering.
    pub fn get_due_schedules(&self) -> Vec<String> {
        let schedules = self.schedules.read();
        let now = chrono::Utc::now();

        schedules
            .values()
            .filter(|s| s.active)
            .filter(|s| {
                // One-time schedule: check if the target time has arrived
                if let Some(ref scheduled) = s.config.scheduled_for {
                    if s.last_triggered.is_some() {
                        return false; // already fired
                    }
                    if let Some(target) = parse_scheduled_local(scheduled, &s.config.timezone) {
                        let diff = target.signed_duration_since(now);
                        return diff.num_seconds() <= 0 && diff.num_seconds().abs() < 120;
                    }
                    return false;
                }

                // Recurring: parse cron and check if it should run now. The
                // `cron` crate requires a 7-field expression and always
                // evaluates `upcoming()` in the caller-supplied timezone —
                // honor `config.timezone` instead of silently treating every
                // schedule as UTC.
                let normalized = normalize_cron_for_crate(&s.cron_expression);
                let tz: chrono_tz::Tz = s.config.timezone.parse().unwrap_or(chrono_tz::UTC);
                if let Ok(schedule) = cron::Schedule::from_str(&normalized)
                    && let Some(next) = schedule.upcoming(tz).next()
                {
                    let diff = next.with_timezone(&chrono::Utc).signed_duration_since(now);
                    return diff.num_seconds().abs() < 60;
                }
                false
            })
            .map(|s| s.event_id.clone())
            .collect()
    }

    /// Mark a schedule as triggered
    pub fn mark_triggered(&self, event_id: &str) {
        let mut schedules = self.schedules.write();
        if let Some(schedule) = schedules.get_mut(event_id) {
            schedule.last_triggered = Some(chrono::Utc::now());
        }
    }

    /// Sync schedules from an external source (e.g., database)
    pub fn sync_schedules(&self, external_schedules: Vec<(String, String, bool, CronSinkConfig)>) {
        let mut schedules = self.schedules.write();

        // Build set of external event IDs
        let external_ids: std::collections::HashSet<_> = external_schedules
            .iter()
            .map(|(id, _, _, _)| id.clone())
            .collect();

        // Remove schedules that no longer exist
        schedules.retain(|id, _| external_ids.contains(id));

        // Add/update schedules
        for (event_id, cron_expr, active, config) in external_schedules {
            schedules
                .entry(event_id.clone())
                .and_modify(|s| {
                    s.cron_expression = cron_expr.clone();
                    s.active = active;
                    s.config = config.clone();
                })
                .or_insert(ScheduleState {
                    event_id,
                    cron_expression: cron_expr,
                    active,
                    config,
                    last_triggered: None,
                });
        }
    }
}

impl Default for InMemoryScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// Helper to parse cron expressions
use std::str::FromStr;

/// Compute the next firing time for a schedule, honoring the schedule's
/// configured timezone (recurring) or the `ScheduledLocal`'s timezone
/// (one-time).
fn next_trigger_for(s: &ScheduleState) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(ref scheduled) = s.config.scheduled_for {
        return parse_scheduled_local(scheduled, &s.config.timezone);
    }

    let normalized = normalize_cron_for_crate(&s.cron_expression);
    let tz: chrono_tz::Tz = s.config.timezone.parse().unwrap_or(chrono_tz::UTC);
    cron::Schedule::from_str(&normalized)
        .ok()
        .and_then(|schedule| schedule.upcoming(tz).next())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[async_trait::async_trait]
impl SchedulerBackend for InMemoryScheduler {
    async fn validate_schedule(
        &self,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        if !config.is_one_time() {
            cron::Schedule::from_str(&normalize_cron_for_crate(cron_expr))
                .map_err(|e| SchedulerError::InvalidCronExpression(e.to_string()))?;
        }

        Ok(())
    }

    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        // Validate cron expression (skip for one-time schedules). Normalize
        // to the 7-field form the `cron` crate expects; UI emits 5- or 6-field.
        if !config.is_one_time() {
            cron::Schedule::from_str(&normalize_cron_for_crate(cron_expr))
                .map_err(|e| SchedulerError::InvalidCronExpression(e.to_string()))?;
        }

        let mut schedules = self.schedules.write();

        if schedules.contains_key(event_id) {
            return Err(SchedulerError::AlreadyExists(event_id.to_string()));
        }

        schedules.insert(
            event_id.to_string(),
            ScheduleState {
                event_id: event_id.to_string(),
                cron_expression: cron_expr.to_string(),
                active: config.active,
                config: config.clone(),
                last_triggered: None,
            },
        );

        Ok(())
    }

    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        // Validate cron expression (skip for one-time schedules). Same 7-field
        // normalization as `create_schedule`.
        if !config.is_one_time() {
            cron::Schedule::from_str(&normalize_cron_for_crate(cron_expr))
                .map_err(|e| SchedulerError::InvalidCronExpression(e.to_string()))?;
        }

        let mut schedules = self.schedules.write();

        let schedule = schedules
            .get_mut(event_id)
            .ok_or_else(|| SchedulerError::NotFound(event_id.to_string()))?;

        schedule.cron_expression = cron_expr.to_string();
        schedule.active = config.active;
        schedule.config = config.clone();

        Ok(())
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let mut schedules = self.schedules.write();
        schedules.remove(event_id);
        Ok(())
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let mut schedules = self.schedules.write();
        let schedule = schedules
            .get_mut(event_id)
            .ok_or_else(|| SchedulerError::NotFound(event_id.to_string()))?;
        schedule.active = true;
        Ok(())
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let mut schedules = self.schedules.write();
        let schedule = schedules
            .get_mut(event_id)
            .ok_or_else(|| SchedulerError::NotFound(event_id.to_string()))?;
        schedule.active = false;
        Ok(())
    }

    async fn schedule_exists(&self, event_id: &str) -> SchedulerResult<bool> {
        let schedules = self.schedules.read();
        Ok(schedules.contains_key(event_id))
    }

    async fn get_schedule(&self, event_id: &str) -> SchedulerResult<Option<ScheduleInfo>> {
        let schedules = self.schedules.read();
        Ok(schedules.get(event_id).map(|s| ScheduleInfo {
            event_id: s.event_id.clone(),
            cron_expression: s.cron_expression.clone(),
            active: s.active,
            last_triggered: s.last_triggered,
            next_trigger: next_trigger_for(s),
        }))
    }

    async fn list_schedules(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
        let schedules = self.schedules.read();
        let offset = offset.unwrap_or(0);
        let limit = limit.unwrap_or(usize::MAX);

        Ok(schedules
            .values()
            .skip(offset)
            .take(limit)
            .map(|s| ScheduleInfo {
                event_id: s.event_id.clone(),
                cron_expression: s.cron_expression.clone(),
                active: s.active,
                last_triggered: s.last_triggered,
                next_trigger: next_trigger_for(s),
            })
            .collect())
    }
}

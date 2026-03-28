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
    let date_parts: Vec<u32> = scheduled.date.split('-').filter_map(|s| s.parse().ok()).collect();
    let time_parts: Vec<u32> = scheduled.time.split(':').filter_map(|s| s.parse().ok()).collect();
    if date_parts.len() < 3 || time_parts.len() < 2 {
        return None;
    }
    let tz: chrono_tz::Tz = timezone.parse().ok()?;
    tz.with_ymd_and_hms(
        date_parts[0] as i32, date_parts[1], date_parts[2],
        time_parts[0], time_parts[1], 0,
    )
    .single()
    .map(|dt| dt.with_timezone(&chrono::Utc))
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

                // Recurring: parse cron and check if it should run now
                if let Ok(schedule) = cron::Schedule::from_str(&s.cron_expression)
                    && let Some(next) = schedule.upcoming(chrono::Utc).next()
                {
                    let diff = next.signed_duration_since(now);
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

#[async_trait::async_trait]
impl SchedulerBackend for InMemoryScheduler {
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        // Validate cron expression (skip for one-time schedules)
        if !config.is_one_time() {
            cron::Schedule::from_str(cron_expr)
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
                active: true,
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
        // Validate cron expression (skip for one-time schedules)
        if !config.is_one_time() {
            cron::Schedule::from_str(cron_expr)
                .map_err(|e| SchedulerError::InvalidCronExpression(e.to_string()))?;
        }

        let mut schedules = self.schedules.write();

        let schedule = schedules
            .get_mut(event_id)
            .ok_or_else(|| SchedulerError::NotFound(event_id.to_string()))?;

        schedule.cron_expression = cron_expr.to_string();
        schedule.config = config.clone();

        Ok(())
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let mut schedules = self.schedules.write();
        schedules
            .remove(event_id)
            .ok_or_else(|| SchedulerError::NotFound(event_id.to_string()))?;
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
        Ok(schedules.get(event_id).map(|s| {
            let next_trigger = if let Some(ref scheduled) = s.config.scheduled_for {
                parse_scheduled_local(scheduled, &s.config.timezone)
            } else {
                cron::Schedule::from_str(&s.cron_expression)
                    .ok()
                    .and_then(|schedule| schedule.upcoming(chrono::Utc).next())
            };

            ScheduleInfo {
                event_id: s.event_id.clone(),
                cron_expression: s.cron_expression.clone(),
                active: s.active,
                last_triggered: s.last_triggered,
                next_trigger,
            }
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
            .map(|s| {
                let next_trigger = if let Some(ref scheduled) = s.config.scheduled_for {
                    parse_scheduled_local(scheduled, &s.config.timezone)
                } else {
                    cron::Schedule::from_str(&s.cron_expression)
                        .ok()
                        .and_then(|schedule| schedule.upcoming(chrono::Utc).next())
                };

                ScheduleInfo {
                    event_id: s.event_id.clone(),
                    cron_expression: s.cron_expression.clone(),
                    active: s.active,
                    last_triggered: s.last_triggered,
                    next_trigger,
                }
            })
            .collect())
    }
}

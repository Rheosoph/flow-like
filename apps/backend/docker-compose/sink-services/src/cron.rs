use crate::api_client::{ApiClient, CronScheduleInfo};
use crate::storage::{CronScheduleState, RedisStorage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub struct CronScheduler {
    api_client: Arc<ApiClient>,
    storage: Option<Arc<RedisStorage>>,
    scheduler: JobScheduler,
    active_jobs: Arc<RwLock<HashMap<String, ActiveJob>>>,
}

struct ActiveJob {
    job_id: Uuid,
    event_id: String,
    cron_expression: String,
}

impl CronScheduler {
    pub async fn new(
        api_client: Arc<ApiClient>,
        storage: Option<Arc<RedisStorage>>,
    ) -> Result<Self, CronError> {
        let scheduler = JobScheduler::new()
            .await
            .map_err(|e| CronError::Scheduler(e.to_string()))?;

        Ok(Self {
            api_client,
            storage,
            scheduler,
            active_jobs: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn start(&self) -> Result<(), CronError> {
        self.scheduler
            .start()
            .await
            .map_err(|e| CronError::Scheduler(e.to_string()))?;

        info!("Cron scheduler started");
        Ok(())
    }

    pub async fn run_sync_loop(&self) {
        let poll_interval = std::time::Duration::from_secs(30);

        loop {
            match self.sync_schedules().await {
                Ok((added, removed)) => {
                    if added > 0 || removed > 0 {
                        info!(added, removed, "Synced cron schedules");
                    }
                }
                Err(e) => {
                    error!("Failed to sync cron schedules: {}", e);
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    async fn sync_schedules(&self) -> Result<(usize, usize), CronError> {
        let schedules = self
            .api_client
            .get_cron_schedules()
            .await
            .map_err(|e| CronError::Api(e.to_string()))?;

        let enabled_schedules: HashMap<String, CronScheduleInfo> = schedules
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| (s.id.clone(), s))
            .collect();

        // Sync to Redis storage if available
        if let Some(ref storage) = self.storage {
            let states: Vec<CronScheduleState> = enabled_schedules
                .values()
                .map(|s| CronScheduleState {
                    event_id: s.event_id.clone(),
                    cron_expression: s.cron_expression.clone(),
                    enabled: s.enabled,
                    last_triggered: s.last_triggered.map(|dt| dt.timestamp()),
                    next_trigger: s.next_trigger.map(|dt| dt.timestamp()),
                })
                .collect();

            if let Err(e) = storage.sync_cron_schedules(states).await {
                warn!("Failed to sync cron schedules to Redis: {}", e);
            }
        }

        self.reconcile_jobs(enabled_schedules).await
    }

    async fn reconcile_jobs(
        &self,
        enabled_schedules: HashMap<String, CronScheduleInfo>,
    ) -> Result<(usize, usize), CronError> {
        let mut added = 0;
        let mut removed = 0;

        let mut active_jobs = self.active_jobs.write().await;

        let current_ids: Vec<String> = active_jobs.keys().cloned().collect();
        for id in current_ids {
            let active = &active_jobs[&id];
            let unchanged = enabled_schedules.get(&id).is_some_and(|schedule| {
                active.event_id == schedule.event_id
                    && active.cron_expression == schedule.cron_expression
            });
            if !unchanged {
                match self.scheduler.remove(&active.job_id).await {
                    Ok(()) => {
                        active_jobs.remove(&id);
                        debug!("Removed obsolete cron job: {}", id);
                        removed += 1;
                    }
                    Err(e) => {
                        // Retain ownership until removal succeeds, so the next
                        // sync cannot create a duplicate trigger.
                        warn!("Failed to remove job {}: {}", id, e);
                    }
                }
            }
        }

        for (id, schedule) in enabled_schedules {
            if !active_jobs.contains_key(&id) {
                match self.add_job(&schedule).await {
                    Ok(job_id) => {
                        active_jobs.insert(
                            id.clone(),
                            ActiveJob {
                                job_id,
                                event_id: schedule.event_id.clone(),
                                cron_expression: schedule.cron_expression.clone(),
                            },
                        );
                        debug!("Added cron job: {} ({})", id, schedule.cron_expression);
                        added += 1;
                    }
                    Err(e) => {
                        error!("Failed to add cron job {}: {}", id, e);
                    }
                }
            }
        }

        Ok((added, removed))
    }

    async fn add_job(&self, schedule: &CronScheduleInfo) -> Result<Uuid, CronError> {
        let event_id = schedule.event_id.clone();
        let api_client = Arc::clone(&self.api_client);
        let storage = self.storage.clone();
        let schedule_id = schedule.id.clone();

        let job = Job::new_async(schedule.cron_expression.as_str(), move |_uuid, _lock| {
            let event_id = event_id.clone();
            let api_client = Arc::clone(&api_client);
            let storage = storage.clone();
            let schedule_id = schedule_id.clone();

            Box::pin(async move {
                info!(
                    "Triggering cron event: {} (schedule: {})",
                    event_id, schedule_id
                );

                match api_client.trigger_event(&event_id, "cron", None).await {
                    Ok(()) => {
                        info!("Successfully triggered cron event: {}", event_id);

                        // Update last_triggered in Redis
                        if let Some(ref storage) = storage {
                            let now = chrono::Utc::now().timestamp();
                            if let Err(e) = storage.update_cron_last_triggered(&event_id, now).await
                            {
                                warn!("Failed to update last_triggered in Redis: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to trigger cron event {}: {}", event_id, e);
                    }
                }
            })
        })
        .map_err(|e| CronError::Job(e.to_string()))?;

        let job_id = self
            .scheduler
            .add(job)
            .await
            .map_err(|e| CronError::Scheduler(e.to_string()))?;

        Ok(job_id)
    }
}

#[derive(Debug)]
pub enum CronError {
    Scheduler(String),
    Api(String),
    Job(String),
}

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CronError::Scheduler(e) => write!(f, "Scheduler error: {}", e),
            CronError::Api(e) => write!(f, "API error: {}", e),
            CronError::Job(e) => write!(f, "Job error: {}", e),
        }
    }
}

impl std::error::Error for CronError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn changed_cron_and_event_replace_the_scheduled_job() {
        let scheduler = CronScheduler::new(
            Arc::new(ApiClient::new("http://unused.invalid", "unused")),
            None,
        )
        .await
        .unwrap();
        let mut schedule = CronScheduleInfo {
            id: "schedule-a".into(),
            event_id: "event-a".into(),
            cron_expression: "0 0 * * * *".into(),
            enabled: true,
            last_triggered: None,
            next_trigger: None,
        };
        let schedules =
            |value: &CronScheduleInfo| HashMap::from([(value.id.clone(), value.clone())]);
        assert_eq!(
            scheduler
                .reconcile_jobs(schedules(&schedule))
                .await
                .unwrap(),
            (1, 0)
        );
        let original = scheduler.active_jobs.read().await["schedule-a"].job_id;
        assert_eq!(
            scheduler
                .reconcile_jobs(schedules(&schedule))
                .await
                .unwrap(),
            (0, 0)
        );
        schedule.cron_expression = "0 15 * * * *".into();
        assert_eq!(
            scheduler
                .reconcile_jobs(schedules(&schedule))
                .await
                .unwrap(),
            (1, 1)
        );
        let changed = scheduler.active_jobs.read().await["schedule-a"].job_id;
        assert_ne!(original, changed);
        schedule.event_id = "event-b".into();
        assert_eq!(
            scheduler
                .reconcile_jobs(schedules(&schedule))
                .await
                .unwrap(),
            (1, 1)
        );
        assert_ne!(
            changed,
            scheduler.active_jobs.read().await["schedule-a"].job_id
        );
        assert_eq!(
            scheduler.reconcile_jobs(HashMap::new()).await.unwrap(),
            (0, 1)
        );
        assert!(scheduler.active_jobs.read().await.is_empty());
    }
}

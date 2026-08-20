use anyhow::Result;
use flow_like::app::App;
use flow_like::flow::event::{Event, EventExecutionMode};
use flow_like::flow::execution::{LogLevel, LogMeta};
use flow_like_types::Value;
use flow_like_types::create_id;
use flow_like_types::tokio;
use flow_like_types::tokio_util::sync::CancellationToken;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant, SystemTime};
use tauri::AppHandle;

use super::{EventRegistration, EventSink, manager::DbConnection};
use crate::functions::flow::run::execute_daemon_event;
use crate::state::TauriFlowLikeState;

const DEFAULT_MIN_RESTART_DELAY_MS: u64 = 1_000;
const DEFAULT_MAX_RESTART_DELAY_MS: u64 = 30_000;
const DEFAULT_BOARD_POLL_INTERVAL_MS: u64 = 3_000;
const DEFAULT_LOG_FLUSH_INTERVAL_MS: u64 = 5_000;
const DEFAULT_LOG_BATCH_SIZE: usize = 500;
const DEFAULT_HEALTHY_RESET_MS: u64 = 60_000;
const MIN_BOARD_POLL_INTERVAL_MS: u64 = 500;
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

static DAEMON_TASKS: Lazy<tokio::sync::Mutex<HashMap<String, DaemonHandle>>> =
    Lazy::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DaemonRestartPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

fn default_restart_policy() -> DaemonRestartPolicy {
    DaemonRestartPolicy::OnFailure
}

fn default_min_restart_delay_ms() -> u64 {
    DEFAULT_MIN_RESTART_DELAY_MS
}

fn default_max_restart_delay_ms() -> u64 {
    DEFAULT_MAX_RESTART_DELAY_MS
}

fn default_board_poll_interval_ms() -> u64 {
    DEFAULT_BOARD_POLL_INTERVAL_MS
}

fn default_log_flush_interval_ms() -> u64 {
    DEFAULT_LOG_FLUSH_INTERVAL_MS
}

fn default_log_batch_size() -> usize {
    DEFAULT_LOG_BATCH_SIZE
}

fn default_healthy_reset_ms() -> u64 {
    DEFAULT_HEALTHY_RESET_MS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSink {
    #[serde(default = "default_restart_policy")]
    pub restart_policy: DaemonRestartPolicy,
    #[serde(default = "default_min_restart_delay_ms")]
    pub min_restart_delay_ms: u64,
    #[serde(default = "default_max_restart_delay_ms")]
    pub max_restart_delay_ms: u64,
    #[serde(default = "default_board_poll_interval_ms")]
    pub board_poll_interval_ms: u64,
    #[serde(default = "default_log_flush_interval_ms")]
    pub log_flush_interval_ms: u64,
    #[serde(default = "default_log_batch_size")]
    pub log_batch_size: usize,
    #[serde(default = "default_healthy_reset_ms")]
    pub healthy_reset_ms: u64,
    #[serde(default)]
    pub payload: Option<Value>,
}

struct DaemonHandle {
    cancellation: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BoardMarker {
    version: (u32, u32, u32),
    updated_secs: u64,
    updated_nanos: u32,
    hash: Option<u64>,
    nodes: usize,
    layers: usize,
    variables: usize,
    comments: usize,
}

impl DaemonSink {
    async fn start_or_replace(
        &self,
        app_handle: &AppHandle,
        registration: EventRegistration,
    ) -> Result<()> {
        let event_id = registration.event_id.clone();
        let app_handle = app_handle.clone();
        let config = self.normalized();

        Self::start_tracked_task(event_id, move |task_cancellation| async move {
            run_daemon_loop(app_handle, registration, config, task_cancellation).await;
        })
        .await;

        Ok(())
    }

    async fn start_tracked_task<F, Fut>(event_id: String, make_future: F) -> CancellationToken
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::stop_event(&event_id).await;

        let generation = create_id();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let returned_cancellation = cancellation.clone();
        let task_event_id = event_id.clone();
        let task_generation = generation.clone();

        let task = tokio::spawn(async move {
            make_future(task_cancellation).await;
            let mut tasks = DAEMON_TASKS.lock().await;
            if tasks
                .get(&task_event_id)
                .is_some_and(|handle| handle.generation == task_generation)
            {
                tasks.remove(&task_event_id);
            }
        });

        DAEMON_TASKS.lock().await.insert(
            event_id,
            DaemonHandle {
                cancellation,
                task,
                generation,
            },
        );

        returned_cancellation
    }

    pub async fn stop_event(event_id: &str) {
        let handle = DAEMON_TASKS.lock().await.remove(event_id);
        let Some(handle) = handle else {
            return;
        };

        handle.cancellation.cancel();
        let mut task = handle.task;

        tokio::select! {
            result = &mut task => {
                if let Err(err) = result {
                    tracing::debug!(event_id, error = %err, "Daemon task finished after cancellation");
                }
            }
            _ = tokio::time::sleep(STOP_TIMEOUT) => {
                tracing::warn!(event_id, "Daemon task did not stop in time; aborting");
                task.abort();
            }
        }
    }

    fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        if normalized.min_restart_delay_ms == 0 {
            normalized.min_restart_delay_ms = DEFAULT_MIN_RESTART_DELAY_MS;
        }
        if normalized.max_restart_delay_ms < normalized.min_restart_delay_ms {
            normalized.max_restart_delay_ms = normalized.min_restart_delay_ms;
        }
        if normalized.board_poll_interval_ms < MIN_BOARD_POLL_INTERVAL_MS {
            normalized.board_poll_interval_ms = MIN_BOARD_POLL_INTERVAL_MS;
        }
        if normalized.log_flush_interval_ms == 0 {
            normalized.log_flush_interval_ms = DEFAULT_LOG_FLUSH_INTERVAL_MS;
        }
        if normalized.log_batch_size == 0 {
            normalized.log_batch_size = DEFAULT_LOG_BATCH_SIZE;
        }
        if normalized.healthy_reset_ms == 0 {
            normalized.healthy_reset_ms = DEFAULT_HEALTHY_RESET_MS;
        }
        normalized
    }

    fn should_restart(&self, failed: bool) -> bool {
        match self.restart_policy {
            DaemonRestartPolicy::Never => false,
            DaemonRestartPolicy::OnFailure => failed,
            DaemonRestartPolicy::Always => true,
        }
    }
}

impl Default for DaemonSink {
    fn default() -> Self {
        Self {
            restart_policy: DaemonRestartPolicy::default(),
            min_restart_delay_ms: DEFAULT_MIN_RESTART_DELAY_MS,
            max_restart_delay_ms: DEFAULT_MAX_RESTART_DELAY_MS,
            board_poll_interval_ms: DEFAULT_BOARD_POLL_INTERVAL_MS,
            log_flush_interval_ms: DEFAULT_LOG_FLUSH_INTERVAL_MS,
            log_batch_size: DEFAULT_LOG_BATCH_SIZE,
            healthy_reset_ms: DEFAULT_HEALTHY_RESET_MS,
            payload: None,
        }
    }
}

#[async_trait::async_trait]
impl EventSink for DaemonSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        self.start_or_replace(app_handle, registration.clone())
            .await
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        Self::stop_event(&registration.event_id).await;
        Ok(())
    }
}

async fn run_daemon_loop(
    app_handle: AppHandle,
    registration: EventRegistration,
    config: DaemonSink,
    supervisor_token: CancellationToken,
) {
    tracing::info!(
        event_id = %registration.event_id,
        app_id = %registration.app_id,
        "Daemon event supervisor started"
    );

    let min_delay = Duration::from_millis(config.min_restart_delay_ms);
    let max_delay = Duration::from_millis(config.max_restart_delay_ms);
    let healthy_reset = Duration::from_millis(config.healthy_reset_ms);
    let mut next_delay = min_delay;

    loop {
        if supervisor_token.is_cancelled() {
            break;
        }

        let event = match load_current_event(&app_handle, &registration).await {
            Ok(event) if event.active => event,
            Ok(_) => {
                tracing::info!(
                    event_id = %registration.event_id,
                    "Daemon event is inactive; stopping supervisor"
                );
                break;
            }
            Err(err) => {
                tracing::warn!(
                    event_id = %registration.event_id,
                    error = %err,
                    "Daemon event could not be loaded; stopping supervisor"
                );
                break;
            }
        };

        if event.event_type != registration.r#type || event.event_type != "daemon" {
            tracing::info!(
                event_id = %registration.event_id,
                current_type = %event.event_type,
                "Daemon event type changed; stopping supervisor"
            );
            break;
        }

        if event.execution_mode != EventExecutionMode::Local {
            tracing::info!(
                event_id = %registration.event_id,
                mode = ?event.execution_mode,
                "Daemon event is not local; stopping supervisor"
            );
            break;
        }

        let initial_marker = if event.board_version.is_none() {
            match load_board_marker(&app_handle, &registration.app_id, &event.board_id).await {
                Ok(marker) => Some(marker),
                Err(err) => {
                    tracing::warn!(
                        event_id = %registration.event_id,
                        board_id = %event.board_id,
                        error = %err,
                        "Could not read initial latest-board marker"
                    );
                    None
                }
            }
        } else {
            None
        };

        let run_token = supervisor_token.child_token();
        let run_started = Instant::now();
        let run_future = execute_daemon_event(
            app_handle.clone(),
            registration.app_id.clone(),
            registration.event_id.clone(),
            config.payload.clone(),
            run_token.clone(),
            registration.offline,
            registration.personal_access_token.clone(),
            Some(registration.oauth_tokens.clone()),
            Duration::from_millis(config.log_flush_interval_ms),
            config.log_batch_size,
        );
        tokio::pin!(run_future);

        let board_poll_interval = Duration::from_millis(config.board_poll_interval_ms);
        let mut poll_interval = tokio::time::interval_at(
            tokio::time::Instant::now() + board_poll_interval,
            board_poll_interval,
        );
        let mut restart_for_board_change = false;
        let run_result = loop {
            tokio::select! {
                _ = supervisor_token.cancelled() => {
                    run_token.cancel();
                    let _ = tokio::time::timeout(STOP_TIMEOUT, &mut run_future).await;
                    return;
                }
                result = &mut run_future => {
                    break Some(result);
                }
                _ = poll_interval.tick(), if initial_marker.is_some() => {
                    let changed = latest_board_changed(
                        &app_handle,
                        &registration.app_id,
                        &event.board_id,
                        initial_marker.as_ref().unwrap(),
                    ).await;

                    if changed {
                        tracing::info!(
                            event_id = %registration.event_id,
                            board_id = %event.board_id,
                            "Latest board changed; restarting daemon run"
                        );
                        restart_for_board_change = true;
                        run_token.cancel();
                        let _ = tokio::time::timeout(STOP_TIMEOUT, &mut run_future).await;
                        break None;
                    }
                }
            }
        };

        if supervisor_token.is_cancelled() {
            break;
        }

        if restart_for_board_change {
            next_delay = min_delay;
            continue;
        }

        let failed = run_failed(run_result);
        if run_started.elapsed() >= healthy_reset {
            next_delay = min_delay;
        }

        if !config.should_restart(failed) {
            tracing::info!(
                event_id = %registration.event_id,
                failed,
                "Daemon run completed and restart policy does not request another run"
            );
            break;
        }

        tracing::warn!(
            event_id = %registration.event_id,
            failed,
            delay_ms = next_delay.as_millis(),
            "Daemon run ended; scheduling restart"
        );

        if sleep_or_cancel(next_delay, &supervisor_token).await {
            break;
        }

        next_delay = (next_delay * 2).min(max_delay);
    }

    tracing::info!(
        event_id = %registration.event_id,
        app_id = %registration.app_id,
        "Daemon event supervisor stopped"
    );
}

async fn sleep_or_cancel(duration: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = token.cancelled() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

fn run_failed(
    result: Option<Result<Option<LogMeta>, crate::functions::TauriFunctionError>>,
) -> bool {
    match result {
        Some(Ok(Some(meta))) => meta.log_level >= LogLevel::Error.to_u8(),
        Some(Ok(None)) => true,
        Some(Err(err)) => {
            tracing::warn!(error = ?err, "Daemon run failed");
            true
        }
        None => false,
    }
}

async fn load_current_event(
    app_handle: &AppHandle,
    registration: &EventRegistration,
) -> Result<Event> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    let app = App::load(registration.app_id.clone(), flow_like_state).await?;
    app.get_event(&registration.event_id, None).await
}

async fn latest_board_changed(
    app_handle: &AppHandle,
    app_id: &str,
    board_id: &str,
    initial_marker: &BoardMarker,
) -> bool {
    match load_board_marker(app_handle, app_id, board_id).await {
        Ok(marker) => marker != *initial_marker,
        Err(err) => {
            tracing::warn!(
                app_id,
                board_id,
                error = %err,
                "Could not poll latest-board marker for daemon"
            );
            false
        }
    }
}

async fn load_board_marker(
    app_handle: &AppHandle,
    app_id: &str,
    board_id: &str,
) -> Result<BoardMarker> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    let app = App::load(app_id.to_string(), flow_like_state).await?;
    let board = app
        .open_board(board_id.to_string(), Some(false), None)
        .await?;
    let board = board.lock().await;
    let (updated_secs, updated_nanos) = system_time_parts(board.updated_at);

    Ok(BoardMarker {
        version: board.version,
        updated_secs,
        updated_nanos,
        hash: board.hash,
        nodes: board.nodes.len(),
        layers: board.layers.len(),
        variables: board.variables.len(),
        comments: board.comments.len(),
    })
}

fn system_time_parts(time: SystemTime) -> (u64, u32) {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs(), duration.subsec_nanos()),
        Err(_) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn tracked_task_exists(event_id: &str) -> bool {
        DAEMON_TASKS.lock().await.contains_key(event_id)
    }

    async fn tracked_task_count(event_id: &str) -> usize {
        DAEMON_TASKS
            .lock()
            .await
            .keys()
            .filter(|id| id.as_str() == event_id)
            .count()
    }

    async fn wait_until_task_removed(event_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !tracked_task_exists(event_id).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    fn log_meta(level: LogLevel) -> LogMeta {
        LogMeta {
            app_id: "app".to_string(),
            run_id: "run".to_string(),
            board_id: "board".to_string(),
            start: 0,
            end: 0,
            log_level: level.to_u8(),
            version: "0.0.1".to_string(),
            nodes: None,
            logs: None,
            node_id: "node".to_string(),
            event_version: None,
            event_id: "event".to_string(),
            payload: Vec::new(),
            is_remote: false,
        }
    }

    #[test]
    fn daemon_config_deserializes_with_operational_defaults() {
        let sink: DaemonSink = serde_json::from_value(serde_json::json!({})).unwrap();

        assert_eq!(sink.restart_policy, DaemonRestartPolicy::OnFailure);
        assert_eq!(sink.min_restart_delay_ms, DEFAULT_MIN_RESTART_DELAY_MS);
        assert_eq!(sink.max_restart_delay_ms, DEFAULT_MAX_RESTART_DELAY_MS);
        assert_eq!(sink.board_poll_interval_ms, DEFAULT_BOARD_POLL_INTERVAL_MS);
        assert_eq!(sink.log_flush_interval_ms, DEFAULT_LOG_FLUSH_INTERVAL_MS);
        assert_eq!(sink.log_batch_size, DEFAULT_LOG_BATCH_SIZE);
        assert_eq!(sink.healthy_reset_ms, DEFAULT_HEALTHY_RESET_MS);
        assert!(sink.payload.is_none());
    }

    #[test]
    fn daemon_config_normalizes_unsafe_values() {
        let sink = DaemonSink {
            restart_policy: DaemonRestartPolicy::Always,
            min_restart_delay_ms: 0,
            max_restart_delay_ms: 10,
            board_poll_interval_ms: 1,
            log_flush_interval_ms: 0,
            log_batch_size: 0,
            healthy_reset_ms: 0,
            payload: Some(serde_json::json!({"ok": true})),
        };

        let normalized = sink.normalized();

        assert_eq!(
            normalized.min_restart_delay_ms,
            DEFAULT_MIN_RESTART_DELAY_MS
        );
        assert_eq!(
            normalized.max_restart_delay_ms,
            DEFAULT_MIN_RESTART_DELAY_MS
        );
        assert_eq!(
            normalized.board_poll_interval_ms,
            MIN_BOARD_POLL_INTERVAL_MS
        );
        assert_eq!(
            normalized.log_flush_interval_ms,
            DEFAULT_LOG_FLUSH_INTERVAL_MS
        );
        assert_eq!(normalized.log_batch_size, DEFAULT_LOG_BATCH_SIZE);
        assert_eq!(normalized.healthy_reset_ms, DEFAULT_HEALTHY_RESET_MS);
        assert_eq!(normalized.restart_policy, DaemonRestartPolicy::Always);
        assert_eq!(normalized.payload, Some(serde_json::json!({"ok": true})));
    }

    #[test]
    fn restart_policy_only_restarts_when_configured() {
        let mut sink = DaemonSink {
            restart_policy: DaemonRestartPolicy::Never,
            ..Default::default()
        };

        assert!(!sink.should_restart(false));
        assert!(!sink.should_restart(true));

        sink.restart_policy = DaemonRestartPolicy::OnFailure;
        assert!(!sink.should_restart(false));
        assert!(sink.should_restart(true));

        sink.restart_policy = DaemonRestartPolicy::Always;
        assert!(sink.should_restart(false));
        assert!(sink.should_restart(true));
    }

    #[test]
    fn run_failure_detection_treats_error_logs_as_failures() {
        assert!(!run_failed(Some(Ok(Some(log_meta(LogLevel::Info))))));
        assert!(run_failed(Some(Ok(Some(log_meta(LogLevel::Error))))));
        assert!(run_failed(Some(Ok(Some(log_meta(LogLevel::Fatal))))));
        assert!(run_failed(Some(Ok(None))));
        assert!(!run_failed(None));
    }

    #[test]
    fn system_time_parts_clamps_pre_epoch_times() {
        assert_eq!(
            system_time_parts(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn tracked_daemon_task_self_cleans_when_completed() {
        let event_id = format!("test-daemon-{}", create_id());
        DaemonSink::stop_event(&event_id).await;

        DaemonSink::start_tracked_task(event_id.clone(), |_| async {}).await;

        wait_until_task_removed(&event_id).await;
    }

    #[tokio::test]
    async fn tracked_daemon_task_replace_and_stop_keep_one_handle() {
        let event_id = format!("test-daemon-{}", create_id());
        DaemonSink::stop_event(&event_id).await;

        let (first_stopped_tx, first_stopped_rx) = tokio::sync::oneshot::channel();
        DaemonSink::start_tracked_task(event_id.clone(), move |token| async move {
            token.cancelled().await;
            let _ = first_stopped_tx.send(());
        })
        .await;

        assert!(tracked_task_exists(&event_id).await);

        let (second_started_tx, second_started_rx) = tokio::sync::oneshot::channel();
        DaemonSink::start_tracked_task(event_id.clone(), move |token| async move {
            let _ = second_started_tx.send(());
            token.cancelled().await;
        })
        .await;

        tokio::time::timeout(Duration::from_secs(1), first_stopped_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), second_started_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracked_task_count(&event_id).await, 1);

        DaemonSink::stop_event(&event_id).await;
        assert!(!tracked_task_exists(&event_id).await);
    }
}

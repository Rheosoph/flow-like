//! Durable outbox that reports finished local runs of hub-backed apps to the
//! hub and pushes the logs of failed ones. Rows live in `runs.db` beside the
//! run index, so a report outlives restarts, offline periods and hub outages.
//!
//! A row never holds a UI session token. It names the session subject, or the
//! sink registration whose PAT ran the event, and the drain resolves a current
//! token when it sends.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flow_like::app::AppVisibility;
use flow_like::flow::board::format::CURRENT_BOARD_FORMAT_VERSION;
use flow_like::flow::execution::log::StoredLogMessage;
use flow_like::flow::execution::{LogLevel, LogMeta, RunStatus, extract_sub_from_jwt};
use flow_like::flow_like_storage::lancedb;
use flow_like::state::FlowLikeState;
use flow_like_types::json;
use flow_like_types::reqwest::{
    self, StatusCode,
    header::{AUTHORIZATION, HeaderValue},
};
use flow_like_types::tokio::{self, sync::Notify};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::{TauriEventSinkManagerState, TauriFlowLikeState};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run_reports (
    run_id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL,
    board_id TEXT NOT NULL,
    hub TEXT NOT NULL,
    report TEXT NOT NULL,
    upload_logs INTEGER NOT NULL,
    auth_kind TEXT NOT NULL,
    auth_ref TEXT NOT NULL,
    report_sent INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    last_error TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS run_reports_due ON run_reports(next_attempt_at, created_at);
";

const ROW_COLUMNS: &str = "run_id, app_id, board_id, hub, report, upload_logs, auth_kind, auth_ref, report_sent, attempts, created_at";

const DRAIN_INTERVAL: Duration = Duration::from_secs(60);
const DRAIN_BATCH: usize = 200;
const BASE_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_UPLOAD_LOGS: usize = 10_000;
const MAX_UPLOAD_BYTES: usize = 3 * 1024 * 1024 + 512 * 1024;
/// Room for the request envelope and the "not uploaded" line.
const RESERVED_UPLOAD_BYTES: usize = 1024;
/// One oversized line would otherwise close the budget for everything after it.
const MAX_LOG_MESSAGE_BYTES: usize = 64 * 1024;
const LOG_PAGE: usize = 1_000;

/// Body of `POST /apps/{app_id}/board/{board_id}/runs/report`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct ReportRunRequest {
    run_id: String,
    node_id: String,
    event_id: Option<String>,
    version: Option<String>,
    log_level: u8,
    start: u64,
    end: u64,
    error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nodes: Option<Vec<(String, u8)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    logs: Option<u64>,
}

impl ReportRunRequest {
    fn from_meta(meta: &LogMeta) -> Self {
        let non_empty = |value: &str| (!value.is_empty()).then(|| value.to_string());
        Self {
            run_id: meta.run_id.clone(),
            node_id: meta.node_id.clone(),
            event_id: non_empty(&meta.event_id),
            version: non_empty(&meta.version),
            log_level: meta.log_level,
            start: meta.start,
            end: meta.end,
            error_message: (meta.log_level >= LogLevel::Error.to_u8())
                .then(|| format!("Local run failed with log_level {}", meta.log_level)),
            event_version: meta.event_version.clone(),
            nodes: meta.nodes.clone(),
            logs: meta.logs,
        }
    }
}

#[derive(Serialize)]
struct UploadLogsRequest<'a> {
    logs: &'a [StoredLogMessage],
}

/// Who a queued report is sent as.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReportAuth {
    /// A UI session; its current token is looked up by subject when sending.
    Session { subject: String },
    /// The PAT of the sink registration that ran this event.
    Sink { event_id: String },
}

impl ReportAuth {
    fn for_token(token: &str, event_id: &str) -> Option<Self> {
        let token = token.strip_prefix("Bearer ").unwrap_or(token);
        if token.starts_with("pat_") {
            return (!event_id.is_empty()).then(|| Self::Sink {
                event_id: event_id.to_string(),
            });
        }
        extract_sub_from_jwt(token)
            .ok()
            .map(|subject| Self::Session { subject })
    }

    fn from_row(kind: &str, reference: String) -> Option<Self> {
        match kind {
            "session" => Some(Self::Session { subject: reference }),
            "sink" => Some(Self::Sink {
                event_id: reference,
            }),
            _ => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Session { .. } => "session",
            Self::Sink { .. } => "sink",
        }
    }

    fn reference(&self) -> &str {
        match self {
            Self::Session { subject } => subject,
            Self::Sink { event_id } => event_id,
        }
    }
}

/// A run that just finished, as the code that executed it knows it.
pub(crate) struct FinishedRun<'a> {
    pub meta: &'a LogMeta,
    pub status: &'a RunStatus,
    pub visibility: &'a AppVisibility,
    pub hub: &'a str,
    pub secure: bool,
    /// The token the run executed with: a UI session JWT or a sink PAT.
    pub token: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
struct QueuedReport {
    run_id: String,
    app_id: String,
    board_id: String,
    hub: String,
    report: ReportRunRequest,
    upload_logs: bool,
    auth: ReportAuth,
    report_sent: bool,
    attempts: u32,
    created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Report,
    Logs,
}

impl QueuedReport {
    fn for_run(run: &FinishedRun<'_>, now: i64) -> Option<Self> {
        if matches!(run.visibility, AppVisibility::Offline) {
            return None;
        }
        let meta = run.meta;
        let hub = flow_like::hub::hub_origin(run.hub, run.secure)?;
        let auth = ReportAuth::for_token(run.token?, &meta.event_id)?;
        Some(Self {
            run_id: meta.run_id.clone(),
            app_id: meta.app_id.clone(),
            board_id: meta.board_id.clone(),
            hub,
            report: ReportRunRequest::from_meta(meta),
            // A cancelled run also ends at Fatal; only a real failure ships its logs.
            upload_logs: matches!(run.status, RunStatus::Failed)
                && meta.log_level >= LogLevel::Error.to_u8(),
            auth,
            report_sent: false,
            attempts: 0,
            created_at: now,
        })
    }

    fn url(&self, stage: Stage) -> String {
        let runs = format!(
            "{}/api/v1/apps/{}/board/{}/runs",
            self.hub, self.app_id, self.board_id
        );
        match stage {
            Stage::Report => format!("{runs}/report"),
            Stage::Logs => format!("{runs}/{}/logs", self.run_id),
        }
    }

    fn log_table(&self) -> LogMeta {
        LogMeta {
            app_id: self.app_id.clone(),
            run_id: self.run_id.clone(),
            board_id: self.board_id.clone(),
            start: self.report.start,
            end: self.report.end,
            log_level: self.report.log_level,
            version: self.report.version.clone().unwrap_or_default(),
            nodes: None,
            logs: self.report.logs,
            node_id: self.report.node_id.clone(),
            event_version: self.report.event_version.clone(),
            event_id: self.report.event_id.clone().unwrap_or_default(),
            payload: Vec::new(),
            is_remote: false,
        }
    }
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedReport> {
    let invalid = |column: usize, error: Box<dyn std::error::Error + Send + Sync>| {
        rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, error)
    };
    let report =
        json::from_str(&row.get::<_, String>(4)?).map_err(|error| invalid(4, error.into()))?;
    let kind = row.get::<_, String>(6)?;
    let auth = ReportAuth::from_row(&kind, row.get(7)?)
        .ok_or_else(|| invalid(6, format!("unknown auth kind '{kind}'").into()))?;
    Ok(QueuedReport {
        run_id: row.get(0)?,
        app_id: row.get(1)?,
        board_id: row.get(2)?,
        hub: row.get(3)?,
        report,
        upload_logs: row.get(5)?,
        auth,
        report_sent: row.get(8)?,
        attempts: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, millis)
}

fn backoff(attempts: u32) -> Duration {
    let doublings = attempts.saturating_sub(1).min(16);
    BASE_BACKOFF.saturating_mul(1 << doublings).min(MAX_BACKOFF)
}

#[derive(Clone)]
pub(crate) struct RunReportQueue {
    conn: Arc<Mutex<Connection>>,
    wake: Arc<Notify>,
}

impl RunReportQueue {
    /// Opens the queue inside `runs.db`. Roots that cannot be opened degrade
    /// to an in-memory queue, so reports then last for the session only.
    pub(crate) fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        match Connection::open(path).and_then(Self::with_connection) {
            Ok(queue) => queue,
            Err(error) => {
                eprintln!(
                    "Failed to open the run report queue at {}: {error}. Falling back to an in-memory queue.",
                    path.display()
                );
                Connection::open_in_memory()
                    .and_then(Self::with_connection)
                    .expect("in-memory sqlite run report queue")
            }
        }
    }

    fn with_connection(conn: Connection) -> rusqlite::Result<Self> {
        crate::run_index::configure_connection(&conn)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            wake: Arc::new(Notify::new()),
        })
    }

    async fn with_conn<T, F>(&self, operation: &'static str, f: F) -> flow_like_types::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| flow_like_types::anyhow!("Run report queue connection poisoned"))?;
            f(&guard)
                .map_err(|error| flow_like_types::anyhow!("Run report queue {operation}: {error}"))
        })
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Run report queue {operation} task failed: {error}")
        })?
    }

    async fn push(&self, report: QueuedReport) -> flow_like_types::Result<()> {
        let body = json::to_string(&report.report)?;
        self.with_conn("push", move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO run_reports (run_id, app_id, board_id, hub, report, upload_logs, auth_kind, auth_ref, report_sent, state, attempts, next_attempt_at, last_error, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11, NULL, ?11)",
                params![
                    report.run_id,
                    report.app_id,
                    report.board_id,
                    report.hub,
                    body,
                    report.upload_logs,
                    report.auth.kind(),
                    report.auth.reference(),
                    report.report_sent,
                    report.attempts,
                    report.created_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Reports whose next attempt is due, the longest-waiting first. Rows that
    /// no longer decode are dropped here; no later attempt can send them.
    async fn due(&self, now: i64, limit: usize) -> flow_like_types::Result<Vec<QueuedReport>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_conn("due", move |conn| {
            let mut statement = conn.prepare_cached(&format!(
                "SELECT {ROW_COLUMNS} FROM run_reports WHERE next_attempt_at <= ?1 \
                 ORDER BY next_attempt_at, created_at, run_id LIMIT ?2"
            ))?;
            let rows = statement.query_map(params![now, limit], |row| {
                Ok((row.get::<_, String>(0)?, read_row(row)))
            })?;
            let mut due = Vec::new();
            let mut unreadable = Vec::new();
            for row in rows {
                match row? {
                    (_, Ok(report)) => due.push(report),
                    (run_id, Err(error)) => {
                        tracing::warn!(run_id = %run_id, error = %error, "Dropping an unreadable run report");
                        unreadable.push(run_id);
                    }
                }
            }
            for run_id in unreadable {
                conn.execute("DELETE FROM run_reports WHERE run_id = ?1", params![run_id])?;
            }
            Ok(due)
        })
        .await
    }

    async fn mark_report_sent(&self, run_id: &str) -> flow_like_types::Result<()> {
        let run_id = run_id.to_string();
        self.with_conn("mark_report_sent", move |conn| {
            conn.execute(
                "UPDATE run_reports SET report_sent = 1, state = 'pending', last_error = NULL WHERE run_id = ?1",
                params![run_id],
            )?;
            Ok(())
        })
        .await
    }

    /// Parks a report until a token for it may exist, without counting an attempt.
    async fn await_auth(&self, run_id: &str, until: i64) -> flow_like_types::Result<()> {
        let run_id = run_id.to_string();
        self.with_conn("await_auth", move |conn| {
            conn.execute(
                "UPDATE run_reports SET state = 'awaiting_auth', next_attempt_at = ?2 WHERE run_id = ?1",
                params![run_id, until],
            )?;
            Ok(())
        })
        .await
    }

    async fn retry(
        &self,
        report: &QueuedReport,
        now: i64,
        error: &str,
        resend_report: bool,
    ) -> flow_like_types::Result<()> {
        let attempts = report.attempts.saturating_add(1);
        let next_attempt_at = now.saturating_add(millis(backoff(attempts)));
        let run_id = report.run_id.clone();
        let error = error.to_string();
        self.with_conn("retry", move |conn| {
            conn.execute(
                "UPDATE run_reports SET state = 'retrying', attempts = ?2, next_attempt_at = ?3, last_error = ?4, \
                 report_sent = CASE WHEN ?5 THEN 0 ELSE report_sent END WHERE run_id = ?1",
                params![run_id, attempts, next_attempt_at, error, resend_report],
            )?;
            Ok(())
        })
        .await
    }

    async fn remove(&self, run_id: &str) -> flow_like_types::Result<()> {
        let run_id = run_id.to_string();
        self.with_conn("remove", move |conn| {
            conn.execute("DELETE FROM run_reports WHERE run_id = ?1", params![run_id])?;
            Ok(())
        })
        .await
    }
}

/// Queues a finished run for the hub and wakes the drain. Offline apps and
/// runs without a hub identity are not reported.
pub(crate) async fn enqueue(app_handle: &AppHandle, run: FinishedRun<'_>) {
    let Some(report) = QueuedReport::for_run(&run, now_millis()) else {
        if !matches!(run.visibility, AppVisibility::Offline) {
            tracing::debug!(run_id = %run.meta.run_id, "Run has no hub identity to report it under");
        }
        return;
    };
    let Some(queue) = app_handle
        .try_state::<RunReportQueue>()
        .map(|queue| queue.inner().clone())
    else {
        tracing::warn!(run_id = %report.run_id, "Run report queue unavailable; the run is not reported");
        return;
    };
    let run_id = report.run_id.clone();
    match queue.push(report).await {
        Ok(()) => queue.wake.notify_one(),
        Err(error) => {
            tracing::warn!(run_id = %run_id, error = %error, "Failed to queue the run report")
        }
    }
}

/// Starts the single task that delivers queued reports. It runs on enqueue and
/// every minute, so reports parked offline or without a token go out later.
pub(crate) fn spawn_drain(app_handle: AppHandle) {
    let Some(queue) = app_handle
        .try_state::<RunReportQueue>()
        .map(|queue| queue.inner().clone())
    else {
        tracing::warn!("Run report queue unavailable; queued runs are not reported");
        return;
    };
    tauri::async_runtime::spawn(async move {
        let client = match reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(error = %error, "Run report drain could not build its HTTP client");
                return;
            }
        };
        loop {
            drain(&app_handle, &queue, &client).await;
            tokio::select! {
                _ = queue.wake.notified() => {}
                _ = tokio::time::sleep(DRAIN_INTERVAL) => {}
            }
        }
    });
}

enum Pass {
    Continue,
    /// The hub did not answer; later reports to it would wait out the same timeout.
    SkipHub,
}

async fn drain(app_handle: &AppHandle, queue: &RunReportQueue, client: &reqwest::Client) {
    let reports = match queue.due(now_millis(), DRAIN_BATCH).await {
        Ok(reports) if reports.is_empty() => return,
        Ok(reports) => reports,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to read the run report queue");
            return;
        }
    };
    let state = match TauriFlowLikeState::construct(app_handle).await {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(error = %error, "Run report drain has no runtime state");
            return;
        }
    };
    let mut unreachable = HashSet::new();
    for report in reports {
        if unreachable.contains(&report.hub) {
            continue;
        }
        let run_id = report.run_id.clone();
        let hub = report.hub.clone();
        match deliver(app_handle, &state, queue, client, report).await {
            Ok(Pass::Continue) => {}
            Ok(Pass::SkipHub) => {
                unreachable.insert(hub);
            }
            Err(error) => {
                tracing::warn!(run_id = %run_id, error = %error, "Failed to update a queued run report")
            }
        }
    }
}

enum TokenLookup {
    Ready(String),
    Pending,
    Gone,
}

fn resolve_token(app_handle: &AppHandle, report: &QueuedReport) -> TokenLookup {
    match &report.auth {
        ReportAuth::Session { subject } => {
            crate::execution_credentials::session_token(&report.hub, subject)
                .map_or(TokenLookup::Pending, TokenLookup::Ready)
        }
        ReportAuth::Sink { event_id } => {
            let Some(manager) = app_handle.try_state::<TauriEventSinkManagerState>() else {
                return TokenLookup::Pending;
            };
            let Ok(db) = manager.0.try_lock().map(|manager| manager.db()) else {
                return TokenLookup::Pending;
            };
            match crate::event_sink::EventSinkManager::access_token(db, &report.app_id, event_id) {
                Ok(Some(token)) => TokenLookup::Ready(token),
                Ok(None) => TokenLookup::Gone,
                Err(error) => {
                    tracing::warn!(event_id = %event_id, error = %error, "Failed to read the sink token for a run report");
                    TokenLookup::Pending
                }
            }
        }
    }
}

async fn deliver(
    app_handle: &AppHandle,
    state: &Arc<FlowLikeState>,
    queue: &RunReportQueue,
    client: &reqwest::Client,
    mut report: QueuedReport,
) -> flow_like_types::Result<Pass> {
    let now = now_millis();
    if now.saturating_sub(report.created_at) > millis(MAX_AGE) {
        tracing::warn!(
            run_id = %report.run_id,
            attempts = report.attempts,
            "Dropping a run report that could not be delivered within 7 days"
        );
        queue.remove(&report.run_id).await?;
        return Ok(Pass::Continue);
    }

    let token = match resolve_token(app_handle, &report) {
        TokenLookup::Ready(token) => token,
        TokenLookup::Pending => {
            queue
                .await_auth(&report.run_id, now.saturating_add(millis(DRAIN_INTERVAL)))
                .await?;
            return Ok(Pass::Continue);
        }
        TokenLookup::Gone => {
            tracing::warn!(
                run_id = %report.run_id,
                "The sink registration that ran this event has no token anymore; dropping its run report"
            );
            queue.remove(&report.run_id).await?;
            return Ok(Pass::Continue);
        }
    };

    if !report.report_sent {
        let outcome = send(client, &report, Stage::Report, &token, &report.report).await;
        if let Some(pass) = settle(queue, &report, now, Stage::Report, outcome).await? {
            return Ok(pass);
        }
        queue.mark_report_sent(&report.run_id).await?;
        report.report_sent = true;
    }

    if report.upload_logs {
        let logs = match read_logs(state, &report).await {
            Ok(Some(logs)) => logs,
            Ok(None) => {
                tracing::info!(
                    run_id = %report.run_id,
                    "The run's local logs are gone; reported the run without them"
                );
                queue.remove(&report.run_id).await?;
                return Ok(Pass::Continue);
            }
            Err(error) => {
                let error = format!("reading the local logs failed: {error}");
                queue.retry(&report, now, &error, false).await?;
                return Ok(Pass::Continue);
            }
        };
        let body = UploadLogsRequest { logs: &logs };
        let outcome = send(client, &report, Stage::Logs, &token, &body).await;
        if let Some(pass) = settle(queue, &report, now, Stage::Logs, outcome).await? {
            return Ok(pass);
        }
    }

    queue.remove(&report.run_id).await?;
    Ok(Pass::Continue)
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Delivered,
    Unreachable(String),
    Retry { error: String, resend_report: bool },
    Rejected(String),
}

fn classify(status: StatusCode, stage: Stage) -> Outcome {
    let error = format!("the hub answered {status}");
    match status.as_u16() {
        200..=299 => Outcome::Delivered,
        // The logs route answers 404 until the run's report row exists.
        404 if stage == Stage::Logs => Outcome::Retry {
            error,
            resend_report: true,
        },
        401 | 408 | 429 | 500..=599 => Outcome::Retry {
            error,
            resend_report: false,
        },
        _ => Outcome::Rejected(error),
    }
}

fn authorization(token: &str) -> String {
    if token.starts_with("pat_") || token.starts_with("Bearer ") {
        token.to_string()
    } else {
        format!("Bearer {token}")
    }
}

async fn send<T: Serialize + ?Sized>(
    client: &reqwest::Client,
    report: &QueuedReport,
    stage: Stage,
    token: &str,
    body: &T,
) -> Outcome {
    let Ok(mut header) = HeaderValue::from_str(&authorization(token)) else {
        return Outcome::Rejected("the token is not a valid header value".to_string());
    };
    header.set_sensitive(true);
    let response = client
        .post(report.url(stage))
        .header(AUTHORIZATION, header)
        .header(
            "x-flow-like-board-format",
            CURRENT_BOARD_FORMAT_VERSION.to_string(),
        )
        .json(body)
        .send()
        .await;
    match response {
        Ok(response) => classify(response.status(), stage),
        Err(error) if error.is_connect() || error.is_timeout() => {
            Outcome::Unreachable(error.to_string())
        }
        Err(error) => Outcome::Retry {
            error: error.to_string(),
            resend_report: false,
        },
    }
}

/// Records a request that did not go through. `None` means it was delivered
/// and the report moves on to its next step.
async fn settle(
    queue: &RunReportQueue,
    report: &QueuedReport,
    now: i64,
    stage: Stage,
    outcome: Outcome,
) -> flow_like_types::Result<Option<Pass>> {
    match outcome {
        Outcome::Delivered => Ok(None),
        Outcome::Unreachable(error) => {
            queue.retry(report, now, &error, false).await?;
            Ok(Some(Pass::SkipHub))
        }
        Outcome::Retry {
            error,
            resend_report,
        } => {
            queue.retry(report, now, &error, resend_report).await?;
            Ok(Some(Pass::Continue))
        }
        Outcome::Rejected(error) => {
            tracing::warn!(
                run_id = %report.run_id,
                stage = ?stage,
                error = %error,
                "The hub rejected a run report; dropping it"
            );
            queue.remove(&report.run_id).await?;
            Ok(Some(Pass::Continue))
        }
    }
}

fn is_missing_table(error: &flow_like_types::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<lancedb::Error>(),
            Some(lancedb::Error::TableNotFound { .. })
        )
    })
}

/// The run's logs as one upload, or `None` when its log table is gone.
async fn read_logs(
    state: &Arc<FlowLikeState>,
    report: &QueuedReport,
) -> flow_like_types::Result<Option<Vec<StoredLogMessage>>> {
    let table = report.log_table();
    let mut selection = LogSelection::new(MAX_UPLOAD_LOGS, MAX_UPLOAD_BYTES);
    'classes: for filter in ["log_level >= 2", "log_level < 2"] {
        let mut offset = 0;
        loop {
            let page = match state
                .query_run(&table, filter, Some(LOG_PAGE), Some(offset))
                .await
            {
                Ok(page) => page,
                Err(error) if is_missing_table(&error) => return Ok(None),
                Err(error) => return Err(error),
            };
            let read = page.len();
            for message in page {
                selection.offer(message.into());
            }
            if selection.is_full() {
                break 'classes;
            }
            if read < LOG_PAGE {
                break;
            }
            offset += read;
        }
    }
    // A full selection stops reading, so only the run's own count knows the rest.
    let total = if selection.is_full() {
        report.report.logs.unwrap_or(0).max(selection.offered)
    } else {
        selection.offered
    };
    Ok(Some(selection.into_logs(total, report.report.end)))
}

/// Chooses what fits one upload. Callers offer warnings and errors first, then
/// the remaining messages in the order the run wrote them; the first message
/// that no longer fits closes the selection.
struct LogSelection {
    max_messages: usize,
    max_bytes: usize,
    kept: Vec<StoredLogMessage>,
    bytes: usize,
    offered: u64,
    closed: bool,
}

impl LogSelection {
    fn new(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            max_messages: max_messages.saturating_sub(1),
            max_bytes: max_bytes.saturating_sub(RESERVED_UPLOAD_BYTES),
            kept: Vec::new(),
            bytes: 0,
            offered: 0,
            closed: false,
        }
    }

    fn offer(&mut self, mut message: StoredLogMessage) {
        self.offered += 1;
        if self.is_full() {
            return;
        }
        truncate_message(&mut message.message);
        let size = json::to_vec(&message).map_or(usize::MAX, |bytes| bytes.len() + 1);
        if self.bytes.saturating_add(size) > self.max_bytes {
            self.closed = true;
            return;
        }
        self.bytes += size;
        self.kept.push(message);
    }

    fn is_full(&self) -> bool {
        self.closed || self.kept.len() >= self.max_messages
    }

    /// The kept messages in chronological order, closed by a line counting
    /// what `total` holds beyond them.
    fn into_logs(self, total: u64, at: u64) -> Vec<StoredLogMessage> {
        let mut logs = self.kept;
        logs.sort_by_key(|log| log.start);
        let dropped = total.saturating_sub(logs.len() as u64);
        if dropped > 0 {
            logs.push(StoredLogMessage {
                message: format!("{dropped} log messages were not uploaded"),
                operation_id: None,
                node_id: None,
                log_level: LogLevel::Info.to_u8(),
                token_in: None,
                token_out: None,
                bit_ids: None,
                start: at,
                end: at,
                fingerprint: None,
            });
        }
        logs
    }
}

fn truncate_message(message: &mut String) {
    if message.len() <= MAX_LOG_MESSAGE_BYTES {
        return;
    }
    let mut end = MAX_LOG_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push_str(" [truncated]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::execution::run_index::{RunIndex, RunQuery};

    const ALICE: &str = "e30.eyJzdWIiOiJhbGljZSJ9.signature";

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("flow-like-run-reports-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn runs_db(&self) -> std::path::PathBuf {
            self.0.join("runs.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn meta(run_id: &str, log_level: u8) -> LogMeta {
        LogMeta {
            app_id: "app".to_string(),
            run_id: run_id.to_string(),
            board_id: "board".to_string(),
            start: 1_000,
            end: 2_000,
            log_level,
            version: "v1-0-0".to_string(),
            nodes: Some(vec![("node".to_string(), log_level)]),
            logs: Some(3),
            node_id: "start".to_string(),
            event_version: None,
            event_id: "event".to_string(),
            payload: Vec::new(),
            is_remote: false,
        }
    }

    fn finished<'a>(
        meta: &'a LogMeta,
        status: &'a RunStatus,
        visibility: &'a AppVisibility,
        token: Option<&'a str>,
    ) -> FinishedRun<'a> {
        FinishedRun {
            meta,
            status,
            visibility,
            hub: "api.example.test",
            secure: true,
            token,
        }
    }

    fn queued(run_id: &str, created_at: i64) -> QueuedReport {
        QueuedReport::for_run(
            &finished(
                &meta(run_id, 3),
                &RunStatus::Failed,
                &AppVisibility::Private,
                Some(ALICE),
            ),
            created_at,
        )
        .unwrap()
    }

    fn log(message: &str, log_level: u8, start: u64) -> StoredLogMessage {
        StoredLogMessage {
            message: message.to_string(),
            operation_id: None,
            node_id: Some("node".to_string()),
            log_level,
            token_in: None,
            token_out: None,
            bit_ids: None,
            start,
            end: start,
            fingerprint: None,
        }
    }

    fn run_ids(reports: &[QueuedReport]) -> Vec<&str> {
        reports
            .iter()
            .map(|report| report.run_id.as_str())
            .collect()
    }

    #[test]
    fn offline_apps_are_never_queued() {
        let meta = meta("run", 3);
        let run = finished(
            &meta,
            &RunStatus::Failed,
            &AppVisibility::Offline,
            Some(ALICE),
        );
        assert!(QueuedReport::for_run(&run, 0).is_none());
    }

    #[test]
    fn server_backed_apps_queue_under_the_session_subject() {
        let meta = meta("run", 1);
        for visibility in [
            AppVisibility::Public,
            AppVisibility::PublicRequestAccess,
            AppVisibility::Private,
            AppVisibility::Prototype,
        ] {
            let report = QueuedReport::for_run(
                &finished(&meta, &RunStatus::Success, &visibility, Some(ALICE)),
                5,
            )
            .unwrap();
            assert_eq!(
                report.auth,
                ReportAuth::Session {
                    subject: "alice".to_string()
                }
            );
            assert_eq!(report.hub, "https://api.example.test");
            assert_eq!(report.created_at, 5);
            assert!(!report.upload_logs);
            assert_eq!(
                report.url(Stage::Report),
                "https://api.example.test/api/v1/apps/app/board/board/runs/report"
            );
            assert_eq!(
                report.url(Stage::Logs),
                "https://api.example.test/api/v1/apps/app/board/board/runs/run/logs"
            );
        }
    }

    #[test]
    fn only_failed_runs_upload_their_logs() {
        let visibility = AppVisibility::Private;
        let upload = |status: RunStatus, log_level: u8| {
            let meta = meta("run", log_level);
            QueuedReport::for_run(&finished(&meta, &status, &visibility, Some(ALICE)), 0)
                .unwrap()
                .upload_logs
        };
        assert!(upload(RunStatus::Failed, 3));
        assert!(upload(RunStatus::Failed, 4));
        assert!(!upload(RunStatus::Failed, 2));
        assert!(!upload(RunStatus::Stopped, 4));
        assert!(!upload(RunStatus::Success, 3));
    }

    #[test]
    fn sink_pats_are_referenced_through_their_registration() {
        let visibility = AppVisibility::Private;
        let status = RunStatus::Success;
        let meta = meta("run", 1);
        let report = QueuedReport::for_run(
            &finished(&meta, &status, &visibility, Some("pat_secret")),
            0,
        )
        .unwrap();
        assert_eq!(
            report.auth,
            ReportAuth::Sink {
                event_id: "event".to_string()
            }
        );
        assert_eq!(report.auth.reference(), "event");

        let mut direct = meta.clone();
        direct.event_id.clear();
        assert!(
            QueuedReport::for_run(
                &finished(&direct, &status, &visibility, Some("pat_secret")),
                0
            )
            .is_none()
        );
        assert!(QueuedReport::for_run(&finished(&meta, &status, &visibility, None), 0).is_none());
        assert!(
            QueuedReport::for_run(&finished(&meta, &status, &visibility, Some("opaque")), 0)
                .is_none()
        );
    }

    #[tokio::test]
    async fn queued_reports_survive_reopening_beside_the_run_index() {
        let dir = TempDir::new();
        let index = crate::run_index::SqliteRunIndex::open(dir.runs_db());
        index.record(&meta("indexed", 1)).await.unwrap();

        let queue = RunReportQueue::open(dir.runs_db());
        let report = queued("queued", 10);
        queue.push(report.clone()).await.unwrap();
        drop(queue);

        let reopened = RunReportQueue::open(dir.runs_db());
        assert_eq!(reopened.due(10, 10).await.unwrap(), vec![report]);
        let stored: String = reopened
            .with_conn("inspect", |conn| {
                conn.query_row(
                    "SELECT hub || report || auth_kind || ':' || auth_ref FROM run_reports",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert!(stored.ends_with("session:alice"));
        assert!(!stored.contains(ALICE));
        assert_eq!(
            index.list(&RunQuery::new("app")).await.unwrap()[0].run_id,
            "indexed"
        );
    }

    #[tokio::test]
    async fn due_reports_come_oldest_first_and_wait_for_their_time() {
        let dir = TempDir::new();
        let queue = RunReportQueue::open(dir.runs_db());
        for (run_id, created_at) in [("late", 30), ("early", 10), ("middle", 20)] {
            queue.push(queued(run_id, created_at)).await.unwrap();
        }

        assert!(queue.due(5, 10).await.unwrap().is_empty());
        assert_eq!(
            run_ids(&queue.due(20, 10).await.unwrap()),
            ["early", "middle"]
        );
        assert_eq!(
            run_ids(&queue.due(100, 10).await.unwrap()),
            ["early", "middle", "late"]
        );
        assert_eq!(
            run_ids(&queue.due(100, 2).await.unwrap()),
            ["early", "middle"]
        );

        queue.await_auth("early", 200).await.unwrap();
        assert_eq!(
            run_ids(&queue.due(100, 10).await.unwrap()),
            ["middle", "late"]
        );
        let due = queue.due(200, 10).await.unwrap();
        assert_eq!(run_ids(&due), ["middle", "late", "early"]);
        assert_eq!(due[2].attempts, 0);
    }

    #[tokio::test]
    async fn retries_back_off_and_can_resend_the_report() {
        let dir = TempDir::new();
        let queue = RunReportQueue::open(dir.runs_db());
        queue.push(queued("run", 0)).await.unwrap();

        queue.mark_report_sent("run").await.unwrap();
        let report = queue.due(0, 1).await.unwrap().remove(0);
        assert!(report.report_sent);

        queue.retry(&report, 1_000, "offline", false).await.unwrap();
        assert!(queue.due(30_999, 1).await.unwrap().is_empty());
        let report = queue.due(31_000, 1).await.unwrap().remove(0);
        assert_eq!(report.attempts, 1);
        assert!(report.report_sent);

        queue.retry(&report, 31_000, "missing", true).await.unwrap();
        assert!(queue.due(90_999, 1).await.unwrap().is_empty());
        let report = queue.due(91_000, 1).await.unwrap().remove(0);
        assert_eq!(report.attempts, 2);
        assert!(!report.report_sent);

        assert_eq!(backoff(1), Duration::from_secs(30));
        assert_eq!(backoff(2), Duration::from_secs(60));
        assert_eq!(backoff(6), Duration::from_secs(16 * 60));
        assert_eq!(backoff(7), MAX_BACKOFF);
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
    }

    #[tokio::test]
    async fn remove_drops_only_the_addressed_report() {
        let dir = TempDir::new();
        let queue = RunReportQueue::open(dir.runs_db());
        queue.push(queued("keep", 0)).await.unwrap();
        queue.push(queued("drop", 1)).await.unwrap();

        queue.remove("drop").await.unwrap();
        queue.remove("drop").await.unwrap();
        assert_eq!(run_ids(&queue.due(10, 10).await.unwrap()), ["keep"]);
    }

    #[tokio::test]
    async fn unreadable_rows_are_dropped_instead_of_blocking_the_queue() {
        let dir = TempDir::new();
        let queue = RunReportQueue::open(dir.runs_db());
        queue.push(queued("good", 1)).await.unwrap();
        queue.push(queued("bad", 0)).await.unwrap();
        queue
            .with_conn("corrupt", |conn| {
                conn.execute(
                    "UPDATE run_reports SET report = 'not json' WHERE run_id = 'bad'",
                    [],
                )
            })
            .await
            .unwrap();

        assert_eq!(run_ids(&queue.due(10, 10).await.unwrap()), ["good"]);
        assert_eq!(run_ids(&queue.due(10, 10).await.unwrap()), ["good"]);
    }

    #[test]
    fn hub_answers_map_to_retry_or_drop() {
        let retry = |resend_report| Outcome::Retry {
            error: String::new(),
            resend_report,
        };
        let kind = |outcome: Outcome| match outcome {
            Outcome::Retry { resend_report, .. } => retry(resend_report),
            Outcome::Rejected(_) => Outcome::Rejected(String::new()),
            other => other,
        };
        let check = |status: u16, stage: Stage| {
            kind(classify(StatusCode::from_u16(status).unwrap(), stage))
        };

        assert_eq!(check(200, Stage::Report), Outcome::Delivered);
        assert_eq!(check(204, Stage::Logs), Outcome::Delivered);
        assert_eq!(check(404, Stage::Logs), retry(true));
        assert_eq!(check(404, Stage::Report), Outcome::Rejected(String::new()));
        for status in [401, 408, 429, 500, 503] {
            assert_eq!(check(status, Stage::Report), retry(false));
            assert_eq!(check(status, Stage::Logs), retry(false));
        }
        for status in [400, 403, 413, 422] {
            assert_eq!(check(status, Stage::Logs), Outcome::Rejected(String::new()));
        }
        assert_eq!(authorization("pat_secret"), "pat_secret");
        assert_eq!(authorization(ALICE), format!("Bearer {ALICE}"));
    }

    #[test]
    fn warnings_and_errors_claim_the_budget_before_the_rest() {
        let mut selection = LogSelection::new(4, MAX_UPLOAD_BYTES);
        for important in [log("error", 3, 30), log("warn", 2, 10)] {
            selection.offer(important);
        }
        for rest in [
            log("first", 1, 5),
            log("second", 0, 20),
            log("third", 1, 40),
        ] {
            selection.offer(rest);
        }
        assert!(selection.is_full());

        let logs = selection.into_logs(5, 99);
        let messages = logs
            .iter()
            .map(|log| log.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            ["first", "warn", "error", "2 log messages were not uploaded"]
        );
        let notice = logs.last().unwrap();
        assert_eq!(notice.log_level, LogLevel::Info.to_u8());
        assert_eq!((notice.start, notice.end), (99, 99));
    }

    #[test]
    fn a_complete_selection_has_no_notice() {
        let mut selection = LogSelection::new(10, MAX_UPLOAD_BYTES);
        selection.offer(log("late", 1, 20));
        selection.offer(log("early", 3, 10));
        assert!(!selection.is_full());
        let logs = selection.into_logs(2, 0);
        let messages = logs
            .iter()
            .map(|log| log.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(messages, ["early", "late"]);
    }

    #[test]
    fn uploads_stay_within_the_byte_and_message_caps() {
        let max_bytes = 64 * 1024;
        let mut selection = LogSelection::new(10_000, max_bytes);
        let line = "x".repeat(1_000);
        for index in 0..500 {
            selection.offer(log(&line, 1, index));
        }
        assert!(selection.is_full());
        let offered = selection.offered;
        let logs = selection.into_logs(offered, 1_000);
        let body = json::to_vec(&UploadLogsRequest { logs: &logs }).unwrap();
        assert!(body.len() <= max_bytes, "{} > {max_bytes}", body.len());
        let notice = logs.last().unwrap();
        assert_eq!(
            notice.message,
            format!("{} log messages were not uploaded", 500 - (logs.len() - 1))
        );

        let mut selection = LogSelection::new(MAX_UPLOAD_LOGS, MAX_UPLOAD_BYTES);
        for index in 0..(MAX_UPLOAD_LOGS as u64 + 5) {
            selection.offer(log("tiny", 1, index));
        }
        let offered = selection.offered;
        let logs = selection.into_logs(offered, 0);
        assert_eq!(logs.len(), MAX_UPLOAD_LOGS);
        assert_eq!(
            logs.last().unwrap().message,
            "6 log messages were not uploaded"
        );
    }

    #[test]
    fn oversized_messages_are_truncated_instead_of_closing_the_upload() {
        let mut selection = LogSelection::new(10, 256 * 1024);
        selection.offer(log(&"é".repeat(MAX_LOG_MESSAGE_BYTES), 3, 1));
        selection.offer(log("after", 1, 2));
        let logs = selection.into_logs(2, 0);
        assert_eq!(logs.len(), 2);
        assert!(logs[0].message.len() <= MAX_LOG_MESSAGE_BYTES + " [truncated]".len());
        assert!(logs[0].message.ends_with(" [truncated]"));
        assert_eq!(logs[1].message, "after");
    }

    #[test]
    fn a_full_selection_counts_what_it_never_read() {
        let mut selection = LogSelection::new(2, MAX_UPLOAD_BYTES);
        selection.offer(log("only", 3, 1));
        assert!(selection.is_full());
        let logs = selection.into_logs(40, 7);
        assert_eq!(logs[1].message, "39 log messages were not uploaded");
    }
}

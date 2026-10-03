//! Person-started runs between the management command that queued them and the placement
//! process that runs them. Memory only: inputs and results never reach the disk, and an agent
//! restart drops both.

use anyhow::Result;
use flow_like_device_protocol::ManagementResponse;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::time::Instant;

/// A run the parent hands to a placement process.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRun {
    pub operation_id: String,
    pub run_id: String,
    pub event_id: String,
    #[serde(default)]
    pub payload: Option<Value>,
    pub time_limit_secs: u64,
}

impl std::fmt::Debug for StartRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StartRun")
            .field("operation_id", &self.operation_id)
            .field("run_id", &self.run_id)
            .field("event_id", &self.event_id)
            .field("payload", &self.payload.as_ref().map(|_| "[REDACTED]"))
            .field("time_limit_secs", &self.time_limit_secs)
            .finish()
    }
}

/// How a handed-out run ended, as the placement process reports it. The parent treats it as
/// untrusted and stores only values it checked itself.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishedRun {
    pub operation_id: String,
    /// One of [`FINISHED_RUNS`].
    pub run: String,
    /// One of [`FAILURE_CODES`] when the run did not succeed.
    #[serde(default)]
    pub code: Option<String>,
    /// With `invalid_fields`: the names of the refused fields.
    #[serde(default)]
    pub fields: Vec<String>,
    pub started_at: i64,
    pub finished_at: i64,
    /// `{"json": value}` or `{"text": "…"}`, at most [`MAX_OUTPUT_BYTES`] serialised.
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub output_bytes: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub attachments: u32,
}

impl std::fmt::Debug for FinishedRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinishedRun")
            .field("operation_id", &self.operation_id)
            .field("run", &self.run)
            .field("code", &self.code)
            .field("fields", &self.fields.len())
            .field("started_at", &self.started_at)
            .field("finished_at", &self.finished_at)
            .field("output", &self.output.as_ref().map(|_| "[REDACTED]"))
            .field("output_bytes", &self.output_bytes)
            .field("truncated", &self.truncated)
            .field("attachments", &self.attachments)
            .finish()
    }
}

/// The parent's answer to one question of a placement process.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBatch {
    #[serde(default)]
    pub start: Vec<StartRun>,
    /// Operation ids of runs handed to this process whose token is to be cancelled.
    #[serde(default)]
    pub cancel: Vec<String>,
}

/// `run` values of a finished run.
pub const FINISHED_RUNS: [&str; 4] = ["succeeded", "failed", "cancelled", "timed_out"];
/// `code` values of a run that did not succeed.
pub const FAILURE_CODES: [&str; 7] = [
    "flow_failed",
    "invalid_fields",
    "cancelled",
    "timed_out",
    "interrupted",
    "not_started",
    "needs_interaction",
];
/// The largest `output` object, as serialised JSON.
pub const MAX_OUTPUT_BYTES: usize = 8_192;

pub(crate) const MAX_RUNNING: usize = 4;
pub(crate) const MAX_WAITING: usize = 8;
/// The largest journal row of a run, as stored.
pub(crate) const MAX_ROW_BYTES: usize = 4_096;
const WAIT_FOR_INSTANCE: Duration = Duration::from_secs(30);
const REPORT_GRACE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(5);
const KEPT_RESULTS: usize = 256;
const KEPT_FOR: Duration = Duration::from_secs(86_400);
const MAX_FIELD_NAMES: usize = 16;
const MAX_FIELD_NAME_CHARS: usize = 64;
const MAX_FIELD_NAMES_BYTES: usize = 2_048;
const MAX_ATTACHMENTS: u32 = 10_000;
const MAX_REPORTED_BYTES: u64 = 1 << 40;

static CONNECTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A process-wide id for one placement process's channel to the parent.
pub fn connection_id() -> u64 {
    CONNECTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What the management command checked before it queued a run.
pub(crate) struct Admission {
    pub operation_id: String,
    pub run_id: String,
    pub event_id: String,
    pub principal: String,
    pub config_revision: u64,
    pub time_limit_secs: u64,
    pub payload: Option<Value>,
}

impl Admission {
    /// The run as its process gets it, and the parent's record of it while it runs. The
    /// payload leaves the parent's memory with it.
    fn hand_out(self, connection: u64, now: Instant, epoch: i64) -> (StartRun, Running) {
        let start = StartRun {
            operation_id: self.operation_id.clone(),
            run_id: self.run_id.clone(),
            event_id: self.event_id.clone(),
            payload: self.payload,
            time_limit_secs: self.time_limit_secs,
        };
        let running = Running {
            operation_id: self.operation_id,
            run_id: self.run_id,
            event_id: self.event_id,
            principal: self.principal,
            connection,
            started_at: epoch,
            deadline: now + Duration::from_secs(self.time_limit_secs) + REPORT_GRACE,
            cancel: false,
            overdue: false,
        };
        (start, running)
    }
}

struct Waiting {
    admission: Admission,
    queued_at: Instant,
}

struct Running {
    operation_id: String,
    run_id: String,
    event_id: String,
    principal: String,
    connection: u64,
    started_at: i64,
    deadline: Instant,
    cancel: bool,
    /// Ended by the parent's own deadline: its row is final, its place stays taken until its
    /// process reports it or ends.
    overdue: bool,
}

#[derive(Default)]
struct Queue {
    reserved: usize,
    waiting: VecDeque<Waiting>,
    running: Vec<Running>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

/// A run that ended: its final row and, for its issuer's read, its output.
struct Ended {
    state_dir: PathBuf,
    placement_id: String,
    operation_id: String,
    run_id: String,
    principal: String,
    response: Value,
    output: Option<Value>,
    at: Instant,
    written: bool,
}

#[derive(Default)]
struct Registry {
    queues: HashMap<(PathBuf, String), Queue>,
    ended: VecDeque<Ended>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Default::default);

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

fn key(state_dir: &Path, placement_id: &str) -> (PathBuf, String) {
    (state_dir.to_path_buf(), placement_id.to_owned())
}

fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn serialized_len(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// A place in a placement's queue, taken inside the journal transaction of `run_event`. It
/// is given back when it is dropped without [`Reservation::enqueue`], so a transaction that
/// rolls back leaves the queue as it was.
pub(crate) struct Reservation {
    key: (PathBuf, String),
    admitted: bool,
}

/// A place for one more run, or `None` when the service is busy: eight runs wait already,
/// or all four running places are held by runs that never reported their end.
pub(crate) fn reserve(state_dir: &Path, placement_id: &str) -> Option<Reservation> {
    let key = key(state_dir, placement_id);
    let mut registry = registry();
    let queue = registry.queues.entry(key.clone()).or_default();
    let wedged = queue.running.len() >= MAX_RUNNING && queue.running.iter().all(|run| run.overdue);
    if wedged || queue.waiting.len() + queue.reserved >= MAX_WAITING {
        return None;
    }
    queue.reserved += 1;
    Some(Reservation {
        key,
        admitted: false,
    })
}

impl Reservation {
    /// Queues the run once its journal row is committed.
    pub(crate) fn enqueue(mut self, admission: Admission) {
        self.admitted = true;
        let mut registry = registry();
        let queue = registry.queues.entry(self.key.clone()).or_default();
        queue.reserved = queue.reserved.saturating_sub(1);
        queue.waiting.push_back(Waiting {
            admission,
            queued_at: Instant::now(),
        });
        ensure_timer(queue, &self.key);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.admitted
            && let Some(queue) = registry().queues.get_mut(&self.key)
        {
            queue.reserved = queue.reserved.saturating_sub(1);
        }
    }
}

/// Takes the reports of runs that ended in the asking process and answers the runs it is to
/// start and to cancel. The database is opened only to hand a run out (`is_current` is asked
/// then) or to record one that ended; an empty question is answered from memory.
pub fn exchange(
    state_dir: &Path,
    placement_id: &str,
    connection: u64,
    finished: Vec<FinishedRun>,
    is_current: &dyn Fn() -> Result<bool>,
) -> Result<RunBatch> {
    let key = key(state_dir, placement_id);
    let Some(wants_runs) = take_reports(&key, connection, finished) else {
        return Ok(RunBatch::default());
    };
    let start = if wants_runs && is_current()? {
        hand_out(
            &key,
            connection,
            placement_revision(state_dir, placement_id)?,
        )
    } else {
        Vec::new()
    };
    let cancel = registry().queues.get(&key).map_or_else(Vec::new, |queue| {
        queue
            .running
            .iter()
            .filter(|run| run.connection == connection && (run.cancel || run.overdue))
            .map(|run| run.operation_id.clone())
            .collect()
    });
    Ok(RunBatch { start, cancel })
}

/// Records the ends a process reports for runs it was given. `None` when the placement never
/// had a run; else whether runs wait for a free place.
fn take_reports(
    key: &(PathBuf, String),
    connection: u64,
    finished: Vec<FinishedRun>,
) -> Option<bool> {
    let mut ended = Vec::new();
    let wants_runs = {
        let mut registry = registry();
        let queue = registry.queues.get_mut(key)?;
        let now = epoch_now();
        for report in finished {
            let index = queue.running.iter().position(|run| {
                run.connection == connection && run.operation_id == report.operation_id
            });
            if let Some(run) = index.map(|index| queue.running.remove(index))
                && !run.overdue
            {
                ended.push(reported(key, run, report, now));
            }
        }
        !queue.waiting.is_empty() && queue.running.len() < MAX_RUNNING
    };
    finish(&key.0, ended);
    Some(wants_runs)
}

/// Hands waiting runs to `connection` while places are free. A run queued for another config
/// revision than the placement's current one is never run: it ends as `interrupted`.
fn hand_out(key: &(PathBuf, String), connection: u64, revision: Option<u64>) -> Vec<StartRun> {
    let mut start = Vec::new();
    let mut ended = Vec::new();
    {
        let mut registry = registry();
        let Some(queue) = registry.queues.get_mut(key) else {
            return start;
        };
        let (now, epoch) = (Instant::now(), epoch_now());
        while queue.running.len() < MAX_RUNNING
            && let Some(waiting) = queue.waiting.pop_front()
        {
            if revision == Some(waiting.admission.config_revision) {
                let (run, running) = waiting.admission.hand_out(connection, now, epoch);
                start.push(run);
                queue.running.push(running);
            } else {
                ended.push(ended_by_parent(
                    key,
                    &waiting.admission,
                    "interrupted",
                    None,
                ));
            }
        }
    }
    finish(&key.0, ended);
    start
}

/// The channel of a placement process ended: its runs that are still open end as
/// `interrupted`, and the places they held are free.
pub fn connection_closed(state_dir: &Path, placement_id: &str, connection: u64) {
    let key = key(state_dir, placement_id);
    let ended = {
        let mut registry = registry();
        let Some(queue) = registry.queues.get_mut(&key) else {
            return;
        };
        let (closed, kept) = std::mem::take(&mut queue.running)
            .into_iter()
            .partition::<Vec<_>, _>(|run| run.connection == connection);
        queue.running = kept;
        closed
            .into_iter()
            .filter(|run| !run.overdue)
            .map(|run| interrupted(&key, run))
            .collect()
    };
    finish(state_dir, ended);
}

/// Whether the run is still waiting or running, as far as its issuer can tell.
pub(crate) fn is_open(state_dir: &Path, placement_id: &str, operation_id: &str) -> bool {
    registry()
        .queues
        .get(&key(state_dir, placement_id))
        .is_some_and(|queue| {
            queue
                .waiting
                .iter()
                .any(|waiting| waiting.admission.operation_id == operation_id)
                || queue
                    .running
                    .iter()
                    .any(|run| run.operation_id == operation_id && !run.overdue)
        })
}

/// Ends a waiting run at once; a handed-out run gets its cancellation with the next answer to
/// its process, which then reports how it ended. False when the run had already ended.
pub(crate) fn cancel(state_dir: &Path, placement_id: &str, operation_id: &str) -> bool {
    let key = key(state_dir, placement_id);
    let mut ended = Vec::new();
    let open = {
        let mut registry = registry();
        let Some(queue) = registry.queues.get_mut(&key) else {
            return false;
        };
        if let Some(index) = queue
            .waiting
            .iter()
            .position(|waiting| waiting.admission.operation_id == operation_id)
            && let Some(waiting) = queue.waiting.remove(index)
        {
            ended.push(ended_by_parent(&key, &waiting.admission, "cancelled", None));
            true
        } else if let Some(run) = queue
            .running
            .iter_mut()
            .find(|run| run.operation_id == operation_id && !run.overdue)
        {
            run.cancel = true;
            true
        } else {
            false
        }
    };
    finish(state_dir, ended);
    open
}

/// Operation ids whose journal row this process may still change, for the start-up sweep.
pub(crate) fn open_operations(state_dir: &Path) -> Vec<String> {
    let registry = registry();
    let queued = registry
        .queues
        .iter()
        .filter(|((directory, _), _)| directory == state_dir)
        .flat_map(|(_, queue)| {
            queue
                .waiting
                .iter()
                .map(|waiting| waiting.admission.operation_id.clone())
                .chain(queue.running.iter().map(|run| run.operation_id.clone()))
        });
    let unwritten = registry
        .ended
        .iter()
        .filter(|ended| !ended.written && ended.state_dir == state_dir)
        .map(|ended| ended.operation_id.clone());
    queued.chain(unwritten).collect()
}

/// Adds what only memory knows to the stored row of a `run_event` for its issuer: the run's
/// progress while it is open, its output once it ended, or that the output is gone.
pub(crate) fn overlay(state_dir: &Path, principal: &str, response: &mut ManagementResponse) {
    let Some((run_id, placement_id)) = run_of(&response.result) else {
        return;
    };
    let mut registry = registry();
    prune_ended(&mut registry, Instant::now());
    if let Some(ended) = registry.ended.iter().find(|ended| {
        ended.state_dir == state_dir
            && ended.operation_id == response.operation_id
            && ended.run_id == run_id
            && ended.principal == principal
    }) {
        ended.answer(response);
    } else if response.state == "accepted" {
        if let Some(queue) = registry.queues.get(&key(state_dir, &placement_id)) {
            queue.progress(&response.operation_id, &run_id, &mut response.result);
        }
    } else if had_output(&response.result) {
        response.result["output_gone"] = json!(true);
    }
}

/// The run id and placement of a `run_event` row.
fn run_of(result: &Value) -> Option<(String, String)> {
    if result["command"] != "run_event" {
        return None;
    }
    Some((
        result["run_id"].as_str()?.to_owned(),
        result["placement_id"].as_str()?.to_owned(),
    ))
}

/// A succeeded run whose row says it returned something.
fn had_output(result: &Value) -> bool {
    result["run"] == "succeeded"
        && (result["output_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
            || result["truncated"] == true)
}

impl Ended {
    /// The final row as memory holds it, with the output for its issuer.
    fn answer(&self, response: &mut ManagementResponse) {
        if response.state == "accepted" {
            response.state = self.response["state"]
                .as_str()
                .unwrap_or("failed")
                .to_owned();
            response.result = self.response["result"].clone();
        }
        if let Some(output) = &self.output {
            response.result["output"] = output.clone();
        }
    }
}

impl Queue {
    /// Marks an open run `queued`, or `running` with its start.
    fn progress(&self, operation_id: &str, run_id: &str, result: &mut Value) {
        if let Some(run) = self
            .running
            .iter()
            .find(|run| run.operation_id == operation_id && run.run_id == run_id)
        {
            result["run"] = json!("running");
            result["started_at"] = json!(run.started_at);
        } else if self
            .waiting
            .iter()
            .any(|waiting| waiting.admission.operation_id == operation_id)
        {
            result["run"] = json!("queued");
        }
    }

    /// A run waits, or runs without its end recorded.
    fn is_open(&self) -> bool {
        !self.waiting.is_empty() || self.running.iter().any(|run| !run.overdue)
    }

    /// Ends what is past its time: a run nobody took within 30 s as `not_started`, a run
    /// handed out and not reported within its limit plus 60 s as `timed_out`. The latter
    /// keeps its place until its process reports it or ends.
    fn expire(&mut self, key: &(PathBuf, String), now: Instant) -> Vec<Ended> {
        let (expired, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition::<Vec<_>, _>(|waiting| {
                now.saturating_duration_since(waiting.queued_at) >= WAIT_FOR_INSTANCE
            });
        self.waiting = waiting.into();
        let mut ended: Vec<Ended> = expired
            .iter()
            .map(|waiting| ended_by_parent(key, &waiting.admission, "not_started", None))
            .collect();
        for run in self
            .running
            .iter_mut()
            .filter(|run| !run.overdue && run.deadline <= now)
        {
            run.overdue = true;
            ended.push(handed_out_end(key, run, "timed_out"));
        }
        ended
    }
}

/// How a run ended, in the words of its final row.
struct Outcome {
    run: &'static str,
    code: Option<&'static str>,
    fields: Vec<String>,
    started_at: Option<i64>,
    finished_at: i64,
    output_bytes: u64,
    truncated: bool,
    attachments: u32,
}

impl Outcome {
    fn ended(run: &'static str, code: &'static str, started_at: Option<i64>) -> Self {
        Self {
            run,
            code: Some(code),
            fields: Vec::new(),
            started_at,
            finished_at: epoch_now(),
            output_bytes: 0,
            truncated: false,
            attachments: 0,
        }
    }
}

/// The final journal row of a run, built only from values the parent checked.
fn final_response(
    placement_id: &str,
    operation_id: &str,
    run_id: &str,
    event_id: &str,
    outcome: Outcome,
) -> Value {
    let succeeded = outcome.run == "succeeded";
    let mut result = json!({
        "command": "run_event",
        "placement_id": placement_id,
        "event_id": event_id,
        "run_id": run_id,
        "run": outcome.run,
        "finished_at": outcome.finished_at,
    });
    if let Some(started_at) = outcome.started_at {
        result["started_at"] = json!(started_at);
    }
    if let Some(code) = outcome.code {
        result["code"] = json!(code);
    }
    if succeeded {
        result["output_bytes"] = json!(outcome.output_bytes);
        result["truncated"] = json!(outcome.truncated);
        result["attachments"] = json!(outcome.attachments);
    }
    let mut fields = outcome.fields;
    let mut response = json!({
        "operation_id": operation_id,
        "state": if succeeded { "completed" } else { "failed" },
        "result": result,
    });
    if outcome.code == Some("invalid_fields") {
        loop {
            response["result"]["fields"] = json!(fields);
            if serialized_len(&response) <= MAX_ROW_BYTES || fields.pop().is_none() {
                break;
            }
        }
    }
    response
}

fn ended_entry(
    key: &(PathBuf, String),
    identity: [&str; 4],
    outcome: Outcome,
    output: Option<Value>,
) -> Ended {
    let [operation_id, run_id, event_id, principal] = identity;
    Ended {
        state_dir: key.0.clone(),
        placement_id: key.1.clone(),
        operation_id: operation_id.to_owned(),
        run_id: run_id.to_owned(),
        principal: principal.to_owned(),
        response: final_response(&key.1, operation_id, run_id, event_id, outcome),
        output,
        at: Instant::now(),
        written: false,
    }
}

/// A run the parent ends itself: one that never started (`not_started`, `interrupted` by an
/// update, `cancelled`) or one whose process never reported (`timed_out`).
fn ended_by_parent(
    key: &(PathBuf, String),
    admission: &Admission,
    code: &'static str,
    started_at: Option<i64>,
) -> Ended {
    let run = match code {
        "cancelled" => "cancelled",
        "timed_out" => "timed_out",
        _ => "failed",
    };
    ended_entry(
        key,
        [
            &admission.operation_id,
            &admission.run_id,
            &admission.event_id,
            &admission.principal,
        ],
        Outcome::ended(run, code, started_at),
        None,
    )
}

fn handed_out_end(key: &(PathBuf, String), run: &Running, code: &'static str) -> Ended {
    let state = if code == "timed_out" {
        "timed_out"
    } else {
        "failed"
    };
    ended_entry(
        key,
        [
            &run.operation_id,
            &run.run_id,
            &run.event_id,
            &run.principal,
        ],
        Outcome::ended(state, code, Some(run.started_at)),
        None,
    )
}

fn interrupted(key: &(PathBuf, String), run: Running) -> Ended {
    handed_out_end(key, &run, "interrupted")
}

/// The run's end as reported, reduced to the fixed words of its row.
fn reported_end(report: &FinishedRun) -> (&'static str, Option<&'static str>) {
    let code = report
        .code
        .as_deref()
        .and_then(|code| FAILURE_CODES.into_iter().find(|known| *known == code));
    match (report.run.as_str(), code) {
        ("succeeded", _) => ("succeeded", None),
        ("cancelled", _) | (_, Some("cancelled")) => ("cancelled", Some("cancelled")),
        ("timed_out", _) | (_, Some("timed_out")) => ("timed_out", Some("timed_out")),
        (_, Some(code)) => ("failed", Some(code)),
        (_, None) => ("failed", Some("flow_failed")),
    }
}

/// At most 16 names of at most 64 characters, together at most 2 KiB as serialised JSON.
fn bounded_names(names: Vec<String>) -> Vec<String> {
    let mut names: Vec<String> = names
        .into_iter()
        .take(MAX_FIELD_NAMES)
        .map(|name| name.chars().take(MAX_FIELD_NAME_CHARS).collect())
        .collect();
    while serialized_len(&names) > MAX_FIELD_NAMES_BYTES {
        names.pop();
    }
    names
}

/// The bytes `c` takes inside a JSON string as `serde_json` writes it.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `{"text": text}` cut at a character boundary until it fits [`MAX_OUTPUT_BYTES`] as sent.
fn fitted_text(mut text: String) -> (Value, bool) {
    let budget = MAX_OUTPUT_BYTES - r#"{"text":""}"#.len();
    let mut used = 0;
    let mut end = text.len();
    for (index, c) in text.char_indices() {
        used += escaped_len(c);
        if used > budget {
            end = index;
            break;
        }
    }
    let cut = end < text.len();
    text.truncate(end);
    let mut output = json!({ "text": text });
    while serialized_len(&output) > MAX_OUTPUT_BYTES {
        let Some(Value::String(text)) = output.get_mut("text") else {
            break;
        };
        text.pop();
    }
    (output, cut)
}

/// The output of a run as the parent keeps it: `{"json": …}` when it fits, else the text of
/// it cut to fit. Anything else a process reports is dropped and marked truncated.
fn bounded_output(output: Value) -> (Option<Value>, bool) {
    let Value::Object(mut object) = output else {
        return (None, true);
    };
    if object.len() != 1 {
        return (None, true);
    }
    let text = if let Some(value) = object.remove("json") {
        let wrapped = json!({ "json": value });
        if serialized_len(&wrapped) <= MAX_OUTPUT_BYTES {
            return (Some(wrapped), false);
        }
        wrapped["json"].to_string()
    } else if let Some(Value::String(text)) = object.remove("text") {
        text
    } else {
        return (None, true);
    };
    let (output, cut) = fitted_text(text);
    (Some(output), cut)
}

/// A report of the process the run was handed to, rebuilt from checked values: fixed words,
/// times between the hand-out and now, bounded counts and names, an output that fits.
fn reported(key: &(PathBuf, String), run: Running, report: FinishedRun, now: i64) -> Ended {
    let (state, code) = reported_end(&report);
    let started_at = report
        .started_at
        .clamp(run.started_at, now.max(run.started_at));
    let finished_at = report.finished_at.clamp(started_at, now.max(started_at));
    let (output, cut) = match report.output {
        Some(output) if state == "succeeded" => bounded_output(output),
        _ => (None, false),
    };
    let outcome = Outcome {
        run: state,
        code,
        fields: if code == Some("invalid_fields") {
            bounded_names(report.fields)
        } else {
            Vec::new()
        },
        started_at: Some(started_at),
        finished_at,
        output_bytes: report.output_bytes.min(MAX_REPORTED_BYTES),
        truncated: report.truncated || cut,
        attachments: report.attachments.min(MAX_ATTACHMENTS),
    };
    ended_entry(
        key,
        [
            &run.operation_id,
            &run.run_id,
            &run.event_id,
            &run.principal,
        ],
        outcome,
        output,
    )
}

/// Keeps results for 24 hours and at most 256 of them; a row not yet in the journal stays.
fn prune_ended(registry: &mut Registry, now: Instant) {
    registry
        .ended
        .retain(|ended| !ended.written || now.saturating_duration_since(ended.at) < KEPT_FOR);
    while registry.ended.len() > KEPT_RESULTS {
        let Some(index) = registry.ended.iter().position(|ended| ended.written) else {
            break;
        };
        registry.ended.remove(index);
    }
}

/// Keeps the ended runs and writes their final rows, with every other row of `state_dir` the
/// journal still lacks. A row that cannot be written is retried by the queue's timer.
fn finish(state_dir: &Path, ended: Vec<Ended>) {
    if ended.is_empty() {
        return;
    }
    {
        let mut registry = registry();
        registry.ended.extend(ended);
        prune_ended(&mut registry, Instant::now());
    }
    write_pending(state_dir);
}

fn write_pending(state_dir: &Path) {
    let rows: Vec<(String, String, String)> = registry()
        .ended
        .iter()
        .filter(|ended| !ended.written && ended.state_dir == state_dir)
        .map(|ended| {
            (
                ended.operation_id.clone(),
                ended.run_id.clone(),
                ended.response.to_string(),
            )
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    let written = write_rows(state_dir, &rows);
    let mut registry = registry();
    let Registry { queues, ended } = &mut *registry;
    for ended in ended.iter_mut().filter(|ended| {
        ended.state_dir == state_dir
            && rows
                .iter()
                .any(|(operation, run, _)| *operation == ended.operation_id && *run == ended.run_id)
    }) {
        match &written {
            Ok(()) => ended.written = true,
            Err(_) => {
                let key = key(&ended.state_dir, &ended.placement_id);
                let queue = queues.entry(key.clone()).or_default();
                ensure_timer(queue, &key);
            }
        }
    }
    if let Err(error) = written {
        tracing::warn!(
            runs = rows.len(),
            "The end of person-started runs was not recorded yet; retrying: {error:#}"
        );
    }
}

/// Writes final rows over rows that are still open, in one transaction.
fn write_rows(state_dir: &Path, rows: &[(String, String, String)]) -> Result<()> {
    let store = crate::state::StateStore::open(&state_dir.join("management.sqlite"))?;
    let transaction = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    for (operation_id, run_id, response) in rows {
        transaction.execute(
            "UPDATE management_operations SET result_json=?3 WHERE operation_id=?1
                AND json_extract(result_json,'$.state')='accepted'
                AND json_extract(result_json,'$.result.run_id')=?2",
            params![operation_id, run_id, response],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn placement_revision(state_dir: &Path, placement_id: &str) -> Result<Option<u64>> {
    let store = crate::state::StateStore::open(&state_dir.join("management.sqlite"))?;
    let revision: Option<i64> = store
        .connection
        .query_row(
            "SELECT config_revision FROM placements WHERE id=?1",
            [placement_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(revision.and_then(|revision| u64::try_from(revision).ok()))
}

/// Starts the queue's timer when it has none. It needs a runtime; without one (a plain
/// test) the queue is ticked by hand.
fn ensure_timer(queue: &mut Queue, key: &(PathBuf, String)) {
    if queue
        .timer
        .as_ref()
        .is_some_and(|timer| !timer.is_finished())
    {
        return;
    }
    queue.timer = tokio::runtime::Handle::try_current().ok().map(|runtime| {
        let key = key.clone();
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(TICK).await;
                let (key, now) = (key.clone(), Instant::now());
                if !matches!(
                    tokio::task::spawn_blocking(move || tick(&key, now)).await,
                    Ok(true)
                ) {
                    break;
                }
            }
        })
    });
}

/// One pass of a queue's timer: it ends what is past its time and retries final rows the
/// journal refused. False once nothing is left to watch; the timer then ends.
fn tick(key: &(PathBuf, String), now: Instant) -> bool {
    let Some(ended) = registry()
        .queues
        .get_mut(key)
        .map(|queue| queue.expire(key, now))
    else {
        return false;
    };
    if ended.is_empty() {
        write_pending(&key.0);
    } else {
        finish(&key.0, ended);
    }
    let mut registry = registry();
    let pending = registry
        .ended
        .iter()
        .any(|ended| !ended.written && ended.state_dir == key.0 && ended.placement_id == key.1);
    let Some(queue) = registry.queues.get_mut(key) else {
        return false;
    };
    let watching = pending || queue.is_open();
    if !watching {
        queue.timer = None;
    }
    watching
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateStore;

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let temp = tempfile::tempdir()?;
            let root = temp.path().canonicalize()?;
            let store = StateStore::open(&root.join("management.sqlite"))?;
            store.connection.execute(
                "INSERT INTO placements(id,config_json,desired_state,intent_revision,observed_state,config_revision) VALUES('notes','{}','running',1,'running',7)",
                [],
            )?;
            Ok(Self { _temp: temp, root })
        }

        fn store(&self) -> Result<StateStore> {
            StateStore::open(&self.root.join("management.sqlite"))
        }

        /// Journals and queues one run the way `run_event` does.
        fn queue(&self, operation_id: &str) -> Result<()> {
            let reservation =
                reserve(&self.root, "notes").ok_or_else(|| anyhow::anyhow!("busy"))?;
            let run_id = format!("run-{operation_id}");
            let row = json!({"operation_id":operation_id,"state":"accepted","result":{"command":"run_event","placement_id":"notes","event_id":"evt_form","run_id":run_id,"run":"queued"}});
            self.store()?.connection.execute(
                "INSERT INTO management_operations(operation_id,request_digest,principal,project_id,placement_id,accepted_at,result_json) VALUES(?1,'digest','owner-user:owner','project','notes',100,?2)",
                params![operation_id, row.to_string()],
            )?;
            reservation.enqueue(Admission {
                operation_id: operation_id.into(),
                run_id,
                event_id: "evt_form".into(),
                principal: "owner-user:owner".into(),
                config_revision: 7,
                time_limit_secs: 30,
                payload: Some(json!({"title":"secret words"})),
            });
            Ok(())
        }

        fn row(&self, operation_id: &str) -> Result<Value> {
            let row: String = self.store()?.connection.query_row(
                "SELECT result_json FROM management_operations WHERE operation_id=?1",
                [operation_id],
                |row| row.get(0),
            )?;
            Ok(serde_json::from_str(&row)?)
        }

        fn ask(&self, connection: u64, finished: Vec<FinishedRun>) -> Result<RunBatch> {
            exchange(&self.root, "notes", connection, finished, &|| Ok(true))
        }

        fn read(&self, operation_id: &str) -> Result<ManagementResponse> {
            let mut response: ManagementResponse = serde_json::from_value(self.row(operation_id)?)?;
            overlay(&self.root, "owner-user:owner", &mut response);
            Ok(response)
        }

        fn key(&self) -> (PathBuf, String) {
            key(&self.root, "notes")
        }
    }

    fn report(operation_id: &str, run: &str) -> FinishedRun {
        FinishedRun {
            operation_id: operation_id.into(),
            run: run.into(),
            code: None,
            fields: Vec::new(),
            started_at: 0,
            finished_at: 0,
            output: None,
            output_bytes: 0,
            truncated: false,
            attachments: 0,
        }
    }

    #[test]
    fn the_thirteenth_run_is_busy_and_a_rolled_back_place_is_given_back() -> Result<()> {
        let fixture = Fixture::new()?;
        for index in 0..4 {
            fixture.queue(&format!("op-{index}"))?;
        }
        assert_eq!(fixture.ask(1, Vec::new())?.start.len(), 4);
        for index in 4..12 {
            fixture.queue(&format!("op-{index}"))?;
        }
        assert!(reserve(&fixture.root, "notes").is_none());
        let other = Fixture::new()?;
        for index in 0..8 {
            other.queue(&format!("op-{index}"))?;
        }
        assert!(reserve(&other.root, "notes").is_none(), "eight waiting");
        let fresh = Fixture::new()?;
        let places: Vec<_> = (0..8)
            .map(|_| reserve(&fresh.root, "notes"))
            .collect::<Option<_>>()
            .unwrap();
        assert!(reserve(&fresh.root, "notes").is_none());
        drop(places);
        assert!(reserve(&fresh.root, "notes").is_some());
        Ok(())
    }

    #[test]
    fn a_run_goes_to_the_first_current_asker_once_without_its_payload_in_debug() -> Result<()> {
        let fixture = Fixture::new()?;
        assert_eq!(fixture.ask(9, Vec::new())?, RunBatch::default());
        fixture.queue("op-1")?;
        let stale = exchange(&fixture.root, "notes", 1, Vec::new(), &|| Ok(false))?;
        assert!(stale.start.is_empty());
        let batch = fixture.ask(2, Vec::new())?;
        assert_eq!(batch.start.len(), 1);
        let run = &batch.start[0];
        assert_eq!(
            (
                run.operation_id.as_str(),
                run.run_id.as_str(),
                run.event_id.as_str()
            ),
            ("op-1", "run-op-1", "evt_form")
        );
        assert_eq!(run.payload, Some(json!({"title":"secret words"})));
        assert_eq!(run.time_limit_secs, 30);
        assert!(!format!("{batch:?}").contains("secret"));
        assert!(fixture.ask(3, Vec::new())?.start.is_empty());
        assert!(fixture.ask(2, Vec::new())?.start.is_empty());
        let read = fixture.read("op-1")?;
        assert_eq!(read.state, "accepted");
        assert_eq!(read.result["run"], "running");
        assert!(read.result["started_at"].is_i64());
        Ok(())
    }

    #[test]
    fn nobody_asking_within_thirty_seconds_ends_the_run_as_not_started() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        assert_eq!(fixture.read("op-1")?.result["run"], "queued");
        let queued_at = Instant::now();
        assert!(tick(&fixture.key(), queued_at + Duration::from_secs(29)));
        assert_eq!(fixture.row("op-1")?["state"], "accepted");
        assert!(!tick(&fixture.key(), queued_at + Duration::from_secs(31)));
        let row = fixture.row("op-1")?;
        assert_eq!(row["state"], "failed");
        assert_eq!(row["result"]["run"], "failed");
        assert_eq!(row["result"]["code"], "not_started");
        assert!(row["result"].get("started_at").is_none());
        assert!(fixture.ask(1, Vec::new())?.start.is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn the_timer_ends_an_untaken_run_with_nobody_reading() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        tokio::time::sleep(Duration::from_secs(36)).await;
        assert_eq!(fixture.row("op-1")?["result"]["code"], "not_started");
        let timer = registry()
            .queues
            .get(&fixture.key())
            .map(|queue| queue.timer.is_none());
        assert_eq!(timer, Some(true), "the timer ends with the last open run");
        Ok(())
    }

    #[test]
    fn an_unreported_run_times_out_and_keeps_its_place_until_its_connection_ends() -> Result<()> {
        let fixture = Fixture::new()?;
        for index in 0..4 {
            fixture.queue(&format!("op-{index}"))?;
        }
        assert_eq!(fixture.ask(1, Vec::new())?.start.len(), 4);
        let handed_at = Instant::now();
        assert!(tick(&fixture.key(), handed_at + Duration::from_secs(89)));
        assert_eq!(fixture.row("op-0")?["state"], "accepted");
        assert!(!tick(&fixture.key(), handed_at + Duration::from_secs(91)));
        let row = fixture.row("op-0")?;
        assert_eq!(
            (&row["state"], &row["result"]["run"], &row["result"]["code"]),
            (&json!("failed"), &json!("timed_out"), &json!("timed_out"))
        );
        assert!(row["result"]["started_at"].is_i64());
        assert!(reserve(&fixture.root, "notes").is_none(), "four hung runs");
        let batch = fixture.ask(1, vec![report("op-0", "succeeded")])?;
        assert_eq!(batch.cancel.len(), 3, "overdue runs are cancelled");
        assert_eq!(fixture.row("op-0")?["result"]["run"], "timed_out");
        assert!(reserve(&fixture.root, "notes").is_some());
        connection_closed(&fixture.root, "notes", 1);
        let registry = registry();
        let queue = registry.queues.get(&fixture.key()).unwrap();
        assert!(queue.running.is_empty());
        Ok(())
    }

    #[test]
    fn cancel_ends_a_waiting_run_and_reaches_a_handed_out_one() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.queue("op-2")?;
        assert!(cancel(&fixture.root, "notes", "op-2"));
        let row = fixture.row("op-2")?;
        assert_eq!(
            (&row["state"], &row["result"]["run"], &row["result"]["code"]),
            (&json!("failed"), &json!("cancelled"), &json!("cancelled"))
        );
        assert!(!cancel(&fixture.root, "notes", "op-2"));
        let batch = fixture.ask(1, Vec::new())?;
        assert_eq!(batch.start.len(), 1);
        assert!(batch.cancel.is_empty());
        assert!(is_open(&fixture.root, "notes", "op-1"));
        assert!(cancel(&fixture.root, "notes", "op-1"));
        assert_eq!(fixture.ask(2, Vec::new())?.cancel, Vec::<String>::new());
        assert_eq!(fixture.ask(1, Vec::new())?.cancel, vec!["op-1".to_owned()]);
        let mut cancelled = report("op-1", "cancelled");
        cancelled.code = Some("cancelled".into());
        assert!(fixture.ask(1, vec![cancelled])?.cancel.is_empty());
        assert_eq!(fixture.row("op-1")?["result"]["run"], "cancelled");
        assert!(!is_open(&fixture.root, "notes", "op-1"));
        assert!(!cancel(&fixture.root, "notes", "op-1"));
        Ok(())
    }

    #[test]
    fn a_finished_run_keeps_its_output_in_memory_and_its_row_on_disk_without_it() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.ask(1, Vec::new())?;
        let mut finished = report("op-1", "succeeded");
        finished.output = Some(json!({"json":{"id":42}}));
        finished.output_bytes = 9;
        finished.started_at = i64::MIN;
        finished.finished_at = i64::MAX;
        fixture.ask(1, vec![finished])?;
        let row = fixture.row("op-1")?;
        assert_eq!(row["state"], "completed");
        assert_eq!(row["result"]["run"], "succeeded");
        assert!(row["result"].get("output").is_none());
        assert_eq!(row["result"]["output_bytes"], 9);
        let (started, finished) = (
            row["result"]["started_at"].as_i64().unwrap(),
            row["result"]["finished_at"].as_i64().unwrap(),
        );
        assert!(started > 0 && started <= finished && finished <= epoch_now());
        let read = fixture.read("op-1")?;
        assert_eq!(read.result["output"], json!({"json":{"id":42}}));
        let mut other = read.clone();
        other.result.as_object_mut().unwrap().remove("output");
        overlay(&fixture.root, "someone-else", &mut other);
        assert!(other.result.get("output").is_none());
        assert_eq!(other.result["output_gone"], true);
        Ok(())
    }

    #[test]
    fn oversized_and_hostile_reports_are_stored_as_bounded_known_values() -> Result<()> {
        let fixture = Fixture::new()?;
        for index in 0..4 {
            fixture.queue(&format!("op-{index}"))?;
        }
        fixture.ask(1, Vec::new())?;
        let hostile = "\"\\\u{1}".repeat(3_000);
        let mut huge = report("op-0", "succeeded");
        huge.output = Some(json!({ "text": hostile }));
        huge.output_bytes = u64::MAX;
        huge.attachments = u32::MAX;
        let mut fields = report("op-1", "failed");
        fields.code = Some("invalid_fields".into());
        fields.fields = (0..100).map(|_| "\u{1}é\"".repeat(200)).collect();
        let mut unknown = report("op-2", "exploded");
        unknown.code = Some("<script>".into());
        unknown.started_at = -5;
        let mut json_output = report("op-3", "succeeded");
        json_output.output = Some(json!({"json": "\\".repeat(9_000)}));
        fixture.ask(1, vec![huge, fields, unknown, json_output])?;

        let huge = fixture.read("op-0")?;
        let output = &huge.result["output"];
        assert!(serialized_len(output) <= MAX_OUTPUT_BYTES);
        assert!(output["text"].as_str().unwrap().starts_with("\"\\\u{1}"));
        assert_eq!(huge.result["truncated"], true);
        assert_eq!(huge.result["output_bytes"], MAX_REPORTED_BYTES);
        assert_eq!(huge.result["attachments"], MAX_ATTACHMENTS);
        assert!(serialized_len(&huge) <= 13 * 1024);

        let fields = fixture.row("op-1")?;
        let names = fields["result"]["fields"].as_array().unwrap();
        assert!(!names.is_empty() && names.len() <= MAX_FIELD_NAMES);
        assert!(
            names
                .iter()
                .all(|name| name.as_str().unwrap().chars().count() <= MAX_FIELD_NAME_CHARS)
        );
        assert!(serialized_len(&fields["result"]["fields"]) <= MAX_FIELD_NAMES_BYTES);

        let unknown = fixture.row("op-2")?;
        assert_eq!(
            (&unknown["result"]["run"], &unknown["result"]["code"]),
            (&json!("failed"), &json!("flow_failed"))
        );
        assert!(unknown["result"]["started_at"].as_i64().unwrap() > 0);

        let json_output = fixture.read("op-3")?;
        assert_eq!(json_output.result["truncated"], true);
        assert!(json_output.result["output"]["text"].is_string());
        assert!(serialized_len(&json_output.result["output"]) <= MAX_OUTPUT_BYTES);

        for id in ["op-0", "op-1", "op-2", "op-3"] {
            assert!(fixture.row(id)?.to_string().len() <= MAX_ROW_BYTES, "{id}");
        }
        Ok(())
    }

    #[test]
    fn a_final_row_the_journal_refused_is_answered_from_memory_and_written_later() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.ask(1, Vec::new())?;
        let mut stored: ManagementResponse = serde_json::from_value(fixture.row("op-1")?)?;
        let database = fixture.root.join("management.sqlite");
        let moved = fixture.root.join("moved");
        std::fs::create_dir(&moved)?;
        let files = [
            "management.sqlite",
            "management.sqlite-wal",
            "management.sqlite-shm",
        ];
        for file in files {
            if fixture.root.join(file).exists() {
                std::fs::rename(fixture.root.join(file), moved.join(file))?;
            }
        }
        std::fs::create_dir(&database)?;
        let mut done = report("op-1", "succeeded");
        done.output = Some(json!({"json": 1}));
        done.output_bytes = 1;
        fixture.ask(1, vec![done])?;
        overlay(&fixture.root, "owner-user:owner", &mut stored);
        assert_eq!(
            (stored.state.as_str(), &stored.result["output"]),
            ("completed", &json!({"json": 1}))
        );
        assert!(
            tick(&fixture.key(), Instant::now()),
            "an unwritten row keeps the timer"
        );
        std::fs::remove_dir(&database)?;
        for file in files {
            if moved.join(file).exists() {
                std::fs::rename(moved.join(file), fixture.root.join(file))?;
            }
        }
        assert!(!tick(&fixture.key(), Instant::now()));
        assert_eq!(fixture.row("op-1")?["state"], "completed");
        Ok(())
    }

    #[test]
    fn reports_of_runs_another_connection_holds_are_ignored() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.ask(1, Vec::new())?;
        fixture.ask(2, vec![report("op-1", "succeeded")])?;
        fixture.ask(1, vec![report("op-9", "succeeded")])?;
        assert_eq!(fixture.row("op-1")?["state"], "accepted");
        assert_eq!(fixture.read("op-1")?.result["run"], "running");
        Ok(())
    }

    #[test]
    fn a_closed_connection_interrupts_its_runs_and_leaves_waiting_ones() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.ask(1, Vec::new())?;
        fixture.queue("op-2")?;
        connection_closed(&fixture.root, "notes", 1);
        let row = fixture.row("op-1")?;
        assert_eq!(
            (&row["state"], &row["result"]["run"], &row["result"]["code"]),
            (&json!("failed"), &json!("failed"), &json!("interrupted"))
        );
        assert_eq!(fixture.read("op-2")?.result["run"], "queued");
        assert_eq!(fixture.ask(5, Vec::new())?.start.len(), 1);
        Ok(())
    }

    #[test]
    fn a_run_queued_before_an_update_is_never_handed_to_the_new_revision() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.store()?.connection.execute(
            "UPDATE placements SET config_revision=8 WHERE id='notes'",
            [],
        )?;
        assert!(fixture.ask(1, Vec::new())?.start.is_empty());
        assert_eq!(fixture.row("op-1")?["result"]["code"], "interrupted");
        Ok(())
    }

    #[test]
    fn results_are_kept_for_a_day_and_at_most_256_and_gone_after_that() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.queue("op-1")?;
        fixture.ask(1, Vec::new())?;
        let mut finished = report("op-1", "succeeded");
        finished.output = Some(json!({"text":"done"}));
        finished.output_bytes = 4;
        fixture.ask(1, vec![finished])?;
        assert_eq!(fixture.read("op-1")?.result["output"]["text"], "done");
        registry()
            .ended
            .retain(|ended| ended.state_dir != fixture.root);
        let read = fixture.read("op-1")?;
        assert!(read.result.get("output").is_none());
        assert_eq!(read.result["output_gone"], true);

        let entry = |index: usize, at: Instant, written: bool| Ended {
            state_dir: fixture.root.clone(),
            placement_id: "notes".into(),
            operation_id: format!("kept-{index}"),
            run_id: format!("run-{index}"),
            principal: "owner-user:owner".into(),
            response: json!({}),
            output: None,
            at,
            written,
        };
        let now = Instant::now();
        let mut kept = Registry::default();
        kept.ended.push_back(entry(0, now, false));
        for index in 1..KEPT_RESULTS + 8 {
            kept.ended.push_back(entry(index, now, true));
        }
        prune_ended(&mut kept, now);
        assert_eq!(kept.ended.len(), KEPT_RESULTS);
        assert_eq!(kept.ended[0].operation_id, "kept-0", "unwritten rows stay");
        assert_eq!(kept.ended[1].operation_id, "kept-9");
        prune_ended(&mut kept, now + KEPT_FOR);
        assert_eq!(kept.ended.len(), 1);
        Ok(())
    }
}

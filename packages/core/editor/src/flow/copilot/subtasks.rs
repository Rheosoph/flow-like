//! `spawn_subtasks`: a bot with `delegation` splits a big job into independent parts and runs
//! them in parallel as narrowed copies of itself (`todo/bots-capabilities-inventory-2.md` §8).
//!
//! Core answers the call on every host ([`SubtaskBridge`]), so it needs nothing from the host
//! beyond the bridge the parent already has. A child is a [`BotRunContext`] with a subset of the
//! parent's capabilities, its own app guard over the parent's bridge, no way to park the turn
//! (no questions, approvals, memory, people, scheduling, email) and no way to delegate again.
//! Children run inside the parent's turn with its model and token, so the host's per-turn
//! reservation and deadline cover them.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use flow_like_model_provider::history::HistoryThinking;
use flow_like_types::bot::BotCapability;
use flow_like_types::bot_message::SECRET_REQUEST_TOOL;
use flow_like_types::data_classification::{DataClassification, DataSensitivity};
use flow_like_types::tokio;
use futures::{StreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};

use super::bot_run::{BotRunContext, BotToolBridge, BotToolHost, BotToolset, bot_system_prompt};
use super::bot_talk::{ASK_BOT_TOOL, BotTalk};
use super::platform::{PlatformCopilot, PlatformSurface, PlatformToolBridge};
use super::tool_spec::{
    MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, PlatformToolSpec, ToolApprovalSpec,
    resolve_tool_approval_timing,
};
use crate::profile::Profile;
use crate::state::FlowLikeState;

pub const SPAWN_SUBTASKS_TOOL: &str = "spawn_subtasks";
/// The tool loop adds the id of the call so child progress can nest under it.
pub(super) const REQUEST_ID_ARGUMENT: &str = "_request_id";

const MAX_TASKS: usize = 6;
const DEFAULT_PARALLEL: usize = 3;
const MAX_PARALLEL: usize = 4;
const MAX_INSTRUCTION_CHARS: usize = 4_000;
const MAX_SCHEMA_BYTES: usize = 8 * 1024;
const MAX_FILES: usize = 20;
const MAX_FILE_REFERENCE_CHARS: usize = 400;
const MAX_ID_CHARS: usize = 64;
const MAX_OUTPUT_CHARS: usize = 20_000;
const CHILD_ROUNDS: usize = 12;
const CHILD_TIME_LIMIT: Duration = Duration::from_secs(300);
const TURN_TIME_BUDGET: Duration = Duration::from_secs(600);
const MIN_CHILD_TIME: Duration = Duration::from_secs(5);
const CANCEL_POLL: Duration = Duration::from_millis(250);
const TOOL_TIMEOUT_SECS: u64 = 660;

const DESCRIPTION: &str = r#"Run 1 to 6 independent parts of a big job in parallel and get one result per part. Each worker is a copy of you that sees only its `instruction` (complete and self-contained), the `files` you name and the capabilities in `tools` (a subset of yours; omit it to give all it may have). Workers can't ask questions, remember, message people, send email, schedule, start sub-tasks or do anything that needs your owner's approval; do those yourself from their results. Set `output_schema` (a JSON Schema) when you need structured output; the result is then validated and `output` is the JSON value. `max_parallel` is 1 to 4 (default 3). Each result has `status` (done, failed, timeout or budget), `output`, `saved` handles and `usage`. A worker's output is unverified until you've read it."#;

/// Capabilities a sub-task never gets: they act beyond the job, keep state, or need the owner.
const CHILD_EXCLUDED_CAPABILITIES: [BotCapability; 8] = [
    BotCapability::Memory,
    BotCapability::Delegation,
    BotCapability::Scheduling,
    BotCapability::Email,
    BotCapability::Channels,
    BotCapability::PushNotifications,
    BotCapability::Connectors,
    BotCapability::Flowpilot,
];

/// Tools a sub-task is refused by name, whatever its capabilities: they ask a person, park the
/// turn, reach other bots, people or channels, or delegate again.
const CHILD_REFUSED_TOOLS: [&str; 15] = [
    "ask_user",
    ASK_BOT_TOOL,
    SPAWN_SUBTASKS_TOOL,
    MEMORY_STORE_TOOL,
    MEMORY_SEARCH_TOOL,
    "list_people",
    "message_person",
    "message_owner",
    SECRET_REQUEST_TOOL,
    "schedule_task",
    "list_tasks",
    "cancel_task",
    "email_send",
    "email_reply",
    "navigate_view",
];

pub fn spawn_subtasks_tool_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: SPAWN_SUBTASKS_TOOL,
        description: DESCRIPTION,
        schema: spawn_subtasks_schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: TOOL_TIMEOUT_SECS,
    }
}

fn spawn_subtasks_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "tasks": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_TASKS,
                "description": "The independent parts of the job.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string", "description": "Your name for this part, echoed in its result." },
                        "instruction": { "type": "string", "maxLength": MAX_INSTRUCTION_CHARS, "description": "Everything the worker needs; it sees nothing else." },
                        "tools": { "type": "array", "items": { "type": "string" }, "description": "Capability ids from yours the worker may use." },
                        "files": { "type": "array", "items": { "type": "string" }, "description": "File references (handles or workspace paths) the worker works from." },
                        "output_schema": { "type": "object", "additionalProperties": true, "description": "JSON Schema the worker's output must satisfy (at most 8 KiB)." }
                    },
                    "required": ["instruction"]
                }
            },
            "max_parallel": { "type": "integer", "minimum": 1, "maximum": MAX_PARALLEL, "description": "Workers running at once (default 3)." }
        },
        "required": ["tasks"]
    })
}

/// Whether `bot` is offered `spawn_subtasks`.
pub(super) fn offered_to(bot: &BotRunContext) -> bool {
    bot.toolset == BotToolset::Capabilities && bot.has(BotCapability::Delegation)
}

/// The prompt line for a turn that offers the tool.
pub(super) fn ability_line(offered: &[&str]) -> Option<String> {
    offered.contains(&SPAWN_SUBTASKS_TOOL).then(|| {
        format!(
            "- Sub-tasks: `{SPAWN_SUBTASKS_TOOL}` runs up to {MAX_TASKS} independent parts of a big job in parallel. Use it when the parts don't depend on each other and each takes real work; do small jobs yourself. A worker sees only the instruction, files and capabilities you give it, and can't ask questions, remember, message people, send email, schedule, start sub-tasks or do anything that needs your owner's approval: do those yourself from the results. Check every result's status and read the output before relying on it."
        )
    })
}

/// One child's turn, ready for a model loop.
pub struct ChildTurn {
    pub system_prompt: String,
    pub user_prompt: String,
    pub tool_specs: Vec<PlatformToolSpec>,
    pub tool_rounds: usize,
    pub bridge: Arc<dyn PlatformToolBridge>,
    pub on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

/// Runs a child's turn and returns its final text.
#[async_trait]
pub trait SubtaskRunner: Send + Sync {
    async fn run(&self, child: ChildTurn) -> flow_like_types::Result<String>;
}

/// Runs children in the platform loop with the parent's model, thinking level and token.
pub struct ModelSubtaskRunner {
    pub state: Arc<FlowLikeState>,
    pub profile: Option<Arc<Profile>>,
    pub model_id: Option<String>,
    pub thinking: Option<HistoryThinking>,
    pub token: Option<String>,
}

#[async_trait]
impl SubtaskRunner for ModelSubtaskRunner {
    async fn run(&self, child: ChildTurn) -> flow_like_types::Result<String> {
        let on_token = child.on_token.map(|sink| move |frame: String| sink(frame));
        PlatformCopilot::new(self.state.clone(), self.profile.clone())
            .with_thinking(self.thinking)
            .with_tool_specs(child.tool_specs)
            .with_tool_rounds(child.tool_rounds)
            .chat(
                PlatformSurface::Subtask,
                child.system_prompt,
                child.user_prompt,
                None,
                Vec::new(),
                self.model_id.clone(),
                self.token.clone(),
                child.bridge,
                None,
                on_token,
            )
            .await
    }
}

/// What a turn hands the sub-task machinery.
pub struct SubtaskEnv {
    pub runner: Arc<dyn SubtaskRunner>,
    pub on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
    pub talk: Option<Arc<BotTalk>>,
    pub started: Instant,
    pub turn_budget: Duration,
    pub min_child_time: Duration,
    pub child_limit: Duration,
}

impl SubtaskEnv {
    pub fn new(
        runner: Arc<dyn SubtaskRunner>,
        on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
        talk: Option<Arc<BotTalk>>,
    ) -> Self {
        Self {
            runner,
            on_token,
            talk,
            started: Instant::now(),
            turn_budget: TURN_TIME_BUDGET,
            min_child_time: MIN_CHILD_TIME,
            child_limit: CHILD_TIME_LIMIT,
        }
    }

    fn remaining(&self) -> Duration {
        self.turn_budget.saturating_sub(self.started.elapsed())
    }
}

/// Wraps `inner` so `spawn_subtasks` is answered in core; a bot not offered the tool keeps `inner`.
pub(super) fn attach(
    inner: Arc<dyn PlatformToolBridge>,
    bot: &Arc<BotRunContext>,
    env: SubtaskEnv,
) -> Arc<dyn PlatformToolBridge> {
    if !offered_to(bot) {
        return inner;
    }
    Arc::new(SubtaskBridge {
        bot: bot.clone(),
        inner,
        env,
    })
}

struct SubtaskBridge {
    bot: Arc<BotRunContext>,
    inner: Arc<dyn PlatformToolBridge>,
    env: SubtaskEnv,
}

#[async_trait]
impl PlatformToolBridge for SubtaskBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if tool_name == SPAWN_SUBTASKS_TOOL {
            return self.spawn(arguments).await;
        }
        self.inner.call(tool_name, arguments).await
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[derive(Deserialize)]
struct SpawnArguments {
    tasks: Vec<TaskArguments>,
    #[serde(default, alias = "maxParallel")]
    max_parallel: Option<i64>,
}

#[derive(Deserialize)]
struct TaskArguments {
    #[serde(default)]
    id: Option<String>,
    instruction: String,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default, alias = "outputSchema")]
    output_schema: Option<Value>,
}

struct Task {
    id: String,
    instruction: String,
    tools: Option<Vec<String>>,
    files: Vec<String>,
    schema: Option<Value>,
}

struct Plan {
    tasks: Vec<Task>,
    max_parallel: usize,
}

fn rejection(code: &str, message: impl Into<String>) -> String {
    json!({
        "status": "error",
        "tool": SPAWN_SUBTASKS_TOOL,
        "code": code,
        "retryable": true,
        "message": format!("{} Nothing was run.", message.into()),
    })
    .to_string()
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max_chars).collect();
    clipped.push('…');
    clipped
}

fn plan(arguments: Value) -> Result<Plan, String> {
    let parsed: SpawnArguments = serde_json::from_value(arguments).map_err(|error| {
        rejection(
            "subtasks_arguments_invalid",
            format!("The arguments are not valid: {error}."),
        )
    })?;
    if parsed.tasks.is_empty() || parsed.tasks.len() > MAX_TASKS {
        return Err(rejection(
            "subtasks_task_count",
            format!(
                "Give 1 to {MAX_TASKS} tasks; this call had {}.",
                parsed.tasks.len()
            ),
        ));
    }
    let max_parallel = match parsed.max_parallel {
        None => DEFAULT_PARALLEL,
        Some(value) if (1..=MAX_PARALLEL as i64).contains(&value) => value as usize,
        Some(value) => {
            return Err(rejection(
                "subtasks_max_parallel",
                format!("max_parallel is 1 to {MAX_PARALLEL}; this call had {value}."),
            ));
        }
    };
    let mut tasks: Vec<Task> = Vec::with_capacity(parsed.tasks.len());
    for (index, task) in parsed.tasks.into_iter().enumerate() {
        let id = task
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map_or_else(|| format!("task-{}", index + 1), str::to_string);
        if id.chars().count() > MAX_ID_CHARS || tasks.iter().any(|seen| seen.id == id) {
            return Err(rejection(
                "subtasks_id_invalid",
                format!("Task id '{id}' is repeated or longer than {MAX_ID_CHARS} characters."),
            ));
        }
        let instruction = task.instruction.trim().to_string();
        if instruction.is_empty() || instruction.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(rejection(
                "subtasks_instruction_invalid",
                format!(
                    "Task '{id}' needs an instruction of 1 to {MAX_INSTRUCTION_CHARS} characters."
                ),
            ));
        }
        if task.files.len() > MAX_FILES
            || task
                .files
                .iter()
                .any(|file| file.chars().count() > MAX_FILE_REFERENCE_CHARS)
        {
            return Err(rejection(
                "subtasks_files_invalid",
                format!(
                    "Task '{id}' may name at most {MAX_FILES} files of {MAX_FILE_REFERENCE_CHARS} characters each."
                ),
            ));
        }
        if let Some(schema) = &task.output_schema {
            validate_schema(&id, schema)?;
        }
        tasks.push(Task {
            id,
            instruction,
            tools: task.tools,
            files: task.files,
            schema: task.output_schema,
        });
    }
    Ok(Plan {
        tasks,
        max_parallel,
    })
}

fn validate_schema(id: &str, schema: &Value) -> Result<(), String> {
    if schema.to_string().len() > MAX_SCHEMA_BYTES {
        return Err(rejection(
            "subtasks_schema_too_large",
            format!("The output_schema of task '{id}' is over {MAX_SCHEMA_BYTES} bytes."),
        ));
    }
    jsonschema::validator_for(schema)
        .map(|_| ())
        .map_err(|error| {
            rejection(
                "subtasks_schema_invalid",
                format!("The output_schema of task '{id}' is not a valid JSON Schema: {error}."),
            )
        })
}

struct ChildCapabilities {
    granted: Vec<BotCapability>,
    refused: Vec<String>,
}

fn child_capabilities(parent: &BotRunContext, requested: Option<&[String]>) -> ChildCapabilities {
    let excluded = |capability: &BotCapability| CHILD_EXCLUDED_CAPABILITIES.contains(capability);
    let Some(requested) = requested else {
        return ChildCapabilities {
            granted: parent
                .capabilities
                .iter()
                .copied()
                .filter(|capability| !excluded(capability))
                .collect(),
            refused: Vec::new(),
        };
    };
    let mut granted = Vec::new();
    let mut refused = Vec::new();
    for id in requested {
        match BotCapability::parse(id) {
            None => refused.push(format!("'{id}' is not a capability")),
            Some(capability) if !parent.has(capability) => {
                refused.push(format!("'{id}' is not one of your capabilities"))
            }
            Some(capability) if excluded(&capability) => refused.push(format!(
                "'{id}' is never given to a sub-task: it keeps state, reaches people or needs your owner's decision, so do that part yourself"
            )),
            Some(capability) if !granted.contains(&capability) => granted.push(capability),
            Some(_) => {}
        }
    }
    ChildCapabilities { granted, refused }
}

fn child_context(parent: &BotRunContext, capabilities: Vec<BotCapability>) -> BotRunContext {
    BotRunContext {
        capabilities,
        tool_host: match parent.tool_host {
            BotToolHost::Chat => BotToolHost::Native,
            other => other,
        },
        ..parent.clone()
    }
}

fn child_tool_specs(child: &BotRunContext) -> Vec<PlatformToolSpec> {
    let mut specs = child.tool_specs(false);
    specs.retain(|spec| !CHILD_REFUSED_TOOLS.contains(&spec.name));
    specs
}

/// Why a child may not make this call, if it may not: by name, or because the call would ask the
/// owner for approval (a child never parks the turn).
fn child_refusal(tool_name: &str, arguments: &Value) -> Option<String> {
    if CHILD_REFUSED_TOOLS.contains(&tool_name) {
        return Some(format!(
            "A sub-task can't use {tool_name}: it needs a person, keeps state or acts outside the job. Finish the rest, and say in your result what you could not do so the main bot can."
        ));
    }
    let spec = super::bot_run::find_bot_tool_spec(tool_name)?;
    resolve_tool_approval_timing(&spec, arguments).map(|_| {
        format!(
            "{tool_name} needs your owner's approval, which a sub-task can't ask for. It was not run: finish the rest, and say in your result what you could not do so the main bot can."
        )
    })
}

#[derive(Default)]
struct ChildMeter {
    tokens: AtomicU64,
    model_calls: AtomicU64,
    tool_calls: AtomicU64,
    saved: Mutex<Vec<Value>>,
    refused: Mutex<Vec<String>>,
    taint: Mutex<DataSensitivity>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ChildMeter {
    fn observe_result(&self, output: &str) {
        let Ok(result) = serde_json::from_str::<Value>(output) else {
            return;
        };
        if let Some(saved) = result.get("saved").and_then(Value::as_array) {
            let mut all = locked(&self.saved);
            for handle in saved {
                if !all.contains(handle) {
                    all.push(handle.clone());
                }
            }
        }
        let level = result
            .get("classification")
            .and_then(Value::as_str)
            .and_then(DataClassification::parse);
        if let Some(level) = level {
            let mut taint = locked(&self.taint);
            *taint = taint.combine(DataSensitivity {
                classification: Some(level),
                pii: false,
            });
        }
    }

    fn observe_frame(&self, frame: &str) {
        let Some(payload) = frame
            .strip_prefix("<usage_stat>")
            .and_then(|rest| rest.strip_suffix("</usage_stat>"))
        else {
            return;
        };
        let Ok(payload) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        let stats = &payload["stats"];
        let tokens = stats["usage"]["total_tokens"].as_u64().unwrap_or(0);
        let calls = stats["calls"].as_array().map_or(0, Vec::len) as u64;
        self.tokens.fetch_add(tokens, Ordering::Relaxed);
        self.model_calls.fetch_add(calls, Ordering::Relaxed);
    }

    fn usage(&self, elapsed: Duration) -> Value {
        json!({
            "tokens": self.tokens.load(Ordering::Relaxed),
            "model_calls": self.model_calls.load(Ordering::Relaxed),
            "tool_calls": self.tool_calls.load(Ordering::Relaxed),
            "elapsed_ms": elapsed.as_millis() as u64,
        })
    }
}

/// The outermost bridge of a child: refuses what a child may not do and meters the rest.
struct ChildGuard {
    inner: Arc<dyn PlatformToolBridge>,
    meter: Arc<ChildMeter>,
}

#[async_trait]
impl PlatformToolBridge for ChildGuard {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if let Some(message) = child_refusal(tool_name, &arguments) {
            locked(&self.meter.refused).push(format!("{tool_name}: {message}"));
            return json!({
                "status": "error",
                "tool": tool_name,
                "code": "subtask_tool_refused",
                "retryable": false,
                "message": message,
            })
            .to_string();
        }
        self.meter.tool_calls.fetch_add(1, Ordering::Relaxed);
        let output = self.inner.call(tool_name, arguments).await;
        self.meter.observe_result(&output);
        output
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Done,
    Failed,
    Timeout,
    Budget,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Budget => "budget",
        }
    }
}

struct Outcome {
    status: Status,
    output: Value,
    error: Option<String>,
}

impl Outcome {
    fn failed(error: impl Into<String>) -> Self {
        Self {
            status: Status::Failed,
            output: Value::Null,
            error: Some(error.into()),
        }
    }
}

enum Limited<T> {
    Finished(T),
    TimedOut,
    Cancelled,
}

struct TaskResult {
    id: String,
    outcome: Outcome,
    saved: Vec<Value>,
    usage: Value,
    refused: Vec<String>,
    tokens: u64,
    model_calls: u64,
    tool_calls: u64,
}

impl TaskResult {
    fn to_json(&self) -> Value {
        let mut result = json!({
            "id": self.id,
            "status": self.outcome.status.as_str(),
            "output": self.outcome.output,
            "saved": self.saved,
            "usage": self.usage,
        });
        if let Some(object) = result.as_object_mut() {
            if let Some(error) = &self.outcome.error {
                object.insert("error".into(), json!(error));
            }
            if !self.refused.is_empty() {
                object.insert("refused".into(), json!(self.refused));
            }
        }
        result
    }
}

impl SubtaskBridge {
    async fn spawn(&self, mut arguments: Value) -> String {
        let parent_request = arguments
            .as_object_mut()
            .and_then(|object| object.remove(REQUEST_ID_ARGUMENT))
            .and_then(|id| id.as_str().map(str::to_string))
            .unwrap_or_else(|| SPAWN_SUBTASKS_TOOL.to_string());
        let plan = match plan(arguments) {
            Ok(plan) => plan,
            Err(rejection) => return rejection,
        };
        let parent_request = parent_request.as_str();
        let mut results: Vec<(usize, TaskResult)> = stream::iter(
            plan.tasks.into_iter().enumerate(),
        )
        .map(|(index, task)| async move { (index, self.run_task(parent_request, task).await) })
        .buffer_unordered(plan.max_parallel)
        .collect()
        .await;
        results.sort_by_key(|(index, _)| *index);
        let results: Vec<TaskResult> = results.into_iter().map(|(_, result)| result).collect();
        json!({
            "status": "ok",
            "results": results.iter().map(TaskResult::to_json).collect::<Vec<_>>(),
            "usage_total": {
                "tokens": results.iter().map(|r| r.tokens).sum::<u64>(),
                "model_calls": results.iter().map(|r| r.model_calls).sum::<u64>(),
                "tool_calls": results.iter().map(|r| r.tool_calls).sum::<u64>(),
                "elapsed_ms": results
                    .iter()
                    .filter_map(|r| r.usage["elapsed_ms"].as_u64())
                    .sum::<u64>(),
            },
        })
        .to_string()
    }

    fn emit(
        &self,
        request_id: &str,
        parent_request: &str,
        frame: String,
        stage: Option<(&str, &str)>,
    ) {
        let Some(sink) = &self.env.on_token else {
            return;
        };
        let mut event = json!({
            "type": "native_specialist_event",
            "requestId": request_id,
            "parentRequestId": parent_request,
            "toolName": SPAWN_SUBTASKS_TOOL,
            "frame": frame,
        });
        if let Some((stage, status)) = stage {
            event["stage"] = json!(stage);
            event["status"] = json!(status);
        }
        sink(event.to_string());
    }

    async fn run_task(&self, parent_request: &str, task: Task) -> TaskResult {
        let started = Instant::now();
        let meter = Arc::new(ChildMeter::default());
        let request_id = format!("{parent_request}:task:{}", task.id);
        let remaining = self.env.remaining();
        let outcome = if self.inner.is_cancelled().await {
            Outcome::failed("The turn was cancelled before this task started.")
        } else if remaining < self.env.min_child_time {
            Outcome {
                status: Status::Budget,
                output: Value::Null,
                error: Some(
                    "Too little of the turn's time was left to start this task.".to_string(),
                ),
            }
        } else {
            self.emit(
                &request_id,
                parent_request,
                String::new(),
                Some(("started", "progress")),
            );
            let outcome = self
                .run_child(
                    &task,
                    remaining.min(self.env.child_limit),
                    &meter,
                    parent_request,
                    &request_id,
                )
                .await;
            let status = if outcome.status == Status::Done {
                "done"
            } else {
                "error"
            };
            self.emit(
                &request_id,
                parent_request,
                String::new(),
                Some(("finished", status)),
            );
            outcome
        };
        if let Some(talk) = &self.env.talk {
            talk.raise_taint(*locked(&meter.taint));
        }
        TaskResult {
            id: task.id,
            outcome,
            saved: locked(&meter.saved).clone(),
            usage: meter.usage(started.elapsed()),
            refused: locked(&meter.refused).clone(),
            tokens: meter.tokens.load(Ordering::Relaxed),
            model_calls: meter.model_calls.load(Ordering::Relaxed),
            tool_calls: meter.tool_calls.load(Ordering::Relaxed),
        }
    }

    async fn limited<T>(&self, limit: Duration, work: impl Future<Output = T>) -> Limited<T> {
        let cancelled = async {
            while !self.inner.is_cancelled().await {
                tokio::time::sleep(CANCEL_POLL).await;
            }
        };
        tokio::select! {
            output = work => Limited::Finished(output),
            _ = tokio::time::sleep(limit) => Limited::TimedOut,
            _ = cancelled => Limited::Cancelled,
        }
    }

    fn sink_for(
        &self,
        meter: &Arc<ChildMeter>,
        parent_request: &str,
        request_id: &str,
    ) -> Arc<dyn Fn(String) + Send + Sync> {
        let meter = meter.clone();
        let events = self.env.on_token.clone();
        let parent_request = parent_request.to_string();
        let request_id = request_id.to_string();
        Arc::new(move |frame: String| {
            meter.observe_frame(&frame);
            if let Some(sink) = &events {
                sink(
                    json!({
                        "type": "native_specialist_event",
                        "requestId": request_id,
                        "parentRequestId": parent_request,
                        "toolName": SPAWN_SUBTASKS_TOOL,
                        "frame": frame,
                    })
                    .to_string(),
                );
            }
        })
    }

    async fn run_child(
        &self,
        task: &Task,
        limit: Duration,
        meter: &Arc<ChildMeter>,
        parent_request: &str,
        request_id: &str,
    ) -> Outcome {
        let deadline = Instant::now() + limit;
        let capabilities = child_capabilities(&self.bot, task.tools.as_deref());
        meter_refusals(meter, &capabilities.refused);
        let child = Arc::new(child_context(&self.bot, capabilities.granted));
        let tool_specs = child_tool_specs(&child);
        let can_read_files = tool_specs
            .iter()
            .any(|spec| matches!(spec.name, "file_read" | "storage_tool" | "file_list"));
        let guard: Arc<dyn PlatformToolBridge> = Arc::new(ChildGuard {
            inner: Arc::new(BotToolBridge {
                bot: child.clone(),
                inner: self.inner.clone(),
            }),
            meter: meter.clone(),
        });
        let sink = self.sink_for(meter, parent_request, request_id);
        let first = ChildTurn {
            system_prompt: child_system_prompt(&child),
            user_prompt: child_user_prompt(task, can_read_files),
            tool_specs,
            tool_rounds: CHILD_ROUNDS,
            bridge: guard.clone(),
            on_token: Some(sink.clone()),
        };
        let text = match self.limited(limit, self.env.runner.run(first)).await {
            Limited::Finished(Ok(text)) => text,
            Limited::Finished(Err(error)) => return Outcome::failed(error.to_string()),
            Limited::TimedOut => return timed_out(limit),
            Limited::Cancelled => return Outcome::failed("The turn was cancelled."),
        };
        let Some(schema) = &task.schema else {
            return Outcome {
                status: Status::Done,
                output: Value::String(clip(text.trim(), MAX_OUTPUT_CHARS)),
                error: None,
            };
        };
        let problem = match check_output(schema, &text) {
            Ok(value) => return done(value),
            Err(problem) => problem,
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left < self.env.min_child_time {
            return invalid_output(&text, &problem);
        }
        let repair = ChildTurn {
            system_prompt: child_system_prompt(&child),
            user_prompt: repair_prompt(task, schema, &text, &problem),
            tool_specs: Vec::new(),
            tool_rounds: 0,
            bridge: guard,
            on_token: Some(sink),
        };
        let repaired = match self.limited(left, self.env.runner.run(repair)).await {
            Limited::Finished(Ok(text)) => text,
            Limited::Finished(Err(error)) => return Outcome::failed(error.to_string()),
            Limited::TimedOut => return timed_out(left),
            Limited::Cancelled => return Outcome::failed("The turn was cancelled."),
        };
        match check_output(schema, &repaired) {
            Ok(value) => done(value),
            Err(problem) => invalid_output(&repaired, &problem),
        }
    }
}

fn meter_refusals(meter: &ChildMeter, refused: &[String]) {
    locked(&meter.refused).extend(refused.iter().cloned());
}

fn done(output: Value) -> Outcome {
    Outcome {
        status: Status::Done,
        output,
        error: None,
    }
}

fn timed_out(limit: Duration) -> Outcome {
    Outcome {
        status: Status::Timeout,
        output: Value::Null,
        error: Some(format!(
            "The task did not finish within {} seconds.",
            limit.as_secs()
        )),
    }
}

fn invalid_output(text: &str, problem: &str) -> Outcome {
    Outcome {
        status: Status::Failed,
        output: Value::String(clip(text.trim(), MAX_OUTPUT_CHARS)),
        error: Some(format!(
            "The output does not match output_schema, even after one repair: {problem}"
        )),
    }
}

fn child_system_prompt(child: &BotRunContext) -> String {
    format!(
        "{}\n\n## YOU ARE A SUB-TASK\nYou are one worker in a job {name} split into parts. {name} gave you the instruction below and reads only your final reply. You see none of the conversation and nobody can answer questions, so never ask; decide on safe defaults and say which. You can't message people, send email, schedule, use memory, start sub-tasks, or run anything that needs your owner's approval. When the instruction needs one of those, do the rest and say exactly what you could not do. Current UTC date: {date}. Reply with the result only, no preamble and no UI fences.",
        bot_system_prompt(child),
        name = child.name,
        date = chrono::Utc::now().format("%Y-%m-%d"),
    )
}

fn schema_contract(schema: &Value) -> String {
    format!(
        "Your final reply must be exactly one JSON value that satisfies this JSON Schema, with no Markdown fence and no other text:\n{schema}"
    )
}

fn child_user_prompt(task: &Task, can_read_files: bool) -> String {
    let mut sections = vec![task.instruction.clone()];
    if !task.files.is_empty() {
        let listing = task
            .files
            .iter()
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n");
        let how = if can_read_files {
            "Open them with your file tools."
        } else {
            "You hold no file tools, so you can't open them; say so if you need them."
        };
        sections.push(format!("Files for this task. {how}\n{listing}"));
    }
    sections.extend(task.schema.as_ref().map(schema_contract));
    sections.join("\n\n")
}

fn repair_prompt(task: &Task, schema: &Value, previous: &str, problem: &str) -> String {
    format!(
        "{}\n\n{}\n\nYour previous reply was rejected: {problem}\n\nPrevious reply:\n{}\n\nReply again with exactly one corrected JSON value and nothing else.",
        task.instruction,
        schema_contract(schema),
        clip(previous.trim(), 4_000),
    )
}

/// The JSON value in a child's reply, if it holds one: the whole reply, a fenced block, or the
/// outermost braces or brackets.
fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim().strip_suffix("```"))
        .map(str::trim);
    if let Some(value) = unfenced.and_then(|body| serde_json::from_str(body).ok()) {
        return Some(value);
    }
    [('{', '}'), ('[', ']')]
        .into_iter()
        .find_map(|(open, close)| {
            let start = trimmed.find(open)?;
            let end = trimmed.rfind(close)?;
            (end > start)
                .then(|| serde_json::from_str(&trimmed[start..=end]).ok())
                .flatten()
        })
}

fn check_output(schema: &Value, text: &str) -> Result<Value, String> {
    let value = extract_json(text).ok_or_else(|| "the reply holds no JSON value".to_string())?;
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| format!("the schema does not compile: {error}"))?;
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .take(5)
        .map(|error| error.to_string())
        .collect();
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::bot_talk::{BotTalkHost, CrewMember, NestedBotTurn};
    use crate::flow::copilot::capability_tools::HostTools;
    use flow_like_types::bot::BotLocation;
    use std::sync::atomic::AtomicBool;

    #[derive(Clone)]
    enum Behavior {
        Reply(String),
        Sleep(Duration),
        CallThenReply(&'static str, Value, String),
        UntilCancelled,
        Fail,
        Sequence(Arc<Mutex<Vec<String>>>),
    }

    #[derive(Debug, Clone)]
    struct Seen {
        tools: Vec<String>,
        system_prompt: String,
        user_prompt: String,
    }

    struct FakeRunner {
        behavior: Box<dyn Fn(&str) -> Behavior + Send + Sync>,
        seen: Mutex<Vec<Seen>>,
        peak: AtomicU64,
        running: AtomicU64,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl FakeRunner {
        fn new(behavior: impl Fn(&str) -> Behavior + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                behavior: Box::new(behavior),
                seen: Mutex::new(Vec::new()),
                peak: AtomicU64::new(0),
                running: AtomicU64::new(0),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn replying(text: &str) -> Arc<Self> {
            let text = text.to_string();
            Self::new(move |_| Behavior::Reply(text.clone()))
        }
    }

    #[async_trait]
    impl SubtaskRunner for FakeRunner {
        async fn run(&self, child: ChildTurn) -> flow_like_types::Result<String> {
            locked(&self.seen).push(Seen {
                tools: child
                    .tool_specs
                    .iter()
                    .map(|s| s.name.to_string())
                    .collect(),
                system_prompt: child.system_prompt.clone(),
                user_prompt: child.user_prompt.clone(),
            });
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            let result = match (self.behavior)(&child.user_prompt) {
                Behavior::Reply(text) => Ok(text),
                Behavior::Fail => Err(flow_like_types::anyhow!("model unavailable")),
                Behavior::Sleep(duration) => {
                    tokio::time::sleep(duration).await;
                    Ok("late".to_string())
                }
                Behavior::CallThenReply(tool, arguments, text) => {
                    let output = child.bridge.call(tool, arguments).await;
                    locked(&self.calls).push((tool.to_string(), output));
                    Ok(text)
                }
                Behavior::UntilCancelled => {
                    while !child.bridge.is_cancelled().await {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(flow_like_types::anyhow!("cancelled"))
                }
                Behavior::Sequence(replies) => {
                    let mut replies = locked(&replies);
                    Ok(if replies.is_empty() {
                        String::new()
                    } else {
                        replies.remove(0)
                    })
                }
            };
            self.running.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }

    struct ParentBridge {
        cancelled: AtomicBool,
        calls: Mutex<Vec<String>>,
        classification: Option<&'static str>,
    }

    impl ParentBridge {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                cancelled: AtomicBool::new(false),
                calls: Mutex::new(Vec::new()),
                classification: None,
            })
        }
    }

    #[async_trait]
    impl PlatformToolBridge for ParentBridge {
        async fn call(&self, tool_name: &str, _: Value) -> String {
            locked(&self.calls).push(tool_name.to_string());
            json!({
                "status": "ok",
                "tool": tool_name,
                "saved": ["handle-1"],
                "classification": self.classification,
            })
            .to_string()
        }

        async fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    struct NoCrew;

    #[async_trait]
    impl BotTalkHost for NoCrew {
        async fn roster(&self, _: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
            Ok(Vec::new())
        }
        async fn has_grant(&self, _: &CrewMember, _: &CrewMember) -> flow_like_types::Result<bool> {
            Ok(false)
        }
        async fn store_grant(&self, _: &CrewMember, _: &CrewMember) -> flow_like_types::Result<()> {
            Ok(())
        }
        async fn exchange_conversation(
            &self,
            _: &CrewMember,
            _: &CrewMember,
        ) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no inbox"))
        }
        async fn record(
            &self,
            _: &str,
            _: &CrewMember,
            _: &str,
        ) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no inbox"))
        }
        async fn run_turn(&self, _: NestedBotTurn) -> flow_like_types::Result<String> {
            Err(flow_like_types::anyhow!("no turns"))
        }
    }

    fn parent_bot() -> BotRunContext {
        BotRunContext {
            bot_id: "bot_boss".into(),
            name: "Boss".into(),
            capabilities: vec![
                BotCapability::Internet,
                BotCapability::Storage,
                BotCapability::Documents,
                BotCapability::Knowledge,
                BotCapability::Memory,
                BotCapability::Scheduling,
                BotCapability::Email,
                BotCapability::Channels,
                BotCapability::PushNotifications,
                BotCapability::Connectors,
                BotCapability::Delegation,
            ],
            workspace_app_id: Some("app-workspace".into()),
            host_tools: HostTools::linked(),
            ..Default::default()
        }
    }

    struct Rig {
        bridge: Arc<dyn PlatformToolBridge>,
        parent: Arc<ParentBridge>,
        runner: Arc<FakeRunner>,
        talk: Arc<BotTalk>,
        events: Arc<Mutex<Vec<String>>>,
    }

    fn rig_with(
        bot: BotRunContext,
        runner: Arc<FakeRunner>,
        parent: Arc<ParentBridge>,
        budget: Duration,
        child_limit: Duration,
    ) -> Rig {
        let bot = Arc::new(bot);
        let talk = Arc::new(BotTalk::for_run(
            Arc::new(NoCrew),
            &bot,
            BotLocation::Desktop,
        ));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let mut env = SubtaskEnv::new(
            runner.clone(),
            Some(Arc::new(move |frame| locked(&sink_events).push(frame))),
            Some(talk.clone()),
        );
        env.turn_budget = budget;
        env.min_child_time = Duration::from_millis(100);
        env.child_limit = child_limit;
        let bridge = attach(parent.clone(), &bot, env);
        Rig {
            bridge,
            parent,
            runner,
            talk,
            events,
        }
    }

    fn make_rig(runner: Arc<FakeRunner>) -> Rig {
        rig_with(
            parent_bot(),
            runner,
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        )
    }

    async fn spawn(rig: &Rig, arguments: Value) -> Value {
        serde_json::from_str(&rig.bridge.call(SPAWN_SUBTASKS_TOOL, arguments).await).unwrap()
    }

    fn tasks(count: usize) -> Value {
        json!({ "tasks": (0..count).map(|i| json!({"instruction": format!("part {i}")})).collect::<Vec<_>>() })
    }

    #[test]
    fn the_tool_is_offered_only_with_delegation() {
        let with = parent_bot();
        assert!(
            with.tool_specs(false)
                .iter()
                .any(|s| s.name == SPAWN_SUBTASKS_TOOL)
        );

        let without = BotRunContext {
            capabilities: with
                .capabilities
                .iter()
                .copied()
                .filter(|c| *c != BotCapability::Delegation)
                .collect(),
            ..with.clone()
        };
        assert!(
            !without
                .tool_specs(false)
                .iter()
                .any(|s| s.name == SPAWN_SUBTASKS_TOOL)
        );
        let flux = BotRunContext {
            toolset: BotToolset::FlowPilot,
            ..with
        };
        assert!(!offered_to(&flux));
    }

    #[test]
    fn it_is_a_core_tool_every_host_runs() {
        for host in [
            BotToolHost::Chat,
            BotToolHost::Native,
            BotToolHost::CoreOnly,
        ] {
            let bot = BotRunContext {
                tool_host: host,
                ..parent_bot()
            };
            assert!(
                bot.tool_specs(false)
                    .iter()
                    .any(|s| s.name == SPAWN_SUBTASKS_TOOL),
                "{host:?}"
            );
        }
    }

    #[test]
    fn the_prompt_promises_the_tool_only_while_it_is_offered() {
        let prompt = bot_system_prompt(&parent_bot());
        assert!(prompt.contains("`spawn_subtasks`"));
        assert!(!prompt.contains("Delegation (not available"), "{prompt}");
        let off = BotRunContext {
            capabilities: vec![BotCapability::Internet],
            ..parent_bot()
        };
        assert!(!bot_system_prompt(&off).contains("`spawn_subtasks`"));
    }

    #[test]
    fn the_schema_is_typed_and_bounded() {
        let schema = spawn_subtasks_schema();
        assert_eq!(schema["properties"]["tasks"]["maxItems"], MAX_TASKS);
        assert_eq!(
            schema["properties"]["max_parallel"]["maximum"],
            MAX_PARALLEL
        );
        assert!(
            schema["properties"]["tasks"]["items"]["properties"]["tools"]["items"]["type"]
                == "string"
        );
    }

    #[tokio::test]
    async fn one_to_six_tasks_are_accepted_and_nothing_else() {
        let rig = make_rig(FakeRunner::replying("ok"));
        for count in [1, 6] {
            let result = spawn(&rig, tasks(count)).await;
            assert_eq!(result["status"], "ok");
            assert_eq!(result["results"].as_array().unwrap().len(), count);
        }
        for count in [0, 7] {
            let result = spawn(&rig, tasks(count)).await;
            assert_eq!(result["code"], "subtasks_task_count", "{count}");
        }
        let before = locked(&rig.runner.seen).len();
        assert_eq!(before, 7);
    }

    #[tokio::test]
    async fn max_parallel_is_bounded_and_caps_concurrency() {
        let runner = FakeRunner::new(|_| Behavior::Sleep(Duration::from_millis(60)));
        let rig = make_rig(runner.clone());
        for bad in [0, 5, -1] {
            let mut args = tasks(2);
            args["max_parallel"] = json!(bad);
            assert_eq!(spawn(&rig, args).await["code"], "subtasks_max_parallel");
        }
        let mut args = tasks(6);
        args["max_parallel"] = json!(2);
        assert_eq!(spawn(&rig, args).await["status"], "ok");
        assert_eq!(runner.peak.load(Ordering::SeqCst), 2);
        runner.peak.store(0, Ordering::SeqCst);
        assert_eq!(spawn(&rig, tasks(6)).await["status"], "ok");
        assert_eq!(runner.peak.load(Ordering::SeqCst), DEFAULT_PARALLEL as u64);
    }

    #[tokio::test]
    async fn arguments_are_validated_before_anything_runs() {
        let rig = make_rig(FakeRunner::replying("ok"));
        let long = "x".repeat(MAX_INSTRUCTION_CHARS + 1);
        let big_schema = json!({"type":"object","description":"x".repeat(MAX_SCHEMA_BYTES)});
        let cases = [
            (
                json!({"tasks":[{"instruction":long}]}),
                "subtasks_instruction_invalid",
            ),
            (
                json!({"tasks":[{"instruction":"  "}]}),
                "subtasks_instruction_invalid",
            ),
            (
                json!({"tasks":[{"id":"a","instruction":"x"},{"id":"a","instruction":"y"}]}),
                "subtasks_id_invalid",
            ),
            (
                json!({"tasks":[{"instruction":"x","output_schema":big_schema}]}),
                "subtasks_schema_too_large",
            ),
            (
                json!({"tasks":[{"instruction":"x","output_schema":{"type":"nope"}}]}),
                "subtasks_schema_invalid",
            ),
            (json!({"nope":1}), "subtasks_arguments_invalid"),
        ];
        for (arguments, code) in cases {
            assert_eq!(spawn(&rig, arguments).await["code"], code);
        }
        assert!(locked(&rig.runner.seen).is_empty());
    }

    #[tokio::test]
    async fn a_child_tool_set_is_a_subset_without_the_forbidden_tools() {
        let runner = FakeRunner::replying("ok");
        let rig = make_rig(runner.clone());
        let parent_tools: Vec<String> = parent_bot()
            .tool_specs(false)
            .iter()
            .map(|s| s.name.to_string())
            .collect();
        spawn(&rig, tasks(1)).await;
        let seen = locked(&runner.seen)[0].clone();
        assert!(
            seen.tools.iter().all(|t| parent_tools.contains(t)),
            "{:?}",
            seen.tools
        );
        assert!(!seen.tools.is_empty());
        for forbidden in CHILD_REFUSED_TOOLS {
            assert!(!seen.tools.iter().any(|t| t == forbidden), "{forbidden}");
        }
        assert!(
            parent_tools
                .iter()
                .any(|t| t == "email_send" || t == "schedule_task")
        );
        assert!(!seen.system_prompt.contains("spawn_subtasks"));
        assert!(seen.system_prompt.contains("YOU ARE A SUB-TASK"));
    }

    #[tokio::test]
    async fn requested_tools_are_intersected_and_refusals_reach_the_parent() {
        let runner = FakeRunner::replying("ok");
        let rig = make_rig(runner.clone());
        let result = spawn(
            &rig,
            json!({"tasks":[{"instruction":"x","tools":["internet","email","delegation","selected_apps","nope"]}]}),
        )
        .await;
        let refused = result["results"][0]["refused"].as_array().unwrap();
        assert_eq!(refused.len(), 4, "{refused:?}");
        let seen = locked(&runner.seen)[0].clone();
        assert!(seen.tools.iter().any(|t| t == "research_agent"));
        assert!(!seen.tools.iter().any(|t| t == "file_read"));
    }

    #[test]
    fn excluded_capabilities_never_reach_a_child() {
        let parent = parent_bot();
        let all = child_capabilities(&parent, None).granted;
        for excluded in CHILD_EXCLUDED_CAPABILITIES {
            assert!(!all.contains(&excluded), "{excluded:?}");
        }
        let asked: Vec<String> = CHILD_EXCLUDED_CAPABILITIES
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        assert!(child_capabilities(&parent, Some(&asked)).granted.is_empty());
    }

    #[tokio::test]
    async fn a_child_cannot_spawn_ask_or_use_forbidden_tools_even_by_name() {
        let runner = FakeRunner::new(|_| {
            Behavior::CallThenReply(
                SPAWN_SUBTASKS_TOOL,
                json!({"tasks":[{"instruction":"deeper"}]}),
                "x".into(),
            )
        });
        let rig = make_rig(runner.clone());
        let result = spawn(&rig, tasks(1)).await;
        assert_eq!(result["results"][0]["status"], "done");
        let (_, output) = locked(&runner.calls)[0].clone();
        assert!(output.contains("subtask_tool_refused"), "{output}");
        assert_eq!(locked(&runner.seen).len(), 1);
        assert!(locked(&rig.parent.calls).is_empty());
        for tool in CHILD_REFUSED_TOOLS {
            assert!(child_refusal(tool, &json!({})).is_some(), "{tool}");
        }
    }

    #[tokio::test]
    async fn an_approval_gated_call_is_refused_not_parked() {
        let runner = FakeRunner::new(|_| {
            Behavior::CallThenReply("knowledge_drop", json!({"name":"notes"}), "x".into())
        });
        let rig = make_rig(runner.clone());
        let result = spawn(&rig, tasks(1)).await;
        let (_, output) = locked(&runner.calls)[0].clone();
        assert!(output.contains("subtask_tool_refused"), "{output}");
        assert!(locked(&rig.parent.calls).is_empty());
        assert_eq!(result["results"][0]["refused"].as_array().unwrap().len(), 1);
        assert!(child_refusal("file_write", &json!({"path":"a.txt","content":"x"})).is_none());
    }

    #[tokio::test]
    async fn a_slow_child_times_out_without_failing_its_siblings() {
        let runner = FakeRunner::new(|prompt| {
            if prompt.contains("part 1") {
                Behavior::Sleep(Duration::from_secs(60))
            } else {
                Behavior::Reply("fast".into())
            }
        });
        let rig = rig_with(
            parent_bot(),
            runner,
            ParentBridge::new(),
            Duration::from_secs(30),
            Duration::from_millis(1_500),
        );
        let result = spawn(&rig, tasks(3)).await;
        let statuses: Vec<&str> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["status"].as_str().unwrap())
            .collect();
        assert_eq!(statuses, ["done", "timeout", "done"], "{result}");
    }

    #[tokio::test]
    async fn no_time_left_reports_budget_without_running() {
        let runner = FakeRunner::replying("ok");
        let rig = rig_with(
            parent_bot(),
            runner.clone(),
            ParentBridge::new(),
            Duration::from_millis(50),
            CHILD_TIME_LIMIT,
        );
        let result = spawn(&rig, tasks(2)).await;
        assert_eq!(result["results"][0]["status"], "budget");
        assert!(locked(&runner.seen).is_empty());
    }

    #[tokio::test]
    async fn output_is_validated_against_the_schema() {
        let runner = FakeRunner::replying("```json\n{\"count\": 3}\n```");
        let rig = make_rig(runner);
        let args = json!({"tasks":[{"instruction":"x","output_schema":{"type":"object","required":["count"],"properties":{"count":{"type":"integer"}}}}]});
        let result = spawn(&rig, args).await;
        assert_eq!(result["results"][0]["status"], "done");
        assert_eq!(result["results"][0]["output"], json!({"count": 3}));
    }

    #[tokio::test]
    async fn an_invalid_output_gets_exactly_one_repair_round() {
        let replies = Arc::new(Mutex::new(vec![
            "not json".to_string(),
            "{\"count\": \"x\"}".to_string(),
            "{\"count\": 1}".to_string(),
        ]));
        let runner = FakeRunner::new({
            let replies = replies.clone();
            move |_| Behavior::Sequence(replies.clone())
        });
        let rig = make_rig(runner.clone());
        let schema =
            json!({"type":"object","required":["count"],"properties":{"count":{"type":"integer"}}});
        let args = json!({"tasks":[{"instruction":"x","output_schema":schema}]});
        let result = spawn(&rig, args).await;
        assert_eq!(result["results"][0]["status"], "failed");
        assert_eq!(locked(&runner.seen).len(), 2);
        assert!(
            locked(&runner.seen)[1]
                .user_prompt
                .contains("previous reply was rejected")
        );
        assert!(locked(&runner.seen)[1].tools.is_empty());

        let replies = Arc::new(Mutex::new(vec![
            "nope".to_string(),
            "{\"count\": 2}".to_string(),
        ]));
        let runner = FakeRunner::new({
            let replies = replies.clone();
            move |_| Behavior::Sequence(replies.clone())
        });
        let rig = make_rig(runner.clone());
        let result = spawn(
            &rig,
            json!({"tasks":[{"instruction":"x","output_schema":schema}]}),
        )
        .await;
        assert_eq!(result["results"][0]["status"], "done");
        assert_eq!(result["results"][0]["output"], json!({"count": 2}));
        assert_eq!(locked(&runner.seen).len(), 2);
    }

    #[tokio::test]
    async fn the_highest_classification_a_child_saw_becomes_the_parents_taint() {
        let runner = FakeRunner::new(|prompt| {
            if prompt.contains("part 1") {
                Behavior::CallThenReply("file_read", json!({"file":"a"}), "x".into())
            } else {
                Behavior::Reply("x".into())
            }
        });
        let parent = Arc::new(ParentBridge {
            cancelled: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            classification: Some("confidential"),
        });
        let rig = rig_with(
            parent_bot(),
            runner,
            parent,
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        assert!(rig.talk.taint().is_empty());
        let result = spawn(&rig, tasks(2)).await;
        assert_eq!(
            rig.talk.taint().classification,
            Some(DataClassification::Confidential)
        );
        assert_eq!(result["results"][1]["saved"], json!(["handle-1"]));
    }

    #[tokio::test]
    async fn cancelling_the_parent_reaches_running_children() {
        let runner = FakeRunner::new(|_| Behavior::UntilCancelled);
        let rig = make_rig(runner);
        let parent = rig.parent.clone();
        let canceller = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            parent.cancelled.store(true, Ordering::SeqCst);
        };
        let (result, ()) = tokio::join!(spawn(&rig, tasks(3)), canceller);
        for entry in result["results"].as_array().unwrap() {
            assert_eq!(entry["status"], "failed", "{entry}");
        }
    }

    #[tokio::test]
    async fn a_failing_child_does_not_fail_its_siblings_and_progress_streams() {
        let runner = FakeRunner::new(|prompt| {
            if prompt.contains("part 0") {
                Behavior::Fail
            } else {
                Behavior::Reply("fine".into())
            }
        });
        let rig = make_rig(runner);
        let mut args = tasks(2);
        args[REQUEST_ID_ARGUMENT] = json!("call-7");
        let result = spawn(&rig, args).await;
        assert_eq!(result["results"][0]["status"], "failed");
        assert_eq!(result["results"][1]["status"], "done");
        assert_eq!(result["results"][1]["output"], "fine");
        let events = locked(&rig.events).clone();
        assert!(events.iter().any(|e| e.contains("native_specialist_event")
            && e.contains("call-7:task:task-1")
            && e.contains("\"stage\":\"started\"")));
        assert!(events.iter().any(|e| e.contains("\"stage\":\"finished\"") && e.contains("\"status\":\"error\"")));
    }

    #[test]
    fn json_is_found_in_fenced_and_chatty_replies() {
        assert_eq!(extract_json("{\"a\":1}"), Some(json!({"a":1})));
        assert_eq!(extract_json("```\n[1,2]\n```"), Some(json!([1, 2])));
        assert_eq!(
            extract_json("Here you go: {\"a\": 2} done"),
            Some(json!({"a":2}))
        );
        assert_eq!(extract_json("no json here"), None);
    }

    #[tokio::test]
    async fn other_tools_pass_through_to_the_parent_bridge() {
        let rig = make_rig(FakeRunner::replying("ok"));
        rig.bridge.call("list_apps", json!({})).await;
        assert_eq!(*locked(&rig.parent.calls), ["list_apps"]);
    }
}

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
use flow_like_types::tokio;
use futures::{StreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};

use super::bot_run::{BotRunContext, BotToolBridge, BotToolHost, BotToolset, bot_system_prompt};
use super::bot_talk::ASK_BOT_TOOL;
use super::platform::{
    Clearance, IncompleteWork, PlatformCopilot, PlatformSurface, PlatformToolBridge, WebAccess,
    WorkStopReason,
};
use super::tool_spec::{
    MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, PlatformToolSpec, ToolApprovalSpec,
    resolve_tool_approval_timing,
};
use super::turn_label::{LabelBridge, TurnCompute, TurnLabel};
use super::vision_reader::ImageDetour;
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
const CHILD_ROUNDS: usize = 64;
const CHILD_TIME_LIMIT: Duration = Duration::from_secs(900);
const TURN_TIME_BUDGET: Duration =
    Duration::from_secs(flow_like_types::bot_turn::DEFAULT_TURN_DEADLINE_SECS);
const MIN_CHILD_TIME: Duration = Duration::from_secs(5);
const CANCEL_POLL: Duration = Duration::from_millis(250);
const TOOL_TIMEOUT_SECS: u64 = 1860;

const DESCRIPTION: &str = r#"Run 1 to 6 independent parts of a big job in parallel and get one result per part. Each worker is a copy of you that sees only its `instruction` (complete and self-contained), the `files` you name and the capabilities in `tools` (a subset of yours; omit it to give all it may have). Workers can't ask questions, remember, message people, send email, schedule, start sub-tasks or do anything that needs your owner's approval; do those yourself from their results. Browser and computer workers share your interactive surfaces and run one at a time. Narrow `tools` for research workers so they can run alongside them. Workers can plan and carry out dependent steps within their own part. Set `output_schema` (a JSON Schema) when you need structured output; the result is then validated and `output` is the JSON value. `max_parallel` is 1 to 4 (default 3). Each result has `status` (done, failed, timeout, budget or blocked), `output`, `saved` handles and `usage`. A worker's output is unverified until you've read it."#;

/// Capabilities a sub-task never gets: they act beyond the job, keep state, or need the owner.
const CHILD_EXCLUDED_CAPABILITIES: [BotCapability; 8] = [
    BotCapability::UserContext,
    BotCapability::Memory,
    BotCapability::Delegation,
    BotCapability::Scheduling,
    BotCapability::Email,
    BotCapability::Channels,
    BotCapability::PushNotifications,
    BotCapability::Flowpilot,
];

/// Tools a sub-task is refused by name, whatever its capabilities: they ask a person, park the
/// turn, reach other bots, people or channels, or delegate again.
const CHILD_REFUSED_TOOLS: [&str; 19] = [
    "user_context_available",
    "user_context_request",
    "user_context_set_status",
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
    "update_task",
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
            "- Sub-tasks: `{SPAWN_SUBTASKS_TOOL}` runs up to {MAX_TASKS} independent parts of a big job in parallel. Use it when the parts don't depend on each other and each takes real work; do small jobs yourself. Give each worker a complete goal, success criteria and relevant evidence. A worker can plan dependent steps within its part, but sees only the instruction, files and capabilities you give it. Browser and computer workers share your surfaces and run one at a time; narrow other workers' capabilities to keep independent research parallel. Workers can't ask questions, remember, message people, send email, schedule, start sub-tasks or do anything that needs your owner's approval: do those yourself from the results. Check every result's status and verify its output before relying on it."
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
    /// The parent turn's data label, for the outbound tools the child's loop runs itself.
    pub label: Arc<TurnLabel>,
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
    /// The bot's sub-task model; used when this host can run it.
    pub subtask_model: Option<String>,
    pub thinking: Option<HistoryThinking>,
    pub token: Option<String>,
    web_access: WebAccess,
    vision: Option<Arc<ImageDetour>>,
    chosen: tokio::sync::OnceCell<Option<String>>,
}

/// The model a child runs on: the bot's sub-task model when this host can run it, else the
/// parent's.
fn child_model(
    subtask_model: Option<&str>,
    runnable: bool,
    parent: Option<&str>,
) -> Option<String> {
    match subtask_model {
        Some(model) if runnable => Some(model.to_string()),
        _ => parent.map(str::to_string),
    }
}

impl ModelSubtaskRunner {
    pub fn new(
        state: Arc<FlowLikeState>,
        profile: Option<Arc<Profile>>,
        model_id: Option<String>,
        subtask_model: Option<String>,
        thinking: Option<HistoryThinking>,
        token: Option<String>,
    ) -> Self {
        Self {
            state,
            profile,
            model_id,
            subtask_model,
            thinking,
            token,
            web_access: WebAccess::default(),
            vision: None,
            chosen: Default::default(),
        }
    }

    /// The owner's own search and page readers, for children that research.
    pub fn with_web_access(mut self, web_access: WebAccess) -> Self {
        self.web_access = web_access;
        self
    }

    /// The parent turn's image detour, so children share its budget.
    pub(super) fn with_vision(mut self, vision: Option<Arc<ImageDetour>>) -> Self {
        self.vision = vision;
        self
    }

    async fn model(&self) -> Option<String> {
        self.chosen
            .get_or_init(|| {
                resolve_subtask_model(
                    &self.state,
                    self.profile.as_deref(),
                    self.subtask_model.as_deref(),
                    self.model_id.as_deref(),
                )
            })
            .await
            .clone()
    }
}

/// The model a bot's sub-tasks and assistants run on: its sub-task model when this host can run
/// it, else the turn's own.
pub async fn resolve_subtask_model(
    state: &Arc<FlowLikeState>,
    profile: Option<&Profile>,
    subtask_model: Option<&str>,
    turn_model: Option<&str>,
) -> Option<String> {
    let runnable = match (subtask_model, profile) {
        (Some(model), Some(profile)) => {
            super::bot_run::runnable_completion_model(state, profile, model)
                .await
                .is_some()
        }
        _ => false,
    };
    child_model(subtask_model, runnable, turn_model)
}

#[async_trait]
impl SubtaskRunner for ModelSubtaskRunner {
    async fn run(&self, child: ChildTurn) -> flow_like_types::Result<String> {
        let on_token = child.on_token.map(|sink| move |frame: String| sink(frame));
        let copilot = PlatformCopilot::new(self.state.clone(), self.profile.clone())
            .with_thinking(self.thinking)
            .with_tool_specs(child.tool_specs)
            .with_tool_rounds(child.tool_rounds)
            .with_web_access(self.web_access.clone())
            .with_vision(self.vision.clone())
            .with_turn_label(child.label);
        // Output-schema repair is a single tool-free completion.
        let copilot = if child.tool_rounds > 0 {
            copilot.with_long_running()
        } else {
            copilot
        };
        let stable_prompt = child.system_prompt.clone();
        let reply: futures::future::BoxFuture<'_, flow_like_types::Result<String>> =
            Box::pin(copilot.chat(
                PlatformSurface::Subtask,
                child.system_prompt,
                child.user_prompt,
                None,
                Vec::new(),
                self.model().await,
                self.token.clone(),
                child.bridge,
                None,
                on_token,
            ));
        flow_like_model_provider::llm::prompt_cache::with_stable_system_prefix(stable_prompt, reply)
            .await
    }
}

/// What a turn hands the sub-task machinery.
pub struct SubtaskEnv {
    pub runner: Arc<dyn SubtaskRunner>,
    pub on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
    pub label: Arc<TurnLabel>,
    pub compute: TurnCompute,
    pub started: Instant,
    pub turn_budget: Duration,
    pub min_child_time: Duration,
    pub child_limit: Duration,
}

impl SubtaskEnv {
    pub fn new(
        runner: Arc<dyn SubtaskRunner>,
        on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
        label: Arc<TurnLabel>,
        compute: TurnCompute,
    ) -> Self {
        Self {
            runner,
            on_token,
            label,
            compute,
            started: Instant::now(),
            turn_budget: TURN_TIME_BUDGET,
            min_child_time: MIN_CHILD_TIME,
            child_limit: CHILD_TIME_LIMIT,
        }
    }

    /// Caps the budget at the time left until the host's turn deadline.
    pub fn with_deadline_at_ms(mut self, deadline_at_ms: Option<i64>) -> Self {
        if let Some(deadline) = deadline_at_ms {
            self.turn_budget = self
                .turn_budget
                .min(time_until(deadline, chrono::Utc::now().timestamp_millis()));
        }
        self
    }

    fn remaining(&self) -> Duration {
        self.turn_budget.saturating_sub(self.started.elapsed())
    }
}

fn time_until(deadline_at_ms: i64, now_ms: i64) -> Duration {
    Duration::from_millis(deadline_at_ms.saturating_sub(now_ms).max(0) as u64)
}

/// Wraps `inner` in the turn's label check and, for a bot offered the tool, answers
/// `spawn_subtasks` in core.
pub(super) fn attach(
    inner: Arc<dyn PlatformToolBridge>,
    bot: &Arc<BotRunContext>,
    env: SubtaskEnv,
) -> Arc<dyn PlatformToolBridge> {
    if bot.toolset != BotToolset::Capabilities {
        return inner;
    }
    let inner: Arc<dyn PlatformToolBridge> = Arc::new(LabelBridge {
        label: env.label.clone(),
        inner,
    });
    if !offered_to(bot) {
        return inner;
    }
    Arc::new(SubtaskBridge {
        bot: bot.clone(),
        inner,
        env,
        interactive: Arc::new(tokio::sync::Mutex::new(())),
    })
}

struct SubtaskBridge {
    bot: Arc<BotRunContext>,
    inner: Arc<dyn PlatformToolBridge>,
    env: SubtaskEnv,
    // Hold the surface for a worker's entire observe/act sequence, across spawn batches.
    interactive: Arc<tokio::sync::Mutex<()>>,
}

fn is_interactive_tool(tool_name: &str) -> bool {
    tool_name.starts_with("browser_") || tool_name.starts_with("computer_")
}

/// Takes the shared browser/computer lock on a worker's first interactive call and keeps it
/// until the worker's run ends.
struct LazySurface {
    lock: Arc<tokio::sync::Mutex<()>>,
    held: tokio::sync::Mutex<Option<tokio::sync::OwnedMutexGuard<()>>>,
    inner: Arc<dyn PlatformToolBridge>,
}

#[async_trait]
impl PlatformToolBridge for LazySurface {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if is_interactive_tool(tool_name) {
            let mut held = self.held.lock().await;
            if held.is_none() {
                *held = Some(self.lock.clone().lock_owned().await);
            }
        }
        self.inner.call(tool_name, arguments).await
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        self.inner.clearance(tool_name, arguments).await
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[async_trait]
impl PlatformToolBridge for SubtaskBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if tool_name == SPAWN_SUBTASKS_TOOL {
            return self.spawn(arguments).await;
        }
        let _surface = if is_interactive_tool(tool_name) {
            match self
                .limited(self.env.remaining(), self.interactive.lock())
                .await
            {
                Limited::Finished(guard) => Some(guard),
                Limited::TimedOut => {
                    return refusal_json(
                        tool_name,
                        "The turn's time budget expired while waiting for the shared browser or computer.",
                    );
                }
                Limited::Cancelled => {
                    return refusal_json(
                        tool_name,
                        "The turn was cancelled while waiting for the shared browser or computer.",
                    );
                }
            }
        } else {
            None
        };
        self.inner.call(tool_name, arguments).await
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        if tool_name == SPAWN_SUBTASKS_TOOL {
            return Clearance::Clear;
        }
        self.inner.clearance(tool_name, arguments).await
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
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
    handles: Vec<Value>,
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
            handles: Vec::new(),
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
        // The parent journals the delegation call. A child must not overwrite its checkpoint.
        recovery: None,
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

const FINISH_THE_REST: &str =
    "Finish the rest, and say in your result what you could not do so the main bot can.";

/// Why a child may not call this tool at all.
fn child_name_refusal(tool_name: &str) -> Option<String> {
    CHILD_REFUSED_TOOLS.contains(&tool_name).then(|| {
        format!(
            "A sub-task can't use {tool_name}: it needs a person, keeps state or acts outside the job. {FINISH_THE_REST}"
        )
    })
}

/// Why a child may not make this call once the bot's guard has set its final arguments: it would
/// ask the owner for approval, and a child never parks the turn.
fn child_approval_refusal(tool_name: &str, arguments: &Value) -> Option<String> {
    let spec = super::bot_run::find_bot_tool_spec(tool_name)?;
    resolve_tool_approval_timing(&spec, arguments).map(|_| {
        format!(
            "{tool_name} needs your owner's approval, which a sub-task can't ask for. It was not run. {FINISH_THE_REST}"
        )
    })
}

fn refusal_json(tool_name: &str, message: &str) -> String {
    json!({
        "status": "error",
        "tool": tool_name,
        "code": "subtask_tool_refused",
        "retryable": false,
        "message": message,
    })
    .to_string()
}

#[derive(Default)]
struct ChildMeter {
    tokens: AtomicU64,
    model_calls: AtomicU64,
    tool_calls: AtomicU64,
    saved: Mutex<Vec<Value>>,
    refused: Mutex<Vec<String>>,
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
/// Steering and room updates stay with the parent turn, which owns their cursors.
struct ChildGuard {
    inner: Arc<dyn PlatformToolBridge>,
    meter: Arc<ChildMeter>,
}

#[async_trait]
impl PlatformToolBridge for ChildGuard {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if let Some(message) = child_name_refusal(tool_name) {
            locked(&self.meter.refused).push(format!("{tool_name}: {message}"));
            return refusal_json(tool_name, &message);
        }
        self.meter.tool_calls.fetch_add(1, Ordering::Relaxed);
        let output = self.inner.call(tool_name, arguments).await;
        self.meter.observe_result(&output);
        output
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        match child_name_refusal(tool_name) {
            Some(message) => Clearance::Refused(refusal_json(tool_name, &message)),
            None => self.inner.clearance(tool_name, arguments).await,
        }
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

/// Sits right above the parent's bridge, below the child's app guard: the arguments it sees are
/// final (a connector call carries its policy), so a call that would ask the owner is refused.
struct ApprovalGate {
    inner: Arc<dyn PlatformToolBridge>,
    meter: Arc<ChildMeter>,
}

#[async_trait]
impl PlatformToolBridge for ApprovalGate {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if let Some(message) = child_approval_refusal(tool_name, &arguments) {
            locked(&self.meter.refused).push(format!("{tool_name}: {message}"));
            return refusal_json(tool_name, &message);
        }
        self.inner.call(tool_name, arguments).await
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        match child_approval_refusal(tool_name, arguments) {
            Some(message) => Clearance::Refused(refusal_json(tool_name, &message)),
            None => self.inner.clearance(tool_name, arguments).await,
        }
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
    Blocked,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Budget => "budget",
            Self::Blocked => "blocked",
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

    fn from_error(error: flow_like_types::Error) -> Self {
        match error.downcast_ref::<IncompleteWork>() {
            Some(incomplete) => Self {
                status: match incomplete.reason {
                    WorkStopReason::Budget => Status::Budget,
                    WorkStopReason::Blocked => Status::Blocked,
                },
                output: Value::String(clip(incomplete.output.trim(), MAX_OUTPUT_CHARS)),
                error: Some(error.to_string()),
            },
            None => Self::failed(error.to_string()),
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
        let mut plan = match plan(arguments) {
            Ok(plan) => plan,
            Err(rejection) => return rejection,
        };
        if let Err(rejection) = self.resolve_files(&mut plan).await {
            return rejection;
        }
        let parent_request = parent_request.as_str();
        let spawned = Instant::now();
        let parallelism = tokio::sync::Semaphore::new(plan.max_parallel);
        let mut results: Vec<(usize, TaskResult)> =
            stream::iter(plan.tasks.into_iter().enumerate())
                .map(|(index, task)| {
                    let parallelism = &parallelism;
                    async move {
                        (
                            index,
                            self.run_task(parent_request, task, parallelism).await,
                        )
                    }
                })
                // Waiting for the shared surface does not occupy a worker slot.
                .buffer_unordered(MAX_TASKS)
                .collect()
                .await;
        self.env
            .compute
            .add_waited(spawned.elapsed().as_millis().min(u64::MAX as u128) as u64);
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

    /// Opens every named file through the parent's own `file_read`, which applies the bot's
    /// scope, and keeps the handle it returns for the child. One unreadable or out-of-scope
    /// reference refuses the whole call before any child runs.
    async fn resolve_files(&self, plan: &mut Plan) -> Result<(), String> {
        let mut resolved: std::collections::HashMap<String, Value> = Default::default();
        for task in &mut plan.tasks {
            for file in &task.files {
                if !resolved.contains_key(file) {
                    let output = self
                        .inner
                        .call("file_read", json!({ "file": file, "max_bytes": 1 }))
                        .await;
                    let result: Value = serde_json::from_str(&output).unwrap_or_default();
                    let handle = result.get("handle").filter(|handle| !handle.is_null());
                    let Some(handle) = handle.filter(|_| result["status"] == "ok") else {
                        let reason = result["error"]["message"]
                            .as_str()
                            .or_else(|| result["message"].as_str())
                            .unwrap_or("it is not readable here");
                        return Err(rejection(
                            "subtasks_file_unresolved",
                            format!(
                                "Task '{}' names the file '{file}', which a sub-task cannot be given: {reason}.",
                                task.id
                            ),
                        ));
                    };
                    resolved.insert(file.clone(), handle.clone());
                }
                task.handles.push(resolved[file].clone());
            }
        }
        Ok(())
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

    async fn run_task(
        &self,
        parent_request: &str,
        task: Task,
        parallelism: &tokio::sync::Semaphore,
    ) -> TaskResult {
        let started = Instant::now();
        let meter = Arc::new(ChildMeter::default());
        let request_id = format!("{parent_request}:task:{}", task.id);
        let remaining = self.env.remaining();
        let outcome = if self.inner.is_cancelled().await {
            Outcome::failed("The turn was cancelled before this task started.")
        } else if remaining < self.env.min_child_time {
            budget_exhausted()
        } else {
            let ready = self
                .limited(remaining, async {
                    parallelism
                        .acquire()
                        .await
                        .expect("the batch semaphore stays open")
                })
                .await;
            match ready {
                Limited::Finished(_slot) => {
                    let remaining = self.env.remaining();
                    if self.inner.is_cancelled().await {
                        Outcome::failed("The turn was cancelled before this task started.")
                    } else if remaining < self.env.min_child_time {
                        budget_exhausted()
                    } else {
                        let running_since = Instant::now();
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
                        self.env.compute.add_child(
                            running_since.elapsed().as_millis().min(u64::MAX as u128) as u64,
                        );
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
                    }
                }
                Limited::TimedOut => budget_exhausted(),
                Limited::Cancelled => {
                    Outcome::failed("The turn was cancelled before this task started.")
                }
            }
        };
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
            biased;
            _ = cancelled => Limited::Cancelled,
            _ = tokio::time::sleep(limit) => Limited::TimedOut,
            output = work => Limited::Finished(output),
        }
    }

    fn sink_for(
        &self,
        meter: &Arc<ChildMeter>,
        parent_request: &str,
        request_id: &str,
        usage_id: &str,
    ) -> Arc<dyn Fn(String) + Send + Sync> {
        let meter = meter.clone();
        let events = self.env.on_token.clone();
        let parent_request = parent_request.to_string();
        let request_id = request_id.to_string();
        let usage_id = usage_id.to_string();
        Arc::new(move |frame: String| {
            meter.observe_frame(&frame);
            if let Some(sink) = &events {
                sink(
                    json!({
                        "type": "native_specialist_event",
                        "requestId": request_id,
                        "usageId": usage_id,
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
                inner: Arc::new(ApprovalGate {
                    inner: Arc::new(LazySurface {
                        lock: self.interactive.clone(),
                        held: tokio::sync::Mutex::new(None),
                        inner: self.inner.clone(),
                    }),
                    meter: meter.clone(),
                }),
            }),
            meter: meter.clone(),
        });
        let sink = self.sink_for(meter, parent_request, request_id, request_id);
        let first = ChildTurn {
            system_prompt: child_system_prompt(&child),
            user_prompt: child_user_prompt(task, can_read_files),
            tool_specs,
            tool_rounds: CHILD_ROUNDS,
            bridge: guard.clone(),
            on_token: Some(sink.clone()),
            label: self.env.label.clone(),
        };
        let text = match self.limited(limit, self.env.runner.run(first)).await {
            Limited::Finished(Ok(text)) => text,
            Limited::Finished(Err(error)) => return Outcome::from_error(error),
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
            on_token: Some(self.sink_for(
                meter,
                parent_request,
                request_id,
                &format!("{request_id}:repair"),
            )),
            label: self.env.label.clone(),
        };
        let repaired = match self.limited(left, self.env.runner.run(repair)).await {
            Limited::Finished(Ok(text)) => text,
            Limited::Finished(Err(error)) => return Outcome::from_error(error),
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

fn budget_exhausted() -> Outcome {
    Outcome {
        status: Status::Budget,
        output: Value::Null,
        error: Some("Too little of the turn's time was left to start this task.".to_string()),
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
        "{}\n\n## YOU ARE A SUB-TASK\nYou are one worker in a job {name} split into parts. {name} gave you the instruction below and reads only your final reply. You see none of the conversation and nobody can answer questions, so never ask; decide on safe defaults and say which. For work with dependent steps, use update_work_plan to track the goal, evidence and next actions, then keep executing until the result is verified or a concrete blocker prevents progress. Update the plan as work finishes. After context compaction, inspect the current browser or computer state before acting again. Never repeat a completed mutation just because its earlier output is no longer in context. You can't message people, send email, schedule, use memory, start sub-tasks, or run anything that needs your owner's approval. When the instruction needs one of those, do the rest and say exactly what you could not do. Report completed work, verification evidence and remaining blockers accurately. Current UTC date: {date}. Reply with the result only, no preamble and no UI fences.",
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
            .zip(&task.handles)
            .map(|(file, handle)| format!("- {file}: pass this handle to file_read: {handle}"))
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

    #[tokio::test]
    async fn child_does_not_consume_the_parent_room_updates() {
        struct RoomBridge;
        #[async_trait]
        impl PlatformToolBridge for RoomBridge {
            async fn call(&self, _: &str, _: Value) -> String {
                "ok".into()
            }
            fn watches_room(&self) -> bool {
                true
            }
            async fn drain_room_updates(&self) -> Vec<String> {
                vec!["participant message for the parent".into()]
            }
        }
        let parent: Arc<dyn PlatformToolBridge> = Arc::new(RoomBridge);
        let child = ChildGuard {
            inner: parent.clone(),
            meter: Arc::new(ChildMeter::default()),
        };
        assert!(!child.watches_room());
        assert!(child.drain_room_updates().await.is_empty());
        assert_eq!(
            parent.drain_room_updates().await,
            ["participant message for the parent"]
        );
    }
    use crate::flow::copilot::bot_talk::{BotTalk, BotTalkHost, CrewMember, NestedBotTurn};
    use crate::flow::copilot::capability_tools::HostTools;
    use flow_like_types::bot::BotLocation;
    use flow_like_types::data_classification::{DataClassification, DataSensitivity};
    use std::sync::atomic::AtomicBool;

    #[derive(Clone)]
    enum Behavior {
        Reply(String),
        Sleep(Duration),
        CallThenReply(&'static str, Value, String),
        UntilCancelled,
        Fail,
        Sequence(Arc<Mutex<Vec<String>>>),
        Incomplete(WorkStopReason, String),
        Wait(Arc<tokio::sync::Notify>),
        Surface(Box<Behavior>),
    }

    #[derive(Debug, Clone)]
    struct Seen {
        tools: Vec<String>,
        system_prompt: String,
        user_prompt: String,
        tool_rounds: usize,
    }

    struct FakeRunner {
        behavior: Box<dyn Fn(&str) -> Behavior + Send + Sync>,
        seen: Mutex<Vec<Seen>>,
        peak: AtomicU64,
        running: AtomicU64,
        calls: Mutex<Vec<(String, String)>>,
        started: tokio::sync::Notify,
    }

    impl FakeRunner {
        fn new(behavior: impl Fn(&str) -> Behavior + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                behavior: Box::new(behavior),
                seen: Mutex::new(Vec::new()),
                peak: AtomicU64::new(0),
                running: AtomicU64::new(0),
                calls: Mutex::new(Vec::new()),
                started: tokio::sync::Notify::new(),
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
                tool_rounds: child.tool_rounds,
            });
            // A surface worker counts as running once it holds the shared surface.
            let mut behavior = (self.behavior)(&child.user_prompt);
            let surface_tool = ["browser_snapshot", "computer_windows"]
                .into_iter()
                .find(|tool| child.tool_specs.iter().any(|spec| spec.name == *tool))
                .unwrap_or("browser_snapshot");
            while let Behavior::Surface(next) = behavior {
                let output = child.bridge.call(surface_tool, json!({})).await;
                locked(&self.calls).push((surface_tool.to_string(), output));
                behavior = *next;
            }
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            struct Running<'a>(&'a AtomicU64);
            impl Drop for Running<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _running = Running(&self.running);
            self.peak.fetch_max(now, Ordering::SeqCst);
            self.started.notify_one();
            let result = match behavior {
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
                Behavior::Incomplete(reason, output) => {
                    Err(IncompleteWork { reason, output }.into())
                }
                Behavior::Wait(release) => {
                    release.notified().await;
                    Ok("finished".into())
                }
                Behavior::Surface(_) => unreachable!("surface wrappers are unwrapped above"),
            };
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
        async fn call(&self, tool_name: &str, arguments: Value) -> String {
            locked(&self.calls).push(tool_name.to_string());
            if tool_name == "file_read" {
                let file = arguments["file"].as_str().unwrap_or_default();
                if file.contains("missing") || file.starts_with("app:other") {
                    return json!({
                        "status": "error",
                        "tool": tool_name,
                        "error": { "code": "not_found", "message": format!("'{file}' is not readable") },
                    })
                    .to_string();
                }
                return json!({
                    "status": "ok",
                    "tool": tool_name,
                    "handle": { "app_id": "app-workspace", "path": file },
                    "classification": self.classification,
                })
                .to_string();
            }
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

    #[test]
    fn quiet_parent_keeps_owner_notifications_denied_in_children() {
        let mut parent = parent_bot();
        parent.suppress_owner_notifications = true;
        let child = child_context(
            &parent,
            vec![BotCapability::PushNotifications, BotCapability::Email],
        );
        assert!(child.suppress_owner_notifications);
        assert!(
            !child
                .tool_specs(false)
                .iter()
                .any(|tool| tool.name == "message_owner")
        );
    }

    struct Rig {
        bridge: Arc<dyn PlatformToolBridge>,
        parent: Arc<ParentBridge>,
        runner: Arc<FakeRunner>,
        talk: Option<Arc<BotTalk>>,
        label: Arc<TurnLabel>,
        compute: TurnCompute,
        events: Arc<Mutex<Vec<String>>>,
    }

    fn rig_with(
        bot: BotRunContext,
        runner: Arc<FakeRunner>,
        parent: Arc<ParentBridge>,
        budget: Duration,
        child_limit: Duration,
    ) -> Rig {
        build_rig(bot, runner, parent, budget, child_limit, true)
    }

    fn build_rig(
        bot: BotRunContext,
        runner: Arc<FakeRunner>,
        parent: Arc<ParentBridge>,
        budget: Duration,
        child_limit: Duration,
        with_talk: bool,
    ) -> Rig {
        let bot = Arc::new(bot);
        let talk = with_talk.then(|| {
            Arc::new(BotTalk::for_run(
                Arc::new(NoCrew),
                &bot,
                BotLocation::Desktop,
            ))
        });
        let label = TurnLabel::new(&bot, talk.clone());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let mut env = SubtaskEnv::new(
            runner.clone(),
            Some(Arc::new(move |frame| locked(&sink_events).push(frame))),
            label.clone(),
            bot.compute.clone(),
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
            label,
            compute: bot.compute.clone(),
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

    fn interactive_bot() -> BotRunContext {
        let mut bot = parent_bot();
        bot.capabilities
            .extend([BotCapability::BrowserUse, BotCapability::ComputerUse]);
        bot.host_tools = HostTools::linked().with_local_machine();
        bot
    }

    #[tokio::test]
    async fn browser_and_computer_workers_share_one_surface_across_batches() {
        let runner = FakeRunner::new(|_| {
            Behavior::Surface(Box::new(Behavior::Sleep(Duration::from_millis(60))))
        });
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let browser = json!({"tasks":[
            {"instruction":"browser one", "tools":["browser_use"]},
            {"instruction":"browser two", "tools":["browser_use"]}
        ]});
        let computer = json!({"tasks":[{"instruction":"computer", "tools":["computer_use"]}]});
        let (first, second) = tokio::join!(spawn(&rig, browser), spawn(&rig, computer));
        for result in [first, second] {
            for task in result["results"].as_array().unwrap() {
                assert_eq!(task["status"], "done", "{task}");
            }
        }
        for (tool, output) in locked(&runner.calls).iter() {
            assert!(output.contains(r#""status":"ok""#), "{tool}: {output}");
        }
        assert_eq!(runner.peak.load(Ordering::SeqCst), 1);
        assert_eq!(locked(&runner.seen).len(), 3);
    }

    #[tokio::test]
    async fn waiting_surface_workers_leave_capacity_for_research() {
        let runner = FakeRunner::new(|prompt| {
            let sleep = Behavior::Sleep(Duration::from_millis(60));
            if prompt.contains("research") {
                sleep
            } else {
                Behavior::Surface(Box::new(sleep))
            }
        });
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let result = spawn(
            &rig,
            json!({"max_parallel":2, "tasks":[
                {"instruction":"browser one", "tools":["browser_use"]},
                {"instruction":"browser two", "tools":["browser_use"]},
                {"instruction":"computer", "tools":["computer_use"]},
                {"instruction":"research", "tools":["internet"]}
            ]}),
        )
        .await;
        assert!(
            result["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|task| task["status"] == "done")
        );
        assert_eq!(runner.peak.load(Ordering::SeqCst), 2);
        assert!(
            locked(&runner.seen)
                .iter()
                .any(|seen| seen.user_prompt.contains("research"))
        );
    }

    #[tokio::test]
    async fn parent_surface_calls_wait_for_the_workers_observe_act_sequence() {
        let release = Arc::new(tokio::sync::Notify::new());
        let worker_release = release.clone();
        let runner = FakeRunner::new(move |_| {
            Behavior::Surface(Box::new(Behavior::Wait(worker_release.clone())))
        });
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let parent_call = async {
            runner.started.notified().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let call = rig.bridge.call("computer_windows", json!({}));
            tokio::pin!(call);
            assert!(futures::poll!(&mut call).is_pending());
            assert_eq!(*locked(&rig.parent.calls), ["browser_snapshot"]);
            release.notify_one();
            call.await;
            assert_eq!(runner.running.load(Ordering::SeqCst), 0);
        };
        let (_, ()) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                spawn(
                    &rig,
                    json!({"tasks":[{"instruction":"inspect the page", "tools":["browser_use"]}]})
                ),
                parent_call
            )
        })
        .await
        .unwrap();
        assert_eq!(
            *locked(&rig.parent.calls),
            ["browser_snapshot", "computer_windows"]
        );
    }

    #[tokio::test]
    async fn cancellation_drops_the_surface_owner_and_its_waiting_workers() {
        let runner = FakeRunner::new(|_| Behavior::Surface(Box::new(Behavior::UntilCancelled)));
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let cancel = async {
            runner.started.notified().await;
            rig.parent.cancelled.store(true, Ordering::SeqCst);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                spawn(
                    &rig,
                    json!({"tasks":[
                        {"instruction":"inspect the page", "tools":["browser_use"]},
                        {"instruction":"inspect another page", "tools":["browser_use"]},
                        {"instruction":"inspect the computer", "tools":["computer_use"]}
                    ]})
                ),
                cancel
            )
        })
        .await
        .unwrap();
        assert!(
            result["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|task| task["status"] == "failed")
        );
        assert_eq!(runner.running.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn queued_surface_workers_use_the_remaining_turn_budget() {
        let runner = FakeRunner::new(|_| {
            Behavior::Surface(Box::new(Behavior::Sleep(Duration::from_secs(10))))
        });
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            Duration::from_secs(1),
            CHILD_TIME_LIMIT,
        );
        let result = spawn(&rig, tasks(2)).await;
        assert_eq!(result["results"][0]["status"], "timeout");
        assert_eq!(result["results"][1]["status"], "timeout");
        assert_eq!(runner.running.load(Ordering::SeqCst), 0);
        assert_eq!(
            spawn(&rig, tasks(1)).await["results"][0]["status"],
            "budget"
        );
        assert_eq!(locked(&runner.seen).len(), 2);
    }

    #[tokio::test]
    async fn workers_without_interactive_calls_run_in_parallel_despite_interactive_tools() {
        let runner = FakeRunner::new(|_| Behavior::Sleep(Duration::from_millis(80)));
        let rig = rig_with(
            interactive_bot(),
            runner.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        spawn(&rig, tasks(3)).await;
        assert_eq!(runner.peak.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_subtask_budget_is_capped_by_the_host_deadline() {
        assert_eq!(time_until(180_000, 0), Duration::from_secs(180));
        assert_eq!(time_until(10, 50), Duration::ZERO);
        let runner = FakeRunner::replying("x");
        let env = rig_env(
            runner,
            Some(chrono::Utc::now().timestamp_millis() + 180_000),
        );
        assert!(env.turn_budget <= Duration::from_secs(180));
        assert!(env.turn_budget > Duration::from_secs(170));
        let past = rig_env(
            FakeRunner::replying("x"),
            Some(chrono::Utc::now().timestamp_millis() - 1),
        );
        assert_eq!(past.turn_budget, Duration::ZERO);
    }

    fn rig_env(runner: Arc<FakeRunner>, deadline: Option<i64>) -> SubtaskEnv {
        SubtaskEnv::new(
            runner,
            None,
            TurnLabel::new(&parent_bot(), None),
            parent_bot().compute.clone(),
        )
        .with_deadline_at_ms(deadline)
    }

    #[tokio::test]
    async fn workers_have_time_to_plan_execute_and_verify_dependent_steps() {
        let runner = FakeRunner::replying("verified");
        let rig = make_rig(runner.clone());
        spawn(&rig, tasks(1)).await;
        let seen = &locked(&runner.seen)[0];
        assert_eq!(seen.tool_rounds, 64);
        assert!(seen.system_prompt.contains("update_work_plan"));
        assert!(
            seen.system_prompt
                .contains("Never repeat a completed mutation")
        );
        assert!(seen.system_prompt.contains("verification evidence"));
        assert!(CHILD_TIME_LIMIT < TURN_TIME_BUDGET);
        assert!(spawn_subtasks_tool_spec().timeout_secs > TURN_TIME_BUDGET.as_secs());
    }

    #[tokio::test]
    async fn unfinished_workers_preserve_partial_output_without_claiming_completion() {
        let runner = FakeRunner::new(|prompt| {
            if prompt.contains("part 0") {
                Behavior::Incomplete(
                    WorkStopReason::Budget,
                    "Implemented changes; tests remain.".into(),
                )
            } else if prompt.contains("part 1") {
                Behavior::Incomplete(
                    WorkStopReason::Blocked,
                    "Read the page; login is required.".into(),
                )
            } else {
                Behavior::Reply("Verified result".into())
            }
        });
        let rig = make_rig(runner);
        let result = spawn(&rig, tasks(3)).await;
        assert_eq!(result["results"][0]["status"], "budget");
        assert_eq!(
            result["results"][0]["output"],
            "Implemented changes; tests remain."
        );
        assert_eq!(result["results"][1]["status"], "blocked");
        assert_eq!(
            result["results"][1]["output"],
            "Read the page; login is required."
        );
        assert_eq!(result["results"][2]["status"], "done");
        assert!(result["results"][0]["error"].is_string());
        assert!(result["results"][1]["error"].is_string());
    }

    #[tokio::test]
    async fn an_unfinished_schema_worker_does_not_run_a_repair_that_could_mask_the_blocker() {
        let runner = FakeRunner::new(|_| {
            Behavior::Incomplete(
                WorkStopReason::Blocked,
                "The data source is unavailable.".into(),
            )
        });
        let rig = make_rig(runner.clone());
        let result = spawn(
            &rig,
            json!({"tasks":[{
                "instruction":"read the data",
                "output_schema":{"type":"object", "required":["count"]}
            }]}),
        )
        .await;
        assert_eq!(result["results"][0]["status"], "blocked");
        assert_eq!(
            result["results"][0]["output"],
            "The data source is unavailable."
        );
        assert_eq!(locked(&runner.seen).len(), 1);
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
            assert!(child_name_refusal(tool).is_some(), "{tool}");
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
        assert!(
            child_approval_refusal("file_write", &json!({"path":"a.txt","content":"x"})).is_none()
        );
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
                Behavior::CallThenReply("file_list", json!({}), "x".into())
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
        assert!(rig.talk.as_ref().unwrap().taint().is_empty());
        let result = spawn(&rig, tasks(2)).await;
        assert_eq!(
            rig.talk.as_ref().unwrap().taint().classification,
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

    fn connector_rules() -> Vec<flow_like_types::bot_connector::ConnectorRule> {
        use flow_like_types::bot_connector::{ConnectorKind, ConnectorRule, ConnectorToolInfo};
        let tool = |name: &str, read_only| ConnectorToolInfo {
            name: name.into(),
            description: String::new(),
            read_only_hint: read_only,
            destructive_hint: None,
            schema_hash: String::new(),
        };
        vec![ConnectorRule {
            id: "c-docs".into(),
            label: "Docs".into(),
            kind: Some(ConnectorKind::Mcp),
            default_policy: None,
            tool_policies: Default::default(),
            confirmed_hashes: Default::default(),
            tools: vec![tool("search", Some(true)), tool("create", None)],
        }]
    }

    #[tokio::test]
    async fn a_child_may_use_a_connector_call_that_runs_but_not_one_that_asks() {
        let calls = |tool: &'static str| {
            FakeRunner::new(move |_| {
                Behavior::CallThenReply(
                    "connector_call",
                    json!({"connector": "Docs", "tool": tool, "arguments": {}}),
                    "x".into(),
                )
            })
        };
        let bot = || BotRunContext {
            connectors: connector_rules(),
            ..parent_bot()
        };
        let reading = calls("search");
        let rig = rig_with(
            bot(),
            reading.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let result = spawn(&rig, tasks(1)).await;
        assert!(result["results"][0].get("refused").is_none(), "{result}");
        assert_eq!(*locked(&rig.parent.calls), ["connector_call"]);

        let writing = calls("create");
        let rig = rig_with(
            bot(),
            writing.clone(),
            ParentBridge::new(),
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
        );
        let result = spawn(&rig, tasks(1)).await;
        assert!(locked(&rig.parent.calls).is_empty());
        assert_eq!(result["results"][0]["refused"].as_array().unwrap().len(), 1);
        let (_, output) = locked(&writing.calls)[0].clone();
        assert!(output.contains("subtask_tool_refused"), "{output}");
    }

    #[tokio::test]
    async fn a_child_classification_raises_the_turn_label_without_bot_talk() {
        let runner = FakeRunner::new(|_| {
            Behavior::CallThenReply("file_read", json!({"file": "a"}), "x".into())
        });
        let parent = Arc::new(ParentBridge {
            cancelled: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            classification: Some("restricted"),
        });
        let rig = build_rig(
            parent_bot(),
            runner,
            parent,
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
            false,
        );
        assert!(rig.talk.is_none() && rig.label.get().is_empty());
        spawn(&rig, tasks(1)).await;
        assert_eq!(
            rig.label.get().classification,
            Some(flow_like_types::data_classification::DataClassification::Restricted)
        );
        let refused = rig
            .bridge
            .call(
                "email_send",
                json!({"to": ["a@b.c"], "subject": "s", "body": "b"}),
            )
            .await;
        assert!(refused.contains("turn_label_outbound_refused"), "{refused}");
        assert!(!locked(&rig.parent.calls).contains(&"email_send".to_string()));
        let fine = rig.bridge.call("list_apps", json!({})).await;
        assert!(!fine.contains("turn_label_outbound_refused"));
    }

    #[tokio::test]
    async fn data_at_the_bots_own_level_does_not_block_outbound_calls() {
        let bot = BotRunContext {
            classification: DataSensitivity {
                classification: Some(DataClassification::Confidential),
                pii: false,
            },
            ..parent_bot()
        };
        let parent = Arc::new(ParentBridge {
            cancelled: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            classification: Some("confidential"),
        });
        let rig = build_rig(
            bot,
            FakeRunner::replying("x"),
            parent,
            TURN_TIME_BUDGET,
            CHILD_TIME_LIMIT,
            false,
        );
        rig.bridge.call("file_read", json!({"file": "a"})).await;
        rig.bridge.call("email_send", json!({})).await;
        assert!(locked(&rig.parent.calls).contains(&"email_send".to_string()));
    }

    #[tokio::test]
    async fn parallel_children_are_charged_their_summed_compute() {
        let runner = FakeRunner::new(|_| Behavior::Sleep(Duration::from_millis(300)));
        let rig = make_rig(runner);
        assert_eq!(rig.compute.extra_millis(), 0);
        let result = spawn(&rig, tasks(3)).await;
        let summed: u64 = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["usage"]["elapsed_ms"].as_u64().unwrap())
            .sum();
        assert!(summed >= 900, "{summed}");
        let extra = rig.compute.extra_millis() as u64;
        assert!(
            extra >= 500,
            "three parallel 300 ms children must cost about 600 ms extra: {extra}"
        );
        assert!(extra <= summed);
    }

    #[test]
    fn compute_without_children_adds_nothing_and_never_goes_negative() {
        let compute = TurnCompute::default();
        compute.add_waited(500);
        assert_eq!(compute.extra_millis(), 0);
        compute.add_child(1_500);
        assert_eq!(compute.extra_millis(), 1_000);
    }

    #[tokio::test]
    async fn named_files_are_opened_through_the_parent_and_given_to_the_child_as_handles() {
        let runner = FakeRunner::replying("ok");
        let rig = make_rig(runner.clone());
        let result = spawn(
            &rig,
            json!({"tasks": [{"instruction": "read it", "files": ["notes/a.txt"]}]}),
        )
        .await;
        assert_eq!(result["results"][0]["status"], "done");
        assert_eq!(*locked(&rig.parent.calls), ["file_read"]);
        let seen = locked(&runner.seen)[0].clone();
        assert!(
            seen.user_prompt.contains("notes/a.txt"),
            "{}",
            seen.user_prompt
        );
        assert!(
            seen.user_prompt.contains("\"app_id\":\"app-workspace\""),
            "{}",
            seen.user_prompt
        );
    }

    #[tokio::test]
    async fn an_unreadable_or_out_of_scope_file_refuses_the_call_before_any_child_runs() {
        let runner = FakeRunner::replying("ok");
        let rig = make_rig(runner.clone());
        for file in ["missing.txt", "app:other/secret.txt"] {
            let result = spawn(
                &rig,
                json!({"tasks": [{"instruction": "a"}, {"instruction": "b", "files": ["ok.txt", file]}]}),
            )
            .await;
            assert_eq!(result["code"], "subtasks_file_unresolved", "{file}");
            assert!(result["message"].as_str().unwrap().contains(file));
        }
        assert!(locked(&runner.seen).is_empty());
    }

    #[test]
    fn the_sub_task_model_is_used_only_where_the_host_can_run_it() {
        assert_eq!(
            child_model(Some("small"), true, Some("big")),
            Some("small".into())
        );
        assert_eq!(
            child_model(Some("small"), false, Some("big")),
            Some("big".into())
        );
        assert_eq!(child_model(None, true, Some("big")), Some("big".into()));
        assert_eq!(child_model(Some("small"), false, None), None);
    }
}

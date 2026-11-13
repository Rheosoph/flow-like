//! Durable bot execution boundaries. A saved tool batch with an unknown outcome is never replayed.

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use flow_like_model_provider::response::LLMUsageStats;
use flow_like_types::{
    Result, anyhow,
    bot_turn::{BotRunCheckpoint, DEFAULT_TURN_DEADLINE_SECS},
};
use rig::{
    OneOrMany,
    message::{AssistantContent, Message, ToolResultContent, UserContent},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::platform::WorkStopReason;
use super::work_plan::WorkPlanState;

const SNAPSHOT_VERSION: u32 = 1;
pub(super) const MAX_RECOVERIES: u32 = 3;
const WAITING_CODES: [&str; 2] = [
    "bot_tool_waiting_for_approval",
    "bot_talk_waiting_for_owner",
];

/// The pause code of a call that a host gate held back for the owner's answer.
pub(super) fn waiting_code(output: &str) -> Option<&'static str> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    let code = value.get("code")?.as_str()?;
    WAITING_CODES.into_iter().find(|waiting| *waiting == code)
}

/// The pause code of a fresh tool output, only when it has the shape of a host gate's hold
/// (`status: "waiting"`); a tool's own error that happens to carry the code does not pause.
pub(super) fn gate_waiting_code(output: &str) -> Option<&'static str> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    (value.get("status")?.as_str()? == "waiting")
        .then(|| waiting_code(output))
        .flatten()
}

/// Storage is scoped to one claimed turn. Hosts must encrypt private state, validate ownership,
/// and reject stale writers with the checkpoint revision and their current claim fence.
#[async_trait]
pub trait BotRunRecovery: Debug + Send + Sync {
    async fn load(&self) -> Result<Option<BotRunCheckpoint>>;
    async fn save(&self, checkpoint: &BotRunCheckpoint) -> Result<()>;
    fn deadline_at_ms(&self) -> Option<i64> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct RecoveryHandle(Arc<dyn BotRunRecovery>);

impl RecoveryHandle {
    pub fn as_store(&self) -> Arc<dyn BotRunRecovery> {
        self.0.clone()
    }
}

impl From<Arc<dyn BotRunRecovery>> for RecoveryHandle {
    fn from(value: Arc<dyn BotRunRecovery>) -> Self {
        Self(value)
    }
}

impl PartialEq for RecoveryHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RecoveryHandle {}

/// The call a lane runs or asks about: its durable id, and whether web content was in the model's
/// context when the call's round began.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallScope {
    pub id: String,
    pub round_web_content: bool,
    /// The call was held in a parked round and runs now that the owner answered its card.
    pub held_replay: bool,
}

flow_like_types::tokio::task_local! {
    static CALL_SCOPE: CallScope;
}

/// Hosts capture this id when a gate confirms that a call has not executed and parks it.
pub fn current_recovery_call_id() -> Option<String> {
    CALL_SCOPE.try_with(|scope| scope.id.clone()).ok()
}

/// Whether the running call's round began with web content in the model's context.
pub fn round_has_web_content() -> bool {
    CALL_SCOPE
        .try_with(|scope| scope.round_web_content)
        .unwrap_or(false)
}

/// Whether the running call is a held call of an answered round. One answer on the card covers
/// every identical call of that round, and no call after it.
pub fn replays_held_call() -> bool {
    CALL_SCOPE
        .try_with(|scope| scope.held_replay)
        .unwrap_or(false)
}

/// The running call's scope, for a host that leaves the task before the call reaches its gate.
pub fn current_call_scope() -> Option<CallScope> {
    CALL_SCOPE.try_with(Clone::clone).ok()
}

/// Runs `future` as the call `scope` names; gates read it through [`current_recovery_call_id`],
/// [`round_has_web_content`] and [`replays_held_call`].
pub async fn scope_call<T>(scope: CallScope, future: impl std::future::Future<Output = T>) -> T {
    CALL_SCOPE.scope(scope, future).await
}

/// A host may publish an already-finished reply without resolving a model or rerunning tools.
pub fn finished_reply(checkpoint: &BotRunCheckpoint) -> Result<Option<String>> {
    Ok(load_checkpoint(checkpoint)?
        .answer
        .map(|answer| answer.output))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryPhase {
    Ready,
    ToolsInFlight,
    Finished,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PendingCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub output: Option<String>,
    /// Known not to have run: the round never dispatched it, or a host gate confirmed it parked
    /// for the owner. The resumed round runs it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub held: bool,
}

impl PendingCall {
    fn open(&self) -> bool {
        self.output.is_none()
    }

    fn tool_call(&self) -> rig::message::ToolCall {
        rig::message::ToolCall::new(
            self.id.clone(),
            rig::message::ToolFunction {
                name: self.name.clone(),
                arguments: self.arguments.clone(),
            },
        )
    }
}

/// A parked round the resume runs again: its calls in declared order, with the results that were
/// recorded before it parked. Only its held calls run.
#[derive(Debug, Clone)]
pub(super) struct HeldRound {
    calls: Vec<PendingCall>,
}

impl HeldRound {
    pub(super) fn of(snapshot: &RunSnapshot) -> Option<Self> {
        snapshot.resumes_held_round().then(|| Self {
            calls: snapshot.pending_calls.clone(),
        })
    }

    pub(super) fn tool_calls(&self) -> Vec<rig::message::ToolCall> {
        self.calls.iter().map(PendingCall::tool_call).collect()
    }

    /// The results recorded before the round parked, by call id.
    pub(super) fn recorded(&self) -> std::collections::HashMap<String, String> {
        self.calls
            .iter()
            .filter_map(|call| Some((call.id.clone(), call.output.clone()?)))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedAnswer {
    pub output: String,
    pub stop_reason: Option<WorkStopReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunSnapshot {
    pub version: u32,
    pub phase: RecoveryPhase,
    pub original_request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_delivery: Option<super::decision_event::EventDelivery>,
    pub history: Vec<Message>,
    pub prompt: Message,
    pub reference_fingerprints: Vec<String>,
    pub work_plan: Value,
    pub steering: Vec<String>,
    pub iteration: usize,
    pub model_calls_reserved: usize,
    pub max_tool_rounds: usize,
    pub plan_step_counter: u32,
    pub empty_continuations: usize,
    pub progress_seen: Vec<String>,
    pub stats: LLMUsageStats,
    /// Bounded usage records saved by the host, including delegated workers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage_stats: Vec<Value>,
    pub web: Value,
    pub delegated_web: Value,
    pub local_app_discovery_returned: bool,
    pub room_refreshes_remaining: usize,
    pub recoveries: u32,
    pub approval_resume: bool,
    pub pending_calls: Vec<PendingCall>,
    pub answer: Option<SavedAnswer>,
    /// Web content was in the model's context when the saved round began.
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_content: bool,
}

impl RunSnapshot {
    pub fn version() -> u32 {
        SNAPSHOT_VERSION
    }

    fn validate(&self) -> Result<()> {
        if self.version != SNAPSHOT_VERSION
            || self.max_tool_rounds == 0
            || self.max_tool_rounds > super::work_plan::BOT_TOOL_ROUNDS
            || self.iteration > self.max_tool_rounds
            || self.model_calls_reserved > self.max_tool_rounds + 1 + 3
            || self.empty_continuations > 3
            || self.room_refreshes_remaining > 3
            || self.recoveries > MAX_RECOVERIES
            || self.history.len() > 2_048
            || self.pending_calls.len() > 256
            || self.reference_fingerprints.len() > 128
            || self.progress_seen.len() > 16_384
        {
            return Err(anyhow!(
                "Execution checkpoint has invalid version or limits"
            ));
        }
        if (self.phase == RecoveryPhase::Finished) != self.answer.is_some()
            || (self.phase == RecoveryPhase::ToolsInFlight) != !self.pending_calls.is_empty()
        {
            return Err(anyhow!("Execution checkpoint phase is inconsistent"));
        }
        for hash in self
            .reference_fingerprints
            .iter()
            .chain(&self.progress_seen)
        {
            blake3::Hash::from_hex(hash).map_err(|_| anyhow!("Invalid checkpoint fingerprint"))?;
        }
        if self
            .pending_calls
            .iter()
            .any(|call| call.name.is_empty() || call.name.len() > 256 || call.id.len() > 512)
        {
            return Err(anyhow!("Execution checkpoint has invalid pending calls"));
        }
        if let Some(call) = self
            .pending_calls
            .iter()
            .find(|call| call.held && !call.open())
        {
            return Err(anyhow!(
                "Execution checkpoint holds {} ({}) although its result is recorded",
                call.name,
                call.id
            ));
        }
        let mut ids = std::collections::HashSet::new();
        if self.pending_calls.iter().any(|call| !ids.insert(&call.id)) {
            return Err(anyhow!("Execution checkpoint repeats a pending call id"));
        }
        if self.phase != RecoveryPhase::Finished {
            validate_tool_pairs(&self.history, &self.prompt)?;
        }
        Ok(())
    }

    /// A parked round whose open calls are all known not to have run: the resume runs them
    /// instead of ending the turn with unknown outcomes.
    pub(super) fn resumes_held_round(&self) -> bool {
        self.phase == RecoveryPhase::ToolsInFlight
            && self.pending_calls.iter().any(PendingCall::open)
            && self
                .pending_calls
                .iter()
                .filter(|call| call.open())
                .all(|call| call.held)
    }

    fn without_transient_content(mut self) -> Self {
        self.history = self.history.iter().map(durable_message).collect();
        self.prompt = durable_message(&self.prompt);
        if self.phase == RecoveryPhase::Finished {
            self.history.clear();
            self.prompt = Message::User {
                content: OneOrMany::one(UserContent::text("Finished execution.")),
            };
        }
        self
    }

    fn promote_completed_batch(&mut self) -> Result<bool> {
        if self.phase != RecoveryPhase::ToolsInFlight
            || self.pending_calls.iter().any(|call| call.output.is_none())
        {
            return Ok(false);
        }
        self.web_content |= self.pending_calls.iter().any(|call| {
            call.output.as_deref().is_some_and(|output| {
                super::turn_label::brings_web_content(&call.name, &call.arguments, output)
            })
        });
        self.history.push(self.prompt.clone());
        self.history.push(Message::Assistant {
            id: None,
            content: OneOrMany::many(
                self.pending_calls
                    .iter()
                    .map(|call| AssistantContent::ToolCall(call.tool_call()))
                    .collect::<Vec<_>>(),
            )
            .expect("batch contains calls"),
        });
        self.prompt = Message::User {
            content: OneOrMany::many(
                self.pending_calls
                    .iter()
                    .map(|call| {
                        UserContent::ToolResult(rig::message::ToolResult {
                            id: call.id.clone(),
                            call_id: None,
                            content: OneOrMany::one(ToolResultContent::text(
                                call.output.clone().expect("batch is complete"),
                            )),
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("batch contains results"),
        };
        self.iteration += 1;
        self.pending_calls.clear();
        self.phase = RecoveryPhase::Ready;
        self.validate()?;
        Ok(true)
    }
}

fn load_checkpoint(checkpoint: &BotRunCheckpoint) -> Result<RunSnapshot> {
    checkpoint.validate().map_err(|error| anyhow!(error))?;
    let snapshot: RunSnapshot = serde_json::from_value(checkpoint.state.clone())?;
    snapshot.validate()?;
    Ok(snapshot)
}

fn next_checkpoint(
    checkpoint: &BotRunCheckpoint,
    snapshot: RunSnapshot,
) -> Result<BotRunCheckpoint> {
    let next = BotRunCheckpoint {
        revision: checkpoint
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow!("Checkpoint revision exhausted"))?,
        state: serde_json::to_value(snapshot)?,
        ..checkpoint.clone()
    };
    next.validate().map_err(|error| anyhow!(error))?;
    Ok(next)
}

/// Whether a promoted round's last call is `call_id` and its saved result is a gate's pause.
fn reports_paused_call(snapshot: &RunSnapshot, call_id: &str, tool_name: &str) -> bool {
    let last_call = snapshot.history.iter().rev().find_map(|message| match message {
        Message::Assistant { content, .. } if content.iter().any(|item| matches!(item, AssistantContent::ToolCall(_))) => Some(content),
        _ => None,
    }).is_some_and(|content| content.iter().any(|item| matches!(item, AssistantContent::ToolCall(call) if call.id == call_id && call.function.name == tool_name)));
    let waiting_result = match &snapshot.prompt {
        Message::User { content } => content.iter().any(|item| match item {
            UserContent::ToolResult(result) if result.id == call_id => {
                result.content.iter().any(|item| match item {
                    ToolResultContent::Text(text) => waiting_code(&text.text).is_some(),
                    _ => false,
                })
            }
            _ => false,
        }),
        _ => false,
    };
    last_call && waiting_result
}

/// Whether a host should resume this checkpoint's parked round as it is: every open call is
/// held, so the resume runs them without another model round (and needs no resume note).
pub fn resumes_held_round(checkpoint: &BotRunCheckpoint) -> bool {
    load_checkpoint(checkpoint).is_ok_and(|snapshot| snapshot.resumes_held_round())
}

/// Confirms the calls a host's gates parked, `(call id, tool)`: each open call among them is held
/// and runs on resume. An open call no gate confirmed keeps its unknown outcome, so a spoofed
/// waiting result is never replayed. Once every open call is held the resume is an approval
/// resume. A round saved before held calls existed (already promoted with the pause as its
/// result) is reconciled as before. The returned checkpoint is ready for the host's fenced save.
pub fn record_held_batch(
    checkpoint: &BotRunCheckpoint,
    parked: &[(String, String)],
) -> Result<Option<BotRunCheckpoint>> {
    let mut snapshot = load_checkpoint(checkpoint)?;
    let changed = match snapshot.phase {
        RecoveryPhase::Ready => {
            let paused = parked
                .iter()
                .any(|(id, tool)| reports_paused_call(&snapshot, id, tool));
            let resumes = paused && !snapshot.approval_resume;
            snapshot.approval_resume |= resumes;
            resumes
        }
        RecoveryPhase::ToolsInFlight => {
            let mut changed = false;
            for call in snapshot
                .pending_calls
                .iter_mut()
                .filter(|call| call.open() && !call.held)
            {
                call.held = parked
                    .iter()
                    .any(|(id, tool)| *id == call.id && *tool == call.name);
                changed |= call.held;
            }
            let resumes = snapshot.resumes_held_round() && !snapshot.approval_resume;
            snapshot.approval_resume |= resumes;
            changed || resumes
        }
        RecoveryPhase::Finished => false,
    };
    if !changed {
        return Ok(None);
    }
    next_checkpoint(checkpoint, snapshot).map(Some)
}

pub(super) fn image_fingerprints(images: &[UserContent]) -> Vec<String> {
    images
        .iter()
        .filter_map(|image| serde_json::to_vec(image).ok())
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
        .collect()
}

fn durable_message(message: &Message) -> Message {
    match message {
        Message::User { content } => {
            let content: Vec<_> = content.iter().map(|item| match item {
                UserContent::Image(_) | UserContent::Audio(_) | UserContent::Video(_) | UserContent::Document(_) =>
                    UserContent::text("[Transient attachment omitted from durable checkpoint. Reopen the original attachment or capture a fresh observation before relying on it.]"),
                UserContent::ToolResult(result) => {
                    let mut result = result.clone();
                    result.content = OneOrMany::many(result.content.iter().map(|content| match content {
                        ToolResultContent::Image(_) => ToolResultContent::text("[Transient tool image omitted; capture a fresh observation.]"),
                        other => other.clone(),
                    }).collect::<Vec<_>>()).expect("tool result content is nonempty");
                    UserContent::ToolResult(result)
                }
                other => other.clone(),
            }).collect();
            Message::User {
                content: OneOrMany::many(content).expect("user message has content"),
            }
        }
        Message::Assistant { id, content } => {
            let mut content: Vec<_> = content
                .iter()
                .filter(|item| {
                    matches!(
                        item,
                        AssistantContent::Text(_) | AssistantContent::ToolCall(_)
                    )
                })
                .cloned()
                .collect();
            if content.is_empty() {
                content.push(AssistantContent::text("[Transient model content omitted.]"));
            }
            Message::Assistant {
                id: id.clone(),
                content: OneOrMany::many(content).expect("assistant message has content"),
            }
        }
        // Runtime snapshots never need a system prompt; policy is rebuilt from current settings.
        Message::System { .. } => Message::User {
            content: OneOrMany::one(UserContent::text(
                "[Previous system context omitted. Current system policy applies.]",
            )),
        },
    }
}

fn validate_tool_pairs(history: &[Message], prompt: &Message) -> Result<()> {
    let mut pending = std::collections::HashSet::new();
    for message in history.iter().chain(std::iter::once(prompt)) {
        match message {
            Message::Assistant { content, .. } => {
                if !pending.is_empty() {
                    return Err(anyhow!("Checkpoint has incomplete tool results"));
                }
                for item in content.iter() {
                    if let AssistantContent::ToolCall(call) = item {
                        if !pending.insert(call.id.clone()) {
                            return Err(anyhow!("Checkpoint has repeated tool call ids"));
                        }
                    }
                }
            }
            Message::User { content } => {
                for item in content.iter() {
                    if let UserContent::ToolResult(result) = item {
                        if !pending.remove(&result.id) {
                            return Err(anyhow!("Checkpoint has an orphan tool result"));
                        }
                    }
                }
            }
            Message::System { .. } => {
                return Err(anyhow!("Checkpoint cannot restore system instructions"));
            }
        }
    }
    if !pending.is_empty() {
        return Err(anyhow!("Checkpoint has incomplete tool results"));
    }
    Ok(())
}

fn unrecorded_call<'a>(
    snapshot: &'a mut RunSnapshot,
    id: &str,
    name: &str,
) -> Result<&'a mut PendingCall> {
    if snapshot.phase != RecoveryPhase::ToolsInFlight {
        return Err(anyhow!(
            "Cannot record a result outside its durable tool batch"
        ));
    }
    let call = snapshot
        .pending_calls
        .iter_mut()
        .find(|call| call.id == id && call.name == name)
        .ok_or_else(|| anyhow!("Tool result for {name} ({id}) does not match its durable call"))?;
    if call.output.is_some() {
        return Err(anyhow!(
            "Tool result for {name} ({id}) was already recorded"
        ));
    }
    Ok(call)
}

pub(super) struct RecoverySession {
    store: Arc<dyn BotRunRecovery>,
    header: BotRunCheckpoint,
}

impl RecoverySession {
    pub async fn terminal_before_model(
        store: Arc<dyn BotRunRecovery>,
        partial_answer: Option<&str>,
    ) -> Result<Option<SavedAnswer>> {
        let (mut recovery, snapshot) = Self::load(store).await?;
        let Some(mut snapshot) = snapshot else {
            return Ok(None);
        };
        if let Some(answer) = snapshot.answer {
            return Ok(Some(answer));
        }
        let plan = WorkPlanState::restore(snapshot.work_plan.clone())?;
        let calls_named = |held: bool| {
            snapshot
                .pending_calls
                .iter()
                .filter(|call| call.open() && call.held == held)
                .map(|call| format!("{} ({})", call.name, call.id))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let held = calls_named(true);
        let stopped = if snapshot.phase == RecoveryPhase::ToolsInFlight
            && !snapshot.resumes_held_round()
        {
            let calls = calls_named(false);
            let known = snapshot
                .pending_calls
                .iter()
                .filter_map(|call| {
                    call.output.as_ref().map(|output| {
                        format!(
                            "- Recorded result: {} ({}), {}",
                            call.name,
                            call.id,
                            super::stream::tool_result_summary(output)
                        )
                    })
                })
                .take(64)
                .collect::<Vec<_>>()
                .join("\n");
            Some((
                WorkStopReason::Blocked,
                format!(
                    "The process stopped while these tool calls were in flight: {calls}. Their outcomes are unknown, so I have not repeated them. Check the affected app, browser, files or external service before continuing.\n{known}"
                ),
            ))
        } else if recovery.expired() {
            Some((
                WorkStopReason::Budget,
                match partial_answer.map(str::trim).filter(|text| !text.is_empty()) {
                    Some(text) => format!(
                        "{text}\n\n(This answer was cut off at the execution deadline; I started no more model or tool work.)"
                    ),
                    None => "The original execution deadline passed while this task was interrupted. I have not started more model or tool work.".into(),
                },
            ))
        } else if snapshot.recoveries >= MAX_RECOVERIES && !snapshot.approval_resume {
            Some((WorkStopReason::Blocked, "This task reached its automatic recovery limit. I have not started another attempt.".into()))
        } else {
            None
        };
        let Some((reason, mut output)) = stopped else {
            return Ok(None);
        };
        if !held.is_empty() {
            output.push_str(&format!(
                "\nThese calls waited for the owner's approval and did not run: {held}."
            ));
        }
        if let Some(handoff) = plan.handoff() {
            output.push_str(&format!("\n\n{handoff}"));
        }
        let answer = SavedAnswer {
            output,
            stop_reason: Some(reason),
        };
        snapshot.phase = RecoveryPhase::Finished;
        snapshot.pending_calls.clear();
        snapshot.answer = Some(answer.clone());
        recovery.save(snapshot).await?;
        Ok(Some(answer))
    }

    pub async fn load(store: Arc<dyn BotRunRecovery>) -> Result<(Self, Option<RunSnapshot>)> {
        if let Some(header) = store.load().await? {
            header.validate().map_err(|error| anyhow!(error))?;
            let snapshot: RunSnapshot = serde_json::from_value(header.state.clone())?;
            snapshot.validate()?;
            return Ok((Self { store, header }, Some(snapshot)));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let deadline = store
            .deadline_at_ms()
            .unwrap_or(now + DEFAULT_TURN_DEADLINE_SECS as i64 * 1_000)
            .min(now + DEFAULT_TURN_DEADLINE_SECS as i64 * 1_000);
        Ok((
            Self {
                store,
                header: BotRunCheckpoint {
                    revision: 0,
                    started_at_ms: now,
                    deadline_at_ms: deadline,
                    state: Value::Null,
                },
            },
            None,
        ))
    }

    pub fn deadline_at_ms(&self) -> i64 {
        self.store
            .deadline_at_ms()
            .map_or(self.header.deadline_at_ms, |deadline| {
                deadline.min(self.header.deadline_at_ms)
            })
    }
    pub fn expired(&self) -> bool {
        chrono::Utc::now().timestamp_millis() >= self.deadline_at_ms()
    }

    pub async fn record_result(
        &mut self,
        id: &str,
        name: &str,
        output: String,
        work_plan: Value,
        web: Value,
        delegated_web: Value,
        stats: LLMUsageStats,
    ) -> Result<()> {
        let mut snapshot: RunSnapshot = serde_json::from_value(self.header.state.clone())?;
        let call = unrecorded_call(&mut snapshot, id, name)?;
        let discovered_apps = name == "list_apps"
            && serde_json::from_str::<Value>(&output)
                .ok()
                .is_some_and(|value| {
                    matches!(
                        value.get("status").and_then(Value::as_str),
                        Some("ok" | "partial")
                    )
                });
        call.output = Some(output);
        call.held = false;
        snapshot.work_plan = work_plan;
        snapshot.web = web;
        snapshot.delegated_web = delegated_web;
        snapshot.stats = stats;
        snapshot.local_app_discovery_returned |= discovered_apps;
        snapshot.empty_continuations = 0;
        snapshot.promote_completed_batch()?;
        self.save(snapshot).await
    }

    /// Calls of a parked round that were never dispatched: they are known not to have run, so the
    /// batch stays in flight and the resumed round runs them.
    pub async fn record_held(&mut self, calls: &[(String, String)]) -> Result<()> {
        if calls.is_empty() {
            return Ok(());
        }
        let mut snapshot: RunSnapshot = serde_json::from_value(self.header.state.clone())?;
        for (id, name) in calls {
            unrecorded_call(&mut snapshot, id, name)?.held = true;
        }
        self.save(snapshot).await
    }

    /// A resumed round is about to run its held calls: they stop being held before any runs, so
    /// a crash during the round leaves them unknown instead of replaying a call that may have run.
    pub async fn release_held(&mut self) -> Result<()> {
        let mut snapshot: RunSnapshot = serde_json::from_value(self.header.state.clone())?;
        if !snapshot.pending_calls.iter().any(|call| call.held) {
            return Ok(());
        }
        for call in &mut snapshot.pending_calls {
            call.held = false;
        }
        self.save(snapshot).await
    }

    pub async fn save(&mut self, snapshot: RunSnapshot) -> Result<()> {
        let snapshot = snapshot.without_transient_content();
        snapshot.validate()?;
        let next = BotRunCheckpoint {
            revision: self
                .header
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow!("Checkpoint revision exhausted"))?,
            state: serde_json::to_value(snapshot)?,
            ..self.header.clone()
        };
        next.validate().map_err(|error| anyhow!(error))?;
        self.store.save(&next).await?;
        self.header = next;
        Ok(())
    }
}

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

flow_like_types::tokio::task_local! {
    static RECOVERY_CALL_ID: String;
}

/// Hosts capture this id when a gate confirms that a call has not executed and parks it.
pub fn current_recovery_call_id() -> Option<String> {
    RECOVERY_CALL_ID.try_with(Clone::clone).ok()
}

pub(super) async fn scope_call<T>(id: String, future: impl std::future::Future<Output = T>) -> T {
    RECOVERY_CALL_ID.scope(id, future).await
}

/// A host may publish an already-finished reply without resolving a model or rerunning tools.
pub fn finished_reply(checkpoint: &BotRunCheckpoint) -> Result<Option<String>> {
    checkpoint.validate().map_err(|error| anyhow!(error))?;
    let snapshot: RunSnapshot = serde_json::from_value(checkpoint.state.clone())?;
    snapshot.validate()?;
    Ok(snapshot.answer.map(|answer| answer.output))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryPhase {
    Ready,
    ToolsInFlight,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PendingCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub output: Option<String>,
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
    pub web: Value,
    pub delegated_web: Value,
    pub local_app_discovery_returned: bool,
    pub room_refreshes_remaining: usize,
    pub recoveries: u32,
    pub approval_resume: bool,
    pub pending_calls: Vec<PendingCall>,
    pub answer: Option<SavedAnswer>,
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
        let mut ids = std::collections::HashSet::new();
        if self.pending_calls.iter().any(|call| !ids.insert(&call.id)) {
            return Err(anyhow!("Execution checkpoint repeats a pending call id"));
        }
        if self.phase != RecoveryPhase::Finished {
            validate_tool_pairs(&self.history, &self.prompt)?;
        }
        Ok(())
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
        self.history.push(self.prompt.clone());
        self.history.push(Message::Assistant {
            id: None,
            content: OneOrMany::many(
                self.pending_calls
                    .iter()
                    .map(|call| {
                        AssistantContent::ToolCall(rig::message::ToolCall::new(
                            call.id.clone(),
                            rig::message::ToolFunction {
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                            },
                        ))
                    })
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

/// Reconcile only the exact call that a trusted host gate knows did not run. Other unresolved
/// calls retain their unknown status. The returned checkpoint is ready for the host's fenced save.
pub fn record_unexecuted_approval(
    checkpoint: &BotRunCheckpoint,
    call_id: &str,
    tool_name: &str,
) -> Result<Option<BotRunCheckpoint>> {
    checkpoint.validate().map_err(|error| anyhow!(error))?;
    let mut snapshot: RunSnapshot = serde_json::from_value(checkpoint.state.clone())?;
    snapshot.validate()?;
    if snapshot.phase == RecoveryPhase::Ready {
        let last_call = snapshot.history.iter().rev().find_map(|message| match message {
            Message::Assistant { content, .. } if content.iter().any(|item| matches!(item, AssistantContent::ToolCall(_))) => Some(content),
            _ => None,
        }).is_some_and(|content| content.iter().any(|item| matches!(item, AssistantContent::ToolCall(call) if call.id == call_id && call.function.name == tool_name)));
        let waiting_result = match &snapshot.prompt {
            Message::User { content } => content.iter().any(|item| match item {
                UserContent::ToolResult(result) if result.id == call_id => {
                    result.content.iter().any(|item| match item {
                        ToolResultContent::Text(text) => serde_json::from_str::<Value>(&text.text)
                            .ok()
                            .is_some_and(|value| {
                                matches!(
                                    value.get("code").and_then(Value::as_str),
                                    Some(
                                        "bot_tool_waiting_for_approval"
                                            | "bot_talk_waiting_for_owner"
                                    )
                                )
                            }),
                        _ => false,
                    })
                }
                _ => false,
            }),
            _ => false,
        };
        if !last_call || !waiting_result || snapshot.approval_resume {
            return Ok(None);
        }
    } else if snapshot.phase == RecoveryPhase::ToolsInFlight {
        let Some(call) = snapshot
            .pending_calls
            .iter_mut()
            .find(|call| call.id == call_id && call.name == tool_name)
        else {
            return Ok(None);
        };
        if call.output.is_some() {
            return Ok(None);
        }
        call.output = Some(serde_json::json!({
        "status": "error", "code": "bot_tool_waiting_for_approval", "executed": false,
        "message": "The host confirmed this call did not execute because it required approval. After the owner answers, reassess the current state and request the action under the current permission; do not repeat completed sibling actions.",
        }).to_string());
        snapshot.promote_completed_batch()?;
    } else {
        return Ok(None);
    }
    snapshot.approval_resume = true;
    let next = BotRunCheckpoint {
        revision: checkpoint
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow!("Checkpoint revision exhausted"))?,
        state: serde_json::to_value(snapshot)?,
        ..checkpoint.clone()
    };
    next.validate().map_err(|error| anyhow!(error))?;
    Ok(Some(next))
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

pub(super) struct RecoverySession {
    store: Arc<dyn BotRunRecovery>,
    header: BotRunCheckpoint,
}

impl RecoverySession {
    pub async fn terminal_before_model(
        store: Arc<dyn BotRunRecovery>,
        _original_request: &str,
    ) -> Result<Option<SavedAnswer>> {
        let (mut recovery, snapshot) = Self::load(store).await?;
        let Some(mut snapshot) = snapshot else {
            return Ok(None);
        };
        if let Some(answer) = snapshot.answer {
            return Ok(Some(answer));
        }
        let plan = WorkPlanState::restore(snapshot.work_plan.clone())?;
        let stopped = if snapshot.phase == RecoveryPhase::ToolsInFlight {
            let calls = snapshot
                .pending_calls
                .iter()
                .filter(|call| call.output.is_none())
                .map(|call| format!("{} ({})", call.name, call.id))
                .collect::<Vec<_>>()
                .join(", ");
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
            Some((WorkStopReason::Budget, "The original execution deadline passed while this task was interrupted. I have not started more model or tool work.".into()))
        } else if snapshot.recoveries >= MAX_RECOVERIES && !snapshot.approval_resume {
            Some((WorkStopReason::Blocked, "This task reached its automatic recovery limit. I have not started another attempt.".into()))
        } else {
            None
        };
        let Some((reason, mut output)) = stopped else {
            return Ok(None);
        };
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
        if snapshot.phase != RecoveryPhase::ToolsInFlight {
            return Err(anyhow!(
                "Cannot record a result outside its durable tool batch"
            ));
        }
        let Some(call) = snapshot
            .pending_calls
            .iter_mut()
            .find(|call| call.id == id && call.name == name)
        else {
            return Err(anyhow!("Tool result does not match its durable call"));
        };
        if call.output.is_some() {
            return Err(anyhow!("Tool result was already recorded"));
        }
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
        snapshot.work_plan = work_plan;
        snapshot.web = web;
        snapshot.delegated_web = delegated_web;
        snapshot.stats = stats;
        snapshot.local_app_discovery_returned |= discovered_apps;
        snapshot.empty_continuations = 0;
        snapshot.promote_completed_batch()?;
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

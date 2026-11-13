//! The server's bot host: claims turns of `server` bots and answers them without a browser.
//! A turn loads the bot (its tools limited to web research and memory, the only ones the server
//! runs itself), picks its model, reserves the owner's usage, reads the last entries of the
//! bot's copy of the conversation and posts the reply as the bot, in the transaction that
//! completes the turn. A failed turn posts its error as the bot's message instead.

use super::{
    approvals,
    history::{self, HistoryError},
    kick,
    ledger::{self, ClaimRequest, Claimed},
    recover,
    reply_stream::ReplyStream,
};
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{bot, bot_turn, conversation},
    error::ApiError,
    execution::{ExecutionJwtParams, TokenType, bind_bot_identity, sign_unattended_execution_jwt},
    routes::{
        ai::{
            copilot::master_flow_like_state,
            global_chat::{
                PROFILE_MODEL_REQUIRED, ensure_metered_or_customer_provider,
                load_user_profile_access, reserve_assistant_usage_with_max_runtime,
                settle_assistant_usage,
            },
        },
        bots::runtime::{chat_bot, server_bot_model},
        chat::hosted_worker,
        inbox::{
            entries::{Sent, validate_send},
            model::{SendEntryRequest, TurnClaim},
            now_millis,
            turns::reply_as_bot,
            wake,
        },
    },
    state::AppState,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset};
use flow_like::{
    flow::copilot::{
        BotToolHost, BotToolset, HeadlessMemory, NotesHandle, PlatformContextInput, ReplyFiles,
        ToolTrail, bot_platform_context,
        bot_talk::{AnsweredRound, BotPresence, BotTalk},
        decision_event::{self, EventDecision},
        find_bot_tool_spec,
        memory::{AssistantMemory, MemoryScope},
        platform::{Clearance, PlatformToolBridge, RoomUpdatesBridge},
        run_bot_chat_with_talk,
        run_recovery::resumes_held_round,
        tool_grants::{GrantStore, ParkedBatch, clear_tokens, resume_note},
        tool_spec::{MEMORY_STORE_TOOL, ToolEffect},
    },
    flow_like_model_provider::provider::is_hosted_provider_name,
    models::llm::ModelUsageContext,
    profile::Profile,
};
use flow_like_types::{
    Value,
    bot::BotLocation,
    bot_message::ApprovalRequest,
    bot_turn::{HEARTBEAT_SECS, TurnAudience, TurnHostKind, failure_message_id, reply_message_id},
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind, plain_envelope},
    tokio::{
        self,
        sync::{Notify, Semaphore},
        task::JoinHandle,
    },
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::json;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The longest a server turn runs, whatever the plan allows.
pub(crate) const MAX_TURN_RUNTIME_MS: i64 =
    flow_like_types::bot_turn::DEFAULT_TURN_DEADLINE_SECS as i64 * 1_000;

pub(super) fn recovery_runtime_ms(
    checkpoint: Option<&flow_like_types::bot_turn::BotRunCheckpoint>,
    now_ms: i64,
) -> i64 {
    match checkpoint.map(|saved| saved.deadline_at_ms.saturating_sub(now_ms)) {
        // The runtime uses the original deadline to prohibit further work. This grace only
        // allows loading and publishing a saved answer or the deadline handoff.
        Some(remaining) if remaining <= 0 => 30_000,
        Some(remaining) => remaining.min(MAX_TURN_RUNTIME_MS),
        None => MAX_TURN_RUNTIME_MS,
    }
}
/// A reply fits one envelope: markdown beyond this is cut.
const MAX_REPLY_CHARS: usize = 48_000;
const DEFAULT_INTERVAL_SECS: u64 = 5;
const DEFAULT_CONCURRENCY: usize = 4;
const RECOVER_EVERY: Duration = Duration::from_secs(60);
/// Due bot tasks are fired this often between recoveries: an indexed query on `nextRunAt`.
const FIRE_TASKS_EVERY: Duration = Duration::from_secs(10);
const RETRY_BACKOFF_SECS: [i64; 2] = [10, 30];

pub const CODE_TIMEOUT: &str = "BOT_TURN_TIMEOUT";
pub const CODE_QUOTA: &str = "BOT_TURN_QUOTA";
pub const CODE_SETUP: &str = "BOT_TURN_SETUP";
/// The owner's profile has no language model the turn can run on; clients offer a model picker.
pub const CODE_NO_MODEL: &str = "BOT_TURN_NO_MODEL";
pub const CODE_MODEL: &str = "BOT_TURN_MODEL_ERROR";
pub const CODE_FAILED: &str = "BOT_TURN_FAILED";
pub const CODE_ENCRYPTED: &str = "BOT_TURN_ENCRYPTED";
pub const CODE_UNREADABLE: &str = "BOT_TURN_UNREADABLE";
pub const CODE_BOT_GONE: &str = "BOT_DELETED";
pub const CODE_REPLY_REFUSED: &str = "BOT_TURN_REPLY_REFUSED";

#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub interval: Duration,
    pub concurrency: usize,
}

impl WorkerConfig {
    /// `BOT_TURN_WORKER_INTERVAL_SECS` (default 5) and `BOT_TURN_WORKER_CONCURRENCY` (default 4).
    pub fn from_env() -> Self {
        let number = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
        };
        Self {
            interval: Duration::from_secs(
                number("BOT_TURN_WORKER_INTERVAL_SECS").unwrap_or(DEFAULT_INTERVAL_SECS),
            ),
            concurrency: number("BOT_TURN_WORKER_CONCURRENCY")
                .map_or(DEFAULT_CONCURRENCY, |value| value as usize),
        }
    }
}

/// Why a turn did not get its answer, and what the bot does about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub text: String,
    /// A transient failure worth another attempt.
    pub retryable: bool,
    /// The bot tells the conversation; otherwise the turn only fails.
    pub post: bool,
    /// Not a failure: the turn asked its owner to approve a call and waits for the answer.
    pub park: Option<ApprovalRequest>,
    /// Numeric usage captured before the turn failed, without provider response text.
    pub usage_stats: Vec<serde_json::Value>,
}

impl Failure {
    pub(crate) fn new(
        code: &'static str,
        text: impl Into<String>,
        retryable: bool,
        post: bool,
    ) -> Self {
        Self {
            code,
            text: text.into(),
            retryable,
            post,
            park: None,
            usage_stats: Vec::new(),
        }
    }

    fn parked(card: ApprovalRequest) -> Self {
        Self {
            park: Some(card),
            ..Self::new(CODE_FAILED, "", false, false)
        }
    }

    fn silent(code: &'static str, text: impl Into<String>) -> Self {
        Self::new(code, text, false, false)
    }

    pub(crate) fn from_api(error: &ApiError) -> Self {
        let reason = error.public_message().unwrap_or("unexpected error");
        if error.public_code() == PROFILE_MODEL_REQUIRED {
            return Self::new(
                CODE_NO_MODEL,
                format!("I can't answer yet: {reason}"),
                false,
                true,
            );
        }
        match error.status() {
            StatusCode::PAYMENT_REQUIRED | StatusCode::TOO_MANY_REQUESTS => Self::new(
                CODE_QUOTA,
                format!("I can't answer right now: {reason}"),
                false,
                true,
            ),
            StatusCode::BAD_REQUEST
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::CONFLICT => Self::new(
                CODE_SETUP,
                format!("I can't answer yet: {reason}"),
                false,
                true,
            ),
            _ => Self::new(
                CODE_FAILED,
                "Something went wrong on my side while answering.",
                true,
                true,
            ),
        }
    }

    /// The failure of a turn nobody asked for: recorded, never posted and never retried.
    fn unrequested(self) -> Self {
        Self {
            retryable: false,
            post: false,
            ..self
        }
    }

    fn model(error: &flow_like_types::Error) -> Self {
        tracing::warn!(%error, "Bot turn: the model run failed");
        if let Some(refusal) = provider_refusal(&format!("{error:#}")) {
            return Self::new(
                CODE_SETUP,
                format!(
                    "I can't answer yet: my language model provider refused the request ({refusal})."
                ),
                false,
                true,
            );
        }
        Self::new(
            CODE_MODEL,
            "The language model didn't answer. You can ask again, or have my owner retry this turn.",
            true,
            true,
        )
    }

    fn timeout() -> Self {
        Self::new(
            CODE_TIMEOUT,
            "I ran out of time before I could answer. You can ask again, or have my owner retry this turn.",
            false,
            true,
        )
    }
}

/// The provider's refusal of authorization or billing (HTTP 401 or 403) in a model error, with
/// the provider's own words when it gave any. Retrying such a request cannot help. The model
/// error reaches the host as text, so this reads rig's `Invalid status code …` message.
fn provider_refusal(error: &str) -> Option<String> {
    const MARKER: &str = "status code";
    let (_, after) = error.split_once(MARKER)?;
    let after = after.trim_start_matches([':', ' ']);
    let status = after.get(..3)?;
    if status != "401" && status != "403" {
        return None;
    }
    let reason = after
        .split_once("with message:")
        .map(|(_, detail)| detail.trim())
        .filter(|detail| !detail.is_empty());
    Some(match reason {
        Some(detail) => format!("{status}: {}", detail.chars().take(240).collect::<String>()),
        None => status.to_string(),
    })
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        Self::from_api(&error)
    }
}

impl From<sea_orm::DbErr> for Failure {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::from_api(&ApiError::from(error))
    }
}

impl From<HistoryError> for Failure {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Db(error) => Self::from_api(&error),
            HistoryError::TriggerMissing | HistoryError::TriggerUnreadable => Self::silent(
                CODE_UNREADABLE,
                "The message this turn answers is gone or can't be read by the server",
            ),
        }
    }
}

/// Transient failures before any side effect may request a retry. The ledger enforces its
/// separate recovery budget; claim generations also count successful approval handoffs.
pub(crate) fn retries(failure: &Failure, _attempts: i32, mutating_started: bool) -> bool {
    failure.retryable && !mutating_started
}

/// Seconds until the next attempt after `attempts` claims.
pub(crate) fn backoff_secs(attempts: i32) -> i64 {
    let index = usize::try_from(attempts.saturating_sub(1)).unwrap_or(0);
    RETRY_BACKOFF_SECS[index.min(RETRY_BACKOFF_SECS.len() - 1)]
}

/// Tools that store a memory. A repeat would store it twice, so they end a turn's retries even
/// though their spec calls them read-only.
const MEMORY_WRITE_TOOLS: [&str; 3] = [MEMORY_STORE_TOOL, "memory_store", "memory_save"];

fn writes_memory(tool_name: &str) -> bool {
    MEMORY_WRITE_TOOLS.contains(&tool_name)
}

/// Whether the memory table holds more than it did before the attempt. Core runs the memory
/// tools itself and never shows them to the bridge, so a failed attempt is checked against the
/// table; a count that can't be read counts as a write.
pub(crate) fn memory_written(before: Option<usize>, after: Option<usize>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => after != before,
        _ => true,
    }
}

async fn memory_count(
    state: &Arc<flow_like::state::FlowLikeState>,
    owner: &str,
    scope: &MemoryScope,
) -> Option<usize> {
    AssistantMemory::status(state.clone(), Some(owner), scope)
        .await
        .ok()
        .map(|status| status.count)
}

/// The tool bridge of a headless turn: core runs web research and memory itself, so every call
/// that reaches the bridge is a tool this host can't run. A call to one with side effects
/// ends the turn's retries.
struct HeadlessBridge {
    state: AppState,
    turn_id: String,
    host_id: String,
    mutating: Arc<AtomicBool>,
    bot_tools: Option<Arc<crate::bot_tools::BotToolsHost>>,
}

/// The model's answer to a tool call this host can't run; a call to a tool with side effects is
/// remembered in `mutating`.
pub(crate) fn refuse_tool(tool_name: &str, mutating: &AtomicBool) -> String {
    if writes_memory(tool_name)
        || find_bot_tool_spec(tool_name)
            .is_some_and(|spec| spec.approval.effect() != ToolEffect::ReadOnly)
    {
        mutating.store(true, Ordering::SeqCst);
    }
    json!({
        "status": "error",
        "tool": tool_name,
        "code": "bot_tool_unavailable_headless",
        "retryable": false,
        "message": "This tool needs an open chat or the owner's desktop and isn't available while the bot answers on its own."
    })
    .to_string()
}

#[async_trait]
impl PlatformToolBridge for HeadlessBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if let Some(bot_tools) = self
            .bot_tools
            .as_ref()
            .filter(|bot_tools| bot_tools.handles(tool_name))
        {
            return bot_tools
                .run_unattended(&self.mutating, tool_name, arguments)
                .await;
        }
        refuse_tool(tool_name, &self.mutating)
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        match self
            .bot_tools
            .as_ref()
            .filter(|bot_tools| bot_tools.handles(tool_name))
        {
            Some(bot_tools) => bot_tools.clearance(tool_name, arguments).await,
            None => Clearance::Clear,
        }
    }

    /// A parked turn is not cancelled: its running calls finish so their outcomes stay known, and
    /// the run pauses after the batch.
    async fn is_cancelled(&self) -> bool {
        !holds_lease_or_retry(&self.state.db, &self.turn_id, &self.host_id).await
    }

    async fn bot_secret(&self, name: &str) -> Option<String> {
        self.bot_tools.as_ref()?.secret(name).await
    }
}

const LEASE_CHECK_RETRIES: u32 = 2;
const LEASE_CHECK_BACKOFF: Duration = Duration::from_millis(150);

/// A database error is retried before it counts as a lost lease; `Ok(false)` is final.
pub(super) async fn holds_lease_or_retry(
    db: &DatabaseConnection,
    turn_id: &str,
    host_id: &str,
) -> bool {
    for attempt in 0..=LEASE_CHECK_RETRIES {
        match ledger::holds_lease(db, turn_id, host_id, now_millis()).await {
            Ok(held) => return held,
            Err(error) => {
                tracing::warn!(turn_id, attempt, %error, "bot turn lease check failed");
                if attempt < LEASE_CHECK_RETRIES {
                    tokio::time::sleep(LEASE_CHECK_BACKOFF * (attempt + 1)).await;
                }
            }
        }
    }
    false
}

async fn turn_requester(
    db: &DatabaseConnection,
    turn: &bot_turn::Model,
    room: Option<&str>,
    kind: ConversationKind,
    addressed: bool,
    reactions: Vec<flow_like::flow::copilot::ReactionNote>,
) -> Result<flow_like::flow::copilot::TurnRequester, ApiError> {
    let requester_id = turn
        .requester_id
        .clone()
        .unwrap_or_else(|| turn.owner_id.clone());
    let ids = [requester_id.as_str(), turn.owner_id.as_str()];
    let names = history::sender_names(db, &ids).await?;
    Ok(flow_like::flow::copilot::TurnRequester {
        requester_name: names.get(&requester_id).cloned(),
        owner_name: names.get(&turn.owner_id).cloned(),
        requester_id,
        owner_id: turn.owner_id.clone(),
        audience: TurnAudience::of(kind),
        room: room.map(str::to_string),
        addressed,
        reactions,
    })
}

fn host_id() -> String {
    format!("server:{}", uuid::Uuid::new_v4())
}

/// The turn's executor token: the owner stays the payer (`sub`), app callbacks run as the bot.
pub(crate) fn executor_token(
    owner_id: &str,
    bot_id: &str,
    conv_id: Option<&str>,
    turn_id: &str,
    workspace_app_id: Option<&str>,
    ttl_secs: i64,
) -> Option<String> {
    let params = ExecutionJwtParams {
        user_id: owner_id.to_string(),
        technical_user_id: None,
        run_id: turn_id.to_string(),
        app_id: workspace_app_id.unwrap_or_default().to_string(),
        board_id: String::new(),
        event_id: None,
        app_chain: None,
        correlation: None,
        callback_url: std::env::var("API_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string()),
        token_type: TokenType::Executor,
        ttl_seconds: Some(ttl_secs.clamp(1, 86_400)),
        shadow: None,
    };
    sign_unattended_execution_jwt(params)
        .and_then(|token| bind_bot_identity(&token, bot_id, conv_id))
        .map_err(|error| tracing::warn!(%error, "Bot turn runs without a model token"))
        .ok()
}

pub(crate) async fn load_profile(
    state: &AppState,
    owner_id: &str,
    profile_id: Option<&str>,
) -> Result<
    (
        Arc<Profile>,
        crate::routes::ai::global_chat::ProfileModelAccess,
    ),
    ApiError,
> {
    let mut loaded = None;
    if profile_id.is_some() {
        loaded = load_user_profile_access(state, owner_id, profile_id).await?;
    }
    if loaded.is_none() {
        loaded = load_user_profile_access(state, owner_id, None).await?;
    }
    loaded.ok_or_else(|| {
        ApiError::bad_request(
            "No profile found for the owner. A synced profile with a language model is required",
        )
    })
}

/// The funding of a turn from what its model and memory run on, as a chat turn computes it.
pub(crate) fn funding_class(model_hosted: bool, memory_hosted: Option<bool>) -> &'static str {
    if model_hosted && memory_hosted != Some(false) {
        "hosted"
    } else if model_hosted || memory_hosted == Some(true) {
        "mixed"
    } else {
        "byok"
    }
}

fn clipped(text: &str) -> String {
    let mut text: String = text.chars().take(MAX_REPLY_CHARS).collect();
    if text.chars().count() == MAX_REPLY_CHARS {
        text.push_str("\n\n_(cut: too long to send)_");
    }
    text
}

/// The bot's message answering `turn`: `text` replying to the trigger, one hop further than it.
pub(crate) fn reply_request(
    turn: &bot_turn::Model,
    message_id: String,
    text: &str,
    error_code: Option<&str>,
) -> SendEntryRequest {
    let mut body = json!({
        "text": text,
        "reply_to": turn.trigger_message_id,
        "hops": u32::try_from(turn.hops).unwrap_or(0).saturating_add(1),
    });
    if let Some(code) = error_code {
        body["error"] = json!({ "code": code });
    }
    SendEntryRequest {
        message_id,
        kind: InboxEntryKind::Message,
        envelope: Some(plain_envelope(body)),
        envelopes: None,
        turn: None,
    }
}

/// The reply an answer posts; none when the bot read along and stayed silent.
pub(crate) fn answer_request(turn: &bot_turn::Model, answer: &Answer) -> Option<SendEntryRequest> {
    (!answer.silent).then(|| {
        let mut request = reply_request(
            turn,
            reply_message_id(&turn.bot_id, &turn.trigger_message_id),
            &clipped(&answer.text),
            None,
        );
        if answer.event_delivery == Some(EventDecision::Respond)
            && let Some(envelope) = request.envelope.as_mut()
        {
            envelope["body"][decision_event::REPLY_DELIVERY_FIELD] = json!("respond");
        }
        if let Some(envelope) = request.envelope.as_mut() {
            super::taint::attach(&mut envelope["body"], &answer.taint);
        }
        if let Some(note) = &answer.note
            && let Some(envelope) = request.envelope.as_mut()
        {
            envelope["body"]["note"] = json!(note);
        }
        if let Some(envelope) = request.envelope.as_mut() {
            flow_like_types::bot_reply_files::attach_reply_files(
                &mut envelope["body"],
                &answer.files,
            );
            flow_like_types::bot_tool_trail::attach_tool_parts(
                &mut envelope["body"],
                &answer.tool_calls,
            );
        }
        if answer.acknowledgement
            && let Some(envelope) = request.envelope.as_mut()
        {
            envelope["body"][flow_like::flow::copilot::decision::ACKNOWLEDGEMENT_FIELD] =
                json!(true);
        }
        if let Some(envelope) = request.envelope.as_mut() {
            flow_like::flow::copilot::bot_usage::attach_usage_stats(
                &mut envelope["body"],
                answer.usage_stats.clone(),
            );
        }
        request
    })
}

/// The turn's memory, kept until its reply is posted: the follow-up work needs the reply.
type FollowUp = Arc<std::sync::Mutex<Option<HeadlessMemory>>>;

/// One attempt at a turn: everything up to the model's answer.
/// A finished turn's answer; `resumed` when the turn slept for an owner's approval.
pub(crate) struct Answer {
    pub(crate) usage_stats: Vec<serde_json::Value>,
    pub(crate) acknowledgement: bool,
    pub(crate) text: String,
    pub(crate) resumed: bool,
    /// The bot read along and chose to stay silent: nothing is posted.
    pub(crate) silent: bool,
    /// A participant wait already completed the held turn in its own transaction.
    pub(crate) completed: bool,
    pub(crate) event_delivery: Option<EventDecision>,
    /// The note the turn last wrote or closed; the reply carries it as `body.note`.
    pub(crate) note: Option<flow_like_types::bot_notes::NoteRef>,
    /// The turn's tool calls, kept on the reply in the owner's direct conversation.
    pub(crate) tool_calls: Vec<flow_like_types::bot_tool_trail::ToolCall>,
    /// The files the bot attached to its reply with `file_attach`.
    pub(crate) files: Vec<flow_like_types::bot_reply_files::ReplyFile>,
    /// The taint the turn ended with; the reply records it for the turns that read it.
    pub(crate) taint: flow_like_types::data_classification::DataSensitivity,
}

type DecisionResult = Option<Result<flow_like::flow::copilot::decision::Decision, ApiError>>;

enum Verdict {
    Proceed,
    Acknowledge,
    Done(Answer),
}

enum Raced<T> {
    Allowed {
        outcome: T,
        decision: DecisionResult,
    },
    Withheld(DecisionResult),
}

#[derive(Default)]
struct CallTrace {
    parallel: bool,
    decision_ms: Option<i64>,
    decision: &'static str,
    model_ms: Option<i64>,
}

fn silent_answer(completed: bool) -> Answer {
    Answer {
        acknowledgement: false,
        text: String::new(),
        resumed: false,
        silent: true,
        completed,
        event_delivery: None,
        note: None,
        tool_calls: Vec::new(),
        files: Vec::new(),
        usage_stats: Vec::new(),
        taint: Default::default(),
    }
}

fn live_call_turn(trigger_kind: Option<InboxEntryKind>) -> bool {
    trigger_kind == Some(InboxEntryKind::Transcript)
}

fn decides_in_parallel(
    live_call: bool,
    kind: ConversationKind,
    addressed: bool,
    has_decision_model: bool,
) -> bool {
    live_call && has_decision_model && (kind == ConversationKind::Direct || addressed)
}

fn decision_label(decision: &DecisionResult) -> &'static str {
    use flow_like::flow::copilot::decision::Decision;
    match decision {
        Some(Ok(Decision::AnswerNow)) => "answer_now",
        Some(Ok(Decision::Acknowledge)) => "acknowledge",
        Some(Ok(Decision::NoAnswer)) => "no_answer",
        Some(Ok(Decision::WaitFor(_))) => "wait_for",
        Some(Err(_)) => "error",
        None => "timeout",
    }
}

fn lets_reply_through(decision: &DecisionResult) -> bool {
    use flow_like::flow::copilot::decision::Decision;
    !matches!(
        decision,
        Some(Ok(Decision::NoAnswer
            | Decision::WaitFor(_)
            | Decision::Acknowledge))
    )
}

/// Runs the reply while the participation decision is pending. The reply's tools stay held until
/// the decision lets the turn answer, and a finished reply is returned only once it has.
async fn race_decision<T>(
    chat: impl Future<Output = T>,
    mut decision: JoinHandle<(DecisionResult, i64)>,
    opener: flow_like::flow::copilot::tool_gate::ToolGateOpener,
    trace: &std::sync::Mutex<CallTrace>,
) -> Raced<T> {
    let mut chat = std::pin::pin!(chat);
    let mut finished = None;
    let decided = loop {
        tokio::select! {
            decided = &mut decision => break decided,
            outcome = &mut chat, if finished.is_none() => finished = Some(outcome),
        }
    };
    let (decided, decision_ms) = decided.unwrap_or_else(|error| {
        tracing::warn!(%error, "Bot participation decision task failed; using normal response handling");
        (None, 0)
    });
    {
        let mut trace = trace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        trace.decision_ms = Some(decision_ms);
        trace.decision = decision_label(&decided);
    }
    if !lets_reply_through(&decided) {
        opener.shut();
        return Raced::Withheld(decided);
    }
    opener.open();
    let outcome = match finished {
        Some(outcome) => outcome,
        None => chat.await,
    };
    Raced::Allowed {
        outcome,
        decision: decided,
    }
}

async fn apply_decision(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    kind: ConversationKind,
    decision: DecisionResult,
) -> Result<Verdict, Failure> {
    use flow_like::flow::copilot::decision::Decision;
    let Some(decision) = settled_decision(decision, &turn.id) else {
        return Ok(Verdict::Proceed);
    };
    match decision {
        Decision::NoAnswer => Ok(Verdict::Done(silent_answer(false))),
        Decision::WaitFor(participant) => {
            // In a direct call, keeping quiet already waits for the only other
            // participant. Their next utterance gets its own decision.
            if kind == ConversationKind::Direct {
                return Ok(Verdict::Done(silent_answer(false)));
            }
            use super::decision_wait::ParkOutcome;
            match super::decision_wait::park_wait(state, turn, host_id, &participant).await? {
                ParkOutcome::Parked | ParkOutcome::LeaseLost => {
                    Ok(Verdict::Done(silent_answer(true)))
                }
                ParkOutcome::Superseded => Ok(Verdict::Done(silent_answer(false))),
                ParkOutcome::AlreadyAnswered | ParkOutcome::Unavailable => Ok(Verdict::Proceed),
            }
        }
        Decision::Acknowledge => Ok(Verdict::Acknowledge),
        Decision::AnswerNow => Ok(Verdict::Proceed),
    }
}

/// What the decision model said. A call that failed or timed out says nothing, so the turn goes
/// on as it would without a decision model: only the hosted proxy knows whether it can run the
/// bot's System One Bit, and an unavailable one must never fail the turn.
pub(super) fn settled_decision(
    decision: DecisionResult,
    turn_id: &str,
) -> Option<flow_like::flow::copilot::decision::Decision> {
    match decision {
        Some(Ok(decision)) => Some(decision),
        Some(Err(error)) => {
            tracing::warn!(%error, turn_id, "Bot participation decision failed; using normal response handling");
            None
        }
        None => {
            tracing::warn!(
                turn_id,
                "Bot participation decision timed out; using normal response handling"
            );
            None
        }
    }
}

async fn attempt(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    mutating: &Arc<AtomicBool>,
    follow_up: &FollowUp,
    unaddressed: &AtomicBool,
    batched: &[String],
) -> Result<Answer, Failure> {
    let started = Instant::now();
    let queued_ms = chrono::Utc::now().timestamp_millis() - turn.created_at.timestamp_millis();
    let trace = std::sync::Mutex::new(CallTrace::default());
    let result = attempt_decided(
        state,
        turn,
        host_id,
        mutating,
        follow_up,
        unaddressed,
        batched,
        None,
        &trace,
    )
    .await;
    let trace = trace
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !trace.decision.is_empty() || trace.model_ms.is_some() {
        tracing::info!(
            target: "bot_call_latency",
            turn_id = %turn.id,
            bot_id = %turn.bot_id,
            queued_ms,
            decision_ms = trace.decision_ms,
            decision = trace.decision,
            parallel = trace.parallel,
            model_ms = trace.model_ms,
            total_ms = started.elapsed().as_millis() as i64,
            ok = result.is_ok(),
            silent = result.as_ref().is_ok_and(|answer| answer.silent),
            "voice call turn"
        );
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn attempt_decided<'a>(
    state: &'a AppState,
    turn: &'a bot_turn::Model,
    host_id: &'a str,
    mutating: &'a Arc<AtomicBool>,
    follow_up: &'a FollowUp,
    unaddressed: &'a AtomicBool,
    batched: &'a [String],
    preset: Option<Verdict>,
    trace: &'a std::sync::Mutex<CallTrace>,
) -> std::pin::Pin<Box<dyn Future<Output = Result<Answer, Failure>> + Send + 'a>> {
    Box::pin(attempt_with(
        state,
        turn,
        host_id,
        mutating,
        follow_up,
        unaddressed,
        batched,
        preset,
        trace,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn attempt_with(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    mutating: &Arc<AtomicBool>,
    follow_up: &FollowUp,
    unaddressed: &AtomicBool,
    batched: &[String],
    preset: Option<Verdict>,
    trace: &std::sync::Mutex<CallTrace>,
) -> Result<Answer, Failure> {
    let owner = turn.owner_id.as_str();
    let recovery = Arc::new(super::recovery::ServerRecovery::new(state, turn, host_id));
    let row = bot::Entity::find_by_id(&turn.bot_id)
        .one(&state.db)
        .await?
        .filter(|row| row.deleted_at.is_none())
        .ok_or_else(|| Failure::silent(CODE_BOT_GONE, "The bot was deleted"))?;
    let mut conversation = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| Failure::silent(CODE_UNREADABLE, "The conversation is gone"))?;
    if ConversationEncryption::parse(&conversation.encryption) == Some(ConversationEncryption::E2e)
    {
        return Err(Failure::silent(
            CODE_ENCRYPTED,
            "The server can't read an end-to-end encrypted conversation",
        ));
    }
    let mut kind = ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
    let trigger = crate::entity::inbox_entry::Entity::find()
        .filter(crate::entity::inbox_entry::Column::InboxId.eq(&turn.bot_id))
        .filter(crate::entity::inbox_entry::Column::ConversationId.eq(&turn.conversation_id))
        .filter(crate::entity::inbox_entry::Column::MessageId.eq(&turn.trigger_message_id))
        .one(&state.db)
        .await?;
    let trigger_kind = trigger
        .as_ref()
        .and_then(|entry| InboxEntryKind::parse(&entry.kind));
    let ephemeral_user_context = trigger.as_ref().is_some_and(|entry| {
        entry.envelope.get("body").is_some_and(|body| {
            flow_like_types::bot_user_context::response_for(body, &entry.sender_id, &turn.bot_id)
        })
    });
    let checkpoint = if ephemeral_user_context || preset.is_some() {
        None
    } else {
        recovery.read().await?
    };
    let saved_delivery = checkpoint
        .as_ref()
        .map(decision_event::saved_delivery)
        .transpose()
        .map_err(|error| Failure::model(&error))?
        .flatten();
    if matches!(
        trigger_kind,
        Some(InboxEntryKind::Message | InboxEntryKind::Transcript)
    ) && let Some(trigger) = trigger.as_ref().filter(|entry| {
        entry
            .envelope
            .get("body")
            .is_some_and(flow_like::flow::copilot::decision::is_acknowledgement_body)
    }) && bot::Entity::find_by_id(&trigger.sender_id)
        .one(&state.db)
        .await?
        .is_some()
    {
        return Ok(Answer {
            acknowledgement: false,
            text: String::new(),
            resumed: false,
            silent: true,
            completed: false,
            event_delivery: None,
            note: None,
            tool_calls: Vec::new(),
            files: Vec::new(),
            usage_stats: Vec::new(),
            taint: Default::default(),
        });
    }
    let reacting = trigger_kind == Some(InboxEntryKind::Reaction);
    let mut reactions = Vec::new();
    let mut room_cursor = turn.trigger_message_id.clone();
    let mut transcript = if reacting {
        let answered: Vec<String> = std::iter::once(turn.trigger_message_id.clone())
            .chain(batched.iter().cloned())
            .collect();
        let found = history::reaction_turn(
            &state.db,
            &turn.bot_id,
            &turn.conversation_id,
            &answered,
            kind,
            owner,
        )
        .await?;
        reactions = found.reactions;
        if let Some(newest) = found.newest_message_id {
            room_cursor = newest;
        }
        found.transcript
    } else {
        history::transcript(
            &state.db,
            &turn.bot_id,
            &turn.conversation_id,
            &turn.trigger_message_id,
            kind,
            owner,
        )
        .await?
    };
    if checkpoint.is_none()
        && reacting
        && reactions.is_empty()
        && transcript.addressed
        && transcript.prompt.is_empty()
    {
        unaddressed.store(true, Ordering::SeqCst);
        return Ok(Answer {
            acknowledgement: false,
            text: String::new(),
            resumed: false,
            silent: true,
            completed: false,
            event_delivery: None,
            note: None,
            tool_calls: Vec::new(),
            files: Vec::new(),
            usage_stats: Vec::new(),
            taint: Default::default(),
        });
    }
    let mut chat_conversation = turn.conversation_id.clone();
    let mut answered_round = None;
    if let Some(answered) = super::talk::answered_ask(&state.db, turn).await? {
        chat_conversation = answered.origin_conversation.clone();
        conversation = conversation::Entity::find_by_id(&chat_conversation)
            .one(&state.db)
            .await?
            .ok_or_else(|| Failure::silent(CODE_UNREADABLE, "The original conversation is gone"))?;
        if ConversationEncryption::parse(&conversation.encryption)
            == Some(ConversationEncryption::E2e)
        {
            return Err(Failure::silent(
                CODE_ENCRYPTED,
                "The server can't resume an end-to-end encrypted conversation",
            ));
        }
        kind = ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
        // The original room's current membership and limits still apply after a delayed reply.
        crate::routes::bots::runtime_token::bot_user_for_turn(
            &state.db,
            owner,
            &turn.bot_id,
            &turn.id,
        )
        .await?;
        transcript = super::talk::ask_result_transcript(&state.db, turn, &answered).await?;
        answered_round = Some(AnsweredRound {
            callee_id: answered.callee_id,
            round: answered.round,
        });
    }
    let asked_by = super::talk::asked_by(&state.db, turn).await?;
    let room_updates = if kind == ConversationKind::Group && answered_round.is_none() {
        super::room_updates::ServerRoomUpdates::after(
            &state.db,
            &turn.bot_id,
            &turn.conversation_id,
            &room_cursor,
            owner,
        )
        .await?
    } else {
        None
    };
    let ancestors: Vec<String> = asked_by.iter().map(|member| member.id.clone()).collect();
    let asker = (!asked_by.is_empty()).then_some(asked_by);
    let addressed = transcript.addressed || asker.is_some();
    unaddressed.store(!addressed || !reactions.is_empty(), Ordering::SeqCst);

    let requester = turn_requester(
        &state.db,
        turn,
        conversation.title.as_deref(),
        kind,
        addressed,
        reactions,
    )
    .await?;
    if let Some(saved) = &checkpoint {
        if let Some(text) = flow_like::flow::copilot::run_recovery::finished_reply(saved)
            .map_err(|error| Failure::model(&error))?
        {
            let failed = decision_event::failed_execution(saved);
            let mut event_delivery = saved_delivery.and_then(|delivery| delivery.choice);
            if !failed
                && saved_delivery
                    .is_some_and(|delivery| delivery.monitoring && delivery.choice.is_none())
            {
                event_delivery = Some(
                    super::event_decision::limit_notify(&state.db, turn, EventDecision::Notify)
                        .await,
                );
                if let Some(event) = super::event_decision::trigger(state, turn).await?
                    && let Some(bit_id) = row
                        .model_policy
                        .get("decision")
                        .and_then(serde_json::Value::as_str)
                {
                    let history: Vec<_> = transcript.history.iter().rev().take(12).rev().map(|message| json!({"role":message.role,"text":flow_like::flow::copilot::decision::excerpt(&message.content, 1_000)})).collect();
                    let previous = super::event_decision::previous_result(state, turn).await;
                    let context = json!({"bot_name":row.name,"instructions":row.instructions,"history":history,
                        "monitor_baseline":super::event_decision::baseline_note(previous.as_deref())});
                    match super::event_decision::classify(
                        state,
                        turn,
                        bit_id,
                        &event,
                        &context,
                        Some(&text),
                    )
                    .await
                    {
                        Ok(choice) => {
                            let choice = super::event_decision::guard_ignore(
                                &state.db, turn, &event, choice,
                            )
                            .await;
                            event_delivery = Some(
                                super::event_decision::limit_notify(
                                    &state.db,
                                    turn,
                                    choice.unwrap_or(EventDecision::Notify),
                                )
                                .await,
                            )
                        }
                        Err(error) => {
                            tracing::warn!(%error, turn_id = %turn.id, "Saved monitoring result could not be classified; delivering it")
                        }
                    }
                }
                let wrapped = decision_event::EventRecovery::new(
                    recovery.clone(),
                    saved_delivery.expect("monitoring delivery exists"),
                );
                wrapped
                    .record(event_delivery)
                    .await
                    .map_err(|error| Failure::model(&error))?;
            }
            if failed {
                event_delivery = None;
            }
            let usage = flow_like::flow::copilot::bot_usage::BotUsage::default();
            usage.restore(saved);
            return Ok(Answer {
                acknowledgement: false,
                silent: event_delivery.is_some_and(EventDecision::quiet)
                    || requester.answer_is_silent(&text),
                text,
                resumed: true,
                completed: false,
                event_delivery,
                note: None,
                tool_calls: Vec::new(),
                files: Vec::new(),
                usage_stats: usage.snapshot(),
                taint: Default::default(),
            });
        }
    }

    let (profile, access) = load_profile(state, owner, row.profile_id.as_deref()).await?;
    let mut bot = chat_bot(state, owner, Some(turn.bot_id.as_str()), &profile)
        .await?
        .ok_or_else(|| Failure::silent(CODE_BOT_GONE, "The bot was deleted"))?;
    bot.context.tool_host = match bot.context.toolset {
        BotToolset::Capabilities => BotToolHost::Hosted,
        BotToolset::FlowPilot => BotToolHost::CoreOnly,
    };
    bot.context.ephemeral_user_context = ephemeral_user_context;
    bot.context.seed_taint = flow_like::flow::copilot::TurnReads {
        personal_context: ephemeral_user_context,
        ..transcript.reads.clone()
    }
    .seed();
    bot.context.recovery = (!ephemeral_user_context).then(|| {
        (recovery.clone() as Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>).into()
    });
    bot.context.requester = Some(requester);
    let requester = bot.context.requester.clone();
    let grants = crate::routes::bots::tool_grants::rows_for_owner(state, &turn.bot_id, owner);
    let seen = grants.list().await.unwrap_or_default();
    // A held round runs the answered calls itself; "make the same call again" would repeat
    // calls that already ran.
    let note = match checkpoint
        .as_ref()
        .filter(|saved| resumes_held_round(saved))
    {
        Some(_) => None,
        None => resume_note(grants.as_ref()).await,
    };
    let resumed = note.is_some() || checkpoint.is_some();
    let event =
        if saved_delivery.is_some() || (!resumed && answered_round.is_none() && asker.is_none()) {
            super::event_decision::trigger(state, turn).await?
        } else {
            None
        };
    let mut event_context = super::event_decision::context(&bot.context, &transcript);
    if event.as_ref().is_some_and(decision_event::monitoring)
        && saved_delivery.is_none_or(|delivery| delivery.choice.is_none())
    {
        let previous = super::event_decision::previous_result(state, turn).await;
        let baseline = super::event_decision::baseline_note(previous.as_deref());
        event_context["monitor_baseline"] = json!(baseline);
        transcript.prompt = format!(
            "{baseline}\n\nCurrent scheduled check:\n{}",
            transcript.prompt
        );
    }
    let decision_bit = bot.model.decision.clone();
    let mut event_delivery = saved_delivery.and_then(|delivery| delivery.choice);
    if let (Some(event), Some(bit_id)) = (&event, decision_bit.as_deref())
        && decision_event::eligible(event)
        && saved_delivery.is_none()
        && !resumed
    {
        let classified =
            super::event_decision::classify(state, turn, bit_id, event, &event_context, None).await;
        let classified = match classified {
            Ok(choice) => {
                Ok(super::event_decision::guard_ignore(&state.db, turn, event, choice).await)
            }
            Err(error) => Err(error),
        };
        match classified {
            Ok(Some(EventDecision::Ignore)) => {
                return Ok(Answer {
                    acknowledgement: false,
                    text: String::new(),
                    resumed: false,
                    silent: true,
                    completed: false,
                    event_delivery: Some(EventDecision::Ignore),
                    note: None,
                    tool_calls: Vec::new(),
                    files: Vec::new(),
                    usage_stats: Vec::new(),
                    taint: Default::default(),
                });
            }
            Ok(choice) => {
                event_delivery = match choice {
                    Some(choice) => {
                        Some(super::event_decision::limit_notify(&state.db, turn, choice).await)
                    }
                    None => None,
                }
            }
            Err(error) => {
                tracing::warn!(%error, turn_id = %turn.id, "Bot event decision failed; processing normally")
            }
        }
        let instruction = if decision_event::monitoring(event) {
            decision_event::MONITOR_INSTRUCTION
        } else {
            event_delivery
                .map(EventDecision::instruction)
                .unwrap_or_default()
        };
        if !instruction.is_empty() {
            transcript
                .prompt
                .push_str(&format!("\n\nHost delivery instruction: {instruction}"));
            bot.context.suppress_owner_notifications = true;
        }
        // A background event's execution failure must remain visible even in a group.
        unaddressed.store(false, Ordering::SeqCst);
    }
    let delivery_state = saved_delivery.or_else(|| {
        bot.context
            .suppress_owner_notifications
            .then(|| decision_event::EventDelivery {
                choice: event_delivery,
                monitoring: event.as_ref().is_some_and(decision_event::monitoring),
            })
    });
    let event_recovery = delivery_state.map(|delivery| {
        Arc::new(decision_event::EventRecovery::new(
            recovery.clone(),
            delivery,
        ))
    });
    if let Some(recovery) = &event_recovery {
        bot.context.suppress_owner_notifications = true;
        bot.context.recovery = Some(
            (recovery.clone() as Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>)
                .into(),
        );
    }
    let live_call = live_call_turn(trigger_kind);
    bot.context.live_call = live_call;
    let mut decision_task = None;
    let mut tool_gate_opener = None;
    match &preset {
        Some(Verdict::Acknowledge) => bot.context.acknowledgement = live_call,
        Some(_) => {}
        None => {
            if (kind == ConversationKind::Group || live_call)
                && event.is_none()
                && answered_round.is_none()
                && asker.is_none()
                && !resumed
                && let Some(bit_id) = decision_bit.as_deref()
            {
                use flow_like::flow::copilot::decision::TIMEOUT_SECS;
                if decides_in_parallel(live_call, kind, addressed, true) {
                    trace
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .parallel = true;
                    let (opener, gate) = flow_like::flow::copilot::tool_gate::tool_gate();
                    bot.context.tool_gate = Some(gate);
                    tool_gate_opener = Some(opener);
                    let (state, turn, context, bit_id, transcript) = (
                        state.clone(),
                        turn.clone(),
                        bot.context.clone(),
                        bit_id.to_string(),
                        transcript.clone(),
                    );
                    decision_task = Some(tokio::spawn(async move {
                        let started = Instant::now();
                        let decision = tokio::time::timeout(
                            Duration::from_secs(TIMEOUT_SECS),
                            super::decision::evaluate(
                                &state,
                                &turn,
                                &context,
                                &bit_id,
                                &transcript,
                                addressed,
                            ),
                        )
                        .await
                        .ok();
                        (decision, started.elapsed().as_millis() as i64)
                    }));
                } else {
                    let started = Instant::now();
                    let decision = tokio::time::timeout(
                        Duration::from_secs(TIMEOUT_SECS),
                        super::decision::evaluate(
                            state,
                            turn,
                            &bot.context,
                            bit_id,
                            &transcript,
                            addressed,
                        ),
                    )
                    .await
                    .ok();
                    {
                        let mut trace = trace
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        trace.decision_ms = Some(started.elapsed().as_millis() as i64);
                        trace.decision = decision_label(&decision);
                    }
                    match apply_decision(state, turn, host_id, kind, decision).await? {
                        Verdict::Done(answer) => return Ok(answer),
                        Verdict::Acknowledge => bot.context.acknowledgement = live_call,
                        Verdict::Proceed => {}
                    }
                }
            }
        }
    }
    let flow_like_state = master_flow_like_state(state).await?;
    let model_id = server_bot_model(state, owner, &flow_like_state, &profile, &bot.model).await;
    if let Some(rejection) = access.rejection(model_id.as_deref()) {
        return Err(rejection.into());
    }
    let selected = profile
        .resolve_completion_model(
            model_id.as_deref(),
            &flow_like::flow::copilot::bot_model_preference(live_call),
            false,
            flow_like::state::FlowLikeState::completion_model_capabilities(&flow_like_state).await,
            None,
            flow_like_state.http_client.clone(),
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let provider = selected
        .try_to_provider()
        .ok_or_else(|| ApiError::bad_request("The bot's model is not a completion provider"))?;
    ensure_metered_or_customer_provider(&provider.provider_name)?;
    let memory_model = match (bot.context.remembers(), bot.context.memory_model(None)) {
        (false, _) => None,
        (true, Some(id)) => Some(id.to_string()),
        (true, None) => stored_memory_model(&flow_like_state, &profile, &bot, owner).await,
    };
    let memory_hosted = match memory_model.as_deref() {
        Some(id) => {
            let bit = profile
                .find_bit(id, flow_like_state.http_client.clone())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let embedding = bit.try_to_embedding().ok_or_else(|| {
                ApiError::bad_request("The bot's memory model is not an embedding provider")
            })?;
            Some(
                embedding
                    .remote
                    .as_ref()
                    .is_some_and(|remote| remote.implementation.is_some())
                    || is_hosted_provider_name(&embedding.provider.provider_name),
            )
        }
        None => None,
    };
    let funding = funding_class(
        is_hosted_provider_name(&provider.provider_name),
        memory_hosted,
    );
    let scope = crate::bot_permission::limited_conversation(
        &state.db,
        &chat_conversation,
        owner,
        &turn.bot_id,
    )
    .await?;
    let usage = reserve_assistant_usage_with_max_runtime(
        state,
        owner,
        &selected.id,
        &provider.provider_name,
        funding,
        None,
        recovery_runtime_ms(checkpoint.as_ref(), chrono::Utc::now().timestamp_millis()),
        hosted_worker::enabled(),
        Some(&crate::middleware::jwt::BotUser {
            bot_id: turn.bot_id.clone(),
            turn_id: turn.id.clone(),
            conv_id: scope.clone(),
        }),
    )
    .await?;
    if !crate::quota::mark_started(state, &usage.operation_id).await? {
        return Err(Failure::new(
            CODE_QUOTA,
            "I can't answer right now: my usage allowance isn't available",
            false,
            true,
        ));
    }
    let started = Instant::now();
    recovery.set_initial_deadline(usage.deadline.timestamp_millis());
    let remaining = (usage.deadline - chrono::Utc::now())
        .num_milliseconds()
        .max(1);
    let conv_id = scope.as_deref();
    let token = executor_token(
        owner,
        &turn.bot_id,
        conv_id,
        &turn.id,
        bot.context.workspace_app_id.as_deref(),
        remaining / 1_000 + 120,
    );
    // A delayed result has an explicit prompt and history from its original conversation.
    // Preparing memory from its exchange trigger would replace that continuation context.
    let memory_turn = if answered_round.is_some() || bot.context.acknowledgement {
        None
    } else {
        super::memory::open_turn(
            state,
            &flow_like_state,
            &profile,
            &bot.context,
            &selected,
            turn,
            &token,
            mutating,
        )
        .await
    };
    if let Some(memory_turn) = &memory_turn {
        bot.context.host_tools =
            memory_turn.host_tools(std::mem::take(&mut bot.context.host_tools));
    }
    let notes = NotesHandle::new(
        super::notes::ServerNotes {
            db: state.db.clone(),
            bot_id: turn.bot_id.clone(),
            conversation_id: chat_conversation.clone(),
            turn: Some((turn.id.clone(), host_id.to_string())),
        }
        .shared(),
        transcript.topic_at,
    );
    bot.context.host_tools = std::mem::take(&mut bot.context.host_tools).with_notes(notes.clone());
    let owner_direct = kind == ConversationKind::Direct
        && answered_round.is_none()
        && asker.is_none()
        && !ephemeral_user_context
        && requester.as_ref().is_some_and(|requester| {
            requester.audience == TurnAudience::Owner && requester.requester_id == owner
        });
    // An ask or a continuation runs on a reconstructed exchange, not the conversation's history.
    if answered_round.is_none() && asker.is_none() {
        bot.context.host_tools = std::mem::take(&mut bot.context.host_tools)
            .with_conversation_key(chat_conversation.clone());
    }
    let trail = owner_direct.then(|| ToolTrail::new(transcript.trail.clone()));
    if let Some(trail) = &trail {
        bot.context.host_tools =
            std::mem::take(&mut bot.context.host_tools).with_trail(trail.clone());
    }
    let reply_files = (!ephemeral_user_context).then(ReplyFiles::new);
    if let Some(files) = &reply_files {
        bot.context.host_tools =
            std::mem::take(&mut bot.context.host_tools).with_reply_files(files.clone());
    }
    let room_memory = kind == ConversationKind::Group;
    let owner_memory_allowed =
        super::memory::memory_policy(&state.db, &chat_conversation, owner, &turn.bot_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|policy| policy.use_owner_memory);
    let memory = match memory_model
        .as_deref()
        .filter(|_| owner_memory_allowed && (memory_turn.is_none() || room_memory))
    {
        Some(id) => {
            open_memory(
                &flow_like_state,
                &profile,
                &bot,
                owner,
                id,
                &token,
                &turn.id,
            )
            .await
        }
        None => None,
    };
    let memory_scope = memory.as_ref().map(|memory| memory.scope().clone());
    let memory_before = match &memory_scope {
        Some(scope) => memory_count(&flow_like_state, owner, scope).await,
        None => None,
    };
    let memory_state = flow_like_state.clone();
    let context = bot_platform_context(
        &bot.context,
        PlatformContextInput {
            user_context: None,
            active_profile: Some((profile.name.as_str(), profile.id.as_str())),
            switchable_profiles: &[],
            open_board: None,
            open_data_studio: None,
            attachments: &[],
        },
    );
    let context = match note {
        Some(note) => format!("{context}\n\n{note}"),
        None => context,
    };
    // A turn answering shared context keeps its own prompt and history: memory holds only
    // value-free notes of it. Other memory turns read live session snapshots from a section.
    let (context, prompt, history) = match &memory_turn {
        Some(memory_turn) if !ephemeral_user_context => {
            let shared =
                history::shared_context(&state.db, &turn.bot_id, &turn.conversation_id).await?;
            let sections = [
                Some(context),
                Some(memory_turn.context().to_string()),
                shared,
            ];
            (
                sections
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                memory_turn.prompt().to_string(),
                memory_turn.history(),
            )
        }
        Some(memory_turn) => (
            format!("{context}\n\n{}", memory_turn.context()),
            transcript.prompt,
            transcript.history,
        ),
        None => (context, transcript.prompt, transcript.history),
    };
    *follow_up
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = memory_turn;
    let bot_tools = crate::bot_tools::turn_host(
        state,
        owner,
        &turn.id,
        &bot.context,
        &profile,
        &flow_like_state,
        &selected,
        token.clone(),
        grants.clone(),
        conv_id,
        Some(&chat_conversation),
        Some(flow_like_bot_tools::TurnSite::Conversation(
            chat_conversation.clone(),
        )),
        Default::default(),
    )
    .await;
    if let Some(bot_tools) = &bot_tools
        && let Some(model) = decision_bit.as_deref()
    {
        bot_tools.set_inference(Arc::new(super::authorization::ServerInference::new(
            state, turn, model,
        )));
    }
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(HeadlessBridge {
        state: state.clone(),
        turn_id: turn.id.clone(),
        host_id: host_id.to_string(),
        mutating: mutating.clone(),
        bot_tools: bot_tools.clone(),
    });
    let bridge: Arc<dyn PlatformToolBridge> = match room_updates {
        Some(updates) => Arc::new(RoomUpdatesBridge::new(bridge, updates)),
        None => bridge,
    };
    let batch = bot_tools.as_ref().map_or_else(
        || Arc::new(ParkedBatch::new(&turn.id)),
        |bot_tools| bot_tools.batch().clone(),
    );
    let talk_host = Arc::new(super::talk::ServerTalkHost::for_turn(
        state.clone(),
        super::talk::HeldTurn {
            turn: turn.clone(),
            host_id: host_id.to_string(),
        },
        ancestors,
        batch.clone(),
    ));
    // A call hears the reply while it is written when the turn answers in its own conversation.
    let streams_reply = live_call && answered_round.is_none() && asker.is_none();
    let talk = match asker {
        Some(asked_by) => BotTalk::resumed(
            talk_host.clone(),
            super::talk::crew_member(&row, BotPresence::Online),
            asked_by,
            Default::default(),
        ),
        None => BotTalk::for_run(talk_host.clone(), &bot.context, BotLocation::Server),
    };
    let talk = Arc::new(talk.with_answered(answered_round.as_ref()));
    let run_taint = talk.clone();
    let acknowledgement = bot.context.acknowledgement;
    let sub_task_compute = bot.context.compute.clone();
    let reply_stream = streams_reply
        .then(|| ReplyStream::open(state, turn, bot.context.tool_gate.clone()))
        .flatten();
    let reply_sink = reply_stream.as_ref().map(ReplyStream::sink);
    let usage_stats = flow_like::flow::copilot::bot_usage::BotUsage::default();
    let usage_sink = usage_stats.clone();
    if let Some(recovery) = bot.context.recovery.take() {
        bot.context.recovery = Some(usage_stats.with_recovery(recovery.as_store()).into());
    }
    bot.context.spoken_drafts = reply_stream.is_some();
    let chat = run_bot_chat_with_talk(
        flow_like_state,
        Some(profile),
        Arc::new(bot.context),
        context,
        prompt,
        attached_images(state, &turn.conversation_id, &turn.trigger_message_id).await,
        history,
        Some(selected.id.clone()),
        Some(flow_like::flow::copilot::bot_thinking(live_call)),
        token,
        bridge,
        memory,
        Some(move |chunk: String| {
            usage_sink.observe(&chunk);
            if let Some(sink) = &reply_sink {
                sink.push(&chunk);
            }
        }),
        Some(talk),
    );
    let chat = tokio::time::timeout(Duration::from_millis(remaining as u64), chat);
    let outcome = match (decision_task, tool_gate_opener) {
        (Some(decision), Some(opener)) => {
            match race_decision(chat, decision, opener, trace).await {
                Raced::Allowed { outcome, .. } => outcome,
                Raced::Withheld(decision) => {
                    let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64
                        + sub_task_compute.extra_millis();
                    if let Err(error) = settle_assistant_usage(state, &usage, elapsed, false).await
                    {
                        tracing::error!(%error, operation_id = %usage.operation_id, "Bot turn usage settlement remains pending");
                    }
                    follow_up
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    return match apply_decision(state, turn, host_id, kind, decision).await? {
                        Verdict::Done(answer) => Ok(answer),
                        verdict => {
                            attempt_decided(
                                state,
                                turn,
                                host_id,
                                mutating,
                                follow_up,
                                unaddressed,
                                batched,
                                Some(verdict),
                                trace,
                            )
                            .await
                        }
                    };
                }
            }
        }
        _ => chat.await,
    };
    if let (Some(stream), Ok(Ok(_))) = (&reply_stream, &outcome) {
        stream.pause();
    }
    trace
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .model_ms = Some(started.elapsed().as_millis() as i64);
    if !matches!(&outcome, Ok(Ok(text)) if !text.trim().is_empty())
        && let Some(scope) = &memory_scope
        && memory_written(
            memory_before,
            memory_count(&memory_state, owner, scope).await,
        )
    {
        mutating.store(true, Ordering::SeqCst);
    }
    let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64
        + sub_task_compute.extra_millis();
    let answered = matches!(&outcome, Ok(Ok(_)));
    if let Err(error) = settle_assistant_usage(state, &usage, elapsed, answered).await {
        tracing::error!(%error, operation_id = %usage.operation_id, "Bot turn usage settlement remains pending");
    }
    if let Some(card) = batch.card() {
        use flow_like::flow::copilot::run_recovery::{BotRunRecovery, record_held_batch};
        if let Some(saved) = recovery.read().await?
            && let Some(updated) = record_held_batch(&saved, &batch.held_calls())
                .map_err(|error| Failure::model(&error))?
        {
            recovery
                .save(&updated)
                .await
                .map_err(|error| Failure::model(&error))?;
        }
        return Err(Failure::parked(card));
    }
    let failed_execution = if event_recovery.is_some() {
        recovery
            .read()
            .await?
            .as_ref()
            .is_some_and(decision_event::failed_execution)
    } else {
        false
    };
    if failed_execution {
        event_delivery = None;
    }
    let answer = match outcome {
        Err(_) => Err(Failure::timeout()),
        Ok(Err(error)) => Err(Failure::model(&error)),
        Ok(Ok(text))
            if requester
                .as_ref()
                .is_some_and(|r| r.answer_is_silent(&text))
                || (acknowledgement
                    && flow_like::flow::copilot::bot_run::is_silent_reply(&text)) =>
        {
            Ok(Answer {
                acknowledgement: false,
                text: String::new(),
                resumed,
                silent: true,
                completed: false,
                event_delivery: None,
                note: None,
                tool_calls: Vec::new(),
                files: Vec::new(),
                usage_stats: Vec::new(),
                taint: Default::default(),
            })
        }
        Ok(Ok(text))
            if text.trim().is_empty() && !event_delivery.is_some_and(EventDecision::quiet) =>
        {
            Err(Failure::new(
                CODE_MODEL,
                "The language model returned no answer. You can ask again, or have my owner retry this turn.",
                true,
                true,
            ))
        }
        Ok(Ok(text)) => {
            clear_tokens(grants.as_ref(), &seen).await;
            let pending_monitor = event_delivery.is_none()
                && !failed_execution
                && (delivery_state.is_some_and(|saved| saved.monitoring)
                    || event.as_ref().is_some_and(decision_event::monitoring));
            if pending_monitor {
                event_delivery = Some(
                    super::event_decision::limit_notify(&state.db, turn, EventDecision::Notify)
                        .await,
                );
            }
            if let Some(event) = &event
                && decision_event::monitoring(event)
                && pending_monitor
            {
                event_delivery = Some(
                    super::event_decision::limit_notify(&state.db, turn, EventDecision::Notify)
                        .await,
                );
                if let Some(bit_id) = decision_bit.as_deref() {
                    match super::event_decision::classify(
                        state,
                        turn,
                        bit_id,
                        event,
                        &event_context,
                        Some(&text),
                    )
                    .await
                    {
                        Ok(choice) => {
                            let choice =
                                super::event_decision::guard_ignore(&state.db, turn, event, choice)
                                    .await;
                            event_delivery = Some(
                                super::event_decision::limit_notify(
                                    &state.db,
                                    turn,
                                    choice.unwrap_or(EventDecision::Notify),
                                )
                                .await,
                            )
                        }
                        Err(error) => {
                            tracing::warn!(%error, turn_id = %turn.id, "Bot monitoring decision failed; delivering the result")
                        }
                    }
                }
            }
            if let Some(recovery) = &event_recovery
                && !failed_execution
                && delivery_state.is_some_and(|saved| saved.choice != event_delivery)
            {
                recovery
                    .record(event_delivery)
                    .await
                    .map_err(|error| Failure::model(&error))?;
            }
            Ok(Answer {
                acknowledgement,
                usage_stats: usage_stats.snapshot(),
                text,
                resumed,
                silent: event_delivery.is_some_and(EventDecision::quiet),
                completed: false,
                event_delivery,
                note: notes.touched(),
                tool_calls: trail.as_ref().map(ToolTrail::calls).unwrap_or_default(),
                files: reply_files
                    .as_ref()
                    .map(ReplyFiles::files)
                    .unwrap_or_default(),
                taint: run_taint.taint(),
            })
        }
    };
    if let Some(stream) = reply_stream {
        stream
            .finish(matches!(&answer, Ok(answer) if !answer.silent))
            .await;
    }
    answer.map_err(|mut failure| {
        failure.usage_stats = usage_stats.snapshot();
        failure
    })
}

/// The embedding model a remembering bot's memories were stored with, when the bot names none of
/// its own: its chats embed with the chat's model, and a turn no chat started must reuse that
/// model to recall them. `None` when nothing is stored yet or the profile can't run the model.
pub(crate) async fn stored_memory_model(
    flow_like_state: &Arc<flow_like::state::FlowLikeState>,
    profile: &Arc<Profile>,
    bot: &crate::routes::bots::runtime::ChatBot,
    owner: &str,
) -> Option<String> {
    if !bot.context.remembers() {
        return None;
    }
    let scope = bot.context.memory_scope(&profile.id);
    let id = AssistantMemory::status(flow_like_state.clone(), Some(owner), &scope)
        .await
        .ok()?
        .embedding_model_id?;
    let bit = profile
        .find_bit(&id, flow_like_state.http_client.clone())
        .await
        .map_err(|error| tracing::warn!(%error, %id, "Bot turn: stored memory model not found"))
        .ok()?;
    bit.try_to_embedding().map(|_| id)
}

async fn open_memory(
    flow_like_state: &Arc<flow_like::state::FlowLikeState>,
    profile: &Arc<Profile>,
    bot: &crate::routes::bots::runtime::ChatBot,
    owner: &str,
    embedding_id: &str,
    token: &Option<String>,
    run_id: &str,
) -> Option<Arc<AssistantMemory>> {
    let bit = profile
        .find_bit(embedding_id, flow_like_state.http_client.clone())
        .await
        .map_err(|error| tracing::warn!(%error, embedding_id, "Bot turn: memory model not found"))
        .ok()?;
    let scope: MemoryScope = bot.context.memory_scope(&profile.id);
    AssistantMemory::open(
        flow_like_state.clone(),
        Some(owner),
        &scope,
        &bit,
        token.clone(),
        Some(ModelUsageContext {
            app_id: None,
            run_id: Some(run_id.to_string()),
            api_base_url: None,
        }),
    )
    .await
    .map(|memory| {
        if bot
            .context
            .requester
            .as_ref()
            .is_some_and(|requester| requester.audience == TurnAudience::Room)
        {
            memory.read_only()
        } else {
            memory
        }
    })
    .map(Arc::new)
    .map_err(|error| tracing::warn!(%error, "Bot turn: memory init failed"))
    .ok()
}

/// Posts `request` as the turn's reply and completes the turn, in one transaction under the
/// conversation's lock; a repeat with the same message id returns the first copy. A failure
/// report also marks the turn failed in that transaction.
pub(crate) async fn commit_reply(
    db: &DatabaseConnection,
    dialect: DbDialect,
    turn: &bot_turn::Model,
    host_id: &str,
    request: SendEntryRequest,
    failure_code: Option<&'static str>,
) -> Result<Sent, ApiError> {
    let outgoing = Arc::new(validate_send(request)?);
    let claim = TurnClaim {
        id: turn.id.clone(),
        host_id: host_id.to_string(),
    };
    retry_transaction(db, dialect, None, &RetryPolicy::default(), |txn| {
        let (turn, claim, outgoing) = (turn.clone(), claim.clone(), outgoing.clone());
        Box::pin(async move {
            let now = now_millis();
            let sent = reply_as_bot(
                txn,
                &turn.conversation_id,
                &turn.bot_id,
                &claim,
                &outgoing,
                now,
            )
            .await?;
            complete_failure_in(txn, &turn, &outgoing.message_id, failure_code, now).await?;
            Ok::<_, ApiError>(sent)
        })
    })
    .await
}

/// A failure report ends the turn `FAILED` in the transaction that sent it, so the turn never
/// reads `DONE` for a message that says it failed.
async fn complete_failure_in<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    message_id: &str,
    failure_code: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    match failure_code {
        Some(code) => {
            ledger::mark_failed_after_reply_in(db, &turn.id, message_id, code, now).await?;
        }
        None => super::recovery::clear_finished_in(db, &turn.id).await?,
    }
    Ok(())
}

/// [`commit_reply`] without the conversation's lock, for tests on SQLite.
#[cfg(test)]
pub(crate) async fn commit_reply_in<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    request: SendEntryRequest,
    failure_code: Option<&'static str>,
) -> Result<Sent, ApiError> {
    let outgoing = validate_send(request)?;
    let claim = TurnClaim {
        id: turn.id.clone(),
        host_id: host_id.to_string(),
    };
    let now = now_millis();
    let sent = crate::routes::inbox::turns::reply_in(
        db,
        &turn.conversation_id,
        &turn.bot_id,
        &claim,
        &outgoing,
        now,
    )
    .await?;
    complete_failure_in(db, turn, &outgoing.message_id, failure_code, now).await?;
    Ok(sent)
}

/// [`commit_reply`], then the recipients are woken and the bots the reply wakes get their turns
/// started.
pub(crate) async fn post_reply(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    mut request: SendEntryRequest,
    failure_code: Option<&'static str>,
    resumed: bool,
) -> Result<(), ApiError> {
    if let Some(answered) = super::talk::answered_ask(&state.db, turn).await? {
        return post_reply_in_origin(state, turn, host_id, &answered.origin_conversation, request)
            .await;
    }
    crate::routes::inbox::reply_files::adopt(
        state,
        &turn.conversation_id,
        &turn.bot_id,
        &mut request,
    )
    .await;
    let sent = commit_reply(
        &state.db,
        state.db_dialect,
        turn,
        host_id,
        request,
        failure_code,
    )
    .await?;
    wake(state, sent.wake).await;
    super::dispatch(state, &sent.bots).await;
    crate::bot_notify::after_reply(
        state,
        &crate::bot_notify::Reply {
            turn_id: &turn.id,
            bot_id: &turn.bot_id,
            message_id: &sent.entry.message_id,
            text: &crate::bot_notify::text_of(Some(&sent.entry.envelope)),
            resumed,
        },
    )
    .await;
    Ok(())
}

/// The reply of a turn that woke on a late answer goes to the conversation the question was
/// asked from.
async fn post_reply_in_origin(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    origin_conversation: &str,
    mut request: SendEntryRequest,
) -> Result<(), ApiError> {
    crate::routes::inbox::reply_files::adopt(
        state,
        origin_conversation,
        &turn.bot_id,
        &mut request,
    )
    .await;
    let outgoing = validate_send(request)?;
    crate::routes::inbox::exchange::reply_in_origin(
        state,
        turn,
        host_id,
        origin_conversation,
        outgoing,
    )
    .await?;
    Ok(())
}

/// Ends the turn after an attempt: the answer is posted, a transient failure is queued again
/// with a backoff, any other failure is posted (or only recorded).
pub(crate) async fn finish(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    result: Result<Answer, Failure>,
    mutating_started: bool,
) {
    let now = now_millis();
    let result = match result {
        Err(Failure {
            park: Some(card), ..
        }) => match approvals::park_held(state, turn, host_id, &card).await {
            Ok(()) => return,
            Err(error) => {
                tracing::warn!(%error, turn_id = %turn.id, "Bot turn: could not park for an approval");
                Err(Failure::new(
                    CODE_SETUP,
                    format!(
                        "I needed your approval to run {} but could not ask you: {}",
                        card.tool_names().join(", "),
                        error.public_message().unwrap_or("unexpected error")
                    ),
                    false,
                    true,
                ))
            }
        },
        other => other,
    };
    match result {
        Ok(answer) => {
            if answer.completed {
                return;
            }
            let Some(request) = answer_request(turn, &answer) else {
                match ledger::complete_silent_in(&state.db, &turn.id, host_id, now).await {
                    Ok(true) => wake(state, [turn.owner_id.clone()]).await,
                    Ok(false) => {
                        tracing::warn!(turn_id = %turn.id, "Bot turn: the lease was lost before the silent outcome was recorded")
                    }
                    Err(error) => {
                        tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the silent outcome was not recorded");
                        record_failure(state, turn, host_id, CODE_FAILED, now).await;
                    }
                }
                return;
            };
            let posted = post_reply(state, turn, host_id, request, None, answer.resumed).await;
            if let Err(error) = posted {
                tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the reply was refused");
                record_failure(state, turn, host_id, CODE_REPLY_REFUSED, now).await;
            }
        }
        Err(failure) if retries(&failure, turn.attempts, mutating_started) => {
            let retry_at = now + ChronoDuration::seconds(backoff_secs(turn.attempts));
            let code = failure.code;
            match state
                .transaction(|txn| {
                    let (turn_id, host_id) = (turn.id.clone(), host_id.to_owned());
                    Box::pin(async move {
                        ledger::release_for_retry_in(txn, &turn_id, &host_id, retry_at, code, now)
                            .await
                            .map_err(ApiError::from)
                    })
                })
                .await
            {
                Ok(ledger::RetryRelease::Requeued) => {
                    kick_later(retry_at);
                    wake(state, [turn.owner_id.clone()]).await;
                }
                Ok(ledger::RetryRelease::Spent) => {
                    report_failure(state, turn, host_id, &failure, now).await
                }
                Ok(ledger::RetryRelease::Lost) => {}
                Err(error) => {
                    tracing::warn!(%error, turn_id = %turn.id, "Bot turn: retry not queued")
                }
            }
        }
        Err(failure) => report_failure(state, turn, host_id, &failure, now).await,
    }
}

async fn report_failure(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    failure: &Failure,
    now: DateTime<FixedOffset>,
) {
    if !failure.post {
        record_failure(state, turn, host_id, failure.code, now).await;
        return;
    }
    let checkpoint = super::recovery::ServerRecovery::new(state, turn, host_id)
        .read()
        .await
        .ok()
        .flatten();
    let request = failure_request(turn, failure, checkpoint.as_ref());
    if let Err(error) = post_reply(state, turn, host_id, request, Some(failure.code), false).await {
        tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the failure report was refused");
        record_failure(state, turn, host_id, failure.code, now).await;
    }
}

pub(crate) fn failure_request(
    turn: &bot_turn::Model,
    failure: &Failure,
    checkpoint: Option<&flow_like_types::bot_turn::BotRunCheckpoint>,
) -> SendEntryRequest {
    let usage = flow_like::flow::copilot::bot_usage::BotUsage::default();
    if let Some(checkpoint) = checkpoint {
        usage.restore(checkpoint);
    }
    for record in &failure.usage_stats {
        if let Some(name) = record["step_name"].as_str()
            && let Ok(stats) = serde_json::from_value(record["stats"].clone())
        {
            usage.record(name, stats);
        }
    }
    let mut request = reply_request(
        turn,
        failure_message_id(&turn.bot_id, &turn.trigger_message_id),
        &failure.text,
        Some(failure.code),
    );
    if let Some(envelope) = request.envelope.as_mut() {
        usage.attach(&mut envelope["body"]);
    }
    request
}

async fn record_failure(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    code: &str,
    now: DateTime<FixedOffset>,
) {
    match ledger::fail_held_in(&state.db, &turn.id, host_id, code, now).await {
        Ok(true) => wake(state, [turn.owner_id.clone()]).await,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, turn_id = %turn.id, "Bot turn: failure not recorded");
        }
    }
}

/// Wakes the worker when a backed-off turn becomes due.
pub(crate) fn kick_later(at: DateTime<FixedOffset>) {
    let wait = (at - now_millis()).to_std().unwrap_or_default();
    tokio::spawn(async move {
        tokio::time::sleep(wait + Duration::from_millis(250)).await;
        kick();
    });
}

fn spawn_heartbeat(
    state: AppState,
    turn_id: String,
    host_id: String,
    lost: Arc<Notify>,
) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
        let started = Instant::now();
        let budget = Duration::from_millis(MAX_TURN_RUNTIME_MS as u64) + HEARTBEAT_GRACE;
        let mut tick = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
        tick.tick().await;
        loop {
            tick.tick().await;
            if started.elapsed() > budget {
                tracing::warn!(turn_id = %turn_id, "Bot turn: heartbeat stopped past the maximum runtime");
                return;
            }
            match ledger::heartbeat_in(&state.db, &turn_id, None, &host_id, now_millis()).await {
                Ok(_) => {}
                Err(error)
                    if error.public_code() == ledger::BOT_TURN_LEASE_LOST
                        || error.status() == StatusCode::NOT_FOUND =>
                {
                    lost.notify_one();
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, turn_id = %turn_id, "Bot turn: heartbeat failed")
                }
            }
        }
    }))
}

const HEARTBEAT_GRACE: Duration = Duration::from_secs(120);

/// Aborts the task when dropped, so a dropped or panicking turn stops renewing its lease.
pub(super) struct AbortOnDrop(pub(super) JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run_while_notifying<T>(
    work: impl Future<Output = T>,
    notification: impl Future<Output = ()>,
) -> T {
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result,
        () = notification => work.await,
    }
}

/// Runs a claimed turn to its end, keeping its lease alive while it runs.
async fn run_claimed(state: &AppState, host_id: &str, claimed: Claimed) {
    let (turn, batched) = (claimed.turn, claimed.batched);
    let lost = Arc::new(Notify::new());
    let heartbeat = spawn_heartbeat(
        state.clone(),
        turn.id.clone(),
        host_id.to_string(),
        lost.clone(),
    );
    let mutating = Arc::new(AtomicBool::new(false));
    let follow_up: FollowUp = Arc::default();
    let unaddressed = AtomicBool::new(false);
    let result = tokio::select! {
        // Status delivery must not delay work when a push provider is unavailable.
        result = run_while_notifying(
            attempt(state, &turn, host_id, &mutating, &follow_up, &unaddressed, &batched),
            wake(state, [turn.owner_id.clone()]),
        ) => Some(result),
        () = lost.notified() => None,
    };
    let result = match result {
        Some(Err(failure)) if failure.park.is_none() && unaddressed.load(Ordering::SeqCst) => {
            Some(Err(failure.unrequested()))
        }
        other => other,
    };
    drop(heartbeat);
    match result {
        Some(result) => {
            let answered = matches!(&result, Ok(answer) if !answer.silent);
            finish(
                state,
                &turn,
                host_id,
                result,
                mutating.load(Ordering::SeqCst),
            )
            .await;
            let memory = follow_up
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let (true, Some(memory)) = (answered, memory) {
                memory.finish();
            }
        }
        None => {
            tracing::warn!(turn_id = %turn.id, "Bot turn: the lease was lost; another host owns it")
        }
    }
}

/// Claims `turn_id`, runs it and then every turn that queued behind it for the same bot and
/// conversation. Returns how many turns it ran.
pub(crate) async fn drive(state: &AppState, host_id: &str, turn_id: &str) -> u32 {
    drive_within(state, host_id, turn_id, None).await
}

/// Lambda's hard maximum; the hosted worker's configured timeout can only be shorter.
const HOSTED_INVOCATION_SECS: u64 = 900;
/// Remaining invocation time below which a queued turn is handed to a fresh invocation.
const HOSTED_CHAIN_RESERVE: Duration = Duration::from_secs(300);

fn hosted_invocation_budget() -> Duration {
    let secs = std::env::var("FLOWLIKE_HOSTED_WORKER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(HOSTED_INVOCATION_SECS);
    Duration::from_secs(secs)
}

fn chain_has_room(elapsed: Duration, budget: Duration) -> bool {
    budget.saturating_sub(elapsed) > HOSTED_CHAIN_RESERVE
}

/// [`drive`] that stops chaining once `invocation` (start, budget) can't cover another turn and
/// enqueues the next one for a fresh hosted invocation instead.
async fn drive_within(
    state: &AppState,
    host_id: &str,
    turn_id: &str,
    invocation: Option<(Instant, Duration)>,
) -> u32 {
    let mut next = turn_id.to_string();
    let mut ran = 0;
    loop {
        let request = ClaimRequest {
            turn_id: next,
            owner_id: None,
            host_kind: TurnHostKind::Server,
            host_id: host_id.to_string(),
            now: now_millis(),
        };
        let claimed = match ledger::claim(state, request).await {
            Ok(Ok(claimed)) => claimed,
            Ok(Err(refusal)) => {
                tracing::debug!(?refusal, "Bot turn not claimed");
                return ran;
            }
            Err(error) => {
                tracing::warn!(%error, "Bot turn claim failed");
                return ran;
            }
        };
        let (bot_id, conversation_id) = (
            claimed.turn.bot_id.clone(),
            claimed.turn.conversation_id.clone(),
        );
        run_claimed(state, host_id, claimed).await;
        ran += 1;
        match ledger::next_head(
            &state.db,
            &bot_id,
            &conversation_id,
            TurnHostKind::Server,
            now_millis(),
        )
        .await
        {
            Ok(Some(head)) => {
                if let Some((started, budget)) = invocation
                    && !chain_has_room(started.elapsed(), budget)
                {
                    if let Err(error) = hosted_worker::enqueue_bot_turn(state, &head.id).await {
                        tracing::warn!(%error, turn_id = %head.id, "Bot turn: handoff to a fresh invocation failed; recovery redispatches it");
                    }
                    return ran;
                }
                next = head.id
            }
            _ => return ran,
        }
    }
}

/// The hosted worker's entry: one invocation drives one turn and what queued behind it, within
/// the invocation's time.
pub(crate) async fn run_hosted(state: &AppState, turn_id: &str) {
    let invocation = (Instant::now(), hosted_invocation_budget());
    drive_within(state, &host_id(), turn_id, Some(invocation)).await;
}

/// One pass: starts a task for each turn waiting for the server while there are free slots.
pub(crate) async fn run_pass(
    state: &AppState,
    host_id: &str,
    slots: &Arc<Semaphore>,
) -> Result<usize, ApiError> {
    let free = slots.available_permits();
    if free == 0 {
        return Ok(0);
    }
    let heads = ledger::queued_heads(
        &state.db,
        None,
        TurnHostKind::Server,
        now_millis(),
        free as u64,
    )
    .await?;
    let mut started = 0;
    for (head, _) in heads {
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            break;
        };
        let (state, host_id) = (state.clone(), host_id.to_string());
        tokio::spawn(async move {
            drive(&state, &host_id, &head.id).await;
            drop(permit);
        });
        started += 1;
    }
    Ok(started)
}

/// Spawns the resident worker: it answers server turns as they arrive (woken by [`kick`], with
/// a ticker as the backstop) and recovers abandoned ones. `None` when
/// `BOT_TURN_WORKER_DISABLED=1`, or on AWS, where the hosted worker function drives turns.
pub fn spawn_worker(state: AppState, config: WorkerConfig) -> Option<JoinHandle<()>> {
    let disabled = std::env::var("BOT_TURN_WORKER_DISABLED")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if disabled || hosted_worker::enabled() {
        tracing::info!("Bot turn worker not spawned here");
        return None;
    }
    tracing::info!(
        interval_secs = config.interval.as_secs(),
        concurrency = config.concurrency,
        "Spawning bot turn worker"
    );
    Some(tokio::spawn(async move {
        let host = host_id();
        let slots = Arc::new(Semaphore::new(config.concurrency.max(1)));
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recovered_at: Option<Instant> = None;
        let mut fired_at: Option<Instant> = None;
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                () = super::KICK.notified() => {}
            }
            if recovered_at.is_none_or(|at| at.elapsed() >= RECOVER_EVERY) {
                recovered_at = Some(Instant::now());
                fired_at = recovered_at;
                if let Err(error) = recover(&state).await {
                    tracing::warn!(%error, "Bot turn recovery failed");
                }
            } else if fired_at.is_none_or(|at| at.elapsed() >= FIRE_TASKS_EVERY) {
                fired_at = Some(Instant::now());
                if let Err(error) = super::tasks::fire_due(&state).await {
                    tracing::warn!(%error, "Bot task sweep incomplete");
                }
            }
            if let Err(error) = run_pass(&state, &host, &slots).await {
                tracing::warn!(%error, "Bot turn pass failed");
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(retryable: bool) -> Failure {
        Failure::new(CODE_FAILED, "x", retryable, true)
    }

    #[test]
    fn a_transient_failure_retries_until_its_attempts_are_used() {
        assert!(retries(&failure(true), 1, false));
        assert!(retries(&failure(true), 2, false));
        assert!(retries(&failure(true), 3, false));
        assert!(!retries(&failure(false), 1, false));
    }

    #[test]
    fn no_retry_once_a_tool_with_side_effects_was_requested() {
        assert!(!retries(&failure(true), 1, true));
    }

    #[test]
    fn a_hosted_invocation_chains_only_while_its_time_can_cover_another_turn() {
        let budget = Duration::from_secs(HOSTED_INVOCATION_SECS);
        assert!(chain_has_room(Duration::ZERO, budget));
        assert!(!chain_has_room(budget - HOSTED_CHAIN_RESERVE, budget));
        assert!(!chain_has_room(budget, budget));
    }

    #[test]
    fn backoff_grows_then_holds() {
        assert_eq!([1, 2, 3, 9].map(backoff_secs), [10, 30, 30, 30]);
    }

    #[test]
    fn funding_follows_where_the_model_and_memory_run() {
        assert_eq!(funding_class(true, None), "hosted");
        assert_eq!(funding_class(true, Some(true)), "hosted");
        assert_eq!(funding_class(true, Some(false)), "mixed");
        assert_eq!(funding_class(false, Some(true)), "mixed");
        assert_eq!(funding_class(false, None), "byok");
    }

    #[test]
    fn setup_errors_say_why_and_quota_errors_do_not_retry() {
        let setup = Failure::from_api(&ApiError::bad_request("This profile has no language model"));
        assert_eq!(
            (setup.code, setup.retryable, setup.post),
            (CODE_SETUP, false, true)
        );
        assert!(setup.text.contains("no language model"));
        let quota = Failure::from_api(&ApiError::payment_required("Plan used up".to_string()));
        assert_eq!((quota.code, quota.retryable), (CODE_QUOTA, false));
        let internal = Failure::from_api(&ApiError::internal("db down"));
        assert_eq!((internal.code, internal.retryable), (CODE_FAILED, true));
    }

    #[test]
    fn a_profile_without_a_usable_model_asks_for_one_whatever_the_plan() {
        use crate::routes::ai::global_chat::ProfileModelAccess;
        let device_only = ProfileModelAccess {
            profile_models: 2,
            ..Default::default()
        };
        let above_plan = ProfileModelAccess {
            profile_models: 1,
            above_plan: 1,
            ..Default::default()
        };
        for access in [ProfileModelAccess::default(), device_only, above_plan] {
            let rejection = access.rejection(None).expect("nothing here can answer");
            let failure = Failure::from_api(&rejection);
            assert_eq!(
                (failure.code, failure.retryable, failure.post),
                (CODE_NO_MODEL, false, true),
                "{access:?}"
            );
        }
    }

    #[test]
    fn a_provider_refusal_of_auth_or_billing_is_a_setup_failure_posted_at_once() {
        for message in [
            "Invalid status code 403 Forbidden with message: The app has no billing owner",
            "CompletionError: HttpError: Invalid status code: 401 Unauthorized",
        ] {
            let failure = Failure::model(&flow_like_types::anyhow!("{message}"));
            assert_eq!(
                (failure.code, failure.retryable, failure.post),
                (CODE_SETUP, false, true),
                "{message}"
            );
        }
        let billing = Failure::model(&flow_like_types::anyhow!(
            "ProviderError: Invalid status code 403 Forbidden with message: The app has no billing owner"
        ));
        assert!(billing.text.contains("403") && billing.text.contains("no billing owner"));

        for message in [
            "Invalid status code 429 Too Many Requests",
            "Invalid status code 502 Bad Gateway",
            "connection reset",
            "tool 4031 failed",
        ] {
            let failure = Failure::model(&flow_like_types::anyhow!("{message}"));
            assert_eq!(
                (failure.code, failure.retryable),
                (CODE_MODEL, true),
                "{message}"
            );
        }
    }

    #[test]
    fn only_transcript_turns_are_live_calls_and_only_direct_or_addressed_ones_race() {
        assert!(live_call_turn(Some(InboxEntryKind::Transcript)));
        assert!(!live_call_turn(Some(InboxEntryKind::Message)));
        assert!(!live_call_turn(None));
        let race = |live, kind, addressed, model| decides_in_parallel(live, kind, addressed, model);
        assert!(race(true, ConversationKind::Direct, false, true));
        assert!(race(true, ConversationKind::Group, true, true));
        assert!(!race(true, ConversationKind::Group, false, true));
        assert!(!race(true, ConversationKind::Direct, true, false));
        assert!(!race(false, ConversationKind::Direct, true, true));
    }

    async fn raced(
        decision: DecisionResult,
        reply_first: bool,
    ) -> (Raced<Option<&'static str>>, bool) {
        use flow_like::flow::copilot::tool_gate::tool_gate;
        let (opener, gate) = tool_gate();
        let tool_ran = Arc::new(AtomicBool::new(false));
        let ran = tool_ran.clone();
        let chat = async move {
            if reply_first {
                return Some("reply");
            }
            if gate.passes().await {
                ran.store(true, Ordering::SeqCst);
                Some("reply")
            } else {
                None
            }
        };
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            (decision, 30)
        });
        let trace = std::sync::Mutex::new(CallTrace::default());
        let result = race_decision(chat, task, opener, &trace).await;
        (result, tool_ran.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn a_no_answer_decision_drops_the_run_before_any_tool_runs() {
        use flow_like::flow::copilot::decision::Decision;
        let (result, tool_ran) = raced(Some(Ok(Decision::NoAnswer)), false).await;
        assert!(matches!(
            result,
            Raced::Withheld(Some(Ok(Decision::NoAnswer)))
        ));
        assert!(!tool_ran);
    }

    #[tokio::test]
    async fn a_finished_reply_is_withheld_until_the_decision_allows_it() {
        use flow_like::flow::copilot::decision::Decision;
        let (result, _) = raced(Some(Ok(Decision::Acknowledge)), true).await;
        assert!(matches!(result, Raced::Withheld(_)));
        let (result, _) = raced(Some(Ok(Decision::AnswerNow)), true).await;
        assert!(matches!(
            result,
            Raced::Allowed {
                outcome: Some("reply"),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn an_answer_now_decision_error_or_timeout_lets_the_tools_and_reply_through() {
        use flow_like::flow::copilot::decision::Decision;
        for decision in [
            Some(Ok(Decision::AnswerNow)),
            Some(Err(ApiError::bad_request("x"))),
            None,
        ] {
            let (result, tool_ran) = raced(decision, false).await;
            assert!(matches!(
                result,
                Raced::Allowed {
                    outcome: Some("reply"),
                    ..
                }
            ));
            assert!(tool_ran);
        }
    }

    #[test]
    fn long_answers_are_cut_to_one_envelope() {
        let long = "a".repeat(MAX_REPLY_CHARS + 10);
        let cut = clipped(&long);
        assert!(cut.chars().count() < MAX_REPLY_CHARS + 60);
        assert!(cut.ends_with("_(cut: too long to send)_"));
        assert_eq!(clipped("short"), "short");
    }
}

async fn attached_images(
    state: &AppState,
    conversation_id: &str,
    message_id: &str,
) -> Option<Vec<flow_like::flow::copilot::ChatImage>> {
    let images =
        crate::routes::inbox::files::message_images(state, conversation_id, message_id).await;
    (!images.is_empty()).then_some(images)
}

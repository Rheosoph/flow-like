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
                ensure_metered_or_customer_provider, load_user_profile_access,
                reserve_assistant_usage_with_max_runtime, settle_assistant_usage,
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
    copilot::flowpilot_thinking,
    flow::copilot::{
        BotToolHost, HeadlessMemory, PlatformContextInput, bot_platform_context,
        bot_talk::{BotPresence, BotTalk},
        find_bot_tool_spec,
        memory::{AssistantMemory, MemoryScope},
        platform::PlatformToolBridge,
        run_bot_chat_with_talk,
        tool_grants::{GrantStore, clear_tokens, resume_note},
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
    bot_turn::{
        HEARTBEAT_SECS, MAX_ATTEMPTS, TurnAudience, TurnHostKind, failure_message_id,
        reply_message_id,
    },
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind, plain_envelope},
    tokio::{
        self,
        sync::{Notify, Semaphore},
        task::JoinHandle,
    },
};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The longest a server turn runs, whatever the plan allows.
const MAX_TURN_RUNTIME_MS: i64 = 600_000;
/// A reply fits one envelope: markdown beyond this is cut.
const MAX_REPLY_CHARS: usize = 48_000;
const DEFAULT_INTERVAL_SECS: u64 = 5;
const DEFAULT_CONCURRENCY: usize = 4;
const RECOVER_EVERY: Duration = Duration::from_secs(60);
const RETRY_BACKOFF_SECS: [i64; 2] = [10, 30];

pub const CODE_TIMEOUT: &str = "BOT_TURN_TIMEOUT";
pub const CODE_QUOTA: &str = "BOT_TURN_QUOTA";
pub const CODE_SETUP: &str = "BOT_TURN_SETUP";
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
}

impl Failure {
    fn new(code: &'static str, text: impl Into<String>, retryable: bool, post: bool) -> Self {
        Self {
            code,
            text: text.into(),
            retryable,
            post,
            park: None,
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

    fn from_api(error: &ApiError) -> Self {
        let reason = error.public_message().unwrap_or("unexpected error");
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

    fn model(error: &flow_like_types::Error) -> Self {
        tracing::warn!(%error, "Bot turn: the model run failed");
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

/// Whether a failed attempt runs again: a transient failure, attempts left and no tool with side
/// effects requested.
pub(crate) fn retries(failure: &Failure, attempts: i32, mutating_started: bool) -> bool {
    failure.retryable
        && !mutating_started
        && u32::try_from(attempts).unwrap_or(u32::MAX) < MAX_ATTEMPTS
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

    async fn is_cancelled(&self) -> bool {
        if self
            .bot_tools
            .as_ref()
            .is_some_and(|bot_tools| bot_tools.parked().is_some())
        {
            return true;
        }
        !ledger::holds_lease(&self.state.db, &self.turn_id, &self.host_id, now_millis())
            .await
            .unwrap_or(true)
    }
}

async fn turn_requester(
    db: &DatabaseConnection,
    turn: &bot_turn::Model,
    room: Option<&str>,
    kind: ConversationKind,
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
    })
}

fn host_id() -> String {
    format!("server:{}", uuid::Uuid::new_v4())
}

pub(crate) fn room_of(kind: ConversationKind, conversation_id: &str) -> Option<&str> {
    matches!(kind, ConversationKind::Group).then_some(conversation_id)
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

async fn load_profile(
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
fn funding_class(model_hosted: bool, memory_hosted: Option<bool>) -> &'static str {
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

/// The turn's memory, kept until its reply is posted: the follow-up work needs the reply.
type FollowUp = Arc<std::sync::Mutex<Option<HeadlessMemory>>>;

/// One attempt at a turn: everything up to the model's answer.
/// A finished turn's answer; `resumed` when the turn slept for an owner's approval.
struct Answer {
    text: String,
    resumed: bool,
}

async fn attempt(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    mutating: &Arc<AtomicBool>,
    follow_up: &FollowUp,
) -> Result<Answer, Failure> {
    let owner = turn.owner_id.as_str();
    let row = bot::Entity::find_by_id(&turn.bot_id)
        .one(&state.db)
        .await?
        .filter(|row| row.deleted_at.is_none())
        .ok_or_else(|| Failure::silent(CODE_BOT_GONE, "The bot was deleted"))?;
    let conversation = conversation::Entity::find_by_id(&turn.conversation_id)
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
    let kind = ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
    let mut transcript = history::transcript(
        &state.db,
        &turn.bot_id,
        &turn.conversation_id,
        &turn.trigger_message_id,
        kind,
        owner,
    )
    .await?;
    if let Some(answered) = super::talk::answered_ask(&state.db, turn).await? {
        transcript = super::talk::ask_result_transcript(&state.db, turn, &answered).await?;
    }
    let asked_by = super::talk::asked_by(&state.db, turn).await?;
    let ancestors: Vec<String> = asked_by.iter().map(|member| member.id.clone()).collect();
    let asker = (!asked_by.is_empty()).then_some(asked_by);

    let (profile, access) = load_profile(state, owner, row.profile_id.as_deref()).await?;
    let mut bot = chat_bot(state, owner, Some(turn.bot_id.as_str()), &profile)
        .await?
        .ok_or_else(|| Failure::silent(CODE_BOT_GONE, "The bot was deleted"))?;
    bot.context.tool_host = BotToolHost::CoreOnly;
    bot.context.requester =
        Some(turn_requester(&state.db, turn, conversation.title.as_deref(), kind).await?);
    let flow_like_state = master_flow_like_state(state).await?;
    let model_id = server_bot_model(state, owner, &flow_like_state, &profile, &bot.model).await;
    if let Some(rejection) = access.rejection(model_id.as_deref()) {
        return Err(rejection.into());
    }
    let selected = profile
        .resolve_completion_model(
            model_id.as_deref(),
            &flow_like::bit::BitModelPreference {
                reasoning_weight: Some(1.0),
                ..Default::default()
            },
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
    let memory_model = bot.context.memory_model(None).map(str::to_string);
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
    let usage = reserve_assistant_usage_with_max_runtime(
        state,
        owner,
        &selected.id,
        &provider.provider_name,
        funding,
        None,
        MAX_TURN_RUNTIME_MS,
        hosted_worker::enabled(),
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
    let remaining = (usage.deadline - chrono::Utc::now())
        .num_milliseconds()
        .max(1);
    let conv_id = room_of(kind, &turn.conversation_id);
    let token = executor_token(
        owner,
        &turn.bot_id,
        conv_id,
        &turn.id,
        bot.context.workspace_app_id.as_deref(),
        remaining / 1_000 + 120,
    );
    let memory_turn = super::memory::open_turn(
        state,
        &flow_like_state,
        &profile,
        &bot.context,
        &selected,
        turn,
        &token,
        mutating,
    )
    .await;
    if let Some(memory_turn) = &memory_turn {
        bot.context.host_tools =
            memory_turn.host_tools(std::mem::take(&mut bot.context.host_tools));
    }
    let memory = match memory_model.as_deref().filter(|_| memory_turn.is_none()) {
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
    let grants = crate::routes::bots::tool_grants::rows_for_owner(state, &turn.bot_id, owner);
    let seen = grants.list().await.unwrap_or_default();
    let note = resume_note(grants.as_ref()).await;
    let resumed = note.is_some();
    let context = match note {
        Some(note) => format!("{context}\n\n{note}"),
        None => context,
    };
    let (context, prompt, history) = match &memory_turn {
        Some(memory_turn) => (
            format!("{context}\n\n{}", memory_turn.context()),
            memory_turn.prompt().to_string(),
            memory_turn.history(),
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
    )
    .await;
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(HeadlessBridge {
        state: state.clone(),
        turn_id: turn.id.clone(),
        host_id: host_id.to_string(),
        mutating: mutating.clone(),
        bot_tools: bot_tools.clone(),
    });
    let talk_host = Arc::new(super::talk::ServerTalkHost::for_turn(
        state.clone(),
        super::talk::HeldTurn {
            turn: turn.clone(),
            host_id: host_id.to_string(),
        },
        ancestors,
    ));
    let talk = Arc::new(match asker {
        Some(asked_by) => BotTalk::resumed(
            talk_host.clone(),
            super::talk::crew_member(&row, BotPresence::Online),
            asked_by,
            Default::default(),
        ),
        None => BotTalk::for_run(talk_host.clone(), &bot.context, BotLocation::Server),
    });
    let chat = run_bot_chat_with_talk(
        flow_like_state,
        Some(profile),
        Arc::new(bot.context),
        context,
        prompt,
        None,
        history,
        Some(selected.id.clone()),
        Some(flowpilot_thinking(None)),
        token,
        bridge,
        memory,
        None::<fn(String)>,
        Some(talk),
    );
    let outcome = tokio::time::timeout(Duration::from_millis(remaining as u64), chat).await;
    if !matches!(&outcome, Ok(Ok(text)) if !text.trim().is_empty())
        && let Some(scope) = &memory_scope
        && memory_written(
            memory_before,
            memory_count(&memory_state, owner, scope).await,
        )
    {
        mutating.store(true, Ordering::SeqCst);
    }
    let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let answered = matches!(&outcome, Ok(Ok(_)));
    if let Err(error) = settle_assistant_usage(state, &usage, elapsed, answered).await {
        tracing::error!(%error, operation_id = %usage.operation_id, "Bot turn usage settlement remains pending");
    }
    if let Some(card) = bot_tools
        .as_ref()
        .and_then(|bot_tools| bot_tools.parked())
        .or_else(|| talk_host.parked_card())
    {
        return Err(Failure::parked(card));
    }
    match outcome {
        Err(_) => Err(Failure::timeout()),
        Ok(Err(error)) => Err(Failure::model(&error)),
        Ok(Ok(text)) if text.trim().is_empty() => Err(Failure::new(
            CODE_MODEL,
            "The language model returned no answer. You can ask again, or have my owner retry this turn.",
            true,
            true,
        )),
        Ok(Ok(text)) => {
            clear_tokens(grants.as_ref(), &seen).await;
            Ok(Answer { text, resumed })
        }
    }
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
    if let Some(code) = failure_code {
        ledger::mark_failed_after_reply_in(db, &turn.id, message_id, code, now).await?;
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
async fn post_reply(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    request: SendEntryRequest,
    failure_code: Option<&'static str>,
    resumed: bool,
) -> Result<(), ApiError> {
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
    request: SendEntryRequest,
) -> Result<(), ApiError> {
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
async fn finish(
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
                        card.tool,
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
        Ok(Answer { text, resumed }) => {
            let request = reply_request(
                turn,
                reply_message_id(&turn.bot_id, &turn.trigger_message_id),
                &clipped(&text),
                None,
            );
            let posted = match super::talk::answered_ask(&state.db, turn).await {
                Ok(Some(answered)) => {
                    post_reply_in_origin(
                        state,
                        turn,
                        host_id,
                        &answered.origin_conversation,
                        request,
                    )
                    .await
                }
                _ => post_reply(state, turn, host_id, request, None, resumed).await,
            };
            if let Err(error) = posted {
                tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the reply was refused");
                record_failure(state, turn, host_id, CODE_REPLY_REFUSED, now).await;
            }
        }
        Err(failure) if retries(&failure, turn.attempts, mutating_started) => {
            let retry_at = now + ChronoDuration::seconds(backoff_secs(turn.attempts));
            match ledger::release_for_retry_in(
                &state.db,
                &turn.id,
                host_id,
                retry_at,
                failure.code,
                now,
            )
            .await
            {
                Ok(true) => kick_later(retry_at),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%error, turn_id = %turn.id, "Bot turn: retry not queued")
                }
            }
        }
        Err(failure) if failure.post => {
            let request = reply_request(
                turn,
                failure_message_id(&turn.bot_id, &turn.trigger_message_id),
                &failure.text,
                Some(failure.code),
            );
            if let Err(error) =
                post_reply(state, turn, host_id, request, Some(failure.code), false).await
            {
                tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the failure report was refused");
                record_failure(state, turn, host_id, failure.code, now).await;
            }
        }
        Err(failure) => record_failure(state, turn, host_id, failure.code, now).await,
    }
}

async fn record_failure(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    code: &str,
    now: DateTime<FixedOffset>,
) {
    if let Err(error) = ledger::fail_held_in(&state.db, &turn.id, host_id, code, now).await {
        tracing::warn!(%error, turn_id = %turn.id, "Bot turn: failure not recorded");
    }
}

/// Wakes the worker when a backed-off turn becomes due.
fn kick_later(at: DateTime<FixedOffset>) {
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
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
        tick.tick().await;
        loop {
            tick.tick().await;
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
    })
}

/// Runs a claimed turn to its end, keeping its lease alive while it runs.
async fn run_claimed(state: &AppState, host_id: &str, claimed: Claimed) {
    let turn = claimed.turn;
    let lost = Arc::new(Notify::new());
    let heartbeat = spawn_heartbeat(
        state.clone(),
        turn.id.clone(),
        host_id.to_string(),
        lost.clone(),
    );
    let mutating = Arc::new(AtomicBool::new(false));
    let follow_up: FollowUp = Arc::default();
    let result = tokio::select! {
        result = attempt(state, &turn, host_id, &mutating, &follow_up) => Some(result),
        () = lost.notified() => None,
    };
    heartbeat.abort();
    match result {
        Some(result) => {
            let answered = result.is_ok();
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
            Ok(Some(head)) => next = head.id,
            _ => return ran,
        }
    }
}

/// The hosted worker's entry: one invocation drives one turn and what queued behind it.
pub(crate) async fn run_hosted(state: &AppState, turn_id: &str) {
    drive(state, &host_id(), turn_id).await;
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
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                () = super::KICK.notified() => {}
            }
            if recovered_at.is_none_or(|at| at.elapsed() >= RECOVER_EVERY) {
                recovered_at = Some(Instant::now());
                if let Err(error) = recover(&state).await {
                    tracing::warn!(%error, "Bot turn recovery failed");
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
        assert!(!retries(&failure(true), 3, false));
        assert!(!retries(&failure(false), 1, false));
    }

    #[test]
    fn no_retry_once_a_tool_with_side_effects_was_requested() {
        assert!(!retries(&failure(true), 1, true));
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
    fn long_answers_are_cut_to_one_envelope() {
        let long = "a".repeat(MAX_REPLY_CHARS + 10);
        let cut = clipped(&long);
        assert!(cut.chars().count() < MAX_REPLY_CHARS + 60);
        assert!(cut.ends_with("_(cut: too long to send)_"));
        assert_eq!(clipped("short"), "short");
    }
}
